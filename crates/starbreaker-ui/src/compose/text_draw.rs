//! Text rendering helpers.

use image::RgbaImage;

use crate::bb_bindings::BindingResolver;
use crate::bb_layout::Rect;
use crate::bb_scene::BbNode;
use crate::text::{FontKind, TextAlign, TextRenderer, VerticalAlign};

use super::ComposeContext;

pub(crate) fn draw_text_node(
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

    let has_explicit_font_size = node.raw.get("fontSize").is_some();
    let explicit_size = if has_explicit_font_size {
        match node.text.as_ref().map(|t| &t.font_size) {
            Some(crate::bb_scene::BbValue::Fixed(px)) if *px > 0.0 => Some(*px),
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

    renderer.draw(
        img,
        text,
        rect,
        FontKind::Sans,
        size_px,
        colour,
        align,
        VerticalAlign::Centre,
        None,
    );
}

pub(crate) fn font_size_from_style(style: &str) -> f32 {
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

pub(crate) fn colour_component_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
