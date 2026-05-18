//! Coarse "did we draw anything" assertions for rendered canvas PNGs.
//!
//! These are not a correctness signal — that comes from human comparison
//! against `reference/in-game/Clipper/*.png` per the Phase 10.5 process
//! rule. These checks exist to catch the failure mode that produced 54
//! blank PNGs in Phases 6–10: an output that compiles, passes per-module
//! tests, but is effectively single-coloured.
//!
//! Each helper takes a rendered [`image::RgbaImage`] and the manufacturer
//! style's background colour, then asserts the image is not trivially
//! blank.  Tests in higher phases call these on real composed output.

use image::RgbaImage;

/// Assert the image is not all the same colour.
///
/// Sampling: every 4th pixel along both axes (~1/16 of the image), to keep
/// the check fast on large canvases.
pub fn assert_not_uniform(img: &RgbaImage, label: &str) {
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
        "[{label}] image is entirely one colour ({:?}); render produced no content",
        first.unwrap_or([0, 0, 0, 0]),
    );
}

/// Assert the image contains at least `min_unique` distinct quantised
/// (4-bit per channel) colours when sampled.
pub fn assert_min_distinct_colours(img: &RgbaImage, min_unique: usize, label: &str) {
    use std::collections::HashSet;
    let (w, h) = img.dimensions();
    let mut seen = HashSet::new();
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let p = img.get_pixel(x, y).0;
            // Quantise to 4 bits per channel.
            let key = ((p[0] as u32 >> 4) << 12)
                | ((p[1] as u32 >> 4) << 8)
                | ((p[2] as u32 >> 4) << 4);
            seen.insert(key);
        }
    }
    assert!(
        seen.len() >= min_unique,
        "[{label}] expected >= {min_unique} distinct colours, got {}",
        seen.len(),
    );
}

/// Assert that at least `min_frac` of sampled pixels differ from `bg` by
/// more than a per-channel tolerance of 16.
///
/// This guards against the Phase 6–10 mode where the render filled the
/// canvas with the bg colour + a vignette gradient and called it done.
pub fn assert_non_background_fraction(
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
        "[{label}] only {:.1}% of sampled pixels differ from bg {bg:?}; expected >= {:.1}%",
        frac * 100.0,
        min_frac * 100.0,
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Self-tests for the harness
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    #[should_panic(expected = "entirely one colour")]
    fn rejects_uniform_image() {
        let img = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255]));
        assert_not_uniform(&img, "uniform");
    }

    #[test]
    fn accepts_mixed_image() {
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        assert_not_uniform(&img, "mixed");
    }

    #[test]
    #[should_panic(expected = "expected >= 5 distinct")]
    fn rejects_few_distinct_colours() {
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        // Only 2 distinct colours.
        assert_min_distinct_colours(&img, 5, "few-distinct");
    }

    #[test]
    #[should_panic(expected = "% of sampled pixels differ from bg")]
    fn rejects_near_bg_image() {
        // Everything is bg.
        let img = RgbaImage::from_pixel(64, 64, Rgba([10, 10, 10, 255]));
        assert_non_background_fraction(&img, [10, 10, 10, 255], 0.1, "all-bg");
    }

    #[test]
    fn accepts_image_with_real_content() {
        // 25 % of pixels are non-bg amber.
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([48, 32, 16, 255]));
        for y in 0..64 {
            for x in 0..16 {
                img.put_pixel(x, y, Rgba([240, 168, 104, 255]));
            }
        }
        assert_non_background_fraction(&img, [48, 32, 16, 255], 0.1, "real-content");
        assert_min_distinct_colours(&img, 2, "real-content");
        assert_not_uniform(&img, "real-content");
    }
}
