//! Manufacturer post-process pass — Phase 8.
//!
//! Applies a sequence of in-place image passes to an [`image::RgbaImage`] that
//! was produced by [`crate::compose::render_canvas`], giving the final rendered
//! screen texture the warm CRT look of the manufacturer's physical display.
//!
//! # Why each pass exists
//!
//! ## Tint pass
//! The raw SWF content is authored in neutral white/grey so it works across all
//! manufacturers.  The physical screen applies a manufacturer-specific tint via
//! its CRT material shader.  For Drake (`drak`), `S_DRAK_HUD.colorStyles[0]`
//! records the primary amber `#FF9E39`.  The fallback used during development is
//! `#FFB04C` (Phase 1 reference-image observation).  The tint is multiplied only
//! onto *lit* (emissive) pixels so the dark background stays black.
//!
//! ## Scanlines
//! Star Citizen Drake screens use a pixel-layout CRT asset family documented in
//! `ui-display-shader-research.md`.  A soft horizontal scanline pattern is
//! applied by darkening every N-th row.  `CrtParams::scanline_period_px` and
//! `CrtParams::scanline_intensity` control the density and strength.
//!
//! ## Pixel grid
//! A complementary vertical grid darkens every N-th column at half the scanline
//! intensity, simulating the sub-pixel column separation of a real CRT.
//!
//! ## Vignette
//! The screen edge is physically darker on a CRT due to electron-beam falloff at
//! the edges.  A simple radial `1 − strength × r²` factor approximates this.
//! `CrtParams::vignette_strength` is the tuning knob.
//!
//! ## Glow
//! A soft additive 3×3 box-blur of the lit channel could simulate phosphor glow.
//! **This pass is currently a no-op stub** — a clean, allocation-light
//! implementation within the 30–50 LOC budget is not feasible without an
//! additional temporary buffer (one full-image allocation).  The pass is
//! architecturally reserved and gated behind `PostProcessOptions::apply_glow`
//! which defaults to `false`.  It is scheduled as a **Phase 10 polish** item.
//!
//! # Color source notes
//! - **Primary tint (amber):** `S_DRAK_HUD.colorStyles[0]` = `#FF9E39`
//!   (DataCore Phase 1 research).  Fallback: `#FFB04C` (reference-image
//!   observation).
//! - **Cyan `screenStates.color` (`#66D6FF`):** This is the **screen backlight
//!   color** (the physical frame glow), sourced from
//!   `SCItemDisplayScreenComponentParams.screenStates[Normal].color`.  It is NOT
//!   the UI content tint.  Do not confuse the two.  The backlight is stored in
//!   [`crate::style::ManufacturerStyle::backlight`] but is intentionally NOT
//!   applied by the tint pass.
//!
//! # Tuning via `CrtParams`
//! All CRT parameters live in [`crate::style::CrtParams`] which is embedded in
//! [`crate::style::ManufacturerStyle`].  Adjust via `ManufacturerStyle::crt`.
//!
//! ```ignore
//! style.crt.scanline_intensity = 0.20;     // heavier scanlines
//! style.crt.vignette_strength  = 0.40;     // darker corners
//! ```

use image::RgbaImage;

use crate::style::ManufacturerStyle;

// ──────────────────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────────────────

/// Options controlling which post-process passes are active.
///
/// All passes except [`apply_glow`][`PostProcessOptions::apply_glow`] default
/// to `true`.  Glow defaults to `false` because the pass is currently a stub
/// (see module-level docs); enabling it is a no-op.
#[derive(Debug, Clone)]
pub struct PostProcessOptions {
    /// Apply the manufacturer primary-tint multiplication to lit pixels.
    pub apply_tint: bool,
    /// Apply horizontal scanline darkening.
    pub apply_scanlines: bool,
    /// Apply vertical pixel-grid darkening.
    pub apply_pixel_grid: bool,
    /// Apply radial corner vignette.
    pub apply_vignette: bool,
    /// Apply soft phosphor glow (currently a stub — Phase 10 follow-up).
    pub apply_glow: bool,
}

impl Default for PostProcessOptions {
    fn default() -> Self {
        Self {
            apply_tint: true,
            apply_scanlines: true,
            apply_pixel_grid: true,
            apply_vignette: true,
            apply_glow: false,
        }
    }
}

/// In-place post-processor that applies CRT and tint passes to a rendered UI image.
pub struct PostProcessor<'a> {
    pub style: &'a ManufacturerStyle,
}

impl<'a> PostProcessor<'a> {
    /// Create a new post-processor bound to the given manufacturer style.
    pub fn new(style: &'a ManufacturerStyle) -> Self {
        Self { style }
    }

    /// Run all enabled passes in fixed order: tint → scanlines → pixel-grid →
    /// vignette → glow.
    ///
    /// All passes operate in place on `img`.  Alpha channel is preserved by
    /// every pass.  No heap allocation is performed except by the glow pass
    /// (currently a no-op).
    pub fn run(&self, img: &mut RgbaImage, opts: &PostProcessOptions) {
        if opts.apply_tint {
            pass_tint(img, self.style);
        }
        if opts.apply_scanlines {
            pass_scanlines(img, self.style);
        }
        if opts.apply_pixel_grid {
            pass_pixel_grid(img, self.style);
        }
        if opts.apply_vignette {
            pass_vignette(img, self.style);
        }
        if opts.apply_glow {
            pass_glow_stub(img, self.style);
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Pass 1 — Tint
// ──────────────────────────────────────────────────────────────────────────────

/// Multiply each lit pixel's RGB by the manufacturer primary tint.
///
/// # Math
/// For a pixel with components `(R, G, B, A)` in `[0, 255]`:
/// 1. Compute rec709 luminance: `L = 0.2126·R/255 + 0.7152·G/255 + 0.0722·B/255`.
/// 2. If `L > 0.02` (lit threshold) **and** `A > 0` (visible):
///    - `R' = clamp(R · tR/255, 0, 255)`
///    - `G' = clamp(G · tG/255, 0, 255)`
///    - `B' = clamp(B · tB/255, 0, 255)`
/// 3. Otherwise: leave the pixel unchanged (keeps background black, transparent areas clear).
///
/// The threshold 0.02 was chosen empirically: it is bright enough to skip
/// near-black background pixels but dark enough to tint dim UI lines.
fn pass_tint(img: &mut RgbaImage, style: &ManufacturerStyle) {
    let t = style.primary_tint;
    let tr = t.r as f32 / 255.0;
    let tg = t.g as f32 / 255.0;
    let tb = t.b as f32 / 255.0;

    const LIT_THRESHOLD: f32 = 0.02;

    for pixel in img.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        if a == 0 {
            continue;
        }
        let lum = 0.2126 * (r as f32 / 255.0)
            + 0.7152 * (g as f32 / 255.0)
            + 0.0722 * (b as f32 / 255.0);
        if lum > LIT_THRESHOLD {
            pixel.0[0] = ((r as f32) * tr).round().clamp(0.0, 255.0) as u8;
            pixel.0[1] = ((g as f32) * tg).round().clamp(0.0, 255.0) as u8;
            pixel.0[2] = ((b as f32) * tb).round().clamp(0.0, 255.0) as u8;
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Pass 2 — Scanlines
// ──────────────────────────────────────────────────────────────────────────────

/// Darken every N-th row by `(1 − scanline_intensity)`.
///
/// Period `N = max(1, round(scanline_period_px))`.  Rows whose index `y % N == 0`
/// are dimmed; all other rows are left unchanged.  Alpha is preserved.
fn pass_scanlines(img: &mut RgbaImage, style: &ManufacturerStyle) {
    let period = (style.crt.scanline_period_px.round() as u32).max(1);
    let dim = 1.0 - style.crt.scanline_intensity;
    let (w, h) = img.dimensions();
    for y in 0..h {
        if y % period == 0 {
            for x in 0..w {
                let p = img.get_pixel_mut(x, y);
                p.0[0] = ((p.0[0] as f32) * dim).round().clamp(0.0, 255.0) as u8;
                p.0[1] = ((p.0[1] as f32) * dim).round().clamp(0.0, 255.0) as u8;
                p.0[2] = ((p.0[2] as f32) * dim).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Pass 3 — Pixel grid
// ──────────────────────────────────────────────────────────────────────────────

/// Darken every N-th column at half the scanline intensity.
///
/// Period `N = max(2, round(pixel_grid_period_px))`.  Skipped when period ≤ 1.
/// Dim factor = `1 − scanline_intensity × 0.5`.  Alpha is preserved.
fn pass_pixel_grid(img: &mut RgbaImage, style: &ManufacturerStyle) {
    let period = style.crt.pixel_grid_period_px.round() as u32;
    if period <= 1 {
        return;
    }
    let dim = 1.0 - style.crt.scanline_intensity * 0.5;
    let (w, h) = img.dimensions();
    for x in 0..w {
        if x % period == 0 {
            for y in 0..h {
                let p = img.get_pixel_mut(x, y);
                p.0[0] = ((p.0[0] as f32) * dim).round().clamp(0.0, 255.0) as u8;
                p.0[1] = ((p.0[1] as f32) * dim).round().clamp(0.0, 255.0) as u8;
                p.0[2] = ((p.0[2] as f32) * dim).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Pass 4 — Vignette
// ──────────────────────────────────────────────────────────────────────────────

/// Apply a radial corner vignette: `factor = 1 − strength × r²`.
///
/// `r` is the Euclidean distance from the image centre normalised to the
/// half-diagonal so that exactly the corner pixels have `r = 1.0`.  The factor
/// is clamped to `[0.0, 1.0]` before multiplication.  Alpha is preserved.
fn pass_vignette(img: &mut RgbaImage, style: &ManufacturerStyle) {
    let strength = style.crt.vignette_strength;
    if strength <= 0.0 {
        return;
    }
    let (w, h) = img.dimensions();
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    // Half-diagonal normalisation so corner pixels have r == 1.0.
    let half_diag = (cx * cx + cy * cy).sqrt();
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 - cx) / half_diag;
            let dy = (y as f32 - cy) / half_diag;
            let r2 = dx * dx + dy * dy;
            let factor = (1.0 - strength * r2).clamp(0.0, 1.0);
            let p = img.get_pixel_mut(x, y);
            p.0[0] = ((p.0[0] as f32) * factor).round().clamp(0.0, 255.0) as u8;
            p.0[1] = ((p.0[1] as f32) * factor).round().clamp(0.0, 255.0) as u8;
            p.0[2] = ((p.0[2] as f32) * factor).round().clamp(0.0, 255.0) as u8;
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Pass 5 — Glow (stub)
// ──────────────────────────────────────────────────────────────────────────────

/// Glow pass — **currently a no-op stub**.
///
/// A 3×3 box-blur of the lit channel additively composited at ~10% would
/// simulate phosphor glow.  The implementation requires one full-image scratch
/// buffer, which is non-trivial to do cleanly within 30–50 LOC without an
/// additional heap allocation.  This is deferred to **Phase 10**.
///
/// The pass is gated behind `PostProcessOptions::apply_glow` which defaults to
/// `false`, so callers using [`PostProcessOptions::default()`] are unaffected.
#[allow(unused_variables)]
fn pass_glow_stub(_img: &mut RgbaImage, _style: &ManufacturerStyle) {
    // Phase 10 follow-up: implement a 3×3 box-blur of lit pixels, additively
    // composited at ~10% intensity.  No-op for now.
}

// ──────────────────────────────────────────────────────────────────────────────
// Unit tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::RgbaColor;
    use crate::style::{CrtParams, ManufacturerStyle};
    use image::{Rgba, RgbaImage};

    fn drake_style() -> ManufacturerStyle {
        ManufacturerStyle {
            name: "drak".into(),
            primary_tint: RgbaColor { r: 255, g: 176, b: 76, a: 255 },
            secondary_tint: None,
            colour_slots: vec![RgbaColor { r: 255, g: 176, b: 76, a: 255 }],
            background: RgbaColor { r: 10, g: 10, b: 10, a: 255 },
            backlight: RgbaColor { r: 102, g: 214, b: 255, a: 255 },
            font_family_hints: vec![],
            crt: CrtParams {
                scanline_period_px: 3.0,
                pixel_grid_period_px: 3.0,
                scanline_intensity: 0.15,
                vignette_strength: 0.3,
            },
        }
    }

    fn solid_image(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([r, g, b, a]))
    }

    // ── Tint pass ─────────────────────────────────────────────────────────────

    /// A fully-black image is not lit (luminance = 0); tint pass must be a no-op.
    #[test]
    fn tint_black_image_is_noop() {
        let style = drake_style();
        let mut img = solid_image(16, 16, 0, 0, 0, 255);
        pass_tint(&mut img, &style);
        for p in img.pixels() {
            assert_eq!(p.0, [0, 0, 0, 255], "black pixels must stay black");
        }
    }

    /// Transparent pixels (alpha = 0) must not be touched by the tint pass.
    #[test]
    fn tint_transparent_pixels_untouched() {
        let style = drake_style();
        let mut img = solid_image(8, 8, 200, 200, 200, 0);
        pass_tint(&mut img, &style);
        for p in img.pixels() {
            assert_eq!(p.0[3], 0, "alpha must remain 0");
            assert_eq!(p.0, [200, 200, 200, 0], "transparent pixels must not change");
        }
    }

    /// A fully-white opaque image should become the manufacturer primary tint (within ±2 rounding).
    #[test]
    fn tint_white_image_yields_primary_tint() {
        let style = drake_style();
        let mut img = solid_image(16, 16, 255, 255, 255, 255);
        pass_tint(&mut img, &style);
        let t = style.primary_tint;
        for p in img.pixels() {
            let dr = (p.0[0] as i16 - t.r as i16).abs();
            let dg = (p.0[1] as i16 - t.g as i16).abs();
            let db = (p.0[2] as i16 - t.b as i16).abs();
            assert!(dr <= 2, "R channel: got {} expected {} ±2", p.0[0], t.r);
            assert!(dg <= 2, "G channel: got {} expected {} ±2", p.0[1], t.g);
            assert!(db <= 2, "B channel: got {} expected {} ±2", p.0[2], t.b);
        }
    }

    // ── Scanline pass ─────────────────────────────────────────────────────────

    /// Every period-th row (period=3 → rows 0, 3, 6, …) must be darker than
    /// adjacent rows.
    #[test]
    fn scanlines_darken_nth_row() {
        let style = drake_style();
        let mut img = solid_image(8, 12, 200, 200, 200, 255);
        pass_scanlines(&mut img, &style);

        let period = style.crt.scanline_period_px.round() as u32;
        for y in 0..12u32 {
            let p = img.get_pixel(0, y);
            if y % period == 0 {
                // Scanline row: must be darker than 200
                assert!(
                    (p.0[0] as u16) < 200,
                    "row {y} should be darkened; got {}",
                    p.0[0]
                );
            } else {
                // Non-scanline row: unchanged
                assert_eq!(p.0[0], 200, "row {y} should be unchanged");
            }
        }
    }

    // ── Vignette pass ─────────────────────────────────────────────────────────

    /// Corners must be darker than the center pixel on a white image.
    #[test]
    fn vignette_darkens_corners_more_than_center() {
        let style = drake_style();
        let mut img = solid_image(64, 64, 200, 200, 200, 255);
        pass_vignette(&mut img, &style);

        let center = img.get_pixel(32, 32).0[0];
        let corner = img.get_pixel(0, 0).0[0];
        assert!(
            corner < center,
            "corner ({corner}) must be darker than center ({center})"
        );
    }

    /// Center pixel should be the brightest (or very close to original) after vignette.
    #[test]
    fn vignette_center_near_original() {
        let style = drake_style();
        let mut img = solid_image(64, 64, 200, 200, 200, 255);
        pass_vignette(&mut img, &style);
        let center = img.get_pixel(32, 32).0[0];
        // Center r ≈ (cx, cy) → r≈0, factor≈1 → pixel stays near 200
        assert!(center > 180, "center pixel should remain bright, got {center}");
    }
}
