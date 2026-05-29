//! Brand-modifier raw-asset rendering helpers.

use tiny_skia::Pixmap;

use crate::bb_assets::UiAssetResolver;
use crate::bb_atlas::AtlasLibrary;
use crate::bb_bindings::BindingResolver;
use crate::bb_layout::Rect;
use crate::bb_scene::BbNode;

use super::{ComposeContext, blit_atlas_image, blit_atlas_image_alpha_mask_tinted, blit_atlas_image_tinted};

pub(crate) fn draw_raw_asset(
    node: &BbNode,
    rect: Rect,
    resolver: &BindingResolver,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
    alpha: f32,
) -> bool {
    let svg_path_raw = node
        .raw
        .get("SvgPath")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            node.raw
                .get("svgPath")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            node.raw
                .get("svgFill")
                .and_then(|s| s.get("svgPath"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        });
    let img_path_raw = node
        .raw
        .get("ImagePath")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            node.raw
                .get("imagePath")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| resolver.resolve_string_binding(node.id))
        .filter(|s| !s.is_empty());

    if svg_path_raw.is_none() && img_path_raw.is_none() {
        return false;
    }

    let iw = rect.w.round().max(1.0) as u32;
    let ih = rect.h.round().max(1.0) as u32;

    if let Some(raw_path) = svg_path_raw {
        let norm = UiAssetResolver::normalise_path(raw_path);
        if !UiAssetResolver::is_reference_overlay(&norm) {
            let fill_override = node_fill_override(node);
            if let Some(svg_bytes) = atlas.fetch_raw(&norm)
                && let Some(img) = crate::bb_svg::rasterize_svg(&svg_bytes, iw, ih, fill_override)
            {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
        }
        return true;
    }

    if let Some(raw_path) = img_path_raw {
        let norm = UiAssetResolver::normalise_path(raw_path);
        if !UiAssetResolver::is_reference_overlay(&norm)
            && let Some(img) = atlas.resolve(&norm, iw, ih)
        {
            let tint = node
                .icon
                .as_ref()
                .and_then(|i| i.tint_colour)
                .or_else(|| node_fill_override(node))
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            if node.raw.get("FillColor").is_some() {
                blit_atlas_image_alpha_mask_tinted(
                    pixmap,
                    &img,
                    rect.x as i32,
                    rect.y as i32,
                    tint,
                    alpha,
                );
            } else {
                blit_atlas_image_tinted(pixmap, &img, rect.x as i32, rect.y as i32, tint, alpha);
            }
        }
        return true;
    }

    false
}

pub(crate) fn draw_manufacturer_logo(
    node: &BbNode,
    rect: Rect,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
    alpha: f32,
    ctx: &ComposeContext<'_>,
) -> bool {
    let brand = node_brand_slug(node, ctx);
    let brand_title = brand_title(&brand);
    let candidates = [
        format!("UI/Textures/Vector/General/BrandLogos/logo_{brand}_a.svg"),
        format!("UI/Textures/Signs/Brands/{brand}/{brand_title}_logo.dds"),
        format!("UI/Textures/Signs/Brands/{brand}/{brand_title}_logo.svg"),
    ];

    let iw = rect.w.round().max(1.0) as u32;
    let ih = rect.h.round().max(1.0) as u32;
    let fill_override = node_fill_override(node);

    for raw_path in candidates {
        let norm = UiAssetResolver::normalise_path(&raw_path);
        if UiAssetResolver::is_reference_overlay(&norm) {
            continue;
        }
        if norm.ends_with(".svg") {
            if let Some(svg_bytes) = atlas.fetch_raw(&norm)
                && let Some(img) = crate::bb_svg::rasterize_svg(&svg_bytes, iw, ih, fill_override)
            {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
                return true;
            }
        } else if let Some(img) = atlas.resolve(&norm, iw, ih) {
            blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            return true;
        }
    }

    false
}

fn brand_slug(identifier: &str) -> String {
    identifier
        .to_ascii_lowercase()
        .trim_start_matches("s_")
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
}

fn node_brand_slug(node: &BbNode, ctx: &ComposeContext<'_>) -> String {
    node.raw
        .get("__BrandIdentifier")
        .and_then(|v| v.as_str())
        .map(brand_slug)
        .unwrap_or_else(|| brand_slug(&ctx.style.name))
}

fn brand_title(slug: &str) -> String {
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Extract fill-colour override for SVG tinting.
pub(crate) fn node_fill_override(node: &BbNode) -> Option<[f32; 4]> {
    if let Some(bg) = &node.background
        && let Some(c) = bg.fill_colour
    {
        return Some(c);
    }

    let obj = node.raw.get("FillColor")?.as_object()?;
    let r = obj.get("r").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let g = obj.get("g").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let b = obj.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let a = obj.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some([r, g, b, a])
}
