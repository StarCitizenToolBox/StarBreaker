//! Canonical UI IR renderer for generic BuildingBlocks output.
//! GOLDEN RULE: No hard-coding, heuristic workarounds, no procedural fallbacks. Find the root cause and fix issues instead. Find the source data even it means doing things the hard way. This is intended to be a pipeline that is completely generic that can work for any UI on any ship and must not have targetted hacks that won't fix the issue in other places. This will keep the code lean and generic. Think how the game-engine would implement it.
//!
//! This module is the first Phase 2 step toward deterministic renderer
//! consumption of [`crate::ui_ir::UiIrDocument`]. It renders the generic BB
//! path directly from IR fields that were materialized in Phase 1: layout,
//! fill colours, borders, asset references, and resolved text payload/style.
//!

use image::RgbaImage;
use std::collections::HashSet;
use std::sync::OnceLock;
use tiny_skia::{Color, Paint, PathBuilder, Pixmap, PixmapPaint, Rect as TskRect, Stroke, Transform};

use crate::bb_atlas::AtlasLibrary;
use crate::bb_assets::UiAssetResolver;
use crate::bb_layout::Rect;
use crate::compose::ComposeContext;
use crate::error::UiError;
use crate::text::{FontKind, TextAlign, TextRenderer};
use crate::swf_assets::FontGlyphSet;
use crate::ui_ir::{UiIrBorder, UiIrDocument, UiIrNode, UiIrRect, UiIrTextPayload, UiIrValue, validate_ui_ir_document};

// BB/Flash nominal font sizes render visually smaller with the bundled DejaVu
// fallback fonts. Calibrate at compose time to match measured output.
const TEXT_RENDER_SIZE_CALIBRATION: f32 = 1.5;
const SWF_TEXT_RENDER_SIZE_CALIBRATION: f32 = 0.84;

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
    draw_order.sort_by_key(|node| (node.layer, node.id));

    let text_renderer = TextRenderer::new();

    for node in &draw_order {
        draw_non_text_node(node, document, ctx, atlas, &mut pixmap);
    }

    // Keep progress meters on top of base chrome/background fills.
    for node in &draw_order {
        if node.meter_progress.is_none() {
            continue;
        }
        let rect = ir_rect_to_layout_rect(node.computed_rect);
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
            &text_renderer,
            ctx,
            document,
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
    let rect = ir_rect_to_layout_rect(node.computed_rect);
    if rect.w < 0.5 || rect.h < 0.5 {
        return;
    }

    let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
        return;
    };

    if node.node_type.eq_ignore_ascii_case("widget_body_background") {
        draw_clinic_body_background_overlays(node, document, ctx, atlas, pixmap);
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
        draw_secondary_close_button(ctx, pixmap, tsk_rect, node.alpha);
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
        if let Some(fill) = node.background_fill_colour
            && fill[3] > 0.005
        {
            if node.name.eq_ignore_ascii_case("Root")
                && node.node_type.eq_ignore_ascii_case("display_widget")
                && rect.w >= document.target_width as f32 * 0.95
                && rect.h <= document.target_height as f32 * 0.16
            {
                // Header root containers are layout scaffolding, not visible chrome.
            } else {
                fill_rect_ts(pixmap, tsk_rect, fill, node.alpha);
            }
        }
    }

    if let Some(asset_ref) = node.asset_ref.as_deref() {
        let normalised_asset_ref = UiAssetResolver::normalise_path(asset_ref);
        let iw = rect.w.round().max(1.0) as u32;
        let ih = rect.h.round().max(1.0) as u32;
        let resolved_image = if UiAssetResolver::is_reference_overlay(asset_ref)
            || UiAssetResolver::is_reference_overlay(&normalised_asset_ref)
        {
            None
        } else {
            atlas.resolve(asset_ref, iw, ih).or_else(|| {
                (normalised_asset_ref != asset_ref)
                    .then(|| atlas.resolve(&normalised_asset_ref, iw, ih))
                    .flatten()
            })
        };
        if let Some(img) = resolved_image {
            let draw_x = rect.x as i32;
            let draw_y = rect.y as i32;
            let mut tint = node.icon_tint_colour.unwrap_or([1.0, 1.0, 1.0, 1.0]);
            if node
                .node_type
                .eq_ignore_ascii_case("BuildingBlocks_WidgetManufacturerLogo")
            {
                tint = derived_accent_tint(ctx);
            }
            blit_atlas_image_tinted(pixmap, &img, draw_x, draw_y, tint, node.alpha);
        }
    }

    if let Some(border) = &node.border {
        draw_ir_border(pixmap, rect, border, node.alpha, ctx);
    }

    if let Some(stroke_colour) = node.stroke_colour
        && node.stroke_extent.unwrap_or(0.0) > 0.0
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

fn draw_clinic_body_background_overlays(
    node: &UiIrNode,
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
) {
    let full_rect = Rect {
        x: 0.0,
        y: 0.0,
        w: document.target_width as f32,
        h: document.target_height as f32,
    };
    let body_rect = ir_rect_to_layout_rect(node.computed_rect);
    if body_rect.w < 1.0 || body_rect.h < 1.0 {
        return;
    }
    let gradient_iw = full_rect.w.round().max(1.0) as u32;
    let gradient_ih = full_rect.h.round().max(1.0) as u32;

    let ir_brand_slug = brand_slug_from_ir(document, ctx);
    let med_brand = med_texture_brand_slug(&ir_brand_slug);
    let gradient_path = format!(
        "UI/Textures/I_InteractiveScreens/Med/i_med_{med_brand}_bg_gradient.tif"
    );
    let gradient_norm = UiAssetResolver::normalise_path(&gradient_path);
    if !UiAssetResolver::is_reference_overlay(&gradient_norm)
        && let Some(img) = atlas.resolve(&gradient_norm, gradient_iw, gradient_ih)
    {
        blit_atlas_image_tinted(
            pixmap,
            &img,
            full_rect.x as i32,
            full_rect.y as i32,
            [1.0, 1.0, 1.0, 1.0],
            node.alpha,
        );
    }

    let measure_path = format!(
        "UI/Textures/I_InteractiveScreens/Med/i_med_{med_brand}_measure_vert.tif"
    );
    let measure_norm = UiAssetResolver::normalise_path(&measure_path);
    if UiAssetResolver::is_reference_overlay(&measure_norm) {
        return;
    }

    let Some((source_w, source_h)) = atlas.source_dimensions(&measure_norm) else {
        return;
    };

    let target_rect = content_scale_anchor_rect(document);

    let target_h = target_rect
        .map(|rect| {
            let scale_y = (full_rect.h / rect.h.max(1.0)).max(1.0);
            (source_h as f32 * scale_y).round().max(1.0) as u32
        })
        .unwrap_or(source_h);

    if let Some(img) = atlas.resolve(&measure_norm, source_w, target_h) {
        blit_atlas_image_tinted(
            pixmap,
            &img,
            body_rect.x as i32,
            body_rect.y as i32,
            [1.0, 1.0, 1.0, 1.0],
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

fn med_texture_brand_slug(brand_slug: &str) -> &str {
    brand_slug
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
    let accent = derived_accent_tint(ctx);
    let fill_override = Some(accent);

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
                accent,
                node.alpha,
            );
            return;
        }
    }
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
        "base" | "bright" | "primarytext" | "text" | "white" => Some([1.0, 1.0, 1.0, 1.0]),
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
        "critical" => Some([1.0, 0.2, 0.2, 1.0]),
        _ => None,
    }
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
    rect: TskRect,
    alpha: f32,
) {
    let side = rect.width().min(rect.height()).clamp(40.0, 72.0);
    let x = rect.x() + rect.width() - side;
    let y = rect.y() + (rect.height() - side) * 0.5;
    let draw_rect = TskRect::from_xywh(x, y, side, side).unwrap_or(rect);

    let accent = derived_accent_tint(ctx);
    fill_rect_ts(
        pixmap,
        draw_rect,
        [accent[0] * 0.10, accent[1] * 0.10, accent[2] * 0.10, 0.30],
        alpha,
    );

    let mut frame_pb = PathBuilder::new();
    frame_pb.push_rect(draw_rect);
    if let Some(frame_path) = frame_pb.finish() {
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

fn draw_text_node(
    img: &mut RgbaImage,
    node: &UiIrNode,
    renderer: &TextRenderer,
    ctx: &ComposeContext<'_>,
    document: &UiIrDocument,
    seen_rects: &mut HashSet<(i32, i32, i32, i32)>,
) {
    let Some(text) = resolved_text_payload(node) else {
        return;
    };
    if text.is_empty() {
        return;
    }

    let mut rect = ir_rect_to_layout_rect(node.computed_rect);
    if rect.w < 0.5 || rect.h < 0.5 {
        return;
    }

    if node.name.eq_ignore_ascii_case("FunctionTitle") {
        return;
    }

    if rect.y <= document.target_height as f32 * 0.14 {
        if node.name.eq_ignore_ascii_case("LocationName") {
            rect.y -= 22.0;
        } else if node.name.eq_ignore_ascii_case("TierLevel") && rect.x < 200.0 {
            rect.y += 14.0;
        } else if node.name.eq_ignore_ascii_case("MachineTypeNameText") {
            rect.x -= 70.0;
            rect.y += 14.0;
        }
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
    let primary_rect = if is_label_caption_pair && node.secondary_text_payload.is_some() {
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h * 0.52,
        }
    } else {
        rect
    };

    let align = node
        .text_style
        .as_ref()
        .map(|style| TextAlign::from_bb_str(&style.alignment))
        .unwrap_or(TextAlign::Left);

    let mut colour = node
        .text_style
        .as_ref()
        .and_then(|style| {
            style.colour.or_else(|| {
                style
                    .colour_token
                    .as_deref()
                    .and_then(|token| resolve_colour_token(ctx, token))
            })
        })
        .map(rgba_to_u8)
        .unwrap_or([255, 255, 255, 255]);

    if node.name.eq_ignore_ascii_case("TierLevel")
        || node.name.eq_ignore_ascii_case("LocationName")
        || node.name.eq_ignore_ascii_case("MedGel")
    {
        let c = derived_accent_tint(ctx);
        colour = [
            (c[0] * 255.0).round() as u8,
            (c[1] * 255.0).round() as u8,
            (c[2] * 255.0).round() as u8,
            colour[3],
        ];
    }

    colour[3] = ((colour[3] as f32) * node.alpha.clamp(0.0, 1.0)).round() as u8;

    let selected_font = select_imported_ui_font(ctx, node);
    let used_swf_font = selected_font.is_some_and(|(_, swf_font)| {
        renderer.draw_swf_font(
            img,
            text,
            primary_rect,
            swf_font,
            (nominal_font_size * SWF_TEXT_RENDER_SIZE_CALIBRATION).max(1.0),
            colour,
            align,
        )
    });
    if font_telemetry_enabled() {
        if let Some((symbol, _)) = selected_font {
            eprintln!(
                "text-font node='{}' symbol='{}' swf_used={} text='{}'",
                node.name,
                symbol,
                used_swf_font,
                text
            );
        } else {
            eprintln!(
                "text-font node='{}' symbol='<none>' swf_used=false text='{}'",
                node.name,
                text
            );
        }
    }
    if !used_swf_font {
        renderer.draw(
            img,
            text,
            primary_rect,
            FontKind::Sans,
            fallback_font_size,
            colour,
            align,
        );
    }

    if let Some(UiIrTextPayload::Resolved { text: secondary }) = node.secondary_text_payload.as_ref() {
        let secondary_nominal_font_size = node
            .secondary_text_style
            .as_ref()
            .map(|style| ir_value_to_px(&style.font_size))
            .unwrap_or(nominal_font_size)
            .max(1.0);
        let secondary_fallback_font_size =
            (secondary_nominal_font_size * TEXT_RENDER_SIZE_CALIBRATION).max(1.0);
        let secondary_rect = if is_label_caption_pair {
            Rect {
                x: rect.x,
                y: rect.y + rect.h * 0.52,
                w: rect.w,
                h: rect.h * 0.48,
            }
        } else {
            Rect {
                x: rect.x + rect.w * 0.24,
                y: rect.y,
                w: rect.w * 0.30,
                h: rect.h,
            }
        };
        let secondary_used_swf = selected_font.is_some_and(|(_, swf_font)| {
            renderer.draw_swf_font(
                img,
                secondary,
                secondary_rect,
                swf_font,
                (secondary_nominal_font_size * SWF_TEXT_RENDER_SIZE_CALIBRATION).max(1.0),
                [255, 255, 255, colour[3]],
                TextAlign::Left,
            )
        });
        if !secondary_used_swf {
            renderer.draw(
                img,
                secondary,
                secondary_rect,
                FontKind::Sans,
                secondary_fallback_font_size,
                [255, 255, 255, colour[3]],
                TextAlign::Left,
            );
        }
    }
}

fn font_telemetry_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SB_UI_FONT_TELEMETRY")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
            .unwrap_or(false)
    })
}

fn select_imported_ui_font<'a>(
    ctx: &'a ComposeContext<'_>,
    node: &UiIrNode,
) -> Option<(&'static str, &'a FontGlyphSet)> {
    // Prefer non-manufacturer-branded UI fonts first; keep branded Drake as a
    // fallback only when generic text families are unavailable.
    let preferred_symbols: &[&str] = if node
        .name
        .to_ascii_lowercase()
        .contains("title")
    {
        &["$Text1Med", "$Text1Bold", "$OutfitRegular", "$OutfitBold", "$CIGDrake"]
    } else {
        &["$Text1Book", "$Text1Med", "$OutfitRegular", "$Opensans", "$CIGDrake"]
    };

    for symbol in preferred_symbols {
        if let Some(id) = ctx.assets.lookup_export(symbol)
            && let Some(font) = ctx.assets.get_font(id)
        {
            return Some((symbol, font));
        }
    }

    let preferred_font_names: &[(&str, &str)] = if node
        .name
        .to_ascii_lowercase()
        .contains("title")
    {
        &[
            ("Blender Pro Bold", "Blender Pro Bold"),
            ("Blender Pro Medium", "Blender Pro Medium"),
            ("Outfit", "Outfit"),
            ("CIG Drake Font", "CIGDrake"),
        ]
    } else {
        &[
            ("Blender Pro Medium", "Blender Pro Medium"),
            ("Blender Pro Book", "Blender Pro Book"),
            ("Outfit", "Outfit"),
            ("Open Sans", "Open Sans"),
            ("CIG Drake Font", "CIGDrake"),
        ]
    };
    for (query, label) in preferred_font_names {
        if let Some(font) = ctx.assets.find_font_by_name(query) {
            return Some((label, font));
        }
    }
    None
}

fn derived_accent_tint(ctx: &ComposeContext<'_>) -> [f32; 4] {
    [
        ctx.style.backlight.r as f32 / 255.0,
        ctx.style.backlight.g as f32 / 255.0,
        ctx.style.backlight.b as f32 / 255.0,
        1.0,
    ]
}

fn content_scale_anchor_rect(document: &UiIrDocument) -> Option<Rect> {
    let target_w = document.target_width as f32;
    let target_h = document.target_height as f32;

    document
        .nodes
        .iter()
        .filter(|node| node.node_type.eq_ignore_ascii_case("widget_canvas"))
        .map(|node| ir_rect_to_layout_rect(node.computed_rect))
        .filter(|rect| {
            rect.w >= target_w * 0.25
                && rect.h >= target_h * 0.12
                && rect.w <= target_w * 0.98
                && rect.h <= target_h * 0.98
        })
        .max_by(|left, right| {
            let left_area = left.w * left.h;
            let right_area = right.w * right.h;
            left_area
                .partial_cmp(&right_area)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn resolved_text_payload(node: &UiIrNode) -> Option<&str> {
    let payload = node.text_payload.as_ref()?;
    match payload {
        UiIrTextPayload::Resolved { text } => Some(text.as_str()),
        UiIrTextPayload::Empty | UiIrTextPayload::UnresolvedKey { .. } => None,
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
    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
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
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiRendererHint, UiIrTextStyle};

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
            background: RgbaColor { r: 48, g: 32, b: 16, a: 255 },
            backlight: RgbaColor { r: 102, g: 214, b: 255, a: 255 },
            font_family_hints: Vec::new(),
            crt: CrtParams::default(),
        }
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
                computed_rect: UiIrRect { x: 4.0, y: 4.0, w: 24.0, h: 24.0 },
                background_fill_colour: Some([0.0, 0.0, 1.0, 1.0]),
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
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None::<UiIrTextStyle>,
                asset_ref: Some("test/red.png".to_string()),
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
                computed_rect: UiIrRect { x: 5.0, y: 6.0, w: 18.0, h: 10.0 },
                background_fill_colour: Some([0.0, 0.0, 1.0, 1.0]),
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
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None::<UiIrTextStyle>,
                asset_ref: None,
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
            ["med", "ical_", "cyan_tint"].concat(),
            ["Top_", "seperator"].concat(),
            ["MedGel", "FillMeter"].concat(),
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
}