//! SVG rasterisation helper for BuildingBlocks UI widgets.
//!
//! Provides [`rasterize_svg`] which parses an SVG document, optionally applies a
//! fill-colour override to every rendered pixel, and returns a decoded RGBA image
//! sized to the caller-supplied target dimensions.
//!
//! # Fill override
//! Many Star Citizen UI SVGs are monochrome masks coloured at runtime by a brand
//! modifier's `FillColor`.  When `fill_override` is `Some([r, g, b, a])`, every
//! non-transparent pixel in the rendered output is recoloured to the override RGB
//! while preserving the rendered SVG alpha mask and scaling opacity by `fill[3]`.

use image::RgbaImage;
use log::warn;
use tiny_skia_011 as tiny_skia;

/// Rasterise `svg_bytes` into an RGBA image of `target_w × target_h` pixels.
///
/// If `fill_override` is `Some([r, g, b, a])` (components in `0.0..=1.0`), every
/// non-transparent pixel is recoloured to the override RGB after rendering.
///
/// Returns `None` when:
/// - `target_w` or `target_h` is zero,
/// - the SVG cannot be parsed (logged at `warn`),
/// - the internal pixmap allocation fails.
pub fn rasterize_svg(
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
) -> Option<RgbaImage> {
    if target_w == 0 || target_h == 0 {
        return None;
    }

    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_bytes, &opts)
        .map_err(|e| {
            warn!("bb_svg: SVG parse failed: {}", e);
            e
        })
        .ok()?;

    let source_w = tree.size().width();
    let source_h = tree.size().height();
    if source_w <= 0.0 || source_h <= 0.0 {
        warn!("bb_svg: SVG has invalid size {}×{}", source_w, source_h);
        return None;
    }

    let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)?;
    let transform = tiny_skia::Transform::from_scale(
        target_w as f32 / source_w,
        target_h as f32 / source_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut bytes = pixmap.take();

    // Apply fill_override as a colour overlay. SVG UI glyphs are authored as
    // monochrome masks (often black strokes), so multiplying by the source RGB
    // would keep black glyphs black instead of applying the BuildingBlocks tint.
    if let Some(fill) = fill_override {
        for chunk in bytes.chunks_exact_mut(4) {
            if chunk[3] > 0 {
                let alpha = (chunk[3] as f32 * fill[3]).clamp(0.0, 255.0);
                chunk[0] = (fill[0].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[1] = (fill[1].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[2] = (fill[2].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[3] = alpha as u8;
            }
        }
    }

    RgbaImage::from_raw(target_w, target_h, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 4×4 white SVG used as a test fixture.
    const WHITE_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
        <rect width="4" height="4" fill="white"/>
    </svg>"#;

    /// A minimal 4×4 red SVG used as a test fixture.
    const RED_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
        <rect width="4" height="4" fill="red"/>
    </svg>"#;

    /// A minimal 4×4 black SVG used as a mask-style test fixture.
    const BLACK_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
        <rect width="4" height="4" fill="black"/>
    </svg>"#;

    #[test]
    fn rasterizes_to_correct_size() {
        let img = rasterize_svg(WHITE_SVG, 32, 32, None).expect("should rasterize");
        assert_eq!((img.width(), img.height()), (32, 32));
    }

    #[test]
    fn rasterizes_to_non_empty_pixmap() {
        let img = rasterize_svg(WHITE_SVG, 8, 8, None).expect("should rasterize");
        // At least one pixel must be non-transparent.
        let any_visible = img.pixels().any(|p| p.0[3] > 0);
        assert!(any_visible, "rasterized image should have non-transparent pixels");
    }

    #[test]
    fn fill_override_tints_pixels() {
        // White SVG + pure-blue fill override → pixels should be blue-ish.
        let fill = Some([0.0, 0.0, 1.0, 1.0]);
        let img = rasterize_svg(WHITE_SVG, 8, 8, fill).expect("should rasterize");
        let centre = img.get_pixel(4, 4).0;
        // The pixel should have effectively zero red and green channels, and visible blue.
        // (tiny-skia stores premultiplied; white pixels become fully blue after override.)
        assert!(
            centre[0] < 30 && centre[2] > 100,
            "centre pixel should be blue-ish after fill override, got {centre:?}"
        );
    }

    #[test]
    fn fill_override_recolours_black_mask_pixels() {
        let fill = Some([115.0 / 255.0, 198.0 / 255.0, 254.0 / 255.0, 1.0]);
        let img = rasterize_svg(BLACK_SVG, 4, 4, fill).expect("should rasterize");
        let px = img.get_pixel(2, 2).0;
        assert!(
            px[0] >= 110 && px[1] >= 190 && px[2] >= 245,
            "black mask pixel should be recoloured cyan, got {px:?}"
        );
    }

    #[test]
    fn fill_override_preserves_straight_rgb_for_partial_alpha_masks() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <rect width="4" height="4" fill="black" opacity="0.5"/>
        </svg>"#;
        let fill = Some([115.0 / 255.0, 198.0 / 255.0, 254.0 / 255.0, 1.0]);
        let img = rasterize_svg(svg, 4, 4, fill).expect("should rasterize");
        let px = img.get_pixel(2, 2).0;
        assert!(px[3] > 80 && px[3] < 180, "expected partial alpha, got {px:?}");
        assert!(px[0] >= 110 && px[1] >= 190 && px[2] >= 245, "RGB should remain straight overlay colour, got {px:?}");
    }

    #[test]
    fn fill_override_none_preserves_red_svg() {
        let img = rasterize_svg(RED_SVG, 4, 4, None).expect("should rasterize");
        let px = img.get_pixel(2, 2).0;
        // Premultiplied red pixel: R > G,B.
        assert!(
            px[0] > px[1] && px[0] > px[2] && px[3] > 0,
            "centre pixel should be red-ish, got {px:?}"
        );
    }

    #[test]
    fn returns_none_for_zero_dimensions() {
        assert!(rasterize_svg(WHITE_SVG, 0, 16, None).is_none());
        assert!(rasterize_svg(WHITE_SVG, 16, 0, None).is_none());
    }

    #[test]
    fn returns_none_for_invalid_svg() {
        let result = rasterize_svg(b"not an svg at all", 16, 16, None);
        assert!(result.is_none(), "invalid SVG bytes should return None");
    }
}
