//! Canvas composer — Phase A3 real BuildingBlocks compositor.
//!
//! Phase 11 left a magenta-grid placeholder.  Phase A3 replaces the internal
//! rendering path with a real compositor that:
//!
//! 1. Lays out merged nodes via [`crate::bb_layout::layout`].
//! 2. Fills each node's background colour (from `node.background.fill_colour`).
//! 3. Blits atlas bitmap / SVG images via [`AtlasLibrary`].
//! 4. Draws borders and placeholder shapes using `tiny-skia` 0.12.
//! 5. Draws text nodes with bundled DejaVu glyphs.
//!
//! Post-process (tint / scanlines / vignette) is disabled until Phase A5.
//!
//! **Public API** (`render_canvas`, `render_canvas_with_postprocess`,
//! `encode_png`, `ComposeContext`, `ComposeTarget`) is unchanged from Phase 11
//! so callers compile without modification.  When `BbScene` resolution
//! succeeds in the pipeline, [`render_bb_scene`] is called directly and
//! `render_canvas` (still magenta) is never reached.

use image::{Rgba, RgbaImage};
use tiny_skia::{
    Color, IntSize, Paint, PathBuilder, Pixmap, PixmapPaint,
    Stroke, Transform, Rect as TskRect,
};

use crate::bb_atlas::AtlasLibrary;
use crate::bb_assets::UiAssetResolver;
use crate::bb_bindings::BindingResolver;
use crate::bb_layout::{self, Rect};
use crate::bb_scene::{BbBorder, BbNode, BbNodeType, BbScene, BbValue};
use crate::canvas::ResolvedCanvas;
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::postprocess::PostProcessOptions;
use crate::style::ManufacturerStyle;
use crate::swf_assets::SwfAssetLibrary;
use crate::swf_render;
use crate::text::{FontKind, TextAlign, TextRenderer};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Shared references needed by the compositor.
pub struct ComposeContext<'a> {
    pub style: &'a ManufacturerStyle,
    pub defaults: &'a DefaultValueRegistry,
    pub assets: &'a SwfAssetLibrary,
}

/// Output canvas dimensions in pixels.
pub struct ComposeTarget {
    pub width: u32,
    pub height: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// A3 real compositor
// ─────────────────────────────────────────────────────────────────────────────

/// Intermediate render state produced by [`render_bb_scene_pass1`].
///
/// Holds the rasterised background+shapes image together with the layout and
/// binding state needed to draw text in [`render_bb_scene_pass2`].  This
/// allows callers (e.g. `pipeline.rs`) to interleave a Flash SWF overlay
/// between the shape pass and the text pass so that BB-derived labels always
/// appear on top.
pub struct BbRenderState {
    pub img: RgbaImage,
    layout: bb_layout::LayoutResult,
    resolver: BindingResolver,
    renderer: TextRenderer,
}

/// Rasterise a merged [`BbScene`] to RGBA using the Phase A3 compositor.
///
/// Called directly from `pipeline.rs` when BbScene resolution succeeds;
/// [`render_canvas`] (the magenta fallback) is only reached when it fails.
pub fn render_bb_scene(
    scene: &BbScene,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    target: ComposeTarget,
) -> Result<RgbaImage, UiError> {
    let mut state = render_bb_scene_pass1(scene, ctx, atlas, target)?;
    render_bb_scene_pass2(&mut state, scene, ctx);
    Ok(state.img)
}

/// Pass 1: fill background, draw non-text nodes.
///
/// Returns a [`BbRenderState`] containing the partially-rendered image and the
/// layout/resolver/renderer state required for the text pass.  The caller may
/// composite additional content (e.g. a SWF overlay) onto `state.img` before
/// calling [`render_bb_scene_pass2`].
pub fn render_bb_scene_pass1(
    scene: &BbScene,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    target: ComposeTarget,
) -> Result<BbRenderState, UiError> {
    if target.width == 0 || target.height == 0 {
        return Err(UiError::RenderError(format!(
            "invalid target size {}×{}",
            target.width, target.height
        )));
    }

    let layout = bb_layout::layout(scene, target.width, target.height);
    let text_renderer = TextRenderer::new();
    let resolver = BindingResolver::from_operations(&scene.operations);

    let mut pixmap = Pixmap::new(target.width, target.height)
        .ok_or_else(|| UiError::RenderError("pixmap allocation failed".into()))?;

    // Fill canvas base with the manufacturer's background colour.
    let bg = &ctx.style.background;
    pixmap.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    for &node_id in &layout.draw_order {
        let Some(node) = scene.nodes.get(&node_id) else {
            continue;
        };
        let Some(&rect) = layout.rects.get(&node_id) else {
            continue;
        };
        // Nodes with sub-pixel area contribute nothing visible.
        if rect.w < 0.5 || rect.h < 0.5 {
            continue;
        }
        draw_node(node, rect, ctx, atlas, &mut pixmap);
    }

    // Convert premultiplied tiny-skia pixmap → straight-alpha RgbaImage.
    let img = pixmap_to_rgba_image(pixmap)?;
    Ok(BbRenderState { img, layout, resolver, renderer: text_renderer })
}

/// Pass 2: draw text nodes onto a [`BbRenderState`] image.
///
/// Should be called after any overlay (e.g. SWF) has been composited onto
/// `state.img` so that BB-derived text labels appear on top.
///
/// Alternate-state siblings sharing the same parent and rect (e.g.
/// `text_BodyValueFaction` / `text_BodyValueVelocity`) all carry
/// `isActive=true` in the static BB data.  We render only the first sibling
/// at any (x, y, w) position to avoid overlapping-text smear.
pub fn render_bb_scene_pass2(
    state: &mut BbRenderState,
    scene: &BbScene,
    ctx: &ComposeContext<'_>,
) {
    let mut seen_text_rects: std::collections::HashSet<(i32, i32, i32)> =
        std::collections::HashSet::new();
    for &node_id in &state.layout.draw_order {
        let Some(node) = scene.nodes.get(&node_id) else {
            continue;
        };
        if !matches!(node.ty, BbNodeType::WidgetTextField | BbNodeType::WidgetText) {
            continue;
        }
        let Some(&rect) = state.layout.rects.get(&node_id) else {
            continue;
        };
        if rect.w < 0.5 || rect.h < 0.5 {
            continue;
        }
        // Height is intentionally excluded from the dedup key: merged child canvases
        // sometimes differ only in their computed height for the same widget (e.g.
        // `text_Title` h=48 vs `text_Component` h=69 at the same x/y/w), which the
        // (x, y, w, h) key would treat as distinct, leaving both texts rendered on
        // top of each other.
        let key = (
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.w.round() as i32,
        );
        if !seen_text_rects.insert(key) {
            continue;
        }
        draw_text_node(
            &mut state.img,
            node,
            rect,
            &state.renderer,
            &state.resolver,
            state.layout.canvas_scale,
            ctx,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase 11 fallback (magenta placeholder)
// ─────────────────────────────────────────────────────────────────────────────

/// Rasterise a `ResolvedCanvas` — fallback when BbScene resolution fails.
///
/// **Phase 11 behaviour:** still produces a bright magenta grid so any binding
/// routed through this path is visually obvious.
pub fn render_canvas(
    _canvas: &ResolvedCanvas,
    _ctx: &ComposeContext<'_>,
    target: ComposeTarget,
) -> Result<RgbaImage, UiError> {
    if target.width == 0 || target.height == 0 {
        return Err(UiError::RenderError(format!(
            "invalid target size {}×{}",
            target.width, target.height
        )));
    }
    magenta_placeholder(target.width, target.height)
}

/// Encode an [`RgbaImage`] to a PNG byte vector.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, UiError> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| UiError::RenderError(format!("PNG encode failed: {e}")))?;
    Ok(buf)
}

/// Rasterise + post-process.
///
/// A3: post-process is kept off (`apply_postprocess: false` in the pipeline).
/// A5 will re-enable it.  This function forwards to [`render_canvas`] for now.
pub fn render_canvas_with_postprocess(
    canvas: &ResolvedCanvas,
    ctx: &ComposeContext<'_>,
    target: ComposeTarget,
    _opts: &PostProcessOptions,
) -> Result<RgbaImage, UiError> {
    render_canvas(canvas, ctx, target)
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-node drawing dispatch
// ─────────────────────────────────────────────────────────────────────────────

fn draw_node(
    node: &BbNode,
    rect: Rect,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
) {
    let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
        return;
    };
    let alpha = node.alpha.clamp(0.0, 1.0);
    let iw = rect.w.round().max(1.0) as u32;
    let ih = rect.h.round().max(1.0) as u32;

    match &node.ty {
        // DisplayWidget is documented as a layout container, but BB authors
        // routinely attach background fill, raw SVG paths, atlas-backed
        // images, and borders directly to DisplayWidget nodes (e.g. brand
        // logos, header borders, status backgrounds on interactive door /
        // panel canvases).  Render the same fill + raw-asset + atlas-image
        // + border stack as WidgetCanvas/WidgetCard so those visuals are
        // not silently dropped.  Nodes with none of those attributes stay
        // a no-op (no pixels drawn).
        BbNodeType::DisplayWidget => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if !draw_raw_asset(node, rect, atlas, pixmap, alpha) {
                if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                    blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
                }
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }
        BbNodeType::WidgetBodyBackground => {
            let bl = &ctx.style.backlight;
            let fill = [
                bl.r as f32 / 255.0,
                bl.g as f32 / 255.0,
                bl.b as f32 / 255.0,
                0.50,
            ];
            fill_rect_ts(pixmap, tsk_rect, fill, alpha);
        }

        // Container widgets: explicit background fill + atlas image + border.
        // Transparent-fill WidgetCanvas nodes are pure layout containers with no
        // pixel output of their own; their children provide visual content.
        BbNodeType::WidgetCanvas => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }

        BbNodeType::WidgetCard
        | BbNodeType::ComponentGeneralButton
        | BbNodeType::ComponentGeneralButtonSecondary => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if !draw_raw_asset(node, rect, atlas, pixmap, alpha) {
                if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                    blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
                }
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }

        // Icon / image widgets: blit atlas or render nothing.
        // BB `WidgetIcon` / `WidgetImage` whose atlas cannot be resolved
        // typically reference DDS textures we have not yet decoded (paint /
        // brand glyph sheets) or runtime-rendered Flash content. Painting a
        // placeholder diamond at full widget size produces large geometric
        // outlines that do not exist in the in-game reference; rendering
        // nothing is structurally honest until the asset can be resolved.
        BbNodeType::WidgetIcon | BbNodeType::WidgetImage => {
            if draw_raw_asset(node, rect, atlas, pixmap, alpha) {
                // Raw-path asset was found and attempted; skip atlas fallback.
            } else if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                let tint = node
                    .icon
                    .as_ref()
                    .and_then(|i| i.tint_colour)
                    .unwrap_or([1.0, 1.0, 1.0, 1.0]);
                blit_atlas_image_tinted(
                    pixmap,
                    &img,
                    rect.x as i32,
                    rect.y as i32,
                    tint,
                    alpha,
                );
            }
        }

        // Text widgets are rendered in a second RgbaImage pass after tiny-skia output.
        BbNodeType::WidgetTextField | BbNodeType::WidgetText => {}

        // Custom shapes: blit atlas when available; otherwise try the SWF
        // asset library by symbol name, then fall back to authored fill/border.
        // BB `WidgetCustomShape` with `rendererType: "Flash"` carries no static
        // geometry — the shape lives in the SWF movie under an ActionScript
        // export name that does NOT match `node.name`.  The Flash overlay step
        // in `pipeline.rs` renders all visual exports after scene composition;
        // `draw_swf_symbol` here is a no-op for Flash-backed shapes (returns
        // false when the name is not found in the SWF exports).
        BbNodeType::WidgetCustomShape => {
            let drew_raw = draw_raw_asset(node, rect, atlas, pixmap, alpha);
            if drew_raw {
                // Raw-path asset was rendered; still apply any authored border.
                if let Some(border) = &node.border {
                    draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
                }
            } else if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
                if let Some(border) = &node.border {
                    draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
                }
            } else {
                // Try SWF shape by symbol name.
                let pt = &ctx.style.primary_tint;
                let tint = Color::from_rgba8(pt.r, pt.g, pt.b, pt.a);
                let drew_swf = swf_render::draw_swf_symbol(
                    pixmap,
                    ctx.assets,
                    &node.name,
                    tsk_rect,
                    tint,
                    alpha,
                );

                if !drew_swf {
                    // Fall through to authored fill / border only when one is
                    // present. Empty Flash-rendered shapes draw nothing.
                    let bg_fill = node_fill_rgba(node);
                    if bg_fill[3] > 0.005 {
                        fill_rect_ts(pixmap, tsk_rect, bg_fill, alpha);
                    }
                    if let Some(border) = &node.border {
                        let max_w = border
                            .top
                            .width
                            .max(border.right.width)
                            .max(border.bottom.width)
                            .max(border.left.width);
                        if max_w > 0.5 {
                            draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
                        }
                    }
                }
            }
        }

        // Unknown widget types: render the same fill + raw-asset +
        // atlas-image + border stack used by DisplayWidget/WidgetCanvas.
        // BB authoring frequently attaches visual data to widget types
        // the renderer does not yet specialize (e.g. WidgetContainer,
        // WidgetList, WidgetSeparator, WidgetLine, WidgetCircle,
        // WidgetLinearProgressMeter, WidgetClone, WidgetManufacturerLogo).
        // Drawing the structural attributes makes those nodes contribute
        // their content instead of a debug outline; nodes carrying none
        // of those attributes simply produce no pixels.
        BbNodeType::Other(_) => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if !draw_raw_asset(node, rect, atlas, pixmap, alpha) {
                if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                    blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
                }
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Brand-modifier raw-asset rendering (R3)
// ─────────────────────────────────────────────────────────────────────────────

/// Attempt to render a brand-modifier asset from `node.raw["SvgPath"]` or
/// `node.raw["ImagePath"]`.
///
/// Returns `true` when either key is present in `node.raw`, indicating to the
/// caller that a raw-path render was attempted and the atlas fallback should be
/// suppressed.  Returns `false` when neither key is present.
///
/// Reference-overlay paths (`_references` hierarchy) are silently skipped —
/// `true` is still returned so the caller does not fall through to an
/// unrelated fallback draw.
fn draw_raw_asset(
    node: &BbNode,
    rect: Rect,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
    alpha: f32,
) -> bool {
    let svg_path_raw = node.raw.get("SvgPath").and_then(|v| v.as_str());
    let img_path_raw = node.raw.get("ImagePath").and_then(|v| v.as_str());

    if svg_path_raw.is_none() && img_path_raw.is_none() {
        return false;
    }

    let iw = rect.w.round().max(1.0) as u32;
    let ih = rect.h.round().max(1.0) as u32;

    if let Some(raw_path) = svg_path_raw {
        let norm = UiAssetResolver::normalise_path(raw_path);
        if !UiAssetResolver::is_reference_overlay(&norm) {
            let fill_override = node_fill_override(node);
            if let Some(svg_bytes) = atlas.fetch_raw(&norm) {
                if let Some(img) = crate::bb_svg::rasterize_svg(&svg_bytes, iw, ih, fill_override)
                {
                    blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
                }
            }
        }
        return true;
    }

    if let Some(raw_path) = img_path_raw {
        let norm = UiAssetResolver::normalise_path(raw_path);
        if !UiAssetResolver::is_reference_overlay(&norm) {
            if let Some(img) = atlas.resolve(&norm, iw, ih) {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
        }
        return true;
    }

    false
}

/// Extract a fill-colour override for SVG tinting from a node.
///
/// Checks `node.background.fill_colour` first, then `node.raw["FillColor"]`
/// as a `{r, g, b, a}` object in `0.0..=1.0` component range.
fn node_fill_override(node: &BbNode) -> Option<[f32; 4]> {
    if let Some(bg) = &node.background {
        if let Some(c) = bg.fill_colour {
            return Some(c);
        }
    }

    let obj = node.raw.get("FillColor")?.as_object()?;
    let r = obj.get("r").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let g = obj.get("g").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let b = obj.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let a = obj.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some([r, g, b, a])
}

// ─────────────────────────────────────────────────────────────────────────────
// Colour helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Return the explicit fill colour for a node, or transparent if none is set.
fn node_fill_rgba(node: &BbNode) -> [f32; 4] {
    node.background
        .as_ref()
        .and_then(|bg| bg.fill_colour)
        .unwrap_or([0.0, 0.0, 0.0, 0.0])
}

/// Build a tiny-skia `Color` from a `[f32; 4]` RGBA value (0–1) and a global
/// alpha multiplier.
fn to_skia_color(rgba: [f32; 4], global_alpha: f32) -> Color {
    let a = (rgba[3] * global_alpha).clamp(0.0, 1.0);
    Color::from_rgba8(
        (rgba[0].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0) as u8,
        (a * 255.0) as u8,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Tiny-skia drawing helpers
// ─────────────────────────────────────────────────────────────────────────────

fn fill_rect_ts(pixmap: &mut Pixmap, rect: TskRect, rgba: [f32; 4], alpha: f32) {
    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
    paint.anti_alias = false;
    pixmap
        .as_mut()
        .fill_rect(rect, &paint, Transform::identity(), None);
}

fn draw_border_ts(
    pixmap: &mut Pixmap,
    rect: TskRect,
    border: &BbBorder,
    ctx: &ComposeContext<'_>,
    alpha: f32,
    raw: &serde_json::Value,
) {
    let max_width = border
        .top
        .width
        .max(border.right.width)
        .max(border.bottom.width)
        .max(border.left.width);
    if max_width < 0.01 {
        return;
    }

    let radius = raw
        .get("border")
        .and_then(|b| b.get("radius"))
        .and_then(|r| r.as_f64())
        .unwrap_or(0.0) as f32;

    let pt = &ctx.style.primary_tint;
    let fallback = [
        pt.r as f32 / 255.0,
        pt.g as f32 / 255.0,
        pt.b as f32 / 255.0,
        0.4,
    ];
    let border_rgba = border
        .top
        .colour
        .or(border.right.colour)
        .or(border.bottom.colour)
        .or(border.left.colour)
        .unwrap_or(fallback);

    let mut paint = Paint::default();
    paint.set_color(to_skia_color(border_rgba, alpha));
    paint.anti_alias = radius > 0.5;

    let path = if radius > 0.5 {
        // tiny-skia 0.12 has no RoundRect; build corner-arc approximation with cubics.
        // kappa ≈ 0.5523 gives a circle-quality Bézier approximation.
        let k = 0.5523 * radius;
        let l = rect.x() + radius;
        let r = rect.x() + rect.width() - radius;
        let t = rect.y() + radius;
        let b = rect.y() + rect.height() - radius;
        let mut pb = PathBuilder::new();
        pb.move_to(l, rect.y());
        pb.line_to(r, rect.y());
        pb.cubic_to(r + k, rect.y(), rect.x() + rect.width(), t - k, rect.x() + rect.width(), t);
        pb.line_to(rect.x() + rect.width(), b);
        pb.cubic_to(rect.x() + rect.width(), b + k, r + k, rect.y() + rect.height(), r, rect.y() + rect.height());
        pb.line_to(l, rect.y() + rect.height());
        pb.cubic_to(l - k, rect.y() + rect.height(), rect.x(), b + k, rect.x(), b);
        pb.line_to(rect.x(), t);
        pb.cubic_to(rect.x(), t - k, l - k, rect.y(), l, rect.y());
        pb.close();
        pb.finish()
    } else {
        Some(PathBuilder::from_rect(rect))
    };

    if let Some(p) = path {
        let mut stroke = Stroke::default();
        stroke.width = max_width.max(1.0);
        pixmap
            .as_mut()
            .stroke_path(&p, &paint, &stroke, Transform::identity(), None);
    }
}

#[allow(dead_code)]
fn draw_diamond_ts(pixmap: &mut Pixmap, rect: TskRect, ctx: &ComposeContext<'_>, alpha: f32) {
    let cx = rect.x() + rect.width() * 0.5;
    let cy = rect.y() + rect.height() * 0.5;
    let rx = rect.width() * 0.4;
    let ry = rect.height() * 0.4;

    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - ry);
    pb.line_to(cx + rx, cy);
    pb.line_to(cx, cy + ry);
    pb.line_to(cx - rx, cy);
    pb.close();

    if let Some(path) = pb.finish() {
        let pt = &ctx.style.primary_tint;
        let color = [
            pt.r as f32 / 255.0,
            pt.g as f32 / 255.0,
            pt.b as f32 / 255.0,
            0.6,
        ];
        let mut paint = Paint::default();
        paint.set_color(to_skia_color(color, alpha));
        paint.anti_alias = true;
        let mut stroke = Stroke::default();
        stroke.width = 1.0;
        pixmap
            .as_mut()
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn draw_text_node(
    img: &mut RgbaImage,
    node: &BbNode,
    rect: Rect,
    renderer: &TextRenderer,
    resolver: &BindingResolver,
    canvas_scale: f32,
    ctx: &ComposeContext<'_>,
) {
    let resolved = resolver.resolve_text_detailed(node.id, &node.raw, ctx.defaults);
    if resolved.text.is_empty() {
        return;
    }
    let text = &resolved.text;

    // Resolve nominal font size:
    //  1. Explicit `fontSize` actually present in raw JSON (BbValue::Fixed) wins.
    //  2. Otherwise derive from `labelProperties.style` (Heading1..6 / Body / Caption).
    //  3. Otherwise default 22 px so widget-name-derived labels are legible at 1600×900.
    let has_explicit_font_size = node.raw.get("fontSize").is_some();
    let explicit_size = if has_explicit_font_size {
        match node.text.as_ref().map(|t| &t.font_size) {
            Some(BbValue::Fixed(px)) if *px > 0.0 => Some(*px),
            _ => None,
        }
    } else {
        None
    };
    let style_size = node
        .raw
        .get("labelProperties")
        .and_then(|lp| lp.get("style"))
        .and_then(|v| v.as_str())
        .map(font_size_from_style);
    let nominal_px = explicit_size.or(style_size).unwrap_or(22.0);
    let size_px = nominal_px * canvas_scale;

    // Horizontal alignment: read from raw `textAlignment` (Left/Center/Right).
    // For name-derived fallback labels, force Center — matches in-game
    // convention where the dynamic-binding text is centred by the SWF layer.
    let align = if resolved.is_name_derived {
        TextAlign::Centre
    } else {
        node.raw
            .get("textAlignment")
            .and_then(|v| v.as_str())
            .map(TextAlign::from_bb_str)
            .or_else(|| node.text.as_ref().map(|t| TextAlign::from_bb_str(&t.alignment)))
            .unwrap_or(TextAlign::Left)
    };

    let mut colour = if let Some(c) = node.text.as_ref().and_then(|t| t.colour) {
        [
            colour_component_to_u8(c[0]),
            colour_component_to_u8(c[1]),
            colour_component_to_u8(c[2]),
            colour_component_to_u8(c[3]),
        ]
    } else {
        let pt = &ctx.style.primary_tint;
        [pt.r, pt.g, pt.b, pt.a]
    };
    colour[3] = ((colour[3] as f32) * node.alpha.clamp(0.0, 1.0)).clamp(0.0, 255.0) as u8;

    renderer.draw(img, text, rect, FontKind::Sans, size_px, colour, align);
}

/// Map BB `labelProperties.style` string to a nominal pixel size in authored space.
///
/// Sizes are approximate; per-manufacturer style overrides apply tints/colours
/// elsewhere. Numbers are tuned to roughly match in-game references for Drake
/// MFD headings and body text at 1600×900 authored resolution.
fn font_size_from_style(style: &str) -> f32 {
    match style {
        "Heading1" => 48.0,
        "Heading2" => 36.0,
        "Heading3" => 28.0,
        "Heading4" => 22.0,
        "Heading5" => 18.0,
        "Heading6" => 16.0,
        "Body" | "Body1" => 16.0,
        "Body2" => 14.0,
        "Caption" => 12.0,
        _ => 18.0,
    }
}

fn colour_component_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

// ─────────────────────────────────────────────────────────────────────────────
// Atlas image blit helpers
// ─────────────────────────────────────────────────────────────────────────────

fn blit_atlas_image(pixmap: &mut Pixmap, img: &RgbaImage, dx: i32, dy: i32, alpha: f32) {
    blit_atlas_image_tinted(pixmap, img, dx, dy, [1.0, 1.0, 1.0, 1.0], alpha);
}

/// Blit a straight-alpha `RgbaImage` onto a premultiplied tiny-skia `Pixmap`.
///
/// The source pixels are tinted and premultiplied before being passed to
/// tiny-skia's `draw_pixmap` for correct SrcOver blending.
fn blit_atlas_image_tinted(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
) {
    let w = img.width();
    let h = img.height();

    // Build a premultiplied source pixmap with tint applied.
    let mut premul: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
    for chunk in img.as_raw().chunks_exact(4) {
        let r = chunk[0] as f32 / 255.0 * tint[0];
        let g = chunk[1] as f32 / 255.0 * tint[1];
        let b = chunk[2] as f32 / 255.0 * tint[2];
        let a = chunk[3] as f32 / 255.0 * tint[3];
        premul.push((r * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((g * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((b * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((a * 255.0).clamp(0.0, 255.0) as u8);
    }

    let Some(size) = IntSize::from_wh(w, h) else {
        return;
    };
    let Some(src_pixmap) = Pixmap::from_vec(premul, size) else {
        return;
    };

    let mut paint = PixmapPaint::default();
    paint.opacity = alpha.clamp(0.0, 1.0);
    pixmap.as_mut().draw_pixmap(
        dx,
        dy,
        src_pixmap.as_ref(),
        &paint,
        Transform::identity(),
        None,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Pixmap → RgbaImage conversion
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a premultiplied tiny-skia `Pixmap` to a straight-alpha `RgbaImage`.
///
/// tiny-skia stores `(R×A, G×A, B×A, A)` per pixel.  `RgbaImage` expects
/// straight `(R, G, B, A)`.  Fully transparent pixels are left as-is to avoid
/// divide-by-zero.
fn pixmap_to_rgba_image(pixmap: Pixmap) -> Result<RgbaImage, UiError> {
    let w = pixmap.width();
    let h = pixmap.height();
    let mut data = pixmap.take();

    for pixel in data.chunks_exact_mut(4) {
        let a = pixel[3] as u32;
        if a > 0 && a < 255 {
            pixel[0] = (pixel[0] as u32 * 255 / a).min(255) as u8;
            pixel[1] = (pixel[1] as u32 * 255 / a).min(255) as u8;
            pixel[2] = (pixel[2] as u32 * 255 / a).min(255) as u8;
        }
    }

    RgbaImage::from_raw(w, h, data)
        .ok_or_else(|| UiError::RenderError("pixmap→RgbaImage conversion failed".into()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Magenta placeholder (fallback when BbScene resolution fails)
// ─────────────────────────────────────────────────────────────────────────────

fn magenta_placeholder(w: u32, h: u32) -> Result<RgbaImage, UiError> {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([255, 0, 255, 255]));
    for y in 0..h {
        for x in 0..w {
            if x % 64 == 0 || y % 64 == 0 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
    }
    Ok(img)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bb_atlas::AssetFetcher;
    use crate::bb_scene::BbScene;
    use crate::canvas::{CanvasRecord, ResolvedCanvas};
    use crate::defaults::DefaultValueRegistry;
    use crate::style::StyleLoader;
    use crate::swf_assets::SwfAssetLibrary;
    use std::collections::BTreeMap;

    fn empty_canvas() -> ResolvedCanvas {
        ResolvedCanvas {
            root: CanvasRecord {
                guid: String::from("00000000"),
                name: String::from("placeholder_test"),
                views: Vec::new(),
                scene: Vec::new(),
                operations: Vec::new(),
            },
            children: Default::default(),
        }
    }

    fn empty_assets() -> SwfAssetLibrary {
        let minimal: Vec<u8> = vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0, 0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        SwfAssetLibrary::new(minimal).expect("minimal SWF is valid")
    }

    struct NullFetcher;
    impl AssetFetcher for NullFetcher {
        fn fetch_image_bytes(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
    }

    fn empty_bb_scene() -> BbScene {
        BbScene {
            canvas_size: (512.0, 256.0),
            roots: vec![],
            nodes: BTreeMap::new(),
            operations: vec![],
        }
    }

    // render_canvas still produces the magenta placeholder.
    #[test]
    fn placeholder_is_predominantly_magenta() {
        let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
        let img = render_canvas(&empty_canvas(), &ctx, ComposeTarget { width: 128, height: 128 })
            .expect("render placeholder");
        let mut magenta = 0usize;
        let mut white = 0usize;
        for px in img.pixels() {
            if px.0 == [255, 0, 255, 255] {
                magenta += 1;
            } else if px.0 == [255, 255, 255, 255] {
                white += 1;
            }
        }
        assert!(magenta > 0, "placeholder should contain magenta pixels");
        assert!(white > 0, "placeholder should contain white grid pixels");
        assert!(
            magenta > white,
            "magenta should dominate (got magenta={magenta}, white={white})"
        );
    }

    #[test]
    fn placeholder_rejects_zero_size() {
        let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
        let err = render_canvas(&empty_canvas(), &ctx, ComposeTarget { width: 0, height: 64 })
            .expect_err("should reject 0 width");
        assert!(matches!(err, UiError::RenderError(_)));
    }

    // A3: render_bb_scene with an empty scene fills the canvas with the
    // background colour and produces no magenta.
    #[test]
    fn render_bb_scene_empty_is_background_colour() {
        let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
        let fetcher = NullFetcher;
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));

        let img = render_bb_scene(
            &empty_bb_scene(),
            &ctx,
            &atlas,
            ComposeTarget { width: 64, height: 64 },
        )
        .expect("render empty bb scene");

        assert_eq!((img.width(), img.height()), (64, 64));

        let bg = style.background;
        // No magenta on any pixel.
        for px in img.pixels() {
            assert_ne!(
                px.0,
                [255, 0, 255, 255],
                "empty scene must not produce magenta"
            );
        }
        // Center pixel must equal the background colour exactly (no post-processing).
        let cx = img.get_pixel(32, 32);
        assert_eq!(
            [cx.0[0], cx.0[1], cx.0[2], cx.0[3]],
            [bg.r, bg.g, bg.b, bg.a],
            "center pixel of empty scene should equal background colour"
        );
    }

    // A3: render_bb_scene rejects zero-size targets.
    #[test]
    fn render_bb_scene_rejects_zero_size() {
        let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
        let fetcher = NullFetcher;
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));

        let err = render_bb_scene(
            &empty_bb_scene(),
            &ctx,
            &atlas,
            ComposeTarget { width: 0, height: 64 },
        )
        .expect_err("should reject 0 width");
        assert!(matches!(err, UiError::RenderError(_)));
    }
}
