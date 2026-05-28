//! Canonical UI IR renderer for generic BuildingBlocks output.
//! GOLDEN RULE: No hard-coding, heuristic workarounds, no procedural fallbacks. Avoid targetted scoping. Find the root cause and fix issues instead. Find the source data even it means doing things the hard way. This is intended to be a pipeline that is completely generic that can work for any UI on any ship and must not have targetted hacks that won't fix the issue in other places. This will keep the code lean and generic. Think how the game-engine would implement it.
//!
//! This module is the first Phase 2 step toward deterministic renderer
//! consumption of [`crate::ui_ir::UiIrDocument`]. It renders the generic BB
//! path directly from IR fields that were materialized in Phase 1: layout,
//! fill colours, borders, asset references, and resolved text payload/style.
//!

use image::RgbaImage;
use image::imageops;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
use tiny_skia::{BlendMode, Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect as TskRect, Stroke, Transform};

use crate::bb_atlas::AtlasLibrary;
use crate::bb_assets::UiAssetResolver;
use crate::bb_layout::Rect;
use crate::compose::ComposeContext;
use crate::error::UiError;
use crate::text::{FontKind, TextAlign, TextRenderer, VerticalAlign};
use crate::swf_assets::FontGlyphSet;
use crate::ui_ir::{
    validate_ui_ir_document, UiIrBorder, UiIrColourBlendMode, UiIrDocument, UiIrNode, UiIrRect,
    UiIrTextPayload, UiIrTextStyle, UiIrValue,
};

// BB/Flash nominal font sizes render visually smaller with the bundled DejaVu
// fallback fonts. Calibrate at compose time to match measured output.
const TEXT_RENDER_SIZE_CALIBRATION: f32 = 1.5;
const SWF_TEXT_RENDER_SIZE_CALIBRATION: f32 = 0.84;
const PAINT_FILE_SWF_SIZE_CALIBRATION: f32 = 0.84;
const LOW_REG_SWF_SIZE_CALIBRATION: f32 = 3.45;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugTextRects {
    pub primary: Rect,
    pub secondary: Option<Rect>,
    pub primary_text_origin: (f32, f32),
    pub secondary_text_origin: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugTextDrawnBounds {
    pub primary: Rect,
    pub secondary: Option<Rect>,
}

/// Render a generic BuildingBlocks IR document without consulting raw BB data.
///
/// This renderer intentionally consumes only the canonical IR plus style/assets.
/// SWF-specific and raw-source fallback behavior remains in the legacy renderer
/// until the later Phase 2 split is complete.
pub fn render_ui_ir_document(
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
) -> Result<RgbaImage, UiError> {
    validate_ui_ir_document(document)
        .map_err(|errors| UiError::RenderError(format!("invalid UI IR: {}", errors.join("; "))))?;

    if document.target_width == 0 || document.target_height == 0 {
        return Err(UiError::RenderError(format!(
            "invalid target size {}x{}",
            document.target_width, document.target_height
        )));
    }

    let mut pixmap = Pixmap::new(document.target_width, document.target_height)
        .ok_or_else(|| UiError::RenderError("pixmap allocation failed".into()))?;

    let bg = &ctx.style.background;
    pixmap.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    let mut draw_order: Vec<&UiIrNode> = document.nodes.iter().filter(|node| node.is_active).collect();
    // Keep authored IR order within each layer. Synthetic ids are an
    // implementation detail and must not affect visual stacking.
    draw_order.sort_by_key(|node| node.layer);

    let text_renderer = TextRenderer::new();

    for node in &draw_order {
        draw_non_text_node(node, document, ctx, atlas, &mut pixmap);
    }

    // Keep progress meters on top of base chrome/background fills.
    for node in &draw_order {
        if node.meter_progress.is_none() {
            continue;
        }
        let rect = resolved_linear_progress_meter_rect(node, document)
            .unwrap_or_else(|| ir_rect_to_layout_rect(node.computed_rect));
        let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
            continue;
        };
        draw_linear_progress_meter(node, ctx, &mut pixmap, tsk_rect);
    }

    let mut img = pixmap_to_rgba_image(pixmap)?;
    let mut text_draw_order = draw_order.clone();
    text_draw_order.sort_by(|left, right| {
        let left_rect = ir_rect_to_layout_rect(left.computed_rect);
        let right_rect = ir_rect_to_layout_rect(right.computed_rect);
        let left_key = (
            left_rect.x.round() as i32,
            left_rect.y.round() as i32,
            left_rect.w.round() as i32,
            left_rect.h.round() as i32,
        );
        let right_key = (
            right_rect.x.round() as i32,
            right_rect.y.round() as i32,
            right_rect.w.round() as i32,
            right_rect.h.round() as i32,
        );
        let left_len = resolved_text_payload(left).map(|text| text.len()).unwrap_or(usize::MAX);
        let right_len = resolved_text_payload(right)
            .map(|text| text.len())
            .unwrap_or(usize::MAX);

        left_key
            .cmp(&right_key)
            .then(left_len.cmp(&right_len))
            .then(left.id.cmp(&right.id))
    });

    let mut seen_text_rects: HashSet<(i32, i32, i32, i32)> = HashSet::new();
    for node in &text_draw_order {
        draw_text_node(
            &mut img,
            node,
            document,
            &text_renderer,
            ctx,
            &mut seen_text_rects,
        );
    }

    Ok(img)
}

fn draw_non_text_node(
    node: &UiIrNode,
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
) {
    let rect = resolved_linear_progress_meter_rect(node, document)
        .unwrap_or_else(|| ir_rect_to_layout_rect(node.computed_rect));
    if rect.w < 0.5 || rect.h < 0.5 {
        return;
    }

    let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
        return;
    };

    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetCircle")
        || node.node_type.eq_ignore_ascii_case("widget_circle")
    {
        draw_widget_circle(node, pixmap, tsk_rect);
        return;
    }

    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetSeparator")
        || node.node_type.eq_ignore_ascii_case("widget_separator")
    {
        draw_widget_separator(ctx, pixmap, node, tsk_rect, node.alpha);
        return;
    }

    if node.meter_progress.is_some() {
        draw_linear_progress_meter(node, ctx, pixmap, tsk_rect);
        return;
    }

    if node
        .node_type
        .eq_ignore_ascii_case("component_general_button_secondary")
        && node
            .icon_preset
            .as_deref()
            .is_some_and(|preset| preset.eq_ignore_ascii_case("GeneralX"))
    {
        draw_secondary_close_button(ctx, pixmap, node, tsk_rect, node.alpha);
        return;
    }

    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetManufacturerLogo")
    {
        draw_manufacturer_logo_ir(node, document, ctx, atlas, pixmap);
    }

    let skip_background_fill = node.custom_shape.is_some() && node.asset_ref.is_some();
    if !skip_background_fill {
        let resolved_background_fill = node.background_fill_colour.or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
                .map(|mut fill| {
                    fill[3] *= node.background_fill_alpha.unwrap_or(1.0);
                    fill
                })
        });
        if let Some(fill) = resolved_background_fill
            && fill[3] > 0.005
        {
            if node.name.eq_ignore_ascii_case("Root")
                && node.node_type.eq_ignore_ascii_case("display_widget")
                && rect.w >= document.target_width as f32 * 0.95
                && rect.h <= document.target_height as f32 * 0.16
            {
                // Header root containers are layout scaffolding, not visible chrome.
            } else {
                fill_rect_ts_with_mode(pixmap, tsk_rect, fill, node.alpha, node_colour_blend_mode(node));
            }
        }
    }

    if let Some(asset_ref) = node.asset_ref.as_deref() {
        let normalised_asset_ref = UiAssetResolver::normalise_path(asset_ref);
        let iw = rect.w.round().max(1.0) as u32;
        let ih = rect.h.round().max(1.0) as u32;
        let fill_override = custom_shape_fill_override(node, ctx).or_else(|| {
            if normalised_asset_ref.ends_with(".svg") {
                node.icon_tint_colour.or_else(|| {
                    node.icon_tint_colour_token
                        .as_deref()
                        .and_then(|token| resolve_colour_token(ctx, token))
                })
            } else {
                None
            }
        });
        let resolved_image = if UiAssetResolver::is_reference_overlay(asset_ref)
            || UiAssetResolver::is_reference_overlay(&normalised_asset_ref)
        {
            None
        } else if normalised_asset_ref.ends_with(".svg") {
            atlas
                .fetch_raw(asset_ref)
                .or_else(|| {
                    (normalised_asset_ref != asset_ref)
                        .then(|| atlas.fetch_raw(&normalised_asset_ref))
                        .flatten()
                })
                .and_then(|svg_bytes| rasterize_svg_for_node(node, &svg_bytes, iw, ih, fill_override))
        } else {
            atlas.resolve(asset_ref, iw, ih).or_else(|| {
                (normalised_asset_ref != asset_ref)
                    .then(|| atlas.resolve(&normalised_asset_ref, iw, ih))
                    .flatten()
            })
        };
        if let Some(mut img) = resolved_image {
            let is_nine_slice_custom_shape = node
                .custom_shape
                .as_ref()
                .is_some_and(|shape| shape.enable_nine_slice_rect.unwrap_or(false));
            if normalised_asset_ref.ends_with(".svg")
                && fill_override.is_some()
                && !is_nine_slice_custom_shape
            {
                img = strip_custom_shape_uniform_matte(&img);
            }
            img = apply_asset_layout_flip(node, img);
            let draw_x = rect.x as i32;
            let draw_y = rect.y as i32;
            let tint = image_tint_for_blit(node, asset_ref, fill_override, ctx);

            let blend_mode = image_blend_mode_for_node(node, asset_ref);

            blit_atlas_image_tinted_with_mode(pixmap, &img, draw_x, draw_y, tint, node.alpha, blend_mode);
        }
    }

    if let Some(border) = &node.border {
        draw_ir_border(pixmap, rect, border, node.alpha, ctx);
    }

    if let Some(stroke_colour) = node.stroke_colour
        && node.stroke_extent.unwrap_or(0.0) > 0.0
        && !node
            .custom_shape
            .as_ref()
            .and_then(|shape| shape.render_shape)
            .unwrap_or(false)
    {
        draw_rect_stroke_ts(
            pixmap,
            tsk_rect,
            stroke_colour,
            node.stroke_extent.unwrap_or(0.0),
            node.alpha,
        );
    }
}

fn brand_slug_from_ir(document: &UiIrDocument, ctx: &ComposeContext<'_>) -> String {
    let source = document.selected_style_source.as_deref().unwrap_or(&ctx.style.name);
    let token = source
        .split(':')
        .next_back()
        .unwrap_or(source)
        .trim()
        .trim_start_matches("s_")
        .to_ascii_lowercase();

    token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
}

fn draw_manufacturer_logo_ir(
    node: &UiIrNode,
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
) {
    let draw_rect = ir_rect_to_layout_rect(node.computed_rect);
    if draw_rect.w < 0.5 || draw_rect.h < 0.5 {
        return;
    }

    let brand = brand_slug_from_ir(document, ctx);
    let logo_brand = brand_logo_slug(&brand);
    let brand_title = brand_title(logo_brand);
    let candidates = [
        format!("UI/Textures/Signs/Brands/{logo_brand}/{brand_title}_logo.svg"),
        format!("UI/Textures/Vector/General/BrandLogos/logo_{logo_brand}_a.svg"),
        format!("UI/Textures/Signs/Brands/{logo_brand}/{brand_title}_logo.dds"),
    ];

    let iw = draw_rect.w.round().max(1.0) as u32;
    let ih = draw_rect.h.round().max(1.0) as u32;
    let tint = manufacturer_logo_tint(node, ctx);
    let fill_override = Some(tint);

    for raw_path in candidates {
        let norm = UiAssetResolver::normalise_path(&raw_path);
        if UiAssetResolver::is_reference_overlay(&norm) {
            continue;
        }

        if norm.ends_with(".svg") {
            if let Some(svg_bytes) = atlas.fetch_raw(&norm)
                && let Some(img) = crate::bb_svg::rasterize_svg(&svg_bytes, iw, ih, fill_override)
            {
                let draw_y = draw_rect.y as i32 - vertical_alpha_balance_offset(&img);
                blit_atlas_image_tinted(
                    pixmap,
                    &img,
                    draw_rect.x as i32,
                    draw_y,
                    [1.0, 1.0, 1.0, 1.0],
                    node.alpha,
                );
                return;
            }
            continue;
        }

        if let Some(img) = atlas.resolve(&norm, iw, ih) {
            blit_atlas_image_tinted(
                pixmap,
                &img,
                draw_rect.x as i32,
                draw_rect.y as i32,
                tint,
                node.alpha,
            );
            return;
        }
    }
}

fn manufacturer_logo_tint(node: &UiIrNode, ctx: &ComposeContext<'_>) -> [f32; 4] {
    node.icon_tint_colour
        .or_else(|| {
            node.icon_tint_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.background_fill_colour)
        .or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.stroke_colour)
        .or_else(|| {
            node.stroke_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .unwrap_or_else(|| derived_accent_tint(ctx))
}

fn vertical_alpha_balance_offset(img: &RgbaImage) -> i32 {
    let mut top = 0i32;
    let mut bottom = 0i32;

    for y in 0..img.height() {
        let has_alpha = (0..img.width()).any(|x| img.get_pixel(x, y)[3] > 0);
        if has_alpha {
            break;
        }
        top += 1;
    }

    for y in (0..img.height()).rev() {
        let has_alpha = (0..img.width()).any(|x| img.get_pixel(x, y)[3] > 0);
        if has_alpha {
            break;
        }
        bottom += 1;
    }

    (top - bottom) / 2
}

fn brand_logo_slug(slug: &str) -> &str {
    match slug {
        "bioc" => "bioticorp",
        other => other,
    }
}

fn brand_title(slug: &str) -> String {
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn resolve_colour_token(ctx: &ComposeContext<'_>, token: &str) -> Option<[f32; 4]> {
    let key = token.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }

    match key.as_str() {
        "accent1" | "base" | "bright" => style_colour_slot_rgba(ctx, 0).or_else(|| Some(style_primary_rgba(ctx))),
        "accent2" | "positive" | "success" => {
            let primary = style_primary_rgba(ctx);
            if let Some(slot_2) = style_colour_slot_rgba(ctx, 2)
                && rgba4_near(slot_2, primary)
            {
                return Some(slot_2);
            }
            style_colour_slot_rgba(ctx, 1).or_else(|| Some(primary))
        },
        "accent3" | "warning" => style_colour_slot_rgba(ctx, 2),
        "accent4" | "critical" | "negative" => style_colour_slot_rgba(ctx, 3).or(Some([1.0, 0.2, 0.2, 1.0])),
        "accent5" | "contactunknown" => style_colour_slot_rgba(ctx, 4),
        "mid" | "contactneutral" => style_colour_slot_rgba(ctx, 5),
        "light" | "disabled" => style_colour_slot_rgba(ctx, 6),
        "highlight" | "contactpositiverep" => style_colour_slot_rgba(ctx, 7),
        "primarytext" | "text" | "foreground" | "fg" | "white" => Some([1.0, 1.0, 1.0, 1.0]),
        "background" => Some([
            ctx.style.background.r as f32 / 255.0,
            ctx.style.background.g as f32 / 255.0,
            ctx.style.background.b as f32 / 255.0,
            ctx.style.background.a as f32 / 255.0,
        ]),
        "secondary" => ctx.style.secondary_tint.as_ref().map(|colour| {
            [
                colour.r as f32 / 255.0,
                colour.g as f32 / 255.0,
                colour.b as f32 / 255.0,
                colour.a as f32 / 255.0,
            ]
        }),
        "surface" => style_colour_slot_rgba(ctx, 8),
        "bg" => style_colour_slot_rgba(ctx, 9),
        "backlight" | "contactagressive" | "contactaggressive" => style_colour_slot_rgba(ctx, 11),
        _ => None,
    }
}

fn resolve_surface_colour_token(ctx: &ComposeContext<'_>, token: &str) -> Option<[f32; 4]> {
    let key = token.trim().to_ascii_lowercase();
    match key.as_str() {
        "accent1" | "accent5" | "contactunknown" => style_colour_slot_rgba(ctx, 4),
        _ => resolve_colour_token(ctx, token),
    }
}

fn style_colour_slot_rgba(ctx: &ComposeContext<'_>, index: usize) -> Option<[f32; 4]> {
    ctx.style.colour_slots.get(index).map(|colour| {
        [
            colour.r as f32 / 255.0,
            colour.g as f32 / 255.0,
            colour.b as f32 / 255.0,
            colour.a as f32 / 255.0,
        ]
    })
}

fn rgba4_near(left: [f32; 4], right: [f32; 4]) -> bool {
    (left[0] - right[0]).abs() < 0.0001
        && (left[1] - right[1]).abs() < 0.0001
        && (left[2] - right[2]).abs() < 0.0001
        && (left[3] - right[3]).abs() < 0.0001
}

fn custom_shape_fill_override(node: &UiIrNode, ctx: &ComposeContext<'_>) -> Option<[f32; 4]> {
    let render_shape = node
        .custom_shape
        .as_ref()
        .and_then(|shape| shape.render_shape)
        .unwrap_or(false);
    if !render_shape {
        return None;
    }

    custom_shape_style_tag_colour_token(node)
        .and_then(|token| resolve_colour_token(ctx, token))
        .or(node.icon_tint_colour)
        .or_else(|| {
            node.icon_tint_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.background_fill_colour)
        .or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.stroke_colour)
        .or_else(|| {
            node.stroke_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
}

fn image_tint_for_blit(
    node: &UiIrNode,
    asset_ref: &str,
    fill_override: Option<[f32; 4]>,
    ctx: &ComposeContext<'_>,
) -> [f32; 4] {
    if fill_override.is_some()
        && UiAssetResolver::normalise_path(asset_ref).ends_with(".svg")
    {
        [1.0, 1.0, 1.0, 1.0]
    } else {
        node.icon_tint_colour
            .or_else(|| {
                node.icon_tint_colour_token
                    .as_deref()
                    .and_then(|token| resolve_colour_token(ctx, token))
            })
            .unwrap_or([1.0, 1.0, 1.0, 1.0])
    }
}

fn image_blend_mode_for_node(node: &UiIrNode, asset_ref: &str) -> BlendMode {
    let render_shape = node
        .custom_shape
        .as_ref()
        .and_then(|shape| shape.render_shape)
        .unwrap_or(false);
    if !render_shape {
        return BlendMode::SourceOver;
    }

    if UiAssetResolver::normalise_path(asset_ref).ends_with(".svg") {
        if custom_shape_has_modify_tag(node) {
            BlendMode::Plus
        } else {
            BlendMode::SourceOver
        }
    } else {
        BlendMode::Plus
    }
}

fn custom_shape_has_modify_tag(node: &UiIrNode) -> bool {
    node.resolved_style_tags.iter().any(|tag| {
        tag.tag_name
            .as_deref()
            .is_some_and(|name| name.trim().eq_ignore_ascii_case("modify"))
    })
}

fn rasterize_custom_shape_svg(
    node: &UiIrNode,
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
) -> Option<RgbaImage> {
    let Some(shape) = node.custom_shape.as_ref() else {
        return crate::bb_svg::rasterize_svg(svg_bytes, target_w, target_h, fill_override);
    };

    let rendered = if shape.enable_nine_slice_rect.unwrap_or(false)
        && let Some(nine_slice_rect) = shape.nine_slice_rect
    {
        crate::bb_svg::rasterize_svg_nine_slice(
            svg_bytes,
            target_w,
            target_h,
            fill_override,
            nine_slice_rect,
            shape.nine_slice_scale.unwrap_or(1.0),
        )
    } else {
        crate::bb_svg::rasterize_svg(svg_bytes, target_w, target_h, fill_override)
    }?;

    if shape.render_shape.unwrap_or(false) {
        Some(expand_nontransparent_bounds_to_target(rendered, target_w, target_h))
    } else {
        Some(rendered)
    }
}

fn expand_nontransparent_bounds_to_target(img: RgbaImage, target_w: u32, target_h: u32) -> RgbaImage {
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 {
        return img;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for y in 0..height {
        for x in 0..width {
            if img.get_pixel(x, y)[3] > 0 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if !found {
        return img;
    }

    if min_x == 0 && min_y == 0 && max_x + 1 == width && max_y + 1 == height {
        return img;
    }

    let crop_w = max_x - min_x + 1;
    let crop_h = max_y - min_y + 1;
    let cropped = imageops::crop_imm(&img, min_x, min_y, crop_w, crop_h).to_image();
    imageops::resize(&cropped, target_w.max(1), target_h.max(1), imageops::FilterType::Lanczos3)
}

fn rasterize_svg_for_node(
    node: &UiIrNode,
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
) -> Option<RgbaImage> {
    // Keep custom-shape SVGs on the custom-shape pipeline so nine-slice and
    // authored shape behavior stay stable. Contain scaling is only used for
    // regular asset-backed widgets.
    if node.custom_shape.is_none()
        && node
            .asset_layout
            .as_ref()
            .and_then(|layout| layout.scaling_behavior.as_deref())
            .is_some_and(|behavior| behavior.eq_ignore_ascii_case("Contain"))
    {
        let layout = node.asset_layout.as_ref().expect("contain layout");
        return crate::bb_svg::rasterize_svg_contained(
            svg_bytes,
            target_w,
            target_h,
            fill_override,
            layout.contain_position_x.unwrap_or(0.5),
            layout.contain_position_y.unwrap_or(0.5),
        );
    }

    rasterize_custom_shape_svg(node, svg_bytes, target_w, target_h, fill_override)
}

fn apply_asset_layout_flip(node: &UiIrNode, image: RgbaImage) -> RgbaImage {
    let Some(layout) = node.asset_layout.as_ref() else {
        return image;
    };

    let mut out = image;
    if layout.flip_horizontal.unwrap_or(false) {
        out = imageops::flip_horizontal(&out);
    }
    if layout.flip_vertical.unwrap_or(false) {
        out = imageops::flip_vertical(&out);
    }
    out
}

fn strip_custom_shape_uniform_matte(img: &RgbaImage) -> RgbaImage {
    let (width, height) = img.dimensions();
    let total_pixels = (width as usize).saturating_mul(height as usize).max(1);

    let mut opaque_counts: HashMap<[u8; 4], usize> = HashMap::new();
    let mut transparent_count = 0usize;
    for chunk in img.as_raw().chunks_exact(4) {
        let px = [chunk[0], chunk[1], chunk[2], chunk[3]];
        if px[3] == 0 {
            transparent_count += 1;
        }
        if px[3] == 255 {
            *opaque_counts.entry(px).or_insert(0) += 1;
        }
    }

    if opaque_counts.is_empty() || transparent_count == 0 {
        return img.clone();
    }

    let Some((matte_px, matte_count)) = opaque_counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(px, count)| (*px, *count))
    else {
        return img.clone();
    };

    let total_opaque: usize = opaque_counts.values().sum();
    let matte_fraction = matte_count as f32 / total_opaque.max(1) as f32;
    let full_image_fraction = matte_count as f32 / total_pixels as f32;
    if matte_fraction < 0.85 || full_image_fraction < 0.9 {
        return img.clone();
    }

    let is_matte = |px: image::Rgba<u8>| {
        px[0] == matte_px[0] && px[1] == matte_px[1] && px[2] == matte_px[2] && px[3] == matte_px[3]
    };

    // Only strip matte that is actually connected to the asset border. This
    // avoids deleting centered monochrome logos where the dominant opaque colour
    // is the glyph itself, not an authored matte backdrop.
    let mut queue: VecDeque<(u32, u32)> = VecDeque::new();
    let mut visited = vec![false; total_pixels];
    let push_seed = |x: u32, y: u32, queue: &mut VecDeque<(u32, u32)>, visited: &mut [bool]| {
        let idx = (y as usize)
            .saturating_mul(width as usize)
            .saturating_add(x as usize);
        if !visited[idx] && is_matte(*img.get_pixel(x, y)) {
            visited[idx] = true;
            queue.push_back((x, y));
        }
    };

    for x in 0..width {
        push_seed(x, 0, &mut queue, &mut visited);
        if height > 1 {
            push_seed(x, height - 1, &mut queue, &mut visited);
        }
    }
    for y in 0..height {
        push_seed(0, y, &mut queue, &mut visited);
        if width > 1 {
            push_seed(width - 1, y, &mut queue, &mut visited);
        }
    }

    if queue.is_empty() {
        return img.clone();
    }

    while let Some((x, y)) = queue.pop_front() {
        if x > 0 {
            let nx = x - 1;
            let ny = y;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
        if x + 1 < width {
            let nx = x + 1;
            let ny = y;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
        if y > 0 {
            let nx = x;
            let ny = y - 1;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
        if y + 1 < height {
            let nx = x;
            let ny = y + 1;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
    }

    let mut out = img.clone();
    for y in 0..height {
        for x in 0..width {
            let idx = (y as usize)
                .saturating_mul(width as usize)
                .saturating_add(x as usize);
            if visited[idx] {
                let px = out.get_pixel_mut(x, y);
                *px = image::Rgba([0, 0, 0, 0]);
            }
        }
    }

    out
}

fn draw_linear_progress_meter(
    node: &UiIrNode,
    ctx: &ComposeContext<'_>,
    pixmap: &mut Pixmap,
    rect: TskRect,
) {
    let glow = [
        ctx.style.backlight.r as f32 / 255.0,
        ctx.style.backlight.g as f32 / 255.0,
        ctx.style.backlight.b as f32 / 255.0,
        (ctx.style.backlight.a as f32 / 255.0).max(0.8),
    ];

    let progress = node.meter_progress.unwrap_or(1.0).clamp(0.0, 1.0);
    if progress <= 0.0 {
        return;
    }

    if let Some(segmented_fill) = node.segmented_fill.as_ref().filter(|fill| fill.enabled) {
        let segment_width = if segmented_fill.segment_spacing_size > 0.0 {
            segmented_fill.segment_spacing_size
        } else {
            segmented_fill.segment_size
        }
        .max(0.0);
        let segment_gap = segmented_fill.segment_size.max(0.0);
        let segment_stride = segment_width + segment_gap;
        let segment_count = segmented_count_for_width(rect.width(), segment_width, segment_gap);
        if segment_count > 0 && segment_stride > 0.0 {
            let active_width = rect.width() * progress;
            let segment_colour = segmented_fill.segment_colour.unwrap_or(glow);
            for idx in 0..segment_count {
                let x = rect.x() + segmented_fill.segment_x_offset + (idx as f32 * segment_stride);
                if x >= rect.right() {
                    break;
                }
                let right = (x + segment_width).min(rect.right());
                if right <= x {
                    continue;
                }
                let segment_end = right - rect.x();
                if segment_end <= active_width {
                    if let Some(segment_rect) =
                        TskRect::from_xywh(x, rect.y(), right - x, rect.height())
                    {
                        fill_rect_ts(pixmap, segment_rect, segment_colour, node.alpha);
                    }
                }
            }
            return;
        }
    }

    let filled_w = (rect.width() * progress).max(1.0);
    if let Some(fill_rect) = TskRect::from_xywh(rect.x(), rect.y(), filled_w, rect.height()) {
        fill_rect_ts(pixmap, fill_rect, glow, node.alpha);
    }
}

pub fn debug_linear_progress_meter_rect(node: &UiIrNode, document: &UiIrDocument) -> Option<Rect> {
    (node.meter_progress.is_some()).then(|| {
        resolved_linear_progress_meter_rect(node, document)
            .unwrap_or_else(|| ir_rect_to_layout_rect(node.computed_rect))
    })
}

pub fn debug_node_draw_rect(node: &UiIrNode, document: &UiIrDocument) -> Rect {
    if let Some(meter_rect) = debug_linear_progress_meter_rect(node, document) {
        return meter_rect;
    }

    ir_rect_to_layout_rect(node.computed_rect)
}

fn resolved_linear_progress_meter_rect(node: &UiIrNode, document: &UiIrDocument) -> Option<Rect> {
    if node.meter_progress.is_none() {
        return None;
    }

    let parent = node
        .parent_id
        .and_then(|parent_id| document.nodes.iter().find(|candidate| candidate.id == parent_id))?;
    if !parent
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
    {
        return None;
    }
    if (node.anchor[1] - 1.0).abs() > 0.01 || node.pivot[1].abs() > 0.01 || node.authored_position[1].abs() > 0.01 {
        return None;
    }

    let mut rect = ir_rect_to_layout_rect(node.computed_rect);
    let text_rects = debug_text_rects(parent)?;
    let (pair_offset_x, _pair_offset_y) = right_anchored_label_caption_pair_offset(
        parent,
        text_rects.primary.h,
        text_rects.secondary.map(|secondary_rect| secondary_rect.h),
    );
    rect.x += pair_offset_x;
    rect.y = text_rects
        .secondary
        .map(|secondary_rect| secondary_rect.y + secondary_rect.h)
        .unwrap_or_else(|| text_rects.primary.y + text_rects.primary.h);
    Some(rect)
}

fn segmented_count_for_width(total_width: f32, segment_width: f32, segment_gap: f32) -> usize {
    if total_width <= 0.0 || segment_width <= 0.0 {
        return 0;
    }
    let stride = segment_width + segment_gap.max(0.0);
    if stride <= 0.0 {
        return 0;
    }
    (total_width / stride).floor().max(0.0) as usize
}

fn draw_secondary_close_button(
    ctx: &ComposeContext<'_>,
    pixmap: &mut Pixmap,
    node: &UiIrNode,
    rect: TskRect,
    alpha: f32,
) {
    let side = rect.width().min(rect.height()).clamp(40.0, 72.0);
    let x = rect.x() + rect.width() - side;
    let y = rect.y() + (rect.height() - side) * 0.5;
    let draw_rect = TskRect::from_xywh(x, y, side, side).unwrap_or(rect);
    let corner_radius = node.corner_radius.unwrap_or(0.0);

    let accent = secondary_close_button_tint(node, ctx);
    if let Some(fill_path) = rounded_rect_path(draw_rect, corner_radius) {
        let mut fill_paint = Paint::default();
        fill_paint.set_color(to_skia_color(
            [accent[0] * 0.10, accent[1] * 0.10, accent[2] * 0.10, 0.30],
            alpha,
        ));
        fill_paint.anti_alias = true;
        pixmap
            .as_mut()
            .fill_path(&fill_path, &fill_paint, FillRule::Winding, Transform::identity(), None);
    }

    if let Some(frame_path) = rounded_rect_path(draw_rect, corner_radius) {
        let mut frame_paint = Paint::default();
        frame_paint.set_color(to_skia_color([accent[0], accent[1], accent[2], 1.0], alpha));
        frame_paint.anti_alias = true;

        let mut frame_stroke = Stroke::default();
        frame_stroke.width = (draw_rect.width() * 0.032).max(1.5);

        pixmap.as_mut().stroke_path(
            &frame_path,
            &frame_paint,
            &frame_stroke,
            Transform::identity(),
            None,
        );
    }

    let inset = (draw_rect.width().min(draw_rect.height()) * 0.24).max(4.0);
    let x0 = draw_rect.x() + inset;
    let y0 = draw_rect.y() + inset;
    let x1 = draw_rect.x() + draw_rect.width() - inset;
    let y1 = draw_rect.y() + draw_rect.height() - inset;

    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(255, 255, 255, (alpha.clamp(0.0, 1.0) * 255.0) as u8));
    paint.anti_alias = true;

    let mut stroke = Stroke::default();
    stroke.width = (draw_rect.width() * 0.042).max(2.0);

    let mut pb1 = PathBuilder::new();
    pb1.move_to(x0, y0);
    pb1.line_to(x1, y1);
    if let Some(path) = pb1.finish() {
        pixmap
            .as_mut()
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    let mut pb2 = PathBuilder::new();
    pb2.move_to(x1, y0);
    pb2.line_to(x0, y1);
    if let Some(path) = pb2.finish() {
        pixmap
            .as_mut()
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn rounded_rect_path(rect: TskRect, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.max(0.0).min(rect.width() * 0.5).min(rect.height() * 0.5);
    let x = rect.x();
    let y = rect.y();
    let w = rect.width();
    let h = rect.height();

    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

fn draw_text_node(
    img: &mut RgbaImage,
    node: &UiIrNode,
    document: &UiIrDocument,
    renderer: &TextRenderer,
    ctx: &ComposeContext<'_>,
    seen_rects: &mut HashSet<(i32, i32, i32, i32)>,
) {
    let Some(text) = resolved_text_payload(node) else {
        return;
    };
    if text.is_empty() {
        return;
    }

    let rect = ir_rect_to_layout_rect(node.computed_rect);
    if rect.w < 0.5 || rect.h < 0.5 {
        return;
    }

    let key = (
        rect.x.round() as i32,
        rect.y.round() as i32,
        rect.w.round() as i32,
        rect.h.round() as i32,
    );
    if !seen_rects.insert(key) {
        return;
    }

    let text_rects = debug_text_rects_with_renderer(node, renderer)
        .unwrap_or(DebugTextRects {
            primary: rect,
            secondary: None,
            primary_text_origin: (rect.x, rect.y),
            secondary_text_origin: None,
        });
    let primary_rect = text_rects.primary;
    let secondary_rect = text_rects.secondary.unwrap_or(rect);
    let nominal_font_size = node
        .text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(18.0)
        .max(1.0);
    let primary_font_style_scale = font_style_scale_modifier(node.text_style.as_ref());
    let fallback_font_size =
        (nominal_font_size * primary_font_style_scale * TEXT_RENDER_SIZE_CALIBRATION).max(1.0);
    let secondary_nominal_font_size = node
        .secondary_text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(nominal_font_size)
        .max(1.0);
    let secondary_font_style_scale = font_style_scale_modifier(
        node.secondary_text_style
            .as_ref()
            .or(node.text_style.as_ref()),
    );
    let secondary_fallback_font_size = (secondary_nominal_font_size
        * secondary_font_style_scale
        * TEXT_RENDER_SIZE_CALIBRATION)
        .max(1.0);

    let align = node
        .text_style
        .as_ref()
        .map(|style| TextAlign::from_bb_str(&style.alignment))
        .unwrap_or(TextAlign::Left);
    let vertical_align = node
        .text_style
        .as_ref()
        .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
        .unwrap_or(VerticalAlign::Centre);

    let mut colour = resolved_text_colour(node, node.text_style.as_ref(), ctx);
    colour[3] = ((colour[3] as f32) * node.alpha.clamp(0.0, 1.0)).round() as u8;

    let requested_font_symbol = font_symbol_from_text_style(node.text_style.as_ref()).unwrap_or("<none>");
    let selected_font = select_imported_ui_font(ctx, node.text_style.as_ref());
    let primary_line_spacing = draw_line_spacing_for_node(node, text, node.text_style.as_ref());
    // Imported SWF glyph symbols already encode their authored shape/weight;
    // applying style scaleModifier on top of SWF sizing causes undersized text
    // on some screens (for example door status labels). Keep scaleModifier for
    // fallback TTF path only.
    let primary_swf_font_scale = swf_text_size_calibration_for_style(node.text_style.as_ref());
    let primary_ttf_font_scale = primary_font_style_scale * TEXT_RENDER_SIZE_CALIBRATION;
    let mut primary_swf_font_size = (nominal_font_size * primary_swf_font_scale).max(1.0);
    let mut primary_rect = apply_font_style_vertical_offset(primary_rect, node.text_style.as_ref());
    if let Some(selection) = selected_font.as_ref() {
        let should_fit_primary = swf_style_should_fit_to_rect(node.text_style.as_ref());
        if should_fit_primary {
        primary_swf_font_size = fit_swf_font_size_to_rect(
            renderer,
            text,
            selection.font,
            primary_rect,
            primary_swf_font_size,
            align,
            vertical_align,
            scale_line_spacing(primary_line_spacing, primary_swf_font_scale),
        );
        }
    }
    let used_swf_font = selected_font.as_ref().is_some_and(|selection| {
        if let Some(inline_rect) = inline_nested_textfield_text_rect(
            node,
            primary_rect,
            document,
            renderer,
            ctx,
            primary_swf_font_size,
        ) {
            primary_rect = inline_rect;
        }
        renderer.draw_swf_font(
            img,
            text,
            primary_rect,
            selection.font,
            ctx.assets.font_edit_text_metrics(&selection.symbol),
            primary_swf_font_size,
            colour,
            align,
            vertical_align,
            scale_line_spacing(primary_line_spacing, primary_swf_font_scale),
        )
    });
    if font_telemetry_enabled() {
        if let Some(selection) = selected_font.as_ref() {
            eprintln!(
                "text-font canvas='{}' node='{}' requested='{}' selected='{}' source='{}' fallback={} swf_used={} nominal_size={:.2} swf_draw_size={:.2} ttf_draw_size={:.2} rect_h={:.2} text='{}'",
                document.canvas_name.as_deref().unwrap_or("<unnamed-canvas>"),
                node.name,
                requested_font_symbol,
                selection.symbol,
                selection.source.as_str(),
                selection.source.is_fallback(),
                used_swf_font,
                nominal_font_size,
                primary_swf_font_size,
                fallback_font_size,
                primary_rect.h,
                text
            );
        } else {
            eprintln!(
                "text-font canvas='{}' node='{}' requested='{}' selected='<none>' source='none' fallback=false swf_used=false nominal_size={:.2} swf_draw_size=0.00 ttf_draw_size={:.2} rect_h={:.2} text='{}'",
                document.canvas_name.as_deref().unwrap_or("<unnamed-canvas>"),
                node.name,
                requested_font_symbol,
                nominal_font_size,
                fallback_font_size,
                primary_rect.h,
                text
            );
        }
    }
    if !used_swf_font {
        if let Some(inline_rect) = inline_nested_textfield_text_rect(
            node,
            primary_rect,
            document,
            renderer,
            ctx,
            fallback_font_size,
        ) {
            primary_rect = inline_rect;
        }
        renderer.draw(
            img,
            text,
            primary_rect,
            FontKind::Sans,
            fallback_font_size,
            colour,
            align,
            vertical_align,
            scale_line_spacing(primary_line_spacing, primary_ttf_font_scale),
        );
    }

    if let Some(UiIrTextPayload::Resolved { text: secondary }) = node.secondary_text_payload.as_ref() {
        let mut secondary_colour = resolved_text_colour(node, node.secondary_text_style.as_ref(), ctx);
        secondary_colour[3] = ((secondary_colour[3] as f32) * node.alpha.clamp(0.0, 1.0)).round() as u8;
        let secondary_align = node
            .secondary_text_style
            .as_ref()
            .map(|style| TextAlign::from_bb_str(&style.alignment))
            .unwrap_or(TextAlign::Left);
        let secondary_vertical_align = node
            .secondary_text_style
            .as_ref()
            .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
            .unwrap_or(VerticalAlign::Centre);
        let secondary_selected_font = select_imported_ui_font(
            ctx,
            node.secondary_text_style
                .as_ref()
                .or(node.text_style.as_ref()),
        );
        let secondary_line_spacing = node
            .secondary_text_style
            .as_ref()
            .or(node.text_style.as_ref())
            .and_then(|style| style.line_spacing);
        let secondary_swf_font_scale = swf_text_size_calibration_for_style(
            node.secondary_text_style.as_ref().or(node.text_style.as_ref()),
        );
        let secondary_ttf_font_scale = secondary_font_style_scale * TEXT_RENDER_SIZE_CALIBRATION;
        let mut secondary_swf_font_size =
            (secondary_nominal_font_size * secondary_swf_font_scale).max(1.0);
        let secondary_rect = apply_font_style_vertical_offset(
            secondary_rect,
            node.secondary_text_style.as_ref().or(node.text_style.as_ref()),
        );
        if let Some(selection) = secondary_selected_font.as_ref() {
            let secondary_style = node.secondary_text_style.as_ref().or(node.text_style.as_ref());
            let should_fit_secondary = swf_style_should_fit_to_rect(secondary_style);
            if should_fit_secondary {
            secondary_swf_font_size = fit_swf_font_size_to_rect(
                renderer,
                secondary,
                selection.font,
                secondary_rect,
                secondary_swf_font_size,
                secondary_align,
                secondary_vertical_align,
                scale_line_spacing(secondary_line_spacing, secondary_swf_font_scale),
            );
            }
        }
            let secondary_used_swf = secondary_selected_font.as_ref().is_some_and(|selection| {
            renderer.draw_swf_font(
                img,
                secondary,
                secondary_rect,
                selection.font,
                ctx.assets.font_edit_text_metrics(&selection.symbol),
                secondary_swf_font_size,
                secondary_colour,
                secondary_align,
                secondary_vertical_align,
                scale_line_spacing(secondary_line_spacing, secondary_swf_font_scale),
            )
        });
        if !secondary_used_swf {
            renderer.draw(
                img,
                secondary,
                secondary_rect,
                FontKind::Sans,
                secondary_fallback_font_size,
                secondary_colour,
                secondary_align,
                secondary_vertical_align,
                scale_line_spacing(secondary_line_spacing, secondary_ttf_font_scale),
            );
        }
    }
}

fn scale_line_spacing(line_spacing: Option<f32>, font_scale: f32) -> Option<f32> {
    line_spacing.map(|spacing| spacing * font_scale)
}

fn draw_line_spacing_for_node(
    node: &UiIrNode,
    text: &str,
    text_style: Option<&UiIrTextStyle>,
) -> Option<f32> {
    let spacing = text_style.and_then(|style| style.line_spacing);
    let leading = font_style_leading_modifier_px(text_style);
    let spacing = match (spacing, leading) {
        (Some(spacing), 0.0) => Some(spacing),
        (Some(spacing), leading) => Some(spacing + leading),
        (None, 0.0) => None,
        (None, leading) => Some(leading),
    };
    if is_large_wrapped_title3_heading(node, text, text_style) {
        spacing.map(|value| value + 4.0)
    } else {
        spacing
    }
}

fn apply_font_style_vertical_offset(rect: Rect, text_style: Option<&UiIrTextStyle>) -> Rect {
    let offset = font_style_top_margin_offset_px(text_style);
    if offset.abs() <= f32::EPSILON {
        rect
    } else {
        Rect { y: rect.y + offset, ..rect }
    }
}

fn is_large_wrapped_title3_heading(
    node: &UiIrNode,
    text: &str,
    text_style: Option<&UiIrTextStyle>,
) -> bool {
    let Some(style) = text_style else {
        return false;
    };
    if style.label_style.as_deref() != Some("Title3") {
        return false;
    }
    if !matches!(VerticalAlign::from_bb_str(&style.vertical_alignment), VerticalAlign::Centre) {
        return false;
    }
    let font_size = ir_value_to_px(&style.font_size);
    font_size >= 90.0
        && node.computed_rect.h >= font_size * 2.0
        && text.split_whitespace().count() >= 3
}

fn resolved_text_colour(
    node: &UiIrNode,
    style: Option<&crate::ui_ir::UiIrTextStyle>,
    ctx: &ComposeContext<'_>,
) -> [u8; 4] {
    explicit_text_colour_override(node)
        .and_then(|token| resolve_colour_token(ctx, token))
        .or_else(|| style.and_then(|style| style.colour))
        .or_else(|| {
            style
                .and_then(|style| style.colour_token.as_deref())
                .and_then(|token| text_colour_token_override(node, token).or(Some(token)))
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or_else(|| style_tag_colour_token(node).and_then(|token| resolve_colour_token(ctx, token)))
        .or_else(|| label_style_colour_token(style, node).and_then(|token| resolve_colour_token(ctx, token)))
        .map(rgba_to_u8)
        .unwrap_or([255, 255, 255, 255])
}

fn explicit_text_colour_override(node: &UiIrNode) -> Option<&'static str> {
    if node.resolved_style_tags.iter().any(|tag| {
        tag.tag_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("UI_Generic_Flag_03"))
    }) {
        Some("Foreground")
    } else {
        None
    }
}

fn text_colour_token_override<'a>(node: &UiIrNode, token: &'a str) -> Option<&'a str> {
    let has_emphasis_flag = node.resolved_style_tags.iter().any(|tag| {
        tag.tag_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("UI_Generic_Flag_03"))
    });
    if has_emphasis_flag && token.eq_ignore_ascii_case("Bright") {
        Some("Foreground")
    } else {
        None
    }
}

fn label_style_colour_token(
    style: Option<&crate::ui_ir::UiIrTextStyle>,
    node: &UiIrNode,
) -> Option<&'static str> {
    if !node.resolved_style_tags.is_empty() {
        return None;
    }
    match style.and_then(|style| style.label_style.as_deref()) {
        Some("Heading1" | "Heading3") => Some("Base"),
        _ => None,
    }
}

fn style_tag_colour_token(node: &UiIrNode) -> Option<&'static str> {
    node.resolved_style_tags.iter().find_map(|tag| {
        let name = tag.tag_name.as_deref()?.trim().to_ascii_lowercase();
        match name.as_str() {
            "primary" | "ui_generic_flag_03" => Some("Foreground"),
            "modify" => Some("Base"),
            _ => None,
        }
    })
}

fn custom_shape_style_tag_colour_token(node: &UiIrNode) -> Option<&'static str> {
    node.resolved_style_tags.iter().find_map(|tag| {
        let name = tag.tag_name.as_deref()?.trim().to_ascii_lowercase();
        match name.as_str() {
            "primary" => Some("Accent1"),
            "modify" => Some("Accent5"),
            _ => None,
        }
    })
}

pub fn debug_text_rects(node: &UiIrNode) -> Option<DebugTextRects> {
    let renderer = TextRenderer::new();
    debug_text_rects_with_renderer(node, &renderer)
}

pub fn debug_text_drawn_bounds(node: &UiIrNode) -> Option<DebugTextDrawnBounds> {
    let renderer = TextRenderer::new();
    debug_text_drawn_bounds_with_renderer(node, &renderer)
}

fn debug_text_rects_with_renderer(node: &UiIrNode, renderer: &TextRenderer) -> Option<DebugTextRects> {
    let text = resolved_text_payload(node)?;
    let rect = ir_rect_to_layout_rect(node.computed_rect);
    if rect.w < 0.5 || rect.h < 0.5 {
        return None;
    }

    let nominal_font_size = node
        .text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(18.0)
        .max(1.0);
    let fallback_font_size = (nominal_font_size * TEXT_RENDER_SIZE_CALIBRATION).max(1.0);
    let is_label_caption_pair = node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair");
    if is_label_caption_pair && node.secondary_text_payload.is_some() {
        let secondary_nominal_font_size = node
            .secondary_text_style
            .as_ref()
            .map(|style| ir_value_to_px(&style.font_size))
            .unwrap_or(nominal_font_size)
            .max(1.0);
        let secondary_fallback_font_size =
            (secondary_nominal_font_size * TEXT_RENDER_SIZE_CALIBRATION).max(1.0);
        let (mut primary, mut secondary) = stacked_label_caption_pair_text_rects(
            rect,
            renderer.measure(text, FontKind::Sans, fallback_font_size).1,
            node.secondary_text_payload
                .as_ref()
                .and_then(|payload| match payload {
                    UiIrTextPayload::Resolved { text } => Some(text.as_str()),
                    UiIrTextPayload::UnresolvedKey { .. }
                    | UiIrTextPayload::IntentionallyEmpty { .. }
                    | UiIrTextPayload::Empty => None,
                })
                .map(|secondary| renderer.measure(secondary, FontKind::Sans, secondary_fallback_font_size).1)
                .unwrap_or(secondary_fallback_font_size),
            node.text_style.as_ref().and_then(|style| style.anchor_to_parent_y),
            node.anchor[0] >= 0.99 && node.pivot[0] >= 0.99,
        );
        let (pair_offset_x, pair_offset_y) = right_anchored_label_caption_pair_offset(
            node,
            primary.h,
            Some(secondary.h),
        );
        primary.x += pair_offset_x;
        primary.y += pair_offset_y;
        secondary.x += pair_offset_x;
        secondary.y += pair_offset_y;
        let primary_align = node
            .text_style
            .as_ref()
            .map(|style| TextAlign::from_bb_str(&style.alignment))
            .unwrap_or(TextAlign::Left);
        let primary_vertical = node
            .text_style
            .as_ref()
            .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
            .unwrap_or(VerticalAlign::Centre);
        let secondary_text = node.secondary_text_payload.as_ref().and_then(|payload| match payload {
            UiIrTextPayload::Resolved { text } => Some(text.as_str()),
            UiIrTextPayload::UnresolvedKey { .. }
            | UiIrTextPayload::IntentionallyEmpty { .. }
            | UiIrTextPayload::Empty => None,
        });
        let primary_text_origin = text_origin_in_rect(
            renderer,
            text,
            primary,
            FontKind::Sans,
            fallback_font_size,
            primary_align,
            primary_vertical,
        );
        let secondary_text_origin = secondary_text.map(|secondary_text| {
            text_origin_in_rect(
                renderer,
                secondary_text,
                secondary,
                FontKind::Sans,
                secondary_fallback_font_size,
                TextAlign::Left,
                VerticalAlign::Centre,
            )
        });
        Some(DebugTextRects {
            primary,
            secondary: Some(secondary),
            primary_text_origin,
            secondary_text_origin,
        })
    } else {
        let align = node
            .text_style
            .as_ref()
            .map(|style| TextAlign::from_bb_str(&style.alignment))
            .unwrap_or(TextAlign::Left);
        let vertical = node
            .text_style
            .as_ref()
            .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
            .unwrap_or(VerticalAlign::Centre);
        let primary_rect = center_anchored_heading_textfield_text_rect(node, rect).unwrap_or(rect);
        Some(DebugTextRects {
            primary: primary_rect,
            secondary: None,
            primary_text_origin: text_origin_in_rect(renderer, text, primary_rect, FontKind::Sans, fallback_font_size, align, vertical),
            secondary_text_origin: None,
        })
    }
}

fn center_anchored_heading_textfield_text_rect(node: &UiIrNode, rect: Rect) -> Option<Rect> {
    let style = node.text_style.as_ref()?;
    if !node.node_type.eq_ignore_ascii_case("widget_text_field") {
        return None;
    }
    if !style
        .label_style
        .as_deref()
        .is_some_and(|label| label.eq_ignore_ascii_case("Heading1"))
    {
        return None;
    }
    if !style.vertical_alignment.eq_ignore_ascii_case("Center") {
        return None;
    }
    let anchor_to_parent_y = style.anchor_to_parent_y?;
    if (anchor_to_parent_y - 0.5).abs() > f32::EPSILON || node.pivot[1].abs() > f32::EPSILON {
        return None;
    }
    if node.anchor[1] > 0.0 && !(node.anchor[0] > 1.0 && node.pivot[0] >= 0.99) {
        return None;
    }

    let top = rect.y + rect.h * anchor_to_parent_y;
    let height = (rect.h * (1.0 - anchor_to_parent_y)).max(1.0);
    Some(Rect { y: top, h: height, ..rect })
}

fn inline_nested_textfield_text_rect(
    node: &UiIrNode,
    rect: Rect,
    document: &UiIrDocument,
    renderer: &TextRenderer,
    ctx: &ComposeContext<'_>,
    child_render_size_px: f32,
) -> Option<Rect> {
    let style = node.text_style.as_ref()?;
    if !node.node_type.eq_ignore_ascii_case("widget_text_field")
        || node.pivot[0] < 0.99
        || node.anchor[0] <= 1.0
        || !style.vertical_alignment.eq_ignore_ascii_case("Center")
    {
        return None;
    }

    let parent_id = node.parent_id?;
    let parent = document.nodes.iter().find(|candidate| candidate.id == parent_id)?;
    let parent_style = parent.text_style.as_ref()?;
    if !parent.node_type.eq_ignore_ascii_case("widget_text_field")
        || !same_label_style(style, parent_style)
        || !parent_style.vertical_alignment.eq_ignore_ascii_case(&style.vertical_alignment)
    {
        return None;
    }

    let parent_text = resolved_text_payload(parent)?;
    let parent_rect = center_anchored_heading_textfield_text_rect(
        parent,
        ir_rect_to_layout_rect(parent.computed_rect),
    )
    .unwrap_or_else(|| ir_rect_to_layout_rect(parent.computed_rect));
    let parent_nominal_size = parent_style_font_size(parent_style);
    let parent_font_style_scale = font_style_scale_modifier(Some(parent_style));
    let parent_swf_size = (parent_nominal_size * parent_font_style_scale * SWF_TEXT_RENDER_SIZE_CALIBRATION).max(1.0);
    let parent_width = select_imported_ui_font(ctx, Some(parent_style))
        .and_then(|selection| {
            renderer.measure_swf_advance_width(parent_text, selection.font, parent_swf_size)
        })
        .unwrap_or_else(|| {
            renderer.measure(
                parent_text,
                FontKind::Sans,
                (parent_nominal_size * parent_font_style_scale * TEXT_RENDER_SIZE_CALIBRATION).max(1.0),
            ).0
        });
    let inline_gap = style.anchor_to_parent_x.unwrap_or(0.0).max(0.0) * child_render_size_px;
    let inline_x = parent_rect.x + parent_width + inline_gap;
    if !inline_x.is_finite() || inline_x >= rect.x || inline_x <= parent_rect.x {
        return None;
    }

    let right = rect.x + rect.w;
    Some(Rect {
        x: inline_x,
        w: (right - inline_x).max(1.0),
        ..rect
    })
}

fn same_label_style(left: &UiIrTextStyle, right: &UiIrTextStyle) -> bool {
    match (left.label_style.as_deref(), right.label_style.as_deref()) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

fn parent_style_font_size(style: &UiIrTextStyle) -> f32 {
    ir_value_to_px(&style.font_size).max(1.0)
}

fn debug_text_drawn_bounds_with_renderer(
    node: &UiIrNode,
    renderer: &TextRenderer,
) -> Option<DebugTextDrawnBounds> {
    let text = resolved_text_payload(node)?;
    let rects = debug_text_rects_with_renderer(node, renderer)?;

    let nominal_font_size = node
        .text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(18.0)
        .max(1.0);
    let fallback_font_size = (nominal_font_size * TEXT_RENDER_SIZE_CALIBRATION).max(1.0);
    let primary_align = node
        .text_style
        .as_ref()
        .map(|style| TextAlign::from_bb_str(&style.alignment))
        .unwrap_or(TextAlign::Left);
    let primary_vertical = node
        .text_style
        .as_ref()
        .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
        .unwrap_or(VerticalAlign::Centre);

    let primary = renderer.measure_drawn_bounds(
        text,
        apply_font_style_vertical_offset(rects.primary, node.text_style.as_ref()),
        FontKind::Sans,
        fallback_font_size,
        primary_align,
        primary_vertical,
        draw_line_spacing_for_node(node, text, node.text_style.as_ref()),
    )?;

    let secondary = if let Some(UiIrTextPayload::Resolved { text: secondary_text }) = node.secondary_text_payload.as_ref() {
        let secondary_nominal_font_size = node
            .secondary_text_style
            .as_ref()
            .map(|style| ir_value_to_px(&style.font_size))
            .unwrap_or(nominal_font_size)
            .max(1.0);
        let secondary_fallback_font_size =
            (secondary_nominal_font_size * TEXT_RENDER_SIZE_CALIBRATION).max(1.0);
        rects.secondary.and_then(|secondary_rect| {
            renderer.measure_drawn_bounds(
                secondary_text,
                apply_font_style_vertical_offset(
                    secondary_rect,
                    node.secondary_text_style.as_ref().or(node.text_style.as_ref()),
                ),
                FontKind::Sans,
                secondary_fallback_font_size,
                TextAlign::Left,
                VerticalAlign::Centre,
                draw_line_spacing_for_node(
                    node,
                    secondary_text,
                    node.secondary_text_style.as_ref().or(node.text_style.as_ref()),
                ),
            )
        })
    } else {
        None
    };

    Some(DebugTextDrawnBounds { primary, secondary })
}

fn text_origin_in_rect(
    renderer: &TextRenderer,
    text: &str,
    rect: Rect,
    kind: FontKind,
    size_px: f32,
    align: TextAlign,
    vertical_align: VerticalAlign,
) -> (f32, f32) {
    let (text_w, text_h) = renderer.measure(text, kind, size_px.max(1.0));
    let x = match align {
        TextAlign::Left => rect.x,
        TextAlign::Centre => rect.x + ((rect.w - text_w) * 0.5).max(0.0),
        TextAlign::Right => rect.x + (rect.w - text_w).max(0.0),
    };
    let y = match vertical_align {
        VerticalAlign::Top => rect.y,
        VerticalAlign::Centre => rect.y + ((rect.h - text_h) * 0.5).max(0.0),
        VerticalAlign::Bottom => rect.y + (rect.h - text_h).max(0.0),
    };
    (x, y)
}

fn stacked_label_caption_pair_text_rects(
    rect: Rect,
    primary_text_h: f32,
    secondary_text_h: f32,
    primary_anchor_y: Option<f32>,
    right_anchored_pair: bool,
) -> (Rect, Rect) {
    let primary_h = primary_text_h.max(1.0).min(rect.h.max(1.0));
    let secondary_h = secondary_text_h.max(1.0).min(rect.h.max(1.0));
    let total_h = primary_h + secondary_h;
    let max_top_padding = (rect.h - total_h).max(0.0);
    let anchor_y = primary_anchor_y.unwrap_or(0.0).clamp(0.0, 0.999);
    let derived_top_padding = if anchor_y > 0.0 {
        ((anchor_y * (primary_h + secondary_h)) - (primary_h * 0.5)) / (1.0 - anchor_y)
    } else {
        0.0
    };
    let top_padding = if right_anchored_pair {
        derived_top_padding.max(0.0)
    } else {
        derived_top_padding.clamp(0.0, max_top_padding)
    };
    let primary_y = rect.y + top_padding;
    // Right-anchored metric stacks (e.g. MEDGELS/value) use two text bands
    // with different font sizes; collapse their handoff by the measured line
    // box delta so the visual gap follows typography instead of fixed pixels.
    let line_box_overlap = (primary_h - secondary_h).max(0.0);
    let secondary_y = (primary_y + primary_h - line_box_overlap).max(rect.y);
    (
        Rect {
            x: rect.x,
            y: primary_y,
            w: rect.w,
            h: primary_h,
        },
        Rect {
            x: rect.x,
            y: secondary_y,
            w: rect.w,
            h: secondary_h,
        },
    )
}

fn right_anchored_label_caption_pair_offset(
    node: &UiIrNode,
    primary_text_h: f32,
    secondary_text_h: Option<f32>,
) -> (f32, f32) {
    if !node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        || node.secondary_text_payload.is_none()
        || node.anchor[0] < 0.99
        || node.pivot[0] < 0.99
    {
        return (0.0, 0.0);
    }

    let primary_h = primary_text_h.max(0.0);
    let secondary_h = secondary_text_h.unwrap_or(primary_h).max(0.0);
    let line_box_delta = (primary_h - secondary_h).max(0.0);
    let stroke_pair_span = node.stroke_extent.unwrap_or(0.0).max(0.0) * 2.0;
    (-(line_box_delta + stroke_pair_span), line_box_delta)
}

fn font_telemetry_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SB_UI_FONT_TELEMETRY")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
            .unwrap_or(false)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontSelectionSource {
    ResolvedRecordSymbol,
    Title3ExportFallback,
    PreferredExportFallback,
    PreferredNameFallback,
}

impl FontSelectionSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::ResolvedRecordSymbol => "resolved-record-symbol",
            Self::Title3ExportFallback => "title3-export-fallback",
            Self::PreferredExportFallback => "preferred-export-fallback",
            Self::PreferredNameFallback => "preferred-name-fallback",
        }
    }

    fn is_fallback(self) -> bool {
        !matches!(self, Self::ResolvedRecordSymbol)
    }
}

struct SelectedImportedFont<'a> {
    symbol: String,
    font: &'a FontGlyphSet,
    source: FontSelectionSource,
}

fn select_imported_ui_font<'a>(
    ctx: &'a ComposeContext<'_>,
    text_style: Option<&UiIrTextStyle>,
) -> Option<SelectedImportedFont<'a>> {
    if let Some(symbol) = font_symbol_from_text_style(text_style)
        && let Some(id) = ctx.assets.lookup_export(symbol)
        && let Some(font) = ctx.assets.get_font(id)
    {
        return Some(SelectedImportedFont {
            symbol: symbol.to_string(),
            font,
            source: FontSelectionSource::ResolvedRecordSymbol,
        });
    }

    let label_style = text_style.and_then(|style| style.label_style.as_deref());

    if matches!(label_style, Some("Title3")) {
        let mut text1_fonts: Vec<(String, &'a FontGlyphSet)> = ctx
            .assets
            .export_entries()
            .filter(|(symbol, _)| symbol.starts_with("$Text1"))
            .filter_map(|(symbol, id)| ctx.assets.get_font(id).map(|font| (symbol.to_string(), font)))
            .collect();

        text1_fonts.sort_by(|(left_name, left_font), (right_name, right_font)| {
            title3_font_weight_rank(left_name, left_font)
                .cmp(&title3_font_weight_rank(right_name, right_font))
                .then_with(|| left_name.cmp(right_name))
        });

        if let Some((symbol, font)) = text1_fonts.into_iter().next() {
            return Some(SelectedImportedFont {
                symbol,
                font,
                source: FontSelectionSource::Title3ExportFallback,
            });
        }
    }

    let preferred_symbols: &[&str] = &["$Text1Book", "$Text1Med", "$OutfitRegular", "$Text1Bold", "$CIGDrake"];

    for symbol in preferred_symbols {
        if let Some(id) = ctx.assets.lookup_export(symbol)
            && let Some(font) = ctx.assets.get_font(id)
        {
            return Some(SelectedImportedFont {
                symbol: symbol.to_string(),
                font,
                source: FontSelectionSource::PreferredExportFallback,
            });
        }
    }

    let preferred_font_names: &[(&str, &str)] = match label_style {
        Some("Title3") => &[
            ("Blender Pro Light", "Blender Pro Light"),
            ("Blender Pro Regular", "Blender Pro Regular"),
            ("Blender Pro Thin", "Blender Pro Thin"),
            ("Blender Pro Book", "Blender Pro Book"),
            ("Blender Pro Medium", "Blender Pro Medium"),
            ("CIG Drake Font", "CIGDrake"),
        ],
        _ => &[
            ("Blender Pro Book", "Blender Pro Book"),
            ("Blender Pro Medium", "Blender Pro Medium"),
            ("Outfit", "Outfit"),
            ("Open Sans", "Open Sans"),
            ("CIG Drake Font", "CIGDrake"),
        ],
    };
    for (query, label) in preferred_font_names {
        if let Some(font) = ctx.assets.find_font_by_name(query) {
            return Some(SelectedImportedFont {
                symbol: label.to_string(),
                font,
                source: FontSelectionSource::PreferredNameFallback,
            });
        }
    }
    None
}

fn resolved_font_record_value(style: Option<&UiIrTextStyle>) -> Option<&serde_json::Value> {
    let record = style?.resolved_font_record.as_ref()?;
    Some(record.get("_RecordValue_").unwrap_or(record))
}

fn font_symbol_from_text_style(style: Option<&UiIrTextStyle>) -> Option<&str> {
    resolved_font_record_value(style)
        .and_then(|value| value.get("font"))
        .and_then(|value| value.as_str())
        .filter(|symbol| !symbol.is_empty())
}

fn font_style_has_paint_file(style: Option<&UiIrTextStyle>) -> bool {
    resolved_font_record_value(style)
        .and_then(|value| value.get("paintFile"))
        .and_then(|value| value.as_str())
        .is_some_and(|paint_file| !paint_file.trim().is_empty())
}

fn swf_text_size_calibration_for_style(style: Option<&UiIrTextStyle>) -> f32 {
    if font_symbol_from_text_style(style).is_some_and(|symbol| symbol.eq_ignore_ascii_case("$Low-Reg")) {
        return LOW_REG_SWF_SIZE_CALIBRATION;
    }
    if font_style_has_paint_file(style) {
        PAINT_FILE_SWF_SIZE_CALIBRATION
    } else {
        SWF_TEXT_RENDER_SIZE_CALIBRATION
    }
}

fn swf_style_should_fit_to_rect(style: Option<&UiIrTextStyle>) -> bool {
    !font_symbol_from_text_style(style).is_some_and(|symbol| symbol.eq_ignore_ascii_case("$Low-Reg"))
}

fn fit_swf_font_size_to_rect(
    _renderer: &TextRenderer,
    text: &str,
    font: &FontGlyphSet,
    rect: Rect,
    requested_size: f32,
    _align: TextAlign,
    _vertical_align: VerticalAlign,
    _line_spacing_px: Option<f32>,
) -> f32 {
    if text.trim().is_empty() || requested_size <= 1.0 || rect.w <= 1.0 || rect.h <= 1.0 {
        return requested_size.max(1.0);
    }

    let height_limit = rect.h * 0.95;
    let measured_height = swf_nominal_line_height_px(font, requested_size);

    let height_scale = if measured_height > height_limit && measured_height > 0.0 {
        height_limit / measured_height
    } else {
        1.0
    };

    (requested_size * height_scale).max(1.0)
}

fn swf_nominal_line_height_px(font: &FontGlyphSet, size_px: f32) -> f32 {
    let ascent = font.ascent.map(|value| value as f32).unwrap_or(820.0);
    let descent = font.descent.map(|value| value as f32).unwrap_or(-204.0);
    let leading = font.leading.map(|value| value as f32).unwrap_or(0.0);
    let units_per_em = (ascent - descent).abs().max(1.0);
    (((ascent - descent + leading) / units_per_em) * size_px).max(1.0)
}

fn font_style_scale_modifier(style: Option<&UiIrTextStyle>) -> f32 {
    resolved_font_record_value(style)
        .and_then(|value| value.get("scaleModifier"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .unwrap_or(1.0)
}

fn font_style_leading_modifier_px(style: Option<&UiIrTextStyle>) -> f32 {
    let modifier = resolved_font_record_value(style)
        .and_then(|value| value.get("leadingModifier"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .unwrap_or(0.0);
    let size_px = style.map(|style| ir_value_to_px(&style.font_size)).unwrap_or(0.0);
    modifier * size_px
}

fn font_style_top_margin_offset_px(style: Option<&UiIrTextStyle>) -> f32 {
    let modifier = resolved_font_record_value(style)
        .and_then(|value| value.get("topMarginModifier"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .unwrap_or(0.0);
    let size_px = style.map(|style| ir_value_to_px(&style.font_size)).unwrap_or(0.0);
    modifier * size_px
}

fn title3_font_weight_rank(symbol: &str, font: &FontGlyphSet) -> i32 {
    let symbol_lower = symbol.to_ascii_lowercase();
    let name_lower = font.name.to_ascii_lowercase();
    let combined = format!("{} {}", symbol_lower, name_lower);
    let name_rank = if combined.contains("thin") {
        0
    } else if combined.contains("light") {
        1
    } else if combined.contains("book") {
        2
    } else if combined.contains("regular") {
        3
    } else if combined.contains("med") || combined.contains("medium") {
        4
    } else if combined.contains("bold") {
        6
    } else {
        5
    };
    name_rank + if font.is_bold { 10 } else { 0 }
}

fn derived_accent_tint(ctx: &ComposeContext<'_>) -> [f32; 4] {
    [
        ctx.style.backlight.r as f32 / 255.0,
        ctx.style.backlight.g as f32 / 255.0,
        ctx.style.backlight.b as f32 / 255.0,
        1.0,
    ]
}

fn resolved_text_payload(node: &UiIrNode) -> Option<&str> {
    let payload = node.text_payload.as_ref()?;
    match payload {
        UiIrTextPayload::Resolved { text } => Some(text.as_str()),
        UiIrTextPayload::Empty
        | UiIrTextPayload::IntentionallyEmpty { .. }
        | UiIrTextPayload::UnresolvedKey { .. } => None,
    }
}

fn draw_ir_border(
    pixmap: &mut Pixmap,
    rect: Rect,
    border: &UiIrBorder,
    alpha: f32,
    ctx: &ComposeContext<'_>,
) {
    draw_border_side(
        pixmap,
        Rect { x: rect.x, y: rect.y, w: rect.w, h: border.top.width },
        border.top.colour.unwrap_or_else(|| style_primary_rgba(ctx)),
        alpha,
    );
    draw_border_side(
        pixmap,
        Rect {
            x: rect.x + rect.w - border.right.width,
            y: rect.y,
            w: border.right.width,
            h: rect.h,
        },
        border.right.colour.unwrap_or_else(|| style_primary_rgba(ctx)),
        alpha,
    );
    draw_border_side(
        pixmap,
        Rect {
            x: rect.x,
            y: rect.y + rect.h - border.bottom.width,
            w: rect.w,
            h: border.bottom.width,
        },
        border.bottom.colour.unwrap_or_else(|| style_primary_rgba(ctx)),
        alpha,
    );
    draw_border_side(
        pixmap,
        Rect { x: rect.x, y: rect.y, w: border.left.width, h: rect.h },
        border.left.colour.unwrap_or_else(|| style_primary_rgba(ctx)),
        alpha,
    );
}

fn draw_border_side(pixmap: &mut Pixmap, rect: Rect, colour: [f32; 4], alpha: f32) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
        return;
    };
    fill_rect_ts(pixmap, tsk_rect, colour, alpha);
}

fn draw_widget_circle(node: &UiIrNode, pixmap: &mut Pixmap, rect: TskRect) {
    let stroke_colour = node.stroke_colour.or(node.background_fill_colour);
    let Some(stroke_colour) = stroke_colour else {
        return;
    };

    let cx = rect.x() + rect.width() * 0.5;
    let cy = rect.y() + rect.height() * 0.5;
    let radius = rect.width().min(rect.height()) * 0.5;
    if radius <= 0.5 {
        return;
    }

    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, radius - 0.5);
    let Some(path) = pb.finish() else {
        return;
    };

    let mut paint = Paint::default();
    paint.set_color(to_skia_color(stroke_colour, node.alpha));
    paint.anti_alias = true;

    let mut stroke = Stroke::default();
    stroke.width = node.stroke_extent.unwrap_or(1.5).max(0.5);
    pixmap
        .as_mut()
        .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn draw_widget_separator(
    ctx: &ComposeContext<'_>,
    pixmap: &mut Pixmap,
    node: &UiIrNode,
    rect: TskRect,
    alpha: f32,
) {
    let draw_rect = widget_separator_draw_rect(rect, node.stroke_extent);
    let colour = node
        .stroke_colour
        .or_else(|| {
            node.stroke_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
        })
        .or(node.background_fill_colour)
        .or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
        })
        .or_else(|| resolve_surface_colour_token(ctx, "Accent5"))
        .unwrap_or_else(|| style_primary_rgba(ctx));
    fill_rect_ts_with_mode(pixmap, draw_rect, colour, alpha, node_colour_blend_mode(node));
}

fn node_colour_blend_mode(node: &UiIrNode) -> BlendMode {
    match node.colour_blend_mode {
        Some(UiIrColourBlendMode::Additive) => BlendMode::Plus,
        Some(UiIrColourBlendMode::SourceOver) | None => BlendMode::SourceOver,
    }
}

fn widget_separator_draw_rect(rect: TskRect, stroke_extent: Option<f32>) -> TskRect {
    let Some(stroke_extent) = stroke_extent else {
        return rect;
    };
    let thickness = (stroke_extent.max(0.5) * 2.0).min(rect.height()).max(1.0);
    TskRect::from_xywh(
        rect.x(),
        rect.y() + (rect.height() - thickness) * 0.5,
        rect.width(),
        thickness,
    )
    .unwrap_or(rect)
}

fn draw_rect_stroke_ts(
    pixmap: &mut Pixmap,
    rect: TskRect,
    rgba: [f32; 4],
    width: f32,
    alpha: f32,
) {
    let mut pb = PathBuilder::new();
    pb.push_rect(rect);
    let Some(path) = pb.finish() else {
        return;
    };

    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
    paint.anti_alias = false;

    let mut stroke = Stroke::default();
    stroke.width = width.max(0.5);
    pixmap
        .as_mut()
        .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn fill_rect_ts(pixmap: &mut Pixmap, rect: TskRect, rgba: [f32; 4], alpha: f32) {
    fill_rect_ts_with_mode(pixmap, rect, rgba, alpha, BlendMode::SourceOver);
}

fn fill_rect_ts_with_mode(
    pixmap: &mut Pixmap,
    rect: TskRect,
    rgba: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
    paint.blend_mode = blend_mode;
    paint.anti_alias = false;
    pixmap
        .as_mut()
        .fill_rect(rect, &paint, Transform::identity(), None);
}

fn blit_atlas_image_tinted(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
) {
    blit_atlas_image_tinted_with_mode(
        pixmap,
        img,
        dx,
        dy,
        tint,
        alpha,
        BlendMode::SourceOver,
    );
}

fn blit_atlas_image_tinted_with_mode(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let w = img.width();
    let h = img.height();

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

    let Some(size) = tiny_skia::IntSize::from_wh(w, h) else {
        return;
    };
    let Some(src_pixmap) = Pixmap::from_vec(premul, size) else {
        return;
    };

    let mut paint = PixmapPaint::default();
    paint.opacity = alpha.clamp(0.0, 1.0);
    paint.blend_mode = blend_mode;
    pixmap
        .as_mut()
        .draw_pixmap(dx, dy, src_pixmap.as_ref(), &paint, Transform::identity(), None);
}

fn pixmap_to_rgba_image(pixmap: Pixmap) -> Result<RgbaImage, UiError> {
    let w = pixmap.width();
    let h = pixmap.height();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for chunk in pixmap.data().chunks_exact(4) {
        let a = chunk[3] as f32 / 255.0;
        if a <= 0.0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        out.push(((chunk[0] as f32 / a).clamp(0.0, 255.0)) as u8);
        out.push(((chunk[1] as f32 / a).clamp(0.0, 255.0)) as u8);
        out.push(((chunk[2] as f32 / a).clamp(0.0, 255.0)) as u8);
        out.push(chunk[3]);
    }
    RgbaImage::from_raw(w, h, out)
        .ok_or_else(|| UiError::RenderError("failed to build image from pixmap".into()))
}

fn to_skia_color(rgba: [f32; 4], global_alpha: f32) -> Color {
    let a = (rgba[3] * global_alpha).clamp(0.0, 1.0);
    Color::from_rgba8(
        (rgba[0].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0) as u8,
        (a * 255.0) as u8,
    )
}

fn style_primary_rgba(ctx: &ComposeContext<'_>) -> [f32; 4] {
    let pt = &ctx.style.primary_tint;
    [
        pt.r as f32 / 255.0,
        pt.g as f32 / 255.0,
        pt.b as f32 / 255.0,
        pt.a as f32 / 255.0,
    ]
}

fn rgba_to_u8(rgba: [f32; 4]) -> [u8; 4] {
    [
        (rgba[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn ir_rect_to_layout_rect(rect: UiIrRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

fn ir_value_to_px(value: &UiIrValue) -> f32 {
    match value {
        UiIrValue::Fixed { value } | UiIrValue::Percent { value } | UiIrValue::Other { value, .. } => *value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use image::Rgba;

    use crate::bb_atlas::AssetFetcher;
    use crate::canvas::RgbaColor;
    use crate::style::{CrtParams, ManufacturerStyle};
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiRendererHint, UiIrAssetLayout, UiIrCustomShape, UiIrStyleTag, UiIrTextStyle};

    fn text_style_with_font_record(record: serde_json::Value) -> UiIrTextStyle {
        UiIrTextStyle {
            font_record: Some("file://./fontstyles/blenderpro-medium.json".into()),
            resolved_font_record: Some(record),
            font_size: UiIrValue::Fixed { value: 18.0 },
            line_spacing: None,
            alignment: "Left".into(),
            vertical_alignment: "Center".into(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: None,
            label_style: None,
        }
    }

    #[test]
    fn font_symbol_reads_buildingblocks_font_style_record() {
        let style = text_style_with_font_record(serde_json::json!({
            "_RecordName_": "BuildingBlocks_FontStyle.BlenderPro-Medium",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_FontStyle",
                "font": "$Text1Med",
                "scaleModifier": 1.0
            }
        }));

        assert_eq!(font_symbol_from_text_style(Some(&style)), Some("$Text1Med"));
    }

    #[test]
    fn font_symbol_reads_unwrapped_font_style_record() {
        let style = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Thin",
            "scaleModifier": 0.92
        }));

        assert_eq!(font_symbol_from_text_style(Some(&style)), Some("$Text1Thin"));
        assert_eq!(font_style_scale_modifier(Some(&style)), 0.92);
    }

    #[test]
    fn font_style_modifiers_scale_with_font_size() {
        let mut style = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Thin",
            "scaleModifier": 1.0,
            "leadingModifier": 0.25,
            "topMarginModifier": -0.5
        }));
        style.font_size = UiIrValue::Fixed { value: 20.0 };

        assert_eq!(font_style_leading_modifier_px(Some(&style)), 5.0);
        assert_eq!(font_style_top_margin_offset_px(Some(&style)), -10.0);
    }

    #[test]
    fn paint_file_font_style_disables_swf_font_path() {
        let style_with_paint = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Low-Reg",
            "paintFile": "UI/fonts/Install/AUDIMRG.slug"
        }));
        let style_without_paint = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Book"
        }));

        assert!(font_style_has_paint_file(Some(&style_with_paint)));
        assert!(!font_style_has_paint_file(Some(&style_without_paint)));
    }

    #[test]
    fn non_low_reg_paint_file_swf_font_scale_guard_stays_near_nominal() {
        let style_with_paint = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Book",
            "paintFile": "UI/fonts/Install/AUDIMRG.slug"
        }));

        let swf_scale = swf_text_size_calibration_for_style(Some(&style_with_paint));
        assert!(
            swf_scale <= 1.5,
            "paint-file SWF size calibration drifted too high ({swf_scale:.2}); likely text-size regression"
        );
    }

    #[test]
    fn low_reg_swf_font_scale_guard_uses_large_override_without_fit() {
        let low_reg_style = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Low-Reg",
            "paintFile": "UI/fonts/Install/AUDIMRG.slug"
        }));

        let swf_scale = swf_text_size_calibration_for_style(Some(&low_reg_style));
        assert!(
            (swf_scale - LOW_REG_SWF_SIZE_CALIBRATION).abs() < f32::EPSILON,
            "Low-Reg SWF calibration drifted ({swf_scale:.2})"
        );
        assert!(
            !swf_style_should_fit_to_rect(Some(&low_reg_style)),
            "Low-Reg style should bypass SWF fit-to-rect to preserve authored large status text"
        );
    }

    #[test]
    fn apply_font_style_vertical_offset_shifts_rect_y() {
        let mut style = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Thin",
            "scaleModifier": 1.0,
            "topMarginModifier": -0.25
        }));
        style.font_size = UiIrValue::Fixed { value: 16.0 };

        let rect = Rect { x: 10.0, y: 20.0, w: 30.0, h: 40.0 };
        assert_eq!(
            apply_font_style_vertical_offset(rect, Some(&style)),
            Rect { x: 10.0, y: 16.0, w: 30.0, h: 40.0 }
        );
    }

    fn assert_not_uniform(img: &RgbaImage, label: &str) {
        let (w, h) = img.dimensions();
        let mut first: Option<[u8; 4]> = None;
        let mut differing = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                let px = img.get_pixel(x, y).0;
                match first {
                    None => first = Some(px),
                    Some(f) if f != px => differing += 1,
                    _ => {}
                }
            }
        }
        assert!(
            differing > 0,
            "[{label}] image is entirely one colour ({:?})",
            first.unwrap_or([0, 0, 0, 0])
        );
    }

    fn assert_non_background_fraction(
        img: &RgbaImage,
        bg: [u8; 4],
        min_frac: f32,
        label: &str,
    ) {
        let (w, h) = img.dimensions();
        let mut total = 0usize;
        let mut non_bg = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                total += 1;
                let p = img.get_pixel(x, y).0;
                let differs = p
                    .iter()
                    .zip(bg.iter())
                    .any(|(a, b)| (*a as i32 - *b as i32).abs() > 16);
                if differs {
                    non_bg += 1;
                }
            }
        }
        let frac = non_bg as f32 / total.max(1) as f32;
        assert!(
            frac >= min_frac,
            "[{label}] only {:.1}% pixels differ from bg; expected >= {:.1}%",
            frac * 100.0,
            min_frac * 100.0,
        );
    }

    struct StubFetcher {
        images: HashMap<String, Vec<u8>>,
    }

    impl AssetFetcher for StubFetcher {
        fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
            self.images.get(&p4k_path.to_ascii_lowercase()).cloned()
        }
    }

    fn stub_style() -> ManufacturerStyle {
        ManufacturerStyle {
            name: "drak".to_string(),
            primary_tint: RgbaColor { r: 240, g: 168, b: 104, a: 255 },
            secondary_tint: None,
            colour_slots: vec![
                RgbaColor { r: 240, g: 168, b: 104, a: 255 },
                RgbaColor { r: 67, g: 221, b: 147, a: 255 },
                RgbaColor { r: 228, g: 218, b: 77, a: 255 },
                RgbaColor { r: 201, g: 51, b: 51, a: 255 },
                RgbaColor { r: 0, g: 113, b: 188, a: 255 },
            ],
            background: RgbaColor { r: 48, g: 32, b: 16, a: 255 },
            backlight: RgbaColor { r: 102, g: 214, b: 255, a: 255 },
            font_family_hints: Vec::new(),
            crt: CrtParams::default(),
        }
    }

    fn minimal_swf_assets() -> crate::swf_assets::SwfAssetLibrary {
        crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse")
    }

    fn blank_node(node_type: &str) -> UiIrNode {
        UiIrNode {
            id: 1,
            parent_id: None,
            children: Vec::new(),
            node_type: node_type.to_string(),
            name: "node".to_string(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 10.0 },
                UiIrValue::Fixed { value: 10.0 },
            ],
            padding: [0.0, 0.0, 0.0, 0.0],
            margin: [0.0, 0.0, 0.0, 0.0],
            overflow_mode: None,
            computed_rect: UiIrRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            background_fill_colour: None,
            corner_radius: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            segmented_fill: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: None,
            asset_ref: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: Vec::new(),
            resolved_style_tags: Vec::new(),
        }
    }

    #[test]
    fn apply_asset_layout_flip_mirrors_horizontally() {
        let mut node = blank_node("display_widget");
        node.asset_layout = Some(UiIrAssetLayout {
            scaling_behavior: None,
            contain_position_x: None,
            contain_position_y: None,
            flip_horizontal: Some(true),
            flip_vertical: None,
        });

        let mut img = RgbaImage::new(2, 1);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, Rgba([0, 255, 0, 255]));

        let flipped = apply_asset_layout_flip(&node, img);
        assert_eq!(flipped.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(flipped.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn large_wrapped_title3_heading_adds_line_gap() {
        let mut node = blank_node("widget_text_field");
        node.computed_rect = UiIrRect { x: 0.0, y: 0.0, w: 1344.0, h: 270.0 };
        let style = UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: UiIrValue::Fixed { value: 103.5 },
            line_spacing: Some(-20.7),
            alignment: "Center".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: Some("Bright".to_string()),
            label_style: Some("Title3".to_string()),
        };

        assert_eq!(
            draw_line_spacing_for_node(&node, "DIGITAL MEDICAL ASSISTANT", Some(&style)),
            Some(-16.7)
        );
    }

    #[test]
    fn widget_separator_uses_centered_svg_stroke_extent() {
        let rect = TskRect::from_xywh(10.0, 20.0, 80.0, 8.0).expect("test rect");
        let draw_rect = widget_separator_draw_rect(rect, Some(1.0));
        assert_eq!(draw_rect.x(), 10.0);
        assert_eq!(draw_rect.y(), 23.0);
        assert_eq!(draw_rect.width(), 80.0);
        assert_eq!(draw_rect.height(), 2.0);

        let fallback = widget_separator_draw_rect(rect, None);
        assert_eq!(fallback, rect);
    }

    #[test]
    fn widget_separator_preserves_sixteen_pixel_authored_rects() {
        let rect = TskRect::from_xywh(10.0, 20.0, 80.0, 16.0).expect("test rect");
        let draw_rect = widget_separator_draw_rect(rect, None);

        assert_eq!(draw_rect, rect);
    }

    #[test]
    fn custom_shape_fill_prefers_fill_tint_over_stroke() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_custom_shape");
        node.asset_ref = Some("shape.svg".to_string());
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.background_fill_colour = Some([0.1, 0.2, 0.3, 1.0]);
        node.stroke_colour = Some([0.8, 0.1, 0.1, 1.0]);

        assert_eq!(custom_shape_fill_override(&node, &ctx), Some([0.1, 0.2, 0.3, 1.0]));
    }

    #[test]
    fn custom_shape_fill_prefers_svg_tint_token_over_background_colour() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_custom_shape");
        node.asset_ref = Some("shape.svg".to_string());
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.background_fill_colour = Some([0.1, 0.9, 0.1, 1.0]);
        node.icon_tint_colour_token = Some("Accent1".to_string());

        assert_eq!(custom_shape_fill_override(&node, &ctx), Some(style_primary_rgba(&ctx)));
    }

    #[test]
    fn svg_fill_override_disables_second_blit_tint() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_custom_shape");
        node.icon_tint_colour = Some([0.2, 0.6, 0.9, 1.0]);

        assert_eq!(
            image_tint_for_blit(&node, "UI/Textures/Vector/General/FingerPrint.svg", Some([0.2, 0.6, 0.9, 1.0]), &ctx),
            [1.0, 1.0, 1.0, 1.0]
        );
        assert_eq!(
            image_tint_for_blit(&node, "UI/Textures/Icons/FingerPrint.dds", Some([0.2, 0.6, 0.9, 1.0]), &ctx),
            [0.2, 0.6, 0.9, 1.0]
        );
    }

    #[test]
    fn manufacturer_logo_tint_prefers_source_icon_tint_over_derived_accent() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_manufacturer_logo");
        node.icon_tint_colour = Some([0.2, 0.6, 0.9, 1.0]);

        assert_eq!(manufacturer_logo_tint(&node, &ctx), [0.2, 0.6, 0.9, 1.0]);
    }

    #[test]
    fn secondary_close_button_tint_prefers_source_stroke_over_derived_accent() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_close_button_secondary");
        node.stroke_colour = Some([0.3, 0.7, 0.2, 1.0]);

        assert_eq!(secondary_close_button_tint(&node, &ctx), [0.3, 0.7, 0.2, 1.0]);
    }

    #[test]
    fn custom_shape_svg_uses_source_over_blend() {
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });

        assert_eq!(
            image_blend_mode_for_node(&node, "UI/Textures/Vector/General/FingerPrint.svg"),
            BlendMode::SourceOver
        );
        assert_eq!(
            image_blend_mode_for_node(&node, "UI/Textures/I_InteractiveScreens/Med/fingerprint_glow.tif"),
            BlendMode::Plus
        );
    }

    #[test]
    fn matte_strip_preserves_sparse_line_art_strokes() {
        let mut img = RgbaImage::new(10, 10);
        for y in 0..10 {
            img.put_pixel(5, y, image::Rgba([0, 0, 0, 255]));
        }

        let stripped = strip_custom_shape_uniform_matte(&img);
        assert_eq!(stripped.get_pixel(5, 5).0, [0, 0, 0, 255]);
    }

    #[test]
    fn matte_strip_removes_dominant_opaque_matte() {
        let mut img = RgbaImage::new(10, 10);
        for y in 0..9 {
            for x in 0..10 {
                img.put_pixel(x, y, image::Rgba([1, 2, 3, 255]));
            }
        }

        let stripped = strip_custom_shape_uniform_matte(&img);
        assert_eq!(stripped.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(stripped.get_pixel(9, 9).0, [0, 0, 0, 0]);
    }

    #[test]
    fn image_tint_token_resolves_without_background_fill() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_image");
        node.icon_tint_colour_token = Some("Base".to_string());

        assert_eq!(
            image_tint_for_blit(&node, "UI/Textures/Shared/panel-bar.tif", None, &ctx),
            style_primary_rgba(&ctx)
        );
    }

    #[test]
    fn color_style_tokens_resolve_to_palette_slots() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };

        assert_eq!(resolve_colour_token(&ctx, "Base"), Some(style_primary_rgba(&ctx)));
        assert_eq!(resolve_colour_token(&ctx, "Accent1"), Some(style_primary_rgba(&ctx)));
        assert_eq!(
            resolve_surface_colour_token(&ctx, "Accent1"),
            Some([0.0, 113.0 / 255.0, 188.0 / 255.0, 1.0])
        );
        assert_eq!(resolve_colour_token(&ctx, "Accent5"), Some([0.0, 113.0 / 255.0, 188.0 / 255.0, 1.0]));

        let drake_hud_like_style = ManufacturerStyle {
            colour_slots: vec![
                style.primary_tint,
                RgbaColor { r: 207, g: 211, b: 0, a: 255 },
                style.primary_tint,
                RgbaColor { r: 201, g: 52, b: 43, a: 255 },
                RgbaColor { r: 243, g: 80, b: 77, a: 255 },
            ],
            ..style.clone()
        };
        let drake_ctx = ComposeContext {
            style: &drake_hud_like_style,
            defaults: &defaults,
            assets: &assets,
        };
        assert_eq!(resolve_colour_token(&drake_ctx, "Accent2"), Some(style_primary_rgba(&drake_ctx)));

        let mut node = blank_node("widget_text_field");
        node.resolved_style_tags = vec![UiIrStyleTag {
            uuid: "tag-primary".to_string(),
            tag_name: Some("Primary".to_string()),
        }];

        assert_eq!(resolved_text_colour(&node, None, &ctx), [255, 255, 255, 255]);

        node.resolved_style_tags[0].tag_name = Some("Modify".to_string());
        assert_eq!(resolved_text_colour(&node, None, &ctx), rgba_to_u8(style_primary_rgba(&ctx)));
    }

    #[test]
    fn untagged_heading_text_resolves_to_accent() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_text_field");
        node.text_style = Some(crate::ui_ir::UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: crate::ui_ir::UiIrValue::Fixed { value: 28.0 },
            line_spacing: None,
            alignment: "Left".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: None,
            label_style: Some("Heading1".to_string()),
        });

        assert_eq!(
            resolved_text_colour(&node, node.text_style.as_ref(), &ctx),
            rgba_to_u8(style_primary_rgba(&ctx))
        );
    }

    #[test]
    fn custom_shape_modify_tag_uses_deep_blue_palette_slot() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.icon_tint_colour_token = Some("Accent1".to_string());
        node.resolved_style_tags = vec![UiIrStyleTag {
            uuid: "tag-modify".to_string(),
            tag_name: Some("Modify".to_string()),
        }];

        assert_eq!(custom_shape_fill_override(&node, &ctx), Some([0.0, 113.0 / 255.0, 188.0 / 255.0, 1.0]));
    }

    #[test]
    fn custom_shape_modify_svg_uses_additive_blend() {
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.resolved_style_tags = vec![UiIrStyleTag {
            uuid: "tag-modify".to_string(),
            tag_name: Some("Modify".to_string()),
        }];

        assert_eq!(
            image_blend_mode_for_node(&node, "UI/Textures/Vector/General/FingerPrint.svg"),
            BlendMode::Plus
        );
    }

    #[test]
    fn rasterize_svg_without_custom_shape_uses_standard_svg_path() {
        let node = blank_node("display_widget");
        let svg = br#"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'>
            <rect x='0' y='0' width='8' height='8' fill='#ffffff'/>
        </svg>"#;

        let image = rasterize_custom_shape_svg(&node, svg, 8, 8, Some([1.0, 0.0, 0.0, 1.0]))
            .expect("svg should rasterize without custom-shape metadata");
        let pixel = image.get_pixel(4, 4);
        assert!(pixel[0] > 200, "expected red tint contribution, got {}", pixel[0]);
        assert!(pixel[1] < 20, "expected low green channel, got {}", pixel[1]);
    }

    #[test]
    fn render_ui_ir_document_renders_text_from_golden_fixture() {
        let document: UiIrDocument = serde_json::from_str(include_str!(
            "../tests/fixtures/ui_ir/expected_testroot_ir.json"
        ))
        .expect("golden fixture should parse");
        let fetcher = StubFetcher { images: HashMap::new() };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };

        let img = render_ui_ir_document(&document, &ctx, &atlas).expect("IR render should succeed");
        assert_eq!(img.dimensions(), (200, 100));
        assert_not_uniform(&img, "ir-text-golden");
        assert_non_background_fraction(&img, [48, 32, 16, 255], 0.005, "ir-text-golden");
    }

    #[test]
    fn render_ui_ir_document_draws_fill_border_and_asset_ref() {
        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "test-guid".to_string(),
            canvas_name: Some("BuildingBlocks_Canvas.TestIrRender".to_string()),
            target_width: 32,
            target_height: 32,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: vec!["test/red.png".to_string()],
            missing_asset_refs: Vec::new(),
            nodes: vec![UiIrNode {
                id: 1,
                parent_id: None,
                children: Vec::new(),
                node_type: "widget_image".to_string(),
                name: "card".to_string(),
                is_active: true,
                layer: 0,
                alpha: 1.0,
                anchor: [0.0, 0.0],
                pivot: [0.0, 0.0],
                authored_position: [4.0, 4.0],
                authored_size: [
                    UiIrValue::Fixed { value: 24.0 },
                    UiIrValue::Fixed { value: 24.0 },
                ],
                padding: [0.0, 0.0, 0.0, 0.0],
                margin: [0.0, 0.0, 0.0, 0.0],
                overflow_mode: None,
                computed_rect: UiIrRect { x: 4.0, y: 4.0, w: 24.0, h: 24.0 },
                background_fill_colour: Some([0.0, 0.0, 1.0, 1.0]),
                corner_radius: None,
                background_fill_alpha: None,
                background_fill_colour_token: None,
                segmented_fill: None,
                border: Some(UiIrBorder {
                    top: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    right: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    bottom: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    left: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                }),
                stroke_colour: None,
                stroke_colour_token: None,
                stroke_extent: None,
                colour_blend_mode: None,
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None::<UiIrTextStyle>,
                asset_ref: Some("test/red.png".to_string()),
                asset_layout: None,
                custom_shape: None,
                style_tag_uuids: Vec::new(),
                resolved_style_tags: Vec::new(),
            }],
        };

        let png = image::RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255]));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(png)
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png)
            .expect("png encoding");

        let fetcher = StubFetcher {
            images: HashMap::from([("test/red.png".to_string(), encoded)]),
        };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };

        let img = render_ui_ir_document(&document, &ctx, &atlas).expect("IR render should succeed");
        let center = img.get_pixel(16, 16).0;
        assert!(center[0] > 0, "asset tint should contribute red at the center");
        let border = img.get_pixel(4, 4).0;
        assert!(border[0] > 200 && border[1] > 200, "border pixel should be yellow-ish");
    }

    #[test]
    fn render_ui_ir_document_is_deterministic_and_layout_sensitive() {
        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "layout-guid".to_string(),
            canvas_name: Some("BuildingBlocks_Canvas.LayoutDeterministic".to_string()),
            target_width: 40,
            target_height: 24,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: vec![UiIrNode {
                id: 1,
                parent_id: None,
                children: Vec::new(),
                node_type: "widget_canvas".to_string(),
                name: "panel".to_string(),
                is_active: true,
                layer: 0,
                alpha: 1.0,
                anchor: [0.0, 0.0],
                pivot: [0.0, 0.0],
                authored_position: [5.0, 6.0],
                authored_size: [
                    UiIrValue::Fixed { value: 18.0 },
                    UiIrValue::Fixed { value: 10.0 },
                ],
                padding: [0.0, 0.0, 0.0, 0.0],
                margin: [0.0, 0.0, 0.0, 0.0],
                overflow_mode: None,
                computed_rect: UiIrRect { x: 5.0, y: 6.0, w: 18.0, h: 10.0 },
                background_fill_colour: Some([0.0, 0.0, 1.0, 1.0]),
                corner_radius: None,
                background_fill_alpha: None,
                background_fill_colour_token: None,
                segmented_fill: None,
                border: Some(UiIrBorder {
                    top: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    right: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    bottom: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    left: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                }),
                stroke_colour: None,
                stroke_colour_token: None,
                stroke_extent: None,
                colour_blend_mode: None,
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None::<UiIrTextStyle>,
                asset_ref: None,
                asset_layout: None,
                custom_shape: None,
                style_tag_uuids: Vec::new(),
                resolved_style_tags: Vec::new(),
            }],
        };

        let fetcher = StubFetcher { images: HashMap::new() };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };

        let img_a = render_ui_ir_document(&document, &ctx, &atlas).expect("first render should succeed");
        let img_b = render_ui_ir_document(&document, &ctx, &atlas).expect("second render should succeed");

        assert_eq!(img_a.as_raw(), img_b.as_raw(), "same IR should render bit-stably");
        assert_eq!(img_a.get_pixel(1, 1).0, [48, 32, 16, 255], "background pixel should remain style background");
        assert!(img_a.get_pixel(5, 6).0[0] > 200 && img_a.get_pixel(5, 6).0[1] > 200, "border origin should be yellow");
        assert!(img_a.get_pixel(12, 10).0[2] > 200, "panel interior should render blue fill at the authored position");
    }

    #[test]
    fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers() {
        let source = include_str!("ir_compose.rs");
        // Hard rule: do not add heuristic marker names. If this trips, fix the
        // structural root cause so composition remains generic across screens.
        // Renaming around this assertion is not an acceptable workaround.
        let forbidden = [
            ["_", "candidate"].concat(),
            ["is_", "med", "ical1_layout"].concat(),
            ["is_", "medical", "_attract_banner_text"].concat(),
            ["is_", "footer", "_brand_text_context"].concat(),
            ["med", "ical_", "cyan_tint"].concat(),
            ["Top_", "seperator"].concat(),
            ["MedGel", "FillMeter"].concat(),
            ["Function", "Title"].concat(),
            ["node", ".name", ".eq_ignore_ascii_case(\"Med", "Gel\")"].concat(),
            ["node", ".name", ".eq_ignore_ascii_case(\"Location", "Name\")"].concat(),
            ["node", ".name", ".eq_ignore_ascii_case(\"Tier", "Level\")"].concat(),
            ["s_", "bioc"].concat(),
            ["s_", "rsi"].concat(),
            ["s_", "aegs"].concat(),
            ["s_", "drak"].concat(),
            ["mockup", "image"].concat(),
            ["i_med_bioc_", "bottom-bar"].concat(),
            ["BG", "Dots"].concat(),
            ["MainMenu", "Canvas"].concat(),
        ];

        for marker in forbidden {
            assert!(
                !source.contains(marker.as_str()),
                "ir_compose hardcoding marker reintroduced: {marker}. This is a hard rule: do not work around this guard by renaming tokens. Keep composition generic for all screens and manageable in scope by fixing the structural root cause instead of reintroducing marker-based hardcoding."
            );
        }
    }

    #[test]
    fn segmented_count_matches_medgel_source_geometry() {
        assert_eq!(segmented_count_for_width(115.0, 3.0, 5.0), 14);
    }

    #[test]
    fn label_caption_pair_stacks_secondary_immediately_below_primary_text_band() {
        let rect = Rect { x: 100.0, y: 20.0, w: 128.0, h: 152.0 };
        let (primary_rect, secondary_rect) = stacked_label_caption_pair_text_rects(
            rect,
            32.0,
            27.0,
            Some(0.5),
            false,
        );

        assert_eq!(primary_rect.y, 47.0);
        assert_eq!(primary_rect.h, 32.0);
        assert_eq!(secondary_rect.y, 74.0);
        assert_eq!(secondary_rect.h, 27.0);
    }

    #[test]
    fn center_anchored_heading_textfield_uses_parent_anchor_text_band() {
        let mut node = blank_node("widget_text_field");
        node.computed_rect = UiIrRect { x: 20.0, y: 25.0, w: 320.0, h: 78.0 };
        node.authored_size = [
            UiIrValue::Fixed { value: 320.0 },
            UiIrValue::Fixed { value: 78.0 },
        ];
        node.anchor = [0.0, -0.12];
        node.pivot = [0.0, 0.0];
        node.text_payload = Some(UiIrTextPayload::Resolved { text: "T3".to_string() });
        node.text_style = Some(UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: UiIrValue::Fixed { value: 41.0 },
            line_spacing: None,
            alignment: "Left".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: Some(0.5),
            colour: None,
            colour_token: None,
            label_style: Some("Heading1".to_string()),
        });

        let rects = debug_text_rects(&node).expect("text rects");
        assert_eq!(rects.primary.y, 64.0);
        assert_eq!(rects.primary.h, 39.0);
    }

    #[test]
    fn nested_right_pivot_heading_textfield_uses_inline_parent_text_advance() {
        let mut parent = blank_node("widget_text_field");
        parent.id = 1;
        parent.children = vec![2];
        parent.computed_rect = UiIrRect { x: 90.0, y: 25.0, w: 780.0, h: 78.0 };
        parent.text_payload = Some(UiIrTextPayload::Resolved { text: "T3".to_string() });
        parent.text_style = Some(UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: UiIrValue::Fixed { value: 41.0 },
            line_spacing: None,
            alignment: "Left".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: Some(0.5),
            anchor_to_parent_y: Some(0.5),
            colour: None,
            colour_token: None,
            label_style: Some("Heading1".to_string()),
        });

        let mut child = blank_node("widget_text_field");
        child.id = 2;
        child.parent_id = Some(1);
        child.anchor = [1.14, 0.0];
        child.pivot = [1.0, 0.0];
        child.computed_rect = UiIrRect { x: 200.0, y: 25.0, w: 780.0, h: 78.0 };
        child.text_payload = Some(UiIrTextPayload::Resolved { text: "INLINE TITLE".to_string() });
        child.text_style = parent.text_style.clone();

        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "test-guid".to_string(),
            canvas_name: None,
            target_width: 400,
            target_height: 160,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: vec![parent, child.clone()],
        };
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
        let rect = ir_rect_to_layout_rect(child.computed_rect);
        let inline = inline_nested_textfield_text_rect(
            &child,
            rect,
            &document,
            &TextRenderer::new(),
            &ctx,
            41.0 * SWF_TEXT_RENDER_SIZE_CALIBRATION,
        )
        .expect("inline rect");

        assert!(inline.x < rect.x, "expected nested title to move left from inflated Auto field");
        assert!(inline.x > 90.0, "expected nested title to remain after parent text");
        assert_eq!(inline.x + inline.w, rect.x + rect.w);
    }

    #[test]
    fn bottom_anchored_progress_meter_uses_label_caption_text_band_bottom() {
        let parent = UiIrNode {
            id: 1,
            parent_id: None,
            children: vec![2],
            node_type: "BuildingBlocks_ComponentLabelCaptionPair".into(),
            name: "pair".into(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [1.0, 0.0],
            pivot: [1.0, 0.0],
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Other {
                    value: 64.0,
                    behavior: "Auto".into(),
                },
                UiIrValue::Other {
                    value: 64.0,
                    behavior: "Auto".into(),
                },
            ],
            padding: [0.0; 4],
            margin: [0.0; 4],
            overflow_mode: None,
            computed_rect: UiIrRect { x: 1736.0, y: -5.5, w: 128.0, h: 152.3 },
            background_fill_colour: None,
            corner_radius: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            segmented_fill: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: Some(UiIrTextPayload::Resolved {
                text: "MEDGELS".into(),
            }),
            secondary_text_payload: Some(UiIrTextPayload::Resolved {
                text: "200/200".into(),
            }),
            secondary_text_style: Some(UiIrTextStyle {
                font_record: None,
                resolved_font_record: None,
                font_size: UiIrValue::Fixed { value: 28.0 },
                line_spacing: None,
                alignment: "Left".into(),
                vertical_alignment: "Center".into(),
                anchor_to_parent_x: None,
                anchor_to_parent_y: None,
                colour: None,
                colour_token: None,
                label_style: None,
            }),
            meter_progress: None,
            text_style: Some(UiIrTextStyle {
                font_record: None,
                resolved_font_record: None,
                font_size: UiIrValue::Fixed { value: 32.0 },
                line_spacing: None,
                alignment: "Left".into(),
                vertical_alignment: "Center".into(),
                anchor_to_parent_x: Some(0.0),
                anchor_to_parent_y: Some(0.5),
                colour: None,
                colour_token: None,
                label_style: None,
            }),
            asset_ref: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: vec![],
            resolved_style_tags: vec![],
        };
        let meter = UiIrNode {
            id: 2,
            parent_id: Some(1),
            children: vec![],
            node_type: "BuildingBlocks_WidgetLinearProgressMeter".into(),
            name: "meter".into(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 1.0],
            pivot: [0.0, 0.0],
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 115.0 },
                UiIrValue::Fixed { value: 15.0 },
            ],
            padding: [0.0; 4],
            margin: [0.0; 4],
            overflow_mode: None,
            computed_rect: UiIrRect { x: 1736.0, y: 146.8, w: 115.0, h: 15.0 },
            background_fill_colour: None,
            corner_radius: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            segmented_fill: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: Some(1.0),
            text_style: None,
            asset_ref: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: vec![],
            resolved_style_tags: vec![],
        };
        let document = UiIrDocument {
            schema_version: 1,
            canvas_guid: "test-canvas".into(),
            canvas_name: None,
            target_width: 1920,
            target_height: 1080,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: crate::ui_ir::UiRendererHint::Bb,
            confidence: 100,
            warnings: vec![],
            unresolved_references: vec![],
            resolved_asset_refs: vec![],
            missing_asset_refs: vec![],
            nodes: vec![parent, meter.clone()],
        };

        let rect = debug_linear_progress_meter_rect(&meter, &document).expect("meter rect");
        let parent_text_rects = debug_text_rects(&document.nodes[0]).expect("parent text rects");
        let expected_y = parent_text_rects
            .secondary
            .map(|secondary_rect| secondary_rect.y + secondary_rect.h)
            .unwrap_or_else(|| parent_text_rects.primary.y + parent_text_rects.primary.h);
        assert!(
            (rect.y - expected_y).abs() < 0.1,
            "expected meter to attach to text-band bottom {}, got {}",
            expected_y,
            rect.y
        );

        let parent_drawn_bounds = debug_text_drawn_bounds(&document.nodes[0]).expect("parent text bounds");
        let drawn_padded_y = match (parent_text_rects.secondary, parent_drawn_bounds.secondary) {
            (Some(secondary_rect), Some(secondary_drawn)) => {
                secondary_rect.y + secondary_rect.h + (secondary_rect.h - secondary_drawn.h)
            }
            _ => parent_text_rects.primary.y + parent_text_rects.primary.h,
        };
        assert!(
            rect.y < drawn_padded_y,
            "expected text-band placement to avoid fallback drawn-bounds padding"
        );
    }
}

fn secondary_close_button_tint(node: &UiIrNode, ctx: &ComposeContext<'_>) -> [f32; 4] {
    node.stroke_colour
        .or_else(|| {
            node.stroke_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.icon_tint_colour)
        .or_else(|| {
            node.icon_tint_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.background_fill_colour)
        .or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .unwrap_or_else(|| derived_accent_tint(ctx))
}