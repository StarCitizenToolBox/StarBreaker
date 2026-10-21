use crate::canvas::RgbaColor;
use crate::error::UiError;

use super::StyleLoader;

fn loader() -> StyleLoader {
    StyleLoader::for_manufacturer("drak")
}

#[test]
fn neutral_fallback_name() {
    assert_eq!(loader().neutral_fallback().name, "drak");
}

#[test]
fn neutral_fallback_is_white_on_black_with_no_palette() {
    // The no-data fallback is deliberately NEUTRAL (white foreground, black
    // background, no palette slots) — never an invented brand palette.
    let style = loader().neutral_fallback();
    assert_eq!(style.primary_tint, RgbaColor { r: 255, g: 255, b: 255, a: 255 });
    assert_eq!(style.background, RgbaColor { r: 0, g: 0, b: 0, a: 255 });
    assert!(style.colour_slots.is_empty());
}

fn full_fixture() -> serde_json::Value {
    serde_json::json!({
        "primaryColor":    { "r": 255, "g": 176, "b": 76,  "a": 255 },
        "secondaryColor":  { "r": 200, "g": 100, "b": 50,  "a": 255 },
        "backgroundColor": { "r": 10,  "g": 10,  "b": 10,  "a": 255 },
        "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 },
        "fontFamilies":    ["Rajdhani", "Orbitron"],
        "crt": {
            "scanlinePeriodPx":  4.0,
            "pixelGridPeriodPx": 3.5,
            "scanlineIntensity": 0.2,
            "vignetteStrength":  0.4
        }
    })
}

#[test]
fn parse_record_primary_tint() {
    let style = loader().parse_record(&full_fixture()).unwrap();
    // hardcoding-guard: synthetic — round-trip of the synthetic full_fixture() JSON
    assert_eq!(
        style.primary_tint,
        RgbaColor {
            r: 255,
            g: 176,
            b: 76,
            a: 255
        }
    );
}

#[test]
fn parse_record_secondary_tint() {
    let style = loader().parse_record(&full_fixture()).unwrap();
    // hardcoding-guard: synthetic — round-trip of the synthetic full_fixture() JSON
    assert_eq!(
        style.secondary_tint,
        Some(RgbaColor {
            r: 200,
            g: 100,
            b: 50,
            a: 255
        })
    );
}

#[test]
fn parse_record_background() {
    let style = loader().parse_record(&full_fixture()).unwrap();
    // hardcoding-guard: synthetic — round-trip of the synthetic full_fixture() JSON
    assert_eq!(
        style.background,
        RgbaColor {
            r: 10,
            g: 10,
            b: 10,
            a: 255
        }
    );
}

#[test]
fn parse_record_backlight() {
    let style = loader().parse_record(&full_fixture()).unwrap();
    // hardcoding-guard: synthetic — round-trip of the synthetic full_fixture() JSON
    assert_eq!(
        style.backlight,
        RgbaColor {
            r: 102,
            g: 214,
            b: 255,
            a: 255
        }
    );
}

#[test]
fn parse_record_font_families() {
    let style = loader().parse_record(&full_fixture()).unwrap();
    assert_eq!(style.font_family_hints, vec!["Rajdhani", "Orbitron"]);
}

#[test]
fn parse_record_crt_params() {
    let style = loader().parse_record(&full_fixture()).unwrap();
    assert!((style.crt.scanline_period_px - 4.0).abs() < 1e-5);
    assert!((style.crt.pixel_grid_period_px - 3.5).abs() < 1e-5);
    assert!((style.crt.scanline_intensity - 0.2).abs() < 1e-5);
    assert!((style.crt.vignette_strength - 0.4).abs() < 1e-5);
}

#[test]
fn parse_record_optional_fields_absent() {
    let minimal = serde_json::json!({
        "primaryColor":    { "r": 255, "g": 176, "b": 76, "a": 255 },
        "backgroundColor": { "r": 0,   "g": 0,   "b": 0,  "a": 255 },
        "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 }
    });
    let style = loader().parse_record(&minimal).unwrap();
    assert_eq!(style.secondary_tint, None);
    assert!(style.font_family_hints.is_empty());
}

#[test]
fn parse_record_missing_primary_color_returns_error() {
    let bad = serde_json::json!({
        "backgroundColor": { "r": 0, "g": 0, "b": 0, "a": 255 },
        "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 }
    });
    assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
}

#[test]
fn parse_record_missing_background_color_returns_error() {
    let bad = serde_json::json!({
        "primaryColor":   { "r": 255, "g": 176, "b": 76, "a": 255 },
        "backlightColor": { "r": 102, "g": 214, "b": 255, "a": 255 }
    });
    assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
}

#[test]
fn parse_record_missing_backlight_color_returns_error() {
    let bad = serde_json::json!({
        "primaryColor":    { "r": 255, "g": 176, "b": 76, "a": 255 },
        "backgroundColor": { "r": 0,   "g": 0,   "b": 0,  "a": 255 }
    });
    assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
}

#[test]
fn parse_record_color_missing_r_channel_returns_error() {
    let bad = serde_json::json!({
        "primaryColor":    { "g": 176, "b": 76 },
        "backgroundColor": { "r": 0,   "g": 0, "b": 0, "a": 255 },
        "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 }
    });
    assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
}

#[test]
#[ignore]
fn snapshot_drake_style_print() {
    use crate::defaults::DefaultValueRegistry;

    let loader = StyleLoader::for_manufacturer("drak");
    let style = loader.neutral_fallback();

    println!("=== Drake Manufacturer Style (fallback) ===");
    println!("name:          {}", style.name);
    println!(
        "primary_tint:  #{:02X}{:02X}{:02X} (a={})",
        style.primary_tint.r, style.primary_tint.g, style.primary_tint.b, style.primary_tint.a
    );
    println!(
        "backlight:     #{:02X}{:02X}{:02X} (a={})",
        style.backlight.r, style.backlight.g, style.backlight.b, style.backlight.a
    );
    println!(
        "background:    #{:02X}{:02X}{:02X} (a={})",
        style.background.r, style.background.g, style.background.b, style.background.a
    );
    println!("font_hints:    {:?}", style.font_family_hints);
    println!(
        "crt:           scanline_period={}px  grid_period={}px  scanline_intensity={}  vignette={}",
        style.crt.scanline_period_px,
        style.crt.pixel_grid_period_px,
        style.crt.scanline_intensity,
        style.crt.vignette_strength
    );

    let screen_states = serde_json::json!([{
        "name": "Normal",
        "lightOn": true,
        "color": { "r": 102, "g": 214, "b": 255, "a": 255 },
        "intensity": 0.025
    }]);
    let mut reg = DefaultValueRegistry::with_well_known_path_defaults();
    reg.ingest_screen_states(&screen_states);

    println!("\n=== DefaultValueRegistry (well-known paths + Clipper screenStates) ===");
    println!("path_count:   {}", reg.path_count());
    for path in [
        "/vehicle/targetname",
        "/vehicle/target/distance",
        "/vehicle/target/bearing",
        "/ship/hp/current",
        "/ship/hp/max",
        "/seatdashboard/powerstate",
        "/seatdashboard/powercurrent",
        "/seatdashboard/powermax",
        "/vehicle/gungroup",
    ] {
        println!("  {path:<35} -> {:?}", reg.lookup_path(path));
    }
    println!(
        "  Normal backlight               -> {:?}",
        reg.screen_state_color("Normal")
    );
}

/// The screen `background` comes from BB_ColorStyle slot 9 (the enum's
/// Background slot), NOT slot 8. Adjudicated 2026-06-13 against two dark-room
/// captures of the Clipper power MFD (plan P5.3): the in-game background's
/// dark-region ratios match the slot-9 render (13/15 region pairs, ratio RMS
/// 2.3x better, response exponent ~0.91 vs a contorted 0.57 for slot 8).
#[test]
fn buildingblocks_record_background_is_palette_slot_9() {
    // hardcoding-guard: synthetic — slot 9 white, all other slots black.
    let mut slots = Vec::new();
    for index in 0..17 {
        let v = if index == 9 { 255 } else { 0 };
        slots.push(serde_json::json!({
            "color": { "_Type_": "SRGBA8", "r": v, "g": v, "b": v, "a": 255 }
        }));
    }
    let record = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Style.SyntheticSlotTest",
        "_RecordValue_": { "colorStyles": slots }
    });
    let style = loader().parse_buildingblocks_style_record(&record).unwrap();
    assert_eq!(
        style.background,
        RgbaColor { r: 255, g: 255, b: 255, a: 255 },
        "background must read slot 9 (enum Background), not slot 8"
    );
}
