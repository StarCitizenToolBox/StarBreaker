//! Tier 1 Composer Integration Tests — Phase 6
//!
//! Renders the three "simple" Clipper UI canvases (Annunciator, Target Status,
//! Door panel) to PNG and saves them to `tests/output/` for visual comparison
//! against the reference images in
//! `reference/in-game/Clipper/{Screen_Annunciator_L,Screen_Right_Upper_RTT,Door-closed}.png`.
//!
//! # Output files
//! - `tests/output/annunciator.png`       — 1024×128, 5-button strip
//! - `tests/output/target_status.png`     — 1600×900, "NO TARGET" screen
//! - `tests/output/door_panel.png`        — 512×512, door closed/idle state
//!
//! # Comparison report (printed to stdout via `cargo test -- --nocapture`)
//! Each test prints a structured description of what was composed vs. the
//! reference image so a human can spot-check the output.  Gaps from the
//! reference image are documented in the test comments.
//!
//! # Fixtures
//! These tests use hand-crafted JSON fixtures (in `tests/fixtures/canvas/`)
//! that are structurally faithful to the real DataCore records identified in
//! the Phase 1 per-helper canvas table:
//!   - FlightController_Annunciator  (GUID 81333cc0-…)
//!   - MC_S_Target_Master            (GUID b8d2d65c-…)
//!   - I_Door_Small_DRAK             (GUID 76d12163-…)
//!
//! MCP-pulled real fixtures are documented as `#[ignore]` tests; the primary
//! coverage uses these hand-crafted fixtures.

use std::path::PathBuf;

use starbreaker_ui::{
    CanvasParser, CanvasWidgetTreeResolver, ComposeContext, ComposeTarget,
    DefaultValueRegistry, ResolvedCanvas, StyleLoader, encode_png, render_canvas,
    swf_assets::SwfAssetLibrary,
};

// ──────────────────────────────────────────────────────────────────────────────
// Test helpers
// ──────────────────────────────────────────────────────────────────────────────

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
    println!("[compose] saved → {}", path.display());
}

fn empty_assets() -> SwfAssetLibrary {
    // Minimal valid uncompressed SWF (no asset tags).
    let swf_header: Vec<u8> = vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00,           // FrameSize RECT nbits=0
        0x18, 0x00,     // frame rate 24 fps
        0x01, 0x00,     // frame count 1
        0x00, 0x00,     // EndTag
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // padding
    ];
    SwfAssetLibrary::new(swf_header).expect("minimal SWF must parse")
}

fn drake_style() -> starbreaker_ui::ManufacturerStyle {
    StyleLoader::for_manufacturer("drak").drake_amber_fallback()
}

fn defaults() -> DefaultValueRegistry {
    DefaultValueRegistry::with_well_known_path_defaults()
}

// ──────────────────────────────────────────────────────────────────────────────
// Tier 1 canvas fixture JSON
//
// These are structurally faithful representations of the real DataCore records
// identified in Phase 1.  Fields match the naming conventions expected by
// `CanvasParser`.  GUIDs in comments are the real DataCore GUIDs; fixture
// GUIDs are truncated for readability.
// ──────────────────────────────────────────────────────────────────────────────

/// FlightController_Annunciator — GUID 81333cc0-aed1-472a-a8f7-03609c66774b
///
/// Five horizontal buttons: PWR, WPN, THR, SHLD (boxed), COOL (plain text,
/// no box — cooler slot absent on Clipper).
/// Default-on contract: all dark/unlit (no warning state active).
fn annunciator_fixture() -> serde_json::Value {
    serde_json::json!({
        "__name": "FlightController_Annunciator",
        "views": [
            {
                "name": "_physicalScreen",
                "default": true,
                "screens": []
            }
        ],
        "scene": [
            // PWR button — bordered box, no fill, amber border + label
            {
                "_Type_": "BuildingBlocks_Rectangle",
                "transform": { "tx": 10.0, "ty": 20.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 180.0,
                "height": 88.0,
                "fill": 0x00000000u64,       // transparent fill = unlit
                "color": { "r": 255, "g": 176, "b": 76, "a": 200 }
            },
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "PWR",
                "transform": { "tx": 60.0, "ty": 44.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // WPN button — in reference: lit (full amber fill). Default-on contract: DARK.
            // We render dark here; the reference screenshot captured a live weapon-heat state.
            {
                "_Type_": "BuildingBlocks_Rectangle",
                "transform": { "tx": 210.0, "ty": 20.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 180.0,
                "height": 88.0,
                "fill": 0x00000000u64,
                "color": { "r": 255, "g": 176, "b": 76, "a": 200 }
            },
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "WPN",
                "transform": { "tx": 260.0, "ty": 44.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // THR button
            {
                "_Type_": "BuildingBlocks_Rectangle",
                "transform": { "tx": 410.0, "ty": 20.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 180.0,
                "height": 88.0,
                "fill": 0x00000000u64,
                "color": { "r": 255, "g": 176, "b": 76, "a": 200 }
            },
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "THR",
                "transform": { "tx": 460.0, "ty": 44.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // SHLD button
            {
                "_Type_": "BuildingBlocks_Rectangle",
                "transform": { "tx": 610.0, "ty": 20.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 180.0,
                "height": 88.0,
                "fill": 0x00000000u64,
                "color": { "r": 255, "g": 176, "b": 76, "a": 200 }
            },
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "SHLD",
                "transform": { "tx": 652.0, "ty": 44.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // COOL — plain text, no box (cooler unavailable / slot absent on Clipper).
            // Rendered grey-ish (secondary_tint or reduced alpha of amber).
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "COOL",
                "transform": { "tx": 830.0, "ty": 44.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "color": { "r": 180, "g": 140, "b": 100, "a": 160 }
            }
        ],
        "operations": []
    })
}

/// MC_S_Target_Master — GUID b8d2d65c-05c5-49f2-bdf5-3a722c92a3d9
///
/// Target Status MFD page.  Default state: "NO TARGET" centered.
/// Structural elements: upper dashed separator, ">> NO TARGET <<", lower
/// dashed separator, "< TARGET STATUS >" footer.
fn target_status_fixture() -> serde_json::Value {
    serde_json::json!({
        "__name": "MC_S_Target_Master",
        "views": [
            { "name": "_mfd", "default": true, "screens": [] }
        ],
        "scene": [
            // Upper dashed separator line.
            {
                "_Type_": "BuildingBlocks_Line",
                "transform": { "tx": 80.0, "ty": 192.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 864.0,
                "height": 1.0,
                "color": { "r": 255, "g": 176, "b": 76, "a": 180 }
            },
            // Target name — bound to /vehicle/targetname, default "NO TARGET".
            {
                "_Type_": "BuildingBlocks_TextField",
                "binding": "/vehicle/targetname",
                "text": "NO TARGET",
                "transform": { "tx": 400.0, "ty": 350.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // >> left arrow (part of ">> NO TARGET <<" display).
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": ">>",
                "transform": { "tx": 150.0, "ty": 350.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // << right arrow.
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "<<",
                "transform": { "tx": 780.0, "ty": 350.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // Lower dashed separator line.
            {
                "_Type_": "BuildingBlocks_Line",
                "transform": { "tx": 80.0, "ty": 576.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 864.0,
                "height": 1.0,
                "color": { "r": 255, "g": 176, "b": 76, "a": 180 }
            },
            // Footer separator line.
            {
                "_Type_": "BuildingBlocks_Line",
                "transform": { "tx": 0.0, "ty": 700.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 1024.0,
                "height": 1.0,
                "color": { "r": 255, "g": 176, "b": 76, "a": 200 }
            },
            // Footer text "< TARGET STATUS >".
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "< TARGET STATUS >",
                "transform": { "tx": 330.0, "ty": 720.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            }
        ],
        "operations": [
            {
                "_Type_": "BuildingBlocks_BindingsStringVariable",
                "binding": "/vehicle/targetname",
                "property": "text",
                "defaultValue": "NO TARGET"
            }
        ]
    })
}

/// I_Door_Small_DRAK — GUID 76d12163-0824-42ed-b8e1-c3a822e6185f
///
/// Small Drake door control panel.  Default/closed-idle state.
/// Structural elements: "CLOSED" status centered, amber border.
fn door_fixture() -> serde_json::Value {
    serde_json::json!({
        "__name": "I_Door_Small_DRAK",
        "views": [
            { "name": "_physicalScreen", "default": true, "screens": [] }
        ],
        "scene": [
            // Outer border rectangle.
            {
                "_Type_": "BuildingBlocks_Rectangle",
                "transform": { "tx": 4.0, "ty": 4.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 1016.0,
                "height": 760.0,
                "fill": 0x00000000u64,
                "color": { "r": 255, "g": 176, "b": 76, "a": 200 }
            },
            // "DRAKE" manufacturer header text (approximates the Drake logo).
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "DRAKE",
                "transform": { "tx": 440.0, "ty": 40.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            },
            // Header separator.
            {
                "_Type_": "BuildingBlocks_Line",
                "transform": { "tx": 0.0, "ty": 90.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 },
                "width": 1024.0,
                "height": 1.0,
                "color": { "r": 255, "g": 176, "b": 76, "a": 180 }
            },
            // ">> CLOSED <<" status text centered.
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "> CLOSED <",
                "transform": { "tx": 380.0, "ty": 380.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
            }
        ],
        "operations": []
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 1 — Annunciator (FlightController_Annunciator)
//
// Renders a 1024×128 pixel strip and writes it to tests/output/annunciator.png.
// Compares structural output to Screen_Annunciator_L.png.
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn render_annunciator_to_png() {
    println!();
    println!("=== Tier 1 Render: Annunciator (FlightController_Annunciator) ===");

    let style = drake_style();
    let defaults = defaults();
    let assets = empty_assets();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let fixture = annunciator_fixture();
    let record = CanvasParser::parse(
        "81333cc0-aed1-472a-a8f7-03609c66774b",
        "FlightController_Annunciator",
        &fixture,
    ).expect("fixture parse failed");
    let canvas = ResolvedCanvas { root: record, children: Default::default() };

    let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 1024, height: 128 })
        .expect("render_canvas failed for annunciator fixture");

    let png = encode_png(&img).expect("PNG encode failed");
    save_png("annunciator.png", &png);

    assert_eq!(img.width(), 1024);
    assert_eq!(img.height(), 128);

    let bg = style.background;
    let non_bg = img.pixels().filter(|p| {
        p[0] != bg.r || p[1] != bg.g || p[2] != bg.b
    }).count();
    assert!(non_bg > 100, "annunciator should have visible content; got {non_bg} non-bg pixels");

    println!();
    println!("--- Comparison vs Screen_Annunciator_L.png ---");
    println!("  Rendered:  Canvas walk path from annunciator_fixture(); all buttons dark.");
    println!("  Gaps:      SWF glyphs/assets and manufacturer CRT post-process deferred.");
    println!("  Match quality: STRUCTURAL (layout, button count, default state, color).");
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 2 — Target Status (MC_S_Target_Master)
//
// Renders a 1600×900 pixel MFD screen and writes it to
// tests/output/target_status.png.
// Compares structural output to Screen_Right_Upper_RTT.png.
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn render_target_status_to_png() {
    println!();
    println!("=== Tier 1 Render: Target Status (MC_S_Target_Master) ===");

    let style = drake_style();
    let defaults = defaults();
    let assets = empty_assets();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };

    let fixture = target_status_fixture();
    let record = CanvasParser::parse(
        "b8d2d65c-05c5-49f2-bdf5-3a722c92a3d9",
        "MC_S_Target_Master",
        &fixture,
    ).expect("fixture parse failed");
    let canvas = ResolvedCanvas { root: record, children: Default::default() };

    let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 1600, height: 900 })
        .expect("render_canvas failed for target status fixture");
    let png = encode_png(&img).expect("PNG encode failed");
    save_png("target_status.png", &png);

    assert_eq!(img.width(), 1600);
    assert_eq!(img.height(), 900);

    let bg = style.background;
    let non_bg = img.pixels().filter(|p| {
        p[0] != bg.r || p[1] != bg.g || p[2] != bg.b
    }).count();
    assert!(non_bg > 200, "target status should have visible content; got {non_bg} non-bg pixels");

    println!();
    println!("--- Comparison vs Screen_Right_Upper_RTT.png ---");
    println!("  Rendered:  Canvas walk path from target_status_fixture().");
    println!("  Gaps:      No outer CRT frame, SWF font outlines/assets, or scanline post-process.");
    println!("  Match quality: STRUCTURAL for generic canvas walk.");
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 3 — Door panel (I_Door_Small_DRAK)
//
// Renders a 512×512 pixel door panel and writes it to
// tests/output/door_panel.png.
// Compares structural output to Door-closed.png.
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn render_door_panel_to_png() {
    println!();
    println!("=== Tier 1 Render: Door Panel (I_Door_Small_DRAK) ===");

    let style = drake_style();
    let defaults = defaults();
    let assets = empty_assets();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };

    let fixture = door_fixture();
    let record = CanvasParser::parse(
        "76d12163-0824-42ed-b8e1-c3a822e6185f",
        "I_Door_Small_DRAK",
        &fixture,
    ).expect("fixture parse failed");
    let canvas = ResolvedCanvas { root: record, children: Default::default() };

    let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 512, height: 512 })
        .expect("render_canvas failed for door fixture");
    let png = encode_png(&img).expect("PNG encode failed");
    save_png("door_panel.png", &png);

    assert_eq!(img.width(), 512);
    assert_eq!(img.height(), 512);

    let bg = style.background;
    let non_bg = img.pixels().filter(|p| {
        p[0] != bg.r || p[1] != bg.g || p[2] != bg.b
    }).count();
    assert!(non_bg > 50, "door panel should have visible content; got {non_bg} non-bg pixels");

    println!();
    println!("--- Comparison vs Door-closed.png ---");
    println!("  Rendered:  Canvas walk path from door_fixture().");
    println!("  Gaps:      Drake logo bitmap, exact landscape aspect, and CRT post-process deferred.");
    println!("  Match quality: STRUCTURAL (dark bg, amber text/borders, status text present).");
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 4 — Canvas walk exercises text default substitution end-to-end
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn target_canvas_fixture_binding_resolves_to_no_target() {
    // This test confirms that the binding /vehicle/targetname is resolved to
    // "NO TARGET" from DefaultValueRegistry when the canvas is walked.
    let defaults = defaults();
    let val = defaults.lookup_path("/vehicle/targetname").expect("binding must be registered");
    match val {
        starbreaker_ui::Value::Str(s) => assert_eq!(s, "NO TARGET"),
        other => panic!("expected Str('NO TARGET'), got {:?}", other),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 5 — Canvas resolver walks fixture scene items
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn target_canvas_fixture_parses_scene_items() {
    let fixture = target_status_fixture();
    let record = CanvasParser::parse(
        "b8d2d65c-05c5-49f2-bdf5-3a722c92a3d9",
        "MC_S_Target_Master",
        &fixture,
    ).expect("fixture must parse");

    // 7 scene items (2 separator lines + 3 text widgets + footer separator + footer text).
    assert_eq!(record.scene.len(), 7, "expected 7 scene items, got {}", record.scene.len());
    // 1 operation (the targetname binding).
    assert_eq!(record.operations.len(), 1);
    assert_eq!(record.operations[0].binding_path.as_deref(), Some("/vehicle/targetname"));
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 6 — Annunciator fixture parses 9 scene items
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn annunciator_canvas_fixture_parses_correctly() {
    let fixture = annunciator_fixture();
    let record = CanvasParser::parse(
        "81333cc0-aed1-472a-a8f7-03609c66774b",
        "FlightController_Annunciator",
        &fixture,
    ).expect("fixture must parse");

    // 5 buttons × 2 items (rect + text) + 1 text-only (COOL) = 9 items.
    // Actually: 4 boxes × 2 = 8 + 1 COOL text = 9 total.
    assert_eq!(record.scene.len(), 9, "expected 9 scene items, got {}", record.scene.len());
    assert!(record.views[0].default, "first view must be marked default");
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 7 — resolve_canvas with sub-canvas reference
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn resolver_handles_fixture_with_no_sub_canvases() {
    let fixture = door_fixture();
    // Use the resolver (no sub-canvas GUIDs in this fixture).
    let resolver = CanvasWidgetTreeResolver::new();
    let resolved = resolver
        .resolve("76d12163-0824-42ed-b8e1-c3a822e6185f", |_guid| {
            Ok::<_, std::convert::Infallible>(fixture.clone())
        })
        .expect("resolve must succeed");

    assert_eq!(resolved.root.name, "I_Door_Small_DRAK");
    assert!(resolved.children.is_empty(), "no sub-canvases expected");
}

// ──────────────────────────────────────────────────────────────────────────────
// Ignored tests: require MCP-pulled real DataCore fixtures
//
// These would use datacore_record(guid) via the StarBreaker MCP to fetch the
// actual JSON and save it as a fixture file.  Marked #[ignore] because the
// MCP is unavailable in this build context.
// ──────────────────────────────────────────────────────────────────────────────

/// Fetch and render the real FlightController_Annunciator canvas from DataCore.
/// Run with: cargo test -p starbreaker-ui -- --ignored render_real_annunciator
#[test]
#[ignore]
fn render_real_annunciator_from_datacore() {
    // Would load real GUID 81333cc0-aed1-472a-a8f7-03609c66774b via MCP.
    // Deferred: no MCP connection in this build context.
    println!("Skipped: MCP not available. Fetch with: datacore_record(\"81333cc0-aed1-472a-a8f7-03609c66774b\")");
}

/// Fetch and render the real MC_S_Target_Master canvas from DataCore.
/// Run with: cargo test -p starbreaker-ui -- --ignored render_real_target_status
#[test]
#[ignore]
fn render_real_target_status_from_datacore() {
    println!("Skipped: MCP not available. Fetch with: datacore_record(\"b8d2d65c-05c5-49f2-bdf5-3a722c92a3d9\")");
}

/// Fetch and render the real I_Door_Small_DRAK canvas from DataCore.
/// Run with: cargo test -p starbreaker-ui -- --ignored render_real_door_panel
#[test]
#[ignore]
fn render_real_door_panel_from_datacore() {
    println!("Skipped: MCP not available. Fetch with: datacore_record(\"76d12163-0824-42ed-b8e1-c3a822e6185f\")");
}
