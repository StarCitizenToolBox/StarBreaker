//! Regression hash tests — Phase 10
//!
//! Renders two canonical Clipper canvases (Annunciator, Target Status) to
//! `RgbaImage` and verifies the SHA-256 of the raw RGBA byte buffer matches a
//! known-good value.
//!
//! The hash is of the raw pixel bytes, **not** the PNG stream, so the test is
//! immune to libpng version drift and encoder option changes.
//!
//! # Updating the expected value
//! When an intentional rendering change is made (e.g. revised widget geometry,
//! new default colour), bump the constant and describe the reason in the commit
//! message. The comment `// Bumped …` above each constant is the prompt for
//! future maintainers.

use sha2::{Digest, Sha256};
use starbreaker_ui::{
    CanvasParser, ComposeContext, ComposeTarget, DefaultValueRegistry, ResolvedCanvas, StyleLoader,
    render_canvas,
    swf_assets::SwfAssetLibrary,
};

// ──────────────────────────────────────────────────────────────────────────────
// Known-good SHA-256 values for the RGBA pixel buffer of each canvas.
//
// Bumped when intentional rendering changes are made.
// Update value + describe reason in commit message.
// ──────────────────────────────────────────────────────────────────────────────

/// FlightController_Annunciator rendered at 1024×128 with Drake amber style.
///
/// Bumped when intentional rendering changes are made.
/// Update value + describe reason in commit message.
const ANNUNCIATOR_RGBA_SHA256: &str =
    "053eca166d0ba5c9d8537e5d07bbad919576748489c18ab71d0cda2a127eae3c";

/// MC_S_Target_Master rendered at 1600×900 with Drake amber style.
///
/// Bumped when intentional rendering changes are made.
/// Update value + describe reason in commit message.
const TARGET_STATUS_RGBA_SHA256: &str =
    "dde10935ecfe4f99fef80b8b3f3e2d86a1c30a3d5f2be754f729693cd635acd3";

// ──────────────────────────────────────────────────────────────────────────────
// Helpers (shared with tier1_render.rs but kept local to avoid cross-test deps)
// ──────────────────────────────────────────────────────────────────────────────

fn empty_assets() -> SwfAssetLibrary {
    let swf_header: Vec<u8> = vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00,
        0x18, 0x00,
        0x01, 0x00,
        0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    SwfAssetLibrary::new(swf_header).expect("minimal SWF must parse")
}

fn drake_style() -> starbreaker_ui::ManufacturerStyle {
    StyleLoader::for_manufacturer("drak").drake_amber_fallback()
}

fn defaults() -> DefaultValueRegistry {
    DefaultValueRegistry::with_well_known_path_defaults()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

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
            {
                "type": "group",
                "name": "root_group",
                "x": 0.0, "y": 0.0, "width": 1024.0, "height": 128.0,
                "children": [
                    {
                        "type": "rect",
                        "name": "btn_pwr",
                        "x": 10.0, "y": 8.0, "width": 180.0, "height": 112.0,
                        "fill_color": "#1a1a1a",
                        "border_color": "#ffaa00",
                        "border_width": 2.0
                    },
                    {
                        "type": "text",
                        "name": "label_pwr",
                        "x": 60.0, "y": 40.0,
                        "text": "PWR",
                        "font_size": 28.0,
                        "color": "#ffaa00"
                    },
                    {
                        "type": "rect",
                        "name": "btn_wpn",
                        "x": 210.0, "y": 8.0, "width": 180.0, "height": 112.0,
                        "fill_color": "#1a1a1a",
                        "border_color": "#ffaa00",
                        "border_width": 2.0
                    },
                    {
                        "type": "text",
                        "name": "label_wpn",
                        "x": 258.0, "y": 40.0,
                        "text": "WPN",
                        "font_size": 28.0,
                        "color": "#ffaa00"
                    },
                    {
                        "type": "rect",
                        "name": "btn_thr",
                        "x": 410.0, "y": 8.0, "width": 180.0, "height": 112.0,
                        "fill_color": "#1a1a1a",
                        "border_color": "#ffaa00",
                        "border_width": 2.0
                    },
                    {
                        "type": "text",
                        "name": "label_thr",
                        "x": 460.0, "y": 40.0,
                        "text": "THR",
                        "font_size": 28.0,
                        "color": "#ffaa00"
                    },
                    {
                        "type": "rect",
                        "name": "btn_shld",
                        "x": 610.0, "y": 8.0, "width": 180.0, "height": 112.0,
                        "fill_color": "#1a1a1a",
                        "border_color": "#ffaa00",
                        "border_width": 2.0
                    },
                    {
                        "type": "text",
                        "name": "label_shld",
                        "x": 650.0, "y": 40.0,
                        "text": "SHLD",
                        "font_size": 28.0,
                        "color": "#ffaa00"
                    },
                    {
                        "type": "text",
                        "name": "label_cool",
                        "x": 860.0, "y": 40.0,
                        "text": "COOL",
                        "font_size": 28.0,
                        "color": "#ffaa00"
                    }
                ]
            }
        ]
    })
}

fn target_status_fixture() -> serde_json::Value {
    serde_json::json!({
        "__name": "MC_S_Target_Master",
        "views": [
            {
                "name": "_physicalScreen",
                "default": true,
                "screens": []
            }
        ],
        "scene": [
            {
                "type": "group",
                "name": "root_group",
                "x": 0.0, "y": 0.0, "width": 1600.0, "height": 900.0,
                "children": [
                    {
                        "type": "rect",
                        "name": "background",
                        "x": 0.0, "y": 0.0, "width": 1600.0, "height": 900.0,
                        "fill_color": "#0a0a12",
                        "border_color": "#ffaa00",
                        "border_width": 1.0
                    },
                    {
                        "type": "text",
                        "name": "no_target_label",
                        "x": 650.0, "y": 420.0,
                        "text": "NO TARGET",
                        "font_size": 48.0,
                        "color": "#ffaa00"
                    },
                    {
                        "type": "rect",
                        "name": "crosshair_h",
                        "x": 700.0, "y": 448.0, "width": 200.0, "height": 4.0,
                        "fill_color": "#ffaa00"
                    },
                    {
                        "type": "rect",
                        "name": "crosshair_v",
                        "x": 798.0, "y": 350.0, "width": 4.0, "height": 200.0,
                        "fill_color": "#ffaa00"
                    }
                ]
            }
        ]
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Regression tests
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn annunciator_rgba_hash_stable() {
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
        .expect("render_canvas failed");

    // Hash the raw RGBA bytes, not the PNG stream.
    let raw_bytes: Vec<u8> = img.pixels().flat_map(|p| p.0).collect();
    let actual = sha256_hex(&raw_bytes);

    assert_eq!(
        actual,
        ANNUNCIATOR_RGBA_SHA256,
        "Annunciator RGBA hash mismatch — if intentional, update ANNUNCIATOR_RGBA_SHA256 and describe the change in the commit message"
    );
}

#[test]
fn target_status_rgba_hash_stable() {
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
        .expect("render_canvas failed");

    let raw_bytes: Vec<u8> = img.pixels().flat_map(|p| p.0).collect();
    let actual = sha256_hex(&raw_bytes);

    assert_eq!(
        actual,
        TARGET_STATUS_RGBA_SHA256,
        "Target Status RGBA hash mismatch — if intentional, update TARGET_STATUS_RGBA_SHA256 and describe the change in the commit message"
    );
}
