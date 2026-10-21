//! TDD tests for brand-entry colour resolution against the brand's Style record.
//!
//! A canvas's `brandStyles[]` container carries `entries` but no `colorStyles`
//! palette — the palette lives on the `BuildingBlocks_Style` record named by
//! `brandIdentifier` (e.g. `s_drak_hud`). Colour modifiers (the MFD footer's
//! `BackgroundColor = Disabled@0.1`, `BorderColorTop = Base@1.0`) must resolve
//! their named roles against that record instead of being silently dropped,
//! and an applied `BackgroundColor` must override the node's authored
//! `background.color` so downstream fill token/alpha readers see the styled
//! value, not the authored at-rest one.

use starbreaker_ui::bb_resolve::resolve_canvas_graph_with_loc;

/// `BuildingBlocks_Style` record carrying the colour palette: slot 0 = Base
/// (orange), slot 8 = Disabled (dark grey).
fn brand_style_record() -> serde_json::Value {
    let mut slots = Vec::new();
    for index in 0..10 {
        let (r, g, b) = match index {
            0 => (1.0, 0.5, 0.0),
            8 => (0.1, 0.1, 0.1),
            _ => (0.5, 0.5, 0.5),
        };
        slots.push(serde_json::json!({
            "_Type_": "BuildingBlocks_ColorStyleEntry",
            "color": {"r": r, "g": g, "b": b, "a": 1.0}
        }));
    }
    serde_json::json!({
        "_RecordName_": "BuildingBlocks_Style.S_Test_Hud",
        "_RecordValue_": {
            "entries": [],
            "colorStyles": slots,
            "textFieldModifiers": []
        }
    })
}

/// A canvas whose brand entry restyles the tagged footer-background widget with
/// named palette colours; the `brandStyles` container itself has NO palette.
fn canvas_with_brand_colour_entry() -> serde_json::Value {
    serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.MC_Test_FooterBg",
        "_RecordValue_": {
            "size": {"x": 400.0, "y": 300.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_DisplayWidget",
                    "name": "base_BG",
                    "isActive": true,
                    "styleTags": [
                        {"_RecordName_": "Tag.aaaa1111-0000-0000-0000-000000000001",
                         "_RecordId_": "aaaa1111-0000-0000-0000-000000000001"}
                    ],
                    "background": {
                        "_Type_": "BuildingBlocks_Background",
                        "enable": true,
                        "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Base", "alpha": 0.3}
                    },
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 400.0},
                        "height": {"behavior": "Fixed", "value": 40.0}
                    }
                }
            ],
            "operations": [],
            "brandStyles": [
                {
                    "_Type_": "BuildingBlocks_BrandStyles",
                    "brandIdentifier": "file://./styles/s_test_hud.json",
                    "entries": [
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "ScreenNameBackground",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": "aaaa1111-0000-0000-0000-000000000001"}
                                }]
                            }],
                            "modifiers": [
                                {"_Type_": "BuildingBlocks_FieldModifierNumber",
                                 "field": "BorderTopWidth", "value": 2.0},
                                {"_Type_": "BuildingBlocks_FieldModifierColor",
                                 "field": "BackgroundColor",
                                 "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Disabled", "alpha": 0.1}},
                                {"_Type_": "BuildingBlocks_FieldModifierColor",
                                 "field": "BorderColorTop",
                                 "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Base", "alpha": 1.0}}
                            ],
                            "transitions": []
                        }
                    ]
                }
            ]
        }
    })
}

fn resolve_scene() -> starbreaker_ui::bb_scene::BbScene {
    let canvas = canvas_with_brand_colour_entry();
    let style_record = brand_style_record();
    let fetch = move |path: &str| -> Result<serde_json::Value, String> {
        if path.to_ascii_lowercase().contains("s_test_hud") {
            Ok(style_record.clone())
        } else {
            Err(format!("unknown record: {path}"))
        }
    };
    resolve_canvas_graph_with_loc(&canvas, Some("test"), &fetch, None).expect("resolve")
}

#[test]
fn brand_colour_modifiers_resolve_against_brand_style_record_palette() {
    let scene = resolve_scene();
    let node = scene
        .nodes
        .values()
        .find(|n| n.name == "base_BG")
        .expect("base_BG node");

    // The named-colour modifiers must not be dropped: token recorded…
    assert_eq!(
        node.raw.get("BackgroundColorToken").and_then(|v| v.as_str()),
        Some("Disabled"),
        "BackgroundColor token must come from the brand entry; raw: {}",
        node.raw
    );
    // …and the border-top colour resolved from the Style record's palette
    // (slot 0 = Base, orange) at the modifier's alpha 1.0.
    let border = node.border.as_ref().expect("border present");
    let top = border.top.colour.expect("border-top colour resolved from palette");
    assert!(
        (top[0] - 1.0).abs() < 0.01 && (top[1] - 0.5).abs() < 0.01 && top[2] < 0.01,
        "border-top must be the palette Base colour; got {top:?}"
    );
}

#[test]
fn applied_background_colour_overrides_authored_background_colour() {
    let scene = resolve_scene();
    let node = scene
        .nodes
        .values()
        .find(|n| n.name == "base_BG")
        .expect("base_BG node");

    // The styled BackgroundColor (Disabled@0.1) replaces the authored at-rest
    // background (Base@0.3) so fill token/alpha readers see the styled value.
    let bg_colour = node
        .raw
        .get("background")
        .and_then(|bg| bg.get("color"))
        .expect("background.color present");
    assert_eq!(
        bg_colour.get("color").and_then(|v| v.as_str()),
        Some("Disabled"),
        "authored background.color must be overridden by the styled colour; got {bg_colour}"
    );
    assert!(
        (bg_colour.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0) - 0.1).abs() < 1e-3,
        "styled background alpha 0.1 must replace the authored 0.3; got {bg_colour}"
    );
}

/// A text node with an authored `background.enable = false` must not grow a
/// drawn background just because a style entry restyles its `BackgroundColor`
/// (the MFD footer screen-name text carries `BackgroundColor = Bright@1.0`
/// from the shared `UnSelectedName` entry yet draws no bar in-game — the
/// colour modifier restyles, it does not enable).
#[test]
fn background_colour_modifier_does_not_enable_disabled_background() {
    let canvas = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.MC_Test_DisabledBg",
        "_RecordValue_": {
            "size": {"x": 400.0, "y": 300.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "text_Name",
                    "isActive": true,
                    "text": "NAME",
                    "styleTags": [
                        {"_RecordName_": "Tag.bbbb2222-0000-0000-0000-000000000002",
                         "_RecordId_": "bbbb2222-0000-0000-0000-000000000002"}
                    ],
                    "background": {
                        "_Type_": "BuildingBlocks_Background",
                        "enable": false,
                        "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Bright", "alpha": 1.0}
                    },
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 200.0},
                        "height": {"behavior": "Fixed", "value": 40.0}
                    }
                }
            ],
            "operations": [],
            "brandStyles": [
                {
                    "_Type_": "BuildingBlocks_BrandStyles",
                    "brandIdentifier": "file://./styles/s_test_hud.json",
                    "entries": [
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "UnSelectedName",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": "bbbb2222-0000-0000-0000-000000000002"}
                                }]
                            }],
                            "modifiers": [
                                {"_Type_": "BuildingBlocks_FieldModifierColor",
                                 "field": "BackgroundColor",
                                 "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Bright", "alpha": 1.0}}
                            ],
                            "transitions": []
                        }
                    ]
                }
            ]
        }
    });
    let style_record = brand_style_record();
    let fetch = move |path: &str| -> Result<serde_json::Value, String> {
        if path.to_ascii_lowercase().contains("s_test_hud") {
            Ok(style_record.clone())
        } else {
            Err(format!("unknown record: {path}"))
        }
    };
    let scene =
        resolve_canvas_graph_with_loc(&canvas, Some("test"), &fetch, None).expect("resolve");
    let defaults = starbreaker_ui::DefaultValueRegistry::with_well_known_path_defaults();
    let ir = starbreaker_ui::ui_ir::compile_ui_ir_from_scene(
        &scene,
        None,
        "test-guid",
        Some("BuildingBlocks_Canvas.MC_Test_DisabledBg"),
        (400, 300),
        &defaults,
        None,
        None,
        &[],
        Vec::new(),
        Vec::new(),
        100,
    );
    let node = ir
        .nodes
        .iter()
        .find(|n| n.name == "text_Name")
        .expect("text node in IR");
    assert!(
        node.background_fill_colour.is_none() && node.background_fill_colour_token.is_none(),
        "an authored disabled background must stay disabled when a style only restyles its colour; got fill={:?} token={:?}",
        node.background_fill_colour,
        node.background_fill_colour_token
    );
}
