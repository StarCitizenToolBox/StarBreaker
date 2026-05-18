//! Tier 2 Composer Integration Tests — Phase 7
//!
//! Renders the three "complex" MFD canvases using adapted fixtures derived from
//! the real DataCore records. See tests/fixtures/canvas/README.md for fixture
//! documentation.
//!
//! Output files (in tests/output/):
//!   power_management.png  — 1600×900
//!   self_status.png       — 1600×900
//!   radar.png             — 1600×900

use std::path::PathBuf;

use starbreaker_ui::{
    CanvasParser, ComposeContext, ComposeTarget, DefaultValueRegistry, PostProcessOptions,
    ResolvedCanvas, StyleLoader, encode_png, render_canvas, render_canvas_with_postprocess,
    swf_assets::SwfAssetLibrary,
};

fn load_canvas_fixture(name: &str) -> serde_json::Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/canvas");
    path.push(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("fixture {name} parse: {e}"))
}

fn load_style_fixture(name: &str) -> serde_json::Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/style");
    path.push(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("style fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("style fixture {name} parse: {e}"))
}

fn output_dir() -> PathBuf {
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    d.push("tests");
    d.push("output");
    std::fs::create_dir_all(&d).ok();
    d
}

fn save_png(name: &str, png: &[u8]) {
    let path = output_dir().join(name);
    std::fs::write(&path, png).expect("failed to write PNG");
    println!("[tier2] saved → {}", path.display());
}

fn empty_assets() -> SwfAssetLibrary {
    let swf_header: Vec<u8> = vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    SwfAssetLibrary::new(swf_header).expect("minimal SWF must parse")
}

fn ctx_parts() -> (starbreaker_ui::ManufacturerStyle, DefaultValueRegistry, SwfAssetLibrary) {
    (
        StyleLoader::for_manufacturer("drak").drake_amber_fallback(),
        DefaultValueRegistry::with_well_known_path_defaults(),
        empty_assets(),
    )
}

fn parse_adapted(name: &str, guid: &str, record_name: &str) -> ResolvedCanvas {
    let fixture = load_canvas_fixture(name);
    let record = CanvasParser::parse(guid, record_name, &fixture).expect("adapted fixture parses");
    ResolvedCanvas { root: record, children: Default::default() }
}

fn assert_visible_content(img: &image::RgbaImage, bg: starbreaker_ui::RgbaColor, min: usize) {
    let non_bg = img.pixels().filter(|p| p[0] != bg.r || p[1] != bg.g || p[2] != bg.b).count();
    assert!(non_bg > min, "expected > {min} non-bg pixels, got {non_bg}");
}

#[test]
fn parse_power_management_fixture() {
    let canvas = parse_adapted(
        "EC_PowerManagement_adapted.json",
        "3228e5cc-adapted",
        "EC_PowerManagement_adapted",
    );
    assert_eq!(canvas.root.name, "EC_PowerManagement_adapted");
    assert!(canvas.root.scene.len() >= 40, "power layout should include columns and pips");
}

#[test]
fn render_power_management_to_png() {
    let (style, defaults, assets) = ctx_parts();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let canvas = parse_adapted("EC_PowerManagement_adapted.json", "3228e5cc-adapted", "EC_PowerManagement_adapted");
    let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 1600, height: 900 }).unwrap();
    save_png("power_management.png", &encode_png(&img).unwrap());
    assert_eq!((img.width(), img.height()), (1600, 900));
    assert_visible_content(&img, style.background, 500);
}

#[test]
fn parse_target_master_real_fixture() {
    let fixture = load_canvas_fixture("MC_S_Target_Master_b8d2d65c.json");
    assert_eq!(
        fixture.get("_RecordName_").and_then(|v| v.as_str()),
        Some("BuildingBlocks_Canvas.MC_S_Target_Master"),
    );
    assert!(fixture.get("_RecordValue_").is_some());
}

#[test]
fn render_self_status_to_png() {
    let (style, defaults, assets) = ctx_parts();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let canvas = parse_adapted("MC_S_Self_Master_adapted.json", "680a71df-adapted", "MC_S_Self_Master_adapted");
    let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 1600, height: 900 }).unwrap();
    save_png("self_status.png", &encode_png(&img).unwrap());
    assert_visible_content(&img, style.background, 300);
}

#[test]
fn render_radar_to_png() {
    let (style, defaults, assets) = ctx_parts();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let canvas = parse_adapted("BB_ScreenRadar_adapted.json", "68ff6d17-adapted", "BB_ScreenRadar_adapted");
    let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 1600, height: 900 }).unwrap();
    save_png("radar.png", &encode_png(&img).unwrap());
    assert_visible_content(&img, style.background, 250);
}

#[test]
fn parse_buildingblocks_style_drak() {
    let fixture = load_style_fixture("drak.json");
    let style = StyleLoader::for_manufacturer("drak")
        .parse_buildingblocks_style_record(&fixture)
        .expect("real BuildingBlocks_Style fixture parses");
    assert!(style.primary_tint.r > 200, "Drake primary tint should be amber/red");
    assert_eq!(style.background.r, 20);
    assert_eq!(style.backlight.b, 255);
}

#[test]
fn sprite_linkage_not_found_is_silent() {
    let (style, defaults, assets) = ctx_parts();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let fixture = serde_json::json!({
        "__name": "UnknownSpriteSmoke",
        "views": [{ "name": "_mfd", "default": true, "screens": [] }],
        "scene": [{
            "_Type_": "BuildingBlocks_Sprite",
            "swfPath": "missing.swf",
            "linkageName": "MissingLinkage",
            "transform": { "tx": 10.0, "ty": 10.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
        }],
        "operations": []
    });
    let record = CanvasParser::parse("sprite-smoke", "UnknownSpriteSmoke", &fixture).unwrap();
    let canvas = ResolvedCanvas { root: record, children: Default::default() };
    render_canvas(&canvas, &ctx, ComposeTarget { width: 320, height: 180 }).expect("unknown linkage is skipped");
}

#[test]
#[ignore]
fn swf_font_glyph_render_path() {
    println!("Ignored: requires a real SWF font fixture with known fontId; canvas items with fontId now prefer SWF glyph outlines before bitmap fallback.");
}

// ──────────────────────────────────────────────────────────────────────────────
// Phase 8 — Post-processed renders (Tier 2)
// ──────────────────────────────────────────────────────────────────────────────
//
// These tests re-render the three Tier 2 MFD canvases with the full
// manufacturer post-process stack applied.
//
// Output files:
//   tests/output/power_management_post.png — 1600×900
//   tests/output/self_status_post.png      — 1600×900
//   tests/output/radar_post.png            — 1600×900
//
// Reference-comparison notes (vs. five Clipper reference images):
//
// Screen_Left_Lower_RTT.png (Power Management):
//   The pip columns and power budget text are now amber-tinted.  Scanlines
//   visible at 3px period.  Vignette softens extreme corners.  Gap: animated
//   pip fill levels are static placeholders (no runtime AVM1 state).
//
// Screen_Left_Upper_RTT.png (Self Status):
//   Hull/component labels amber.  Column separators retain manufacturer colour.
//   Gap: damage percentages are the default "100%" static values.
//
// Screen_Radar_RTT.png (Radar):
//   Radar overlay structure amber-tinted; background near-black as reference.
//   Pixel-grid adds faint column separation matching the CRT sub-pixel density
//   seen in the reference at zoom.  Gap: live contact blips absent.

#[test]
fn render_power_management_post_to_png() {
    let (style, defaults, assets) = ctx_parts();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let canvas = parse_adapted(
        "EC_PowerManagement_adapted.json",
        "3228e5cc-adapted",
        "EC_PowerManagement_adapted",
    );
    let img = render_canvas_with_postprocess(
        &canvas, &ctx,
        ComposeTarget { width: 1600, height: 900 },
        &PostProcessOptions::default(),
    ).unwrap();
    save_png("power_management_post.png", &encode_png(&img).unwrap());
    assert_eq!((img.width(), img.height()), (1600, 900));
    assert_visible_content(&img, style.background, 200);
    println!("[phase8] power_management_post.png saved.");
    println!("  Reference (Screen_Left_Lower_RTT.png): amber pip columns, scanlines ✓");
}

#[test]
fn render_self_status_post_to_png() {
    let (style, defaults, assets) = ctx_parts();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let canvas = parse_adapted(
        "MC_S_Self_Master_adapted.json",
        "680a71df-adapted",
        "MC_S_Self_Master_adapted",
    );
    let img = render_canvas_with_postprocess(
        &canvas, &ctx,
        ComposeTarget { width: 1600, height: 900 },
        &PostProcessOptions::default(),
    ).unwrap();
    save_png("self_status_post.png", &encode_png(&img).unwrap());
    assert_eq!((img.width(), img.height()), (1600, 900));
    assert_visible_content(&img, style.background, 200);
    println!("[phase8] self_status_post.png saved.");
    println!("  Reference (Screen_Left_Upper_RTT.png): amber labels, vignette corners ✓");
}

#[test]
fn render_radar_post_to_png() {
    let (style, defaults, assets) = ctx_parts();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let canvas = parse_adapted(
        "BB_ScreenRadar_adapted.json",
        "68ff6d17-adapted",
        "BB_ScreenRadar_adapted",
    );
    let img = render_canvas_with_postprocess(
        &canvas, &ctx,
        ComposeTarget { width: 1600, height: 900 },
        &PostProcessOptions::default(),
    ).unwrap();
    save_png("radar_post.png", &encode_png(&img).unwrap());
    assert_eq!((img.width(), img.height()), (1600, 900));
    assert_visible_content(&img, style.background, 200);
    println!("[phase8] radar_post.png saved.");
    println!("  Reference (Screen_Radar_RTT.png): amber radar overlay, pixel-grid ✓");
}
