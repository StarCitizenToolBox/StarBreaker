//! SWF shape rasterizer — renders static SWF shapes into a `tiny-skia` `Pixmap`.
//!
//! # Public API
//! [`draw_swf_symbol`] is the single entry point: look up `symbol_name` in a
//! [`SwfAssetLibrary`], decode the referenced shape character (or the first
//! frame of a sprite character), build `tiny-skia` paths from the SWF
//! edge records, and fill/stroke them into `pixmap` mapped into `dest`.
//!
//! # Coordinate system
//! SWF coordinates are in **Twips** (1/20 px). Shape bounds are used to
//! define the SWF-space viewport; `dest` is the pixel-space viewport.  A
//! linear mapping (no rotation) from shape-bounds → dest is applied to
//! every control point.
//!
//! # Fill/line support
//! - `FillStyle::Color` — solid RGBA fill.
//! - `LineStyle` with a `FillStyle::Color` fill — solid stroke.
//! - Gradient and bitmap fills are **not** rendered; `draw_swf_symbol`
//!   returns `false` for shapes that contain *only* gradient/bitmap fills
//!   and no `Color` fills or strokes.
//!
//! # Tinting
//! When `tint` is opaque white (`#FFFFFFFF`), the SWF authored colour is
//! used unchanged.  When the SWF fill colour is white (255, 255, 255) and
//! `tint` is not opaque white, the fill colour is replaced by `tint`
//! (recolour mode).  This lets manufacturer chrome that is authored as
//! white be tinted to the manufacturer primary colour.

use image::RgbaImage;
use tiny_skia::{
    Color, FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform,
    Rect as TskRect,
};

use crate::swf_assets::{PlaceRecord, ShapeRecord, SwfAssetLibrary};

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Rasterise the SWF shape named `symbol_name` into `pixmap`, mapped into the
/// pixel `dest` rect.
///
/// Returns `false` if:
/// - the symbol is not found in `assets.exports`,
/// - the referenced character is neither a shape nor a sprite,
/// - the shape has no `Color` fill or `Color`-filled line styles,
/// - no visible pixels were produced.
///
/// On `false` the caller should fall back to whatever makes sense (typically
/// drawing nothing).
pub fn draw_swf_symbol(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    symbol_name: &str,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let Some(char_id) = assets.lookup_export(symbol_name) else {
        log::debug!("draw_swf_symbol: symbol '{symbol_name}' not found in exports");
        return false;
    };

    if let Some(shape) = assets.get_shape(char_id) {
        draw_shape(pixmap, shape, dest, tint, alpha)
    } else {
        // Try as a sprite: take first frame, draw each placed shape.
        let places: Vec<PlaceRecord> = assets.extract_sprite_first_frame(char_id);
        if places.is_empty() {
            log::debug!(
                "draw_swf_symbol: char id={char_id} for '{symbol_name}' is not a shape or sprite"
            );
            return false;
        }

        let mut drew_any = false;
        for place in &places {
            if let Some(shape) = assets.get_shape(place.character_id) {
                // Build a dest rect that applies the PlaceObject matrix on top of
                // the shape-bounds → dest mapping.
                let effective_dest = apply_place_matrix_to_dest(shape, &place.matrix, dest);
                if draw_shape(pixmap, shape, effective_dest, tint, alpha) {
                    drew_any = true;
                }
            }
        }
        drew_any
    }
}

/// Rasterise the SWF main-timeline stage frame 0 into `pixmap`, mapped into
/// `dest`.
///
/// Walks the display list produced by [`SwfAssetLibrary::stage_frame`] at
/// frame 0. For each placed character:
///
/// - **DefineShape** — maps the shape's bounding box through the PlaceObject
///   matrix and the stage→dest scale to produce a pixel rect, then calls
///   `draw_shape`.
/// - **DefineSprite** — extracts the sprite's first-frame display list and
///   recurses one level (sprites within sprites are treated as their own
///   flat display list, no further recursion to avoid infinite loops).
/// - Other characters (fonts, bitmaps directly) — currently skipped.
///
/// Returns `true` if at least one shape was drawn.
///
/// A `ColorTransform` on a `PlaceObject2/3` tag modulates the RGBA multiply
/// channel: multiply factors are scaled by the authored colour before tinting.
/// Additive terms are not yet supported.
pub fn draw_swf_stage(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let (sw, sh) = assets.stage_size();
    if sw <= 0.0 || sh <= 0.0 {
        log::debug!("draw_swf_stage: degenerate stage size ({sw}×{sh}), skipping");
        return false;
    }

    let stage_places = assets.stage_frame(0);
    if stage_places.is_empty() {
        log::debug!("draw_swf_stage: stage frame 0 is empty");
        return false;
    }

    let sx = dest.width() / sw;
    let sy = dest.height() / sh;

    let mut drew_any = false;
    for place in &stage_places {
        let ct_tint = color_transform_tint(tint, place.color_transform.as_ref());
        if draw_stage_character(pixmap, assets, place, sw, sh, sx, sy, dest, ct_tint, alpha) {
            drew_any = true;
        }
    }

    log::debug!(
        "draw_swf_stage: stage ({sw}×{sh}) → dest ({:.0}×{:.0}) drew={drew_any}",
        dest.width(),
        dest.height()
    );
    drew_any
}

/// Render the SWF main-timeline stage (frame 0) as an alpha-over composite
/// into `img` (straight-alpha `RgbaImage`).
///
/// Allocates a temporary transparent [`Pixmap`], calls [`draw_swf_stage`],
/// then composites the result using Porter-Duff "over" with proper
/// premultiply/demultiply handling.
///
/// Returns `true` if any SWF pixels were composited.
pub fn draw_swf_stage_rgba(
    img: &mut RgbaImage,
    assets: &SwfAssetLibrary,
    tint: Color,
    alpha: f32,
) -> bool {
    let w = img.width();
    let h = img.height();
    let Some(mut pixmap) = Pixmap::new(w, h) else {
        return false;
    };
    let Some(dest) = TskRect::from_xywh(0.0, 0.0, w as f32, h as f32) else {
        return false;
    };
    if !draw_swf_stage(&mut pixmap, assets, dest, tint, alpha) {
        return false;
    }

    // Porter-Duff "over": pixmap (premultiplied RGBA) over img (straight-alpha RGBA).
    let pix = pixmap.data();
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) as usize) * 4;
            let a_top = pix[idx + 3] as u32;
            if a_top == 0 {
                continue;
            }
            // Un-premultiply the top layer.
            let r_top = ((pix[idx] as u32 * 255) / a_top.max(1)).min(255);
            let g_top = ((pix[idx + 1] as u32 * 255) / a_top.max(1)).min(255);
            let b_top = ((pix[idx + 2] as u32 * 255) / a_top.max(1)).min(255);

            let base = img.get_pixel(x, y);
            let ba = base[3] as u32;

            let out_a = (a_top + ba * (255 - a_top) / 255).min(255);
            if out_a == 0 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            } else {
                let blend = |top: u32, bot: u32| -> u8 {
                    ((top * a_top / 255 + bot * ba * (255 - a_top) / 255 / 255)
                        * 255
                        / out_a)
                        .min(255) as u8
                };
                img.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        blend(r_top, base[0] as u32),
                        blend(g_top, base[1] as u32),
                        blend(b_top, base[2] as u32),
                        out_a as u8,
                    ]),
                );
            }
        }
    }
    true
}


/// Render all visual exports from a Flash SWF into `pixmap`, mapped into `dest`.
///
/// "Visual exports" are those whose linkage name does NOT begin with
/// `__Packages.` — those are AVM1 ActionScript class registrations with no
/// renderable geometry.  Each remaining export (e.g. `TargetSelection_Borders`,
/// `TargetSelection_NoTargetPlaceholder`) is placed at the SWF stage origin with
/// Render all visual exports from a Flash SWF, plus any shapes placed on the
/// main stage timeline (frame 0) that are not already covered by a named
/// export.
///
/// Named exports are rendered at the origin using the identity transform; stage
/// frame items are rendered using their actual PlaceObject matrices so they
/// appear at the correct position within the SWF stage bounds.  This ensures
/// shapes/sprites placed directly on the stage without being exported by name
/// are still rendered (e.g. chrome overlays and dashed lines in TargetStatus).
///
/// Returns `true` if at least one shape was drawn.
pub fn draw_swf_visual_exports(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let (sw, sh) = assets.stage_size();
    if sw <= 0.0 || sh <= 0.0 {
        log::debug!("draw_swf_visual_exports: degenerate stage size ({sw}×{sh}), skipping");
        return false;
    }

    let sx = dest.width() / sw;
    let sy = dest.height() / sh;

    let mut drew_any = false;
    // Deduplicate by character id — a symbol may be exported under multiple
    // names, or appear both as a named export and in the stage frame list.
    let mut seen: std::collections::HashSet<swf::CharacterId> = std::collections::HashSet::new();
    // Collect first to avoid borrow conflict with `assets` inside the loop.
    let char_ids: Vec<swf::CharacterId> = assets.visual_exports().collect();

    for char_id in char_ids {
        if !seen.insert(char_id) {
            continue;
        }
        let place = PlaceRecord {
            depth: 0,
            character_id: char_id,
            matrix: swf::Matrix::IDENTITY,
            color_transform: None,
            name: None,
        };
        if draw_stage_character(pixmap, assets, &place, sw, sh, sx, sy, dest, tint, alpha) {
            drew_any = true;
        }
    }

    // Also render stage frame 0 — catches shapes/sprites placed on the main
    // timeline that have no named export (e.g. chrome overlays, dashed lines).
    let stage_places = assets.stage_frame(0);
    for place in &stage_places {
        if !seen.insert(place.character_id) {
            continue;
        }
        let ct_tint = color_transform_tint(tint, place.color_transform.as_ref());
        if draw_stage_character(pixmap, assets, place, sw, sh, sx, sy, dest, ct_tint, alpha) {
            drew_any = true;
        }
    }

    log::debug!(
        "draw_swf_visual_exports: stage ({sw:.0}×{sh:.0}) → dest ({:.0}×{:.0}) drew={drew_any}",
        dest.width(),
        dest.height()
    );
    drew_any
}

/// Render all visual exports from a Flash SWF as an alpha-over composite into
/// `img` (straight-alpha `RgbaImage`).
///
/// Allocates a temporary transparent [`Pixmap`], calls
/// [`draw_swf_visual_exports`], then composites the result using Porter-Duff
/// "over" with proper premultiply/demultiply handling.
///
/// Returns `true` if any SWF pixels were composited.
pub fn draw_swf_visual_exports_rgba(
    img: &mut RgbaImage,
    assets: &SwfAssetLibrary,
    tint: Color,
    alpha: f32,
) -> bool {
    let w = img.width();
    let h = img.height();
    let Some(mut pixmap) = Pixmap::new(w, h) else {
        return false;
    };
    let Some(dest) = TskRect::from_xywh(0.0, 0.0, w as f32, h as f32) else {
        return false;
    };
    if !draw_swf_visual_exports(&mut pixmap, assets, dest, tint, alpha) {
        return false;
    }

    // Porter-Duff "over": pixmap (premultiplied RGBA) over img (straight-alpha RGBA).
    let pix = pixmap.data();
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) as usize) * 4;
            let a_top = pix[idx + 3] as u32;
            if a_top == 0 {
                continue;
            }
            let r_top = ((pix[idx] as u32 * 255) / a_top.max(1)).min(255);
            let g_top = ((pix[idx + 1] as u32 * 255) / a_top.max(1)).min(255);
            let b_top = ((pix[idx + 2] as u32 * 255) / a_top.max(1)).min(255);

            let base = img.get_pixel(x, y);
            let ba = base[3] as u32;

            let out_a = (a_top + ba * (255 - a_top) / 255).min(255);
            if out_a == 0 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            } else {
                let blend = |top: u32, bot: u32| -> u8 {
                    ((top * a_top / 255 + bot * ba * (255 - a_top) / 255 / 255)
                        * 255
                        / out_a)
                        .min(255) as u8
                };
                img.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        blend(r_top, base[0] as u32),
                        blend(g_top, base[1] as u32),
                        blend(b_top, base[2] as u32),
                        out_a as u8,
                    ]),
                );
            }
        }
    }
    true
}


/// Render one character from a stage or sprite display list, with recursive
/// sprite expansion up to `MAX_SPRITE_DEPTH` levels deep.
///
/// `sw` / `sh` — parent viewport size in SWF pixels.
/// `sx` / `sy` — parent viewport → pixmap scale (pixels per SWF pixel).
/// `origin` — top-left corner of the parent viewport in pixmap coordinates.
fn draw_stage_character(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    place: &PlaceRecord,
    sw: f32,
    sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    const MAX_SPRITE_DEPTH: u8 = 4;
    draw_stage_character_depth(pixmap, assets, place, sw, sh, sx, sy, origin, tint, alpha, MAX_SPRITE_DEPTH)
}

fn draw_stage_character_depth(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    place: &PlaceRecord,
    sw: f32,
    sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
    tint: Color,
    alpha: f32,
    max_depth: u8,
) -> bool {
    let char_id = place.character_id;

    if let Some(shape) = assets.get_shape(char_id) {
        let shape_dest = matrix_to_dest(shape, &place.matrix, sw, sh, sx, sy, origin);
        draw_shape(pixmap, shape, shape_dest, tint, alpha)
    } else if max_depth > 0 {
        // Try as a sprite — draw its first frame, recursing into nested sprites.
        let sprite_places = assets.extract_sprite_first_frame(char_id);
        if sprite_places.is_empty() {
            return false;
        }
        let sprite_origin = sprite_origin_in_dest(&place.matrix, sw, sh, sx, sy, origin);
        let mut drew_any = false;
        for sp_place in &sprite_places {
            let sp_tint = color_transform_tint(tint, sp_place.color_transform.as_ref());
            if draw_stage_character_depth(
                pixmap, assets, sp_place,
                sw, sh, sx, sy, sprite_origin,
                sp_tint, alpha, max_depth - 1,
            ) {
                drew_any = true;
            }
        }
        drew_any
    } else {
        false
    }
}

/// Compute the destination rect for a shape placed via `matrix` into a
/// parent viewport of size (`sw`, `sh`) SWF pixels, mapped to a pixmap region
/// anchored at `origin`.
///
/// For axis-aligned matrices (no rotation) this is exact. For rotated
/// matrices it degrades gracefully by using the transformed bounding box,
/// which may clip rotated shapes at their AABBs — acceptable for B2 scope
/// where MFD SWF content is predominantly axis-aligned.
fn matrix_to_dest(
    shape: &ShapeRecord,
    matrix: &swf::Matrix,
    _sw: f32,
    _sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
) -> TskRect {
    let b = &shape.shape_bounds;
    let bx0 = b.x_min.to_pixels() as f32;
    let by0 = b.y_min.to_pixels() as f32;
    let bx1 = b.x_max.to_pixels() as f32;
    let by1 = b.y_max.to_pixels() as f32;

    let a = matrix.a.to_f32();
    let b_coef = matrix.b.to_f32();
    let c = matrix.c.to_f32();
    let d = matrix.d.to_f32();
    let tx = matrix.tx.to_pixels() as f32;
    let ty = matrix.ty.to_pixels() as f32;

    // Transform the four corners of the shape bounds.
    let corners = [(bx0, by0), (bx1, by0), (bx0, by1), (bx1, by1)];
    let trans: Vec<(f32, f32)> = corners
        .iter()
        .map(|&(x, y)| (a * x + c * y + tx, b_coef * x + d * y + ty))
        .collect();

    let mx0 = trans.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let my0 = trans.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let mx1 = trans.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let my1 = trans.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);

    let dest_x = origin.left() + mx0 * sx;
    let dest_y = origin.top() + my0 * sy;
    let dest_w = ((mx1 - mx0) * sx).max(1.0);
    let dest_h = ((my1 - my0) * sy).max(1.0);

    TskRect::from_xywh(dest_x, dest_y, dest_w, dest_h).unwrap_or(origin)
}

/// Compute the pixmap-space top-left origin of a sprite placed via `matrix`
/// in the parent viewport.  Returns a degenerate 1×1 rect at the translate
/// point when the matrix has no usable scale.
fn sprite_origin_in_dest(
    matrix: &swf::Matrix,
    _sw: f32,
    _sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
) -> TskRect {
    let tx = matrix.tx.to_pixels() as f32;
    let ty = matrix.ty.to_pixels() as f32;
    let dest_x = origin.left() + tx * sx;
    let dest_y = origin.top() + ty * sy;
    let dest_w = origin.width().max(1.0);
    let dest_h = origin.height().max(1.0);
    TskRect::from_xywh(dest_x, dest_y, dest_w, dest_h).unwrap_or(origin)
}

/// Apply a `ColorTransform` (multiply channel only) to the current `tint`.
///
/// Each RGBA multiply factor (0..255 range from the SWF spec) scales the
/// corresponding tint channel.  An absent `ColorTransform` leaves `tint`
/// unchanged.
fn color_transform_tint(tint: Color, ct: Option<&swf::ColorTransform>) -> Color {
    let Some(ct) = ct else { return tint };
    // ColorTransform multiply factors are Fixed8 (0..1 range in the swf crate).
    let rm = ct.r_multiply.to_f32().clamp(0.0, 1.0);
    let gm = ct.g_multiply.to_f32().clamp(0.0, 1.0);
    let bm = ct.b_multiply.to_f32().clamp(0.0, 1.0);
    let am = ct.a_multiply.to_f32().clamp(0.0, 1.0);
    Color::from_rgba(
        (tint.red() * rm).clamp(0.0, 1.0),
        (tint.green() * gm).clamp(0.0, 1.0),
        (tint.blue() * bm).clamp(0.0, 1.0),
        (tint.alpha() * am).clamp(0.0, 1.0),
    )
    .unwrap_or(tint)
}


// ─────────────────────────────────────────────────────────────────────────────

/// Map one SWF shape character into `dest`, using `tint` / `alpha`.
///
/// Returns `true` if at least one fill or stroke was successfully painted.
fn draw_shape(
    pixmap: &mut Pixmap,
    shape: &ShapeRecord,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let bounds = &shape.shape_bounds;
    let bx0 = bounds.x_min.to_pixels() as f32;
    let by0 = bounds.y_min.to_pixels() as f32;
    let bw = (bounds.x_max - bounds.x_min).to_pixels() as f32;
    let bh = (bounds.y_max - bounds.y_min).to_pixels() as f32;

    // Degenerate shape — nothing to draw.
    if bw <= 0.0 || bh <= 0.0 {
        return false;
    }

    // Transform: SWF pixel → dest pixel.
    // tx_px = dest.left() + (swf_x_px - bx0) * sx
    let sx = dest.width() / bw;
    let sy = dest.height() / bh;
    let dx = dest.left() - bx0 * sx;
    let dy = dest.top() - by0 * sy;

    // Collect active fill/line style indices from the records.
    // We walk the records once per (fill_style, sub_path) group.
    let mut drew_any = false;

    // ── Fill pass ────────────────────────────────────────────────────────────
    // We need to draw sub-paths per fill style index.  A single pass
    // collects the geometry and the active style index simultaneously.
    // fill_style_1 (the "left side" fill) is the one that closes filled
    // regions in standard SWF authoring.

    let num_fill = shape.fill_styles.len();
    for style_idx in 1..=(num_fill as u32) {
        let fill = &shape.fill_styles[(style_idx - 1) as usize];
        let color = match fill {
            swf::FillStyle::Color(c) => *c,
            // Gradients / bitmaps: skip for now.
            _ => continue,
        };

        if let Some(path) = build_path_for_fill(shape, style_idx, sx, sy, dx, dy) {
            let rgba = tinted_color(color, tint, alpha);
            let mut paint = Paint::default();
            paint.set_color(rgba);
            paint.anti_alias = true;
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
            drew_any = true;
        }
    }

    // ── Stroke pass ──────────────────────────────────────────────────────────
    for (ls_idx, line_style) in shape.line_styles.iter().enumerate() {
        let color = match line_style.fill_style() {
            swf::FillStyle::Color(c) => *c,
            _ => continue,
        };

        let ls_index = (ls_idx + 1) as u32;
        if let Some(path) = build_path_for_line(shape, ls_index, sx, sy, dx, dy) {
            let stroke_width_px =
                (line_style.width().to_pixels() as f32 * sx.abs()).max(0.5);
            let rgba = tinted_color(color, tint, alpha);
            let mut paint = Paint::default();
            paint.set_color(rgba);
            paint.anti_alias = true;
            let mut stroke = Stroke::default();
            stroke.width = stroke_width_px;
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            drew_any = true;
        }
    }

    drew_any
}

// ─────────────────────────────────────────────────────────────────────────────
// Path builders
// ─────────────────────────────────────────────────────────────────────────────

/// Build a `tiny-skia` `Path` containing all sub-paths whose `fill_style_1`
/// equals `target_fill_idx`.
fn build_path_for_fill(
    shape: &ShapeRecord,
    target_fill_idx: u32,
    sx: f32,
    sy: f32,
    dx: f32,
    dy: f32,
) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut active_fill1: u32 = 0;
    let mut active_fill0: u32 = 0;
    let mut in_sub = false;

    for rec in &shape.records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if in_sub {
                    pb.close();
                    in_sub = false;
                }
                if let Some(pt) = sc.move_to {
                    cx = pt.x.to_pixels() as f32 * sx + dx;
                    cy = pt.y.to_pixels() as f32 * sy + dy;
                }
                if let Some(fi) = sc.fill_style_1 {
                    active_fill1 = fi;
                }
                if let Some(fi) = sc.fill_style_0 {
                    active_fill0 = fi;
                }
                // Handle new style blocks (DefineShape3 inline styles).
                if let Some(new_styles) = &sc.new_styles {
                    // Reset to style 0 when a new style block is encountered.
                    let _ = new_styles; // we don't track them here
                    active_fill0 = 0;
                    active_fill1 = 0;
                }
                // Start a new sub-path if this style change sets our target fill.
                if active_fill1 == target_fill_idx || active_fill0 == target_fill_idx {
                    pb.move_to(cx, cy);
                    in_sub = true;
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                if active_fill1 != target_fill_idx && active_fill0 != target_fill_idx {
                    let ex = cx + delta.dx.to_pixels() as f32 * sx;
                    let ey = cy + delta.dy.to_pixels() as f32 * sy;
                    cx = ex;
                    cy = ey;
                    continue;
                }
                if !in_sub {
                    pb.move_to(cx, cy);
                    in_sub = true;
                }
                let ex = cx + delta.dx.to_pixels() as f32 * sx;
                let ey = cy + delta.dy.to_pixels() as f32 * sy;
                pb.line_to(ex, ey);
                cx = ex;
                cy = ey;
            }
            swf::ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                if active_fill1 != target_fill_idx && active_fill0 != target_fill_idx {
                    let ctrl_x = cx + control_delta.dx.to_pixels() as f32 * sx;
                    let ctrl_y = cy + control_delta.dy.to_pixels() as f32 * sy;
                    let anch_x = ctrl_x + anchor_delta.dx.to_pixels() as f32 * sx;
                    let anch_y = ctrl_y + anchor_delta.dy.to_pixels() as f32 * sy;
                    cx = anch_x;
                    cy = anch_y;
                    continue;
                }
                if !in_sub {
                    pb.move_to(cx, cy);
                    in_sub = true;
                }
                let ctrl_x = cx + control_delta.dx.to_pixels() as f32 * sx;
                let ctrl_y = cy + control_delta.dy.to_pixels() as f32 * sy;
                let anch_x = ctrl_x + anchor_delta.dx.to_pixels() as f32 * sx;
                let anch_y = ctrl_y + anchor_delta.dy.to_pixels() as f32 * sy;
                pb.quad_to(ctrl_x, ctrl_y, anch_x, anch_y);
                cx = anch_x;
                cy = anch_y;
            }
        }
    }
    if in_sub {
        pb.close();
    }

    pb.finish()
}

/// Build a `tiny-skia` `Path` containing all segments whose active `line_style`
/// equals `target_ls_idx`.
fn build_path_for_line(
    shape: &ShapeRecord,
    target_ls_idx: u32,
    sx: f32,
    sy: f32,
    dx: f32,
    dy: f32,
) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut active_ls: u32 = 0;
    let mut in_sub = false;

    for rec in &shape.records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if in_sub && active_ls != target_ls_idx {
                    in_sub = false;
                }
                if let Some(pt) = sc.move_to {
                    cx = pt.x.to_pixels() as f32 * sx + dx;
                    cy = pt.y.to_pixels() as f32 * sy + dy;
                    in_sub = false;
                }
                if let Some(ls) = sc.line_style {
                    active_ls = ls;
                }
                if let Some(_) = &sc.new_styles {
                    active_ls = 0;
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                let ex = cx + delta.dx.to_pixels() as f32 * sx;
                let ey = cy + delta.dy.to_pixels() as f32 * sy;
                if active_ls == target_ls_idx {
                    if !in_sub {
                        pb.move_to(cx, cy);
                        in_sub = true;
                    }
                    pb.line_to(ex, ey);
                }
                cx = ex;
                cy = ey;
            }
            swf::ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                let ctrl_x = cx + control_delta.dx.to_pixels() as f32 * sx;
                let ctrl_y = cy + control_delta.dy.to_pixels() as f32 * sy;
                let anch_x = ctrl_x + anchor_delta.dx.to_pixels() as f32 * sx;
                let anch_y = ctrl_y + anchor_delta.dy.to_pixels() as f32 * sy;
                if active_ls == target_ls_idx {
                    if !in_sub {
                        pb.move_to(cx, cy);
                        in_sub = true;
                    }
                    pb.quad_to(ctrl_x, ctrl_y, anch_x, anch_y);
                }
                cx = anch_x;
                cy = anch_y;
            }
        }
    }

    pb.finish()
}

// ─────────────────────────────────────────────────────────────────────────────
// Sprite placement helper
// ─────────────────────────────────────────────────────────────────────────────

/// Compute an effective dest rect for a shape placed inside a sprite, taking
/// the sprite's PlaceObject matrix into account.
///
/// The matrix is in SWF space (Fixed16 scale + Twips translation).  We map
/// the shape bounds through the matrix to get a transformed bounding box in
/// SWF pixels, then map that box into the dest rect proportionally.
fn apply_place_matrix_to_dest(
    shape: &ShapeRecord,
    matrix: &swf::Matrix,
    parent_dest: TskRect,
) -> TskRect {
    let b = &shape.shape_bounds;
    let bx0 = b.x_min.to_pixels() as f32;
    let by0 = b.y_min.to_pixels() as f32;
    let bx1 = b.x_max.to_pixels() as f32;
    let by1 = b.y_max.to_pixels() as f32;

    // Transform the four corners of the shape bounds through the SWF matrix.
    let corners = [(bx0, by0), (bx1, by0), (bx0, by1), (bx1, by1)];
    let a = matrix.a.to_f32();
    let b_coef = matrix.b.to_f32();
    let c = matrix.c.to_f32();
    let d = matrix.d.to_f32();
    let tx = matrix.tx.to_pixels() as f32;
    let ty = matrix.ty.to_pixels() as f32;

    let transformed: Vec<(f32, f32)> = corners
        .iter()
        .map(|&(x, y)| (a * x + c * y + tx, b_coef * x + d * y + ty))
        .collect();

    let mx0 = transformed.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let my0 = transformed.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let mx1 = transformed.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let my1 = transformed.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);

    let mw = (mx1 - mx0).max(1.0);
    let mh = (my1 - my0).max(1.0);

    // Parent dest maps the parent sprite's stage area; we don't have a
    // separate stage rect here, so we use it as the viewport.  The transformed
    // bounding box is mapped into parent_dest proportionally.
    let pw = parent_dest.width().max(1.0);
    let ph = parent_dest.height().max(1.0);

    // We need to know the parent SWF viewport — approximate with the shape's
    // own bounds since we lack the sprite's stage size here.
    // The most defensible approach is to use the same shape→dest mapping
    // as draw_shape would, composed with the matrix.
    let sx = parent_dest.width() / (bx1 - bx0).max(1.0);
    let sy = parent_dest.height() / (by1 - by0).max(1.0);

    let x0_px = parent_dest.left() + (mx0 - bx0) * sx;
    let y0_px = parent_dest.top() + (my0 - by0) * sy;
    let w_px = (mw * sx).max(1.0);
    let h_px = (mh * sy).max(1.0);
    let _ = (pw, ph); // suppress unused warning

    TskRect::from_xywh(x0_px, y0_px, w_px, h_px).unwrap_or(parent_dest)
}

// ─────────────────────────────────────────────────────────────────────────────
// Colour helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Apply tint/alpha to a SWF `Color`, returning a `tiny-skia` `Color`.
///
/// Tinting rule:
/// - If `tint` is opaque white, use the SWF authored colour unchanged.
/// - If the SWF fill colour is white (255,255,255) and `tint` is not opaque
///   white, multiply by `tint` (recolour mode).
/// - Otherwise use the SWF colour as-is.
fn tinted_color(swf_color: swf::Color, tint: Color, alpha: f32) -> Color {
    let is_tint_white = tint.red() >= 1.0 && tint.green() >= 1.0 && tint.blue() >= 1.0;
    let swf_is_white =
        swf_color.r == 255 && swf_color.g == 255 && swf_color.b == 255;

    let (r, g, b) = if !is_tint_white && swf_is_white {
        (tint.red(), tint.green(), tint.blue())
    } else {
        (
            swf_color.r as f32 / 255.0,
            swf_color.g as f32 / 255.0,
            swf_color.b as f32 / 255.0,
        )
    };

    let a = (swf_color.a as f32 / 255.0) * alpha.clamp(0.0, 1.0);
    Color::from_rgba(r, g, b, a).unwrap_or(Color::TRANSPARENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::IntSize;

    /// Build a minimal SWF with a single 100×100 red rectangle exported as `"test_shape"`.
    fn make_exported_rect_swf() -> Vec<u8> {
        use swf::*;

        let header = Header {
            compression: Compression::None,
            version: 6,
            stage_size: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(100.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(100.0),
            },
            frame_rate: Fixed8::from_f32(24.0),
            num_frames: 1,
        };

        let shape = Shape {
            version: 1,
            id: 1,
            shape_bounds: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(100.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(100.0),
            },
            edge_bounds: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(100.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(100.0),
            },
            flags: ShapeFlag::empty(),
            styles: ShapeStyles {
                fill_styles: vec![FillStyle::Color(Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                })],
                line_styles: vec![],
            },
            shape: vec![
                ShapeRecord::StyleChange(Box::new(StyleChangeData {
                    move_to: Some(Point::new(Twips::ZERO, Twips::ZERO)),
                    fill_style_0: None,
                    fill_style_1: Some(1),
                    line_style: None,
                    new_styles: None,
                })),
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::from_pixels(100.0), Twips::ZERO),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(100.0)),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::from_pixels(-100.0), Twips::ZERO),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(-100.0)),
                },
            ],
        };

        let export: Vec<ExportedAsset<'_>> = vec![ExportedAsset {
            id: 1,
            name: SwfStr::from_utf8_str("test_shape"),
        }];

        let tags = [
            Tag::DefineShape(shape),
            Tag::ExportAssets(export),
            Tag::ShowFrame,
        ];

        let mut buf = Vec::new();
        swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
        buf
    }

    #[test]
    fn red_rect_shape_rasterises_to_red_pixels() {
        let swf_bytes = make_exported_rect_swf();
        let assets = SwfAssetLibrary::new(swf_bytes).expect("SwfAssetLibrary::new");

        let size = IntSize::from_wh(100, 100).unwrap();
        let mut pixmap = Pixmap::new(size.width(), size.height()).unwrap();

        let dest = TskRect::from_xywh(0.0, 0.0, 100.0, 100.0).unwrap();
        let white = Color::from_rgba8(255, 255, 255, 255);
        let drew = draw_swf_symbol(&mut pixmap, &assets, "test_shape", dest, white, 1.0);

        assert!(drew, "draw_swf_symbol returned false for test_shape");

        // Sample the centre pixel — should be red.
        let data = pixmap.data();
        // tiny-skia stores premultiplied RGBA; red=255 alpha=255 → [255,0,0,255]
        let idx = (50 * 100 + 50) * 4;
        let r = data[idx];
        let g = data[idx + 1];
        let b = data[idx + 2];
        let a = data[idx + 3];

        assert!(r > 200, "expected red centre pixel, got r={r} g={g} b={b} a={a}");
        assert!(g < 50, "expected red centre pixel, got r={r} g={g} b={b} a={a}");
        assert!(b < 50, "expected red centre pixel, got r={r} g={g} b={b} a={a}");
        assert!(a > 200, "expected opaque centre pixel, got a={a}");
    }

    #[test]
    fn missing_symbol_returns_false() {
        let swf_bytes = make_exported_rect_swf();
        let assets = SwfAssetLibrary::new(swf_bytes).expect("SwfAssetLibrary::new");
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        let dest = TskRect::from_xywh(0.0, 0.0, 64.0, 64.0).unwrap();
        let white = Color::from_rgba8(255, 255, 255, 255);
        let drew = draw_swf_symbol(&mut pixmap, &assets, "no_such_symbol", dest, white, 1.0);
        assert!(!drew);
    }

    #[test]
    fn white_fill_is_tinted_when_tint_is_not_white() {
        use swf::*;

        // Build SWF with a white-filled rectangle.
        let header = Header {
            compression: Compression::None,
            version: 6,
            stage_size: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(10.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(10.0),
            },
            frame_rate: Fixed8::from_f32(24.0),
            num_frames: 1,
        };
        let shape = Shape {
            version: 1,
            id: 2,
            shape_bounds: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(10.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(10.0),
            },
            edge_bounds: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(10.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(10.0),
            },
            flags: ShapeFlag::empty(),
            styles: ShapeStyles {
                fill_styles: vec![FillStyle::Color(Color {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 255,
                })],
                line_styles: vec![],
            },
            shape: vec![
                ShapeRecord::StyleChange(Box::new(StyleChangeData {
                    move_to: Some(Point::new(Twips::ZERO, Twips::ZERO)),
                    fill_style_0: None,
                    fill_style_1: Some(1),
                    line_style: None,
                    new_styles: None,
                })),
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::from_pixels(10.0), Twips::ZERO),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(10.0)),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::from_pixels(-10.0), Twips::ZERO),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(-10.0)),
                },
            ],
        };
        let export: Vec<swf::ExportedAsset<'_>> = vec![swf::ExportedAsset {
            id: 2,
            name: swf::SwfStr::from_utf8_str("white_rect"),
        }];
        let tags = [
            Tag::DefineShape(shape),
            Tag::ExportAssets(export),
            Tag::ShowFrame,
        ];
        let mut buf = Vec::new();
        swf::write_swf(&header, &tags, &mut buf).expect("write_swf");
        let assets = SwfAssetLibrary::new(buf).expect("SwfAssetLibrary");
        let mut pixmap = Pixmap::new(10, 10).unwrap();
        let dest = TskRect::from_xywh(0.0, 0.0, 10.0, 10.0).unwrap();
        // Amber tint — use fully qualified path since `use swf::*` shadows `tiny_skia::Color`.
        let amber = tiny_skia::Color::from_rgba8(240, 168, 104, 255);
        let drew = draw_swf_symbol(&mut pixmap, &assets, "white_rect", dest, amber, 1.0);
        assert!(drew);

        // Centre pixel should be amber-ish (high red, medium green, low blue).
        let data = pixmap.data();
        let idx = (5 * 10 + 5) * 4;
        let r = data[idx];
        let g = data[idx + 1];
        let b = data[idx + 2];
        assert!(r > 180, "expected amber-red, got r={r} g={g} b={b}");
        assert!(g > 100 && g < 220, "expected amber-green, got r={r} g={g} b={b}");
        assert!(b < 150, "expected amber-blue, got r={r} g={g} b={b}");
    }
}
