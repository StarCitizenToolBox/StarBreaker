//! Post-process pixel passes for tint and CRT effects.

use image::RgbaImage;

use crate::style::ManufacturerStyle;

/// Multiply each lit pixel's RGB by the manufacturer primary tint.
pub(super) fn pass_tint(img: &mut RgbaImage, style: &ManufacturerStyle) {
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

/// Darken every N-th row by `(1 - scanline_intensity)`.
pub(super) fn pass_scanlines(img: &mut RgbaImage, style: &ManufacturerStyle) {
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

/// Darken every N-th column at half the scanline intensity.
pub(super) fn pass_pixel_grid(img: &mut RgbaImage, style: &ManufacturerStyle) {
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

/// Apply a radial corner vignette: `factor = 1 - strength * r^2`.
pub(super) fn pass_vignette(img: &mut RgbaImage, style: &ManufacturerStyle) {
    let strength = style.crt.vignette_strength;
    if strength <= 0.0 {
        return;
    }
    let (w, h) = img.dimensions();
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
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

/// Glow pass stub (no-op).
#[allow(unused_variables)]
pub(super) fn pass_glow_stub(_img: &mut RgbaImage, _style: &ManufacturerStyle) {}
