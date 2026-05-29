use crate::canvas::RgbaColor;
use crate::style::{CrtParams, ManufacturerStyle};
use image::{Rgba, RgbaImage};

use super::passes::{pass_scanlines, pass_tint, pass_vignette};

fn drake_style() -> ManufacturerStyle {
    ManufacturerStyle {
        name: "drak".into(),
        primary_tint: RgbaColor {
            r: 255,
            g: 176,
            b: 76,
            a: 255,
        },
        secondary_tint: None,
        colour_slots: vec![RgbaColor {
            r: 255,
            g: 176,
            b: 76,
            a: 255,
        }],
        background: RgbaColor {
            r: 10,
            g: 10,
            b: 10,
            a: 255,
        },
        backlight: RgbaColor {
            r: 102,
            g: 214,
            b: 255,
            a: 255,
        },
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

#[test]
fn tint_black_image_is_noop() {
    let style = drake_style();
    let mut img = solid_image(16, 16, 0, 0, 0, 255);
    pass_tint(&mut img, &style);
    for p in img.pixels() {
        assert_eq!(p.0, [0, 0, 0, 255], "black pixels must stay black");
    }
}

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

#[test]
fn scanlines_darken_nth_row() {
    let style = drake_style();
    let mut img = solid_image(8, 12, 200, 200, 200, 255);
    pass_scanlines(&mut img, &style);

    let period = style.crt.scanline_period_px.round() as u32;
    for y in 0..12u32 {
        let p = img.get_pixel(0, y);
        if y % period == 0 {
            assert!(
                (p.0[0] as u16) < 200,
                "row {y} should be darkened; got {}",
                p.0[0]
            );
        } else {
            assert_eq!(p.0[0], 200, "row {y} should be unchanged");
        }
    }
}

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

#[test]
fn vignette_center_near_original() {
    let style = drake_style();
    let mut img = solid_image(64, 64, 200, 200, 200, 255);
    pass_vignette(&mut img, &style);
    let center = img.get_pixel(32, 32).0[0];
    assert!(center > 180, "center pixel should remain bright, got {center}");
}
