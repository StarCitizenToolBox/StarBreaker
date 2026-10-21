use super::tests_support::make_test_scene;
use super::*;
use crate::bb_scene::BbValue;
use serde_json::json;
    #[test]
    fn test_unconditional_entry_applies_to_all() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "Alpha",
                            "value": 0.5
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert_eq!(scene.nodes.get(&1).unwrap().alpha, 0.5);
    }
    #[test]
    fn test_conditional_entry_matches_tag() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "conditions": [
                            {
                                "tag": {
                                    "_RecordId_": "tag-uuid-1"
                                }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "Alpha",
                            "value": 0.75
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert_eq!(scene.nodes.get(&1).unwrap().alpha, 0.75);
    }
    #[test]
    fn test_conditional_entry_no_match() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "conditions": [
                            {
                                "tag": {
                                    "_RecordId_": "nonexistent-tag"
                                }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "Alpha",
                            "value": 0.25
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert_eq!(scene.nodes.get(&1).unwrap().alpha, 1.0); // Unchanged
    }
    #[test]
    fn test_string_modifier() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "SvgPath",
                            "value": "UI/Textures/test.svg"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("SvgPath").and_then(|v| v.as_str()),
            Some("UI/Textures/test.svg")
        );
    }
    #[test]
    fn test_string_modifier_localization_uses_node_params() {
        struct TestLocFetcher;
        impl LocFetcher for TestLocFetcher {
            fn fetch_loc(&self, key: &str) -> Option<String> {
                match key {
                    "Med_T_Tier" => Some("T%d".to_string()),
                    _ => None,
                }
            }
        }
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().raw = json!({
            "paramInputValues": [
                { "name": "T", "defaultValue": 3 }
            ]
        });
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "Label",
                            "value": "@Med_T_Tier,P=T"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, Some(&TestLocFetcher));
        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(node.raw.get("Label").and_then(|v| v.as_str()), Some("T3"));
    }
    #[test]
    fn test_color_modifier_0_to_1() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().background = Some(Default::default());
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierColor",
                            "field": "FillColor",
                            "value": {
                                "r": 0.5,
                                "g": 0.75,
                                "b": 1.0,
                                "a": 1.0
                            }
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let node = scene.nodes.get(&1).unwrap();
        let color = node.background.as_ref().unwrap().fill_colour.unwrap();
        assert_eq!(color, [0.5, 0.75, 1.0, 1.0]);
    }
    #[test]
    fn test_color_modifier_0_to_255() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().background = Some(Default::default());
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierColor",
                            "field": "BackgroundColor",
                            "value": {
                                "r": 128.0,
                                "g": 192.0,
                                "b": 255.0,
                                "a": 255.0
                            }
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let node = scene.nodes.get(&1).unwrap();
        let color = node.background.as_ref().unwrap().fill_colour.unwrap();
        // Should be normalized to 0..1
        assert!((color[0] - 128.0 / 255.0).abs() < 0.01);
        assert!((color[1] - 192.0 / 255.0).abs() < 0.01);
        assert!((color[2] - 1.0).abs() < 0.01);
    }
    #[test]
    fn test_named_base_color_maps_to_slot_zero() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().background = Some(Default::default());
        let palette = json!({
            "colorStyles": [
                { "color": { "r": 115, "g": 198, "b": 254, "a": 255 } }
            ]
        });
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierColor",
                    "field": "FillColor",
                    "color": {
                        "_Type_": "BuildingBlocks_ColorStyle",
                        "color": "Base",
                        "alpha": 1.0
                    }
                }]
            })],
            raw: &palette,
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let color = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        assert!((color[0] - 115.0 / 255.0).abs() < 0.001);
        assert!((color[1] - 198.0 / 255.0).abs() < 0.001);
        assert!((color[2] - 254.0 / 255.0).abs() < 0.001);
        assert_eq!(color[3], 1.0);
    }
    #[test]
    fn named_accent1_color_maps_to_first_accent_slot() {
        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).unwrap();
        node.ty = BbNodeType::DisplayWidget;
        node.background = Some(Default::default());
        let palette = json!({
            "colorStyles": [
                { "color": { "r": 115, "g": 198, "b": 254, "a": 255 } },
                { "color": { "r": 67, "g": 221, "b": 147, "a": 255 } },
                { "color": { "r": 228, "g": 218, "b": 77, "a": 255 } },
                { "color": { "r": 201, "g": 51, "b": 51, "a": 255 } },
                { "color": { "r": 0, "g": 113, "b": 188, "a": 255 } }
            ]
        });
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierColor",
                    "field": "FillColor",
                    "color": {
                        "_Type_": "BuildingBlocks_ColorStyle",
                        "color": "Accent1",
                        "alpha": 1.0
                    }
                }]
            })],
            raw: &palette,
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let node = scene.nodes.get(&1).unwrap();
        let color = node.background.as_ref().unwrap().fill_colour.unwrap();
        assert!((color[0] - 0.0 / 255.0).abs() < 0.001);
        assert!((color[1] - 113.0 / 255.0).abs() < 0.001);
        assert!((color[2] - 188.0 / 255.0).abs() < 0.001);
        assert_eq!(node.raw.get("FillColorToken").and_then(|value| value.as_str()), Some("Accent1"));
    }
    #[test]
    fn test_boolean_modifier_is_active() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierBoolean",
                            "field": "IsActive",
                            "value": false
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert_eq!(scene.nodes.get(&1).unwrap().is_active, false);
    }
    /// `StyleSelectorConditionNotTag` matches a node when the tag is **absent**.
    /// This is the footer's at-rest hide rule: `BG_Neutral` carries
    /// `NotTag(warning-active)` + `IsActive=false`, so a node that does not carry
    /// the warning-active state tag is hidden. Node 1 only carries `tag-uuid-1`.
    #[test]
    fn not_tag_matches_when_tag_absent() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{ "conditions": [{
                    "_Type_": "BuildingBlocks_StyleSelectorConditionNotTag",
                    "tag": { "_RecordId_": "warning-active" }
                }]}],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierBoolean",
                    "field": "IsActive",
                    "value": false
                }]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert!(!scene.nodes.get(&1).unwrap().is_active,
            "NotTag(absent) must match → IsActive=false applied");
    }

    /// `NotTag` must NOT match when the tag is present (the inverse of the rule).
    #[test]
    fn not_tag_does_not_match_when_tag_present() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{ "conditions": [{
                    "_Type_": "BuildingBlocks_StyleSelectorConditionNotTag",
                    "tag": { "_RecordId_": "tag-uuid-1" }
                }]}],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierBoolean",
                    "field": "IsActive",
                    "value": false
                }]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert!(scene.nodes.get(&1).unwrap().is_active,
            "NotTag(present) must NOT match → node stays active");
    }

    /// `StyleSelectorConditionAllOfTag` matches only when **every** listed tag is
    /// present. This is the footer's show rule (`BG_Warning` requires all of the
    /// warning state tags), so at rest — when those tags are absent — it must not
    /// apply.
    #[test]
    fn all_of_tag_matches_only_when_all_present() {
        // All present → matches.
        let mut scene = make_test_scene();
        let entry = |tags: serde_json::Value| json!({
            "conditionsList": [{ "conditions": [{
                "_Type_": "BuildingBlocks_StyleSelectorConditionAllOfTag",
                "tags": tags
            }]}],
            "modifiers": [{
                "_Type_": "BuildingBlocks_FieldModifierBoolean",
                "field": "IsActive",
                "value": false
            }]
        });
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[entry(json!([{ "_RecordId_": "tag-uuid-1" }]))],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert!(!scene.nodes.get(&1).unwrap().is_active,
            "AllOfTag([present]) must match");

        // One tag absent → must NOT match.
        let mut scene2 = make_test_scene();
        let brand2 = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[entry(json!([
                { "_RecordId_": "tag-uuid-1" },
                { "_RecordId_": "absent-state-tag" }
            ]))],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene2, &brand2, None);
        assert!(scene2.nodes.get(&1).unwrap().is_active,
            "AllOfTag([present, absent]) must NOT match");
    }

    #[test]
    fn test_border_color_modifier() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierColor",
                            "field": "BorderColorTop",
                            "value": {
                                "r": 1.0,
                                "g": 0.0,
                                "b": 0.0,
                                "a": 1.0
                            }
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let node = scene.nodes.get(&1).unwrap();
        assert!(node.border.is_some());
        let color = node.border.as_ref().unwrap().top.colour.unwrap();
        assert_eq!(color, [1.0, 0.0, 0.0, 1.0]);
    }
    #[test]
    fn test_size_modifier() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "SizeX",
                            "value": 640.0
                        }
                    },
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "SizeY",
                            "value": 480.0
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(node.sizing.width, BbValue::Fixed(640.0));
        assert_eq!(node.sizing.height, BbValue::Fixed(480.0));
    }

/// A `Parent(...)`-wrapped entry whose unwrapped conditions match a
/// `WidgetTextField` itself targets that field's IMPLICIT TEXT-FORMAT CHILD:
/// the engine renders a textfield's text as a child element of the widget, so
/// `Parent(Tag(Size_3))` sizes the text of `Size_3`-tagged fields (the MFD
/// content host's per-brand `FontSizeSmall` table; verified on the power
/// screen's emissions header, battery card and OUTPUT "2"/"/16" against the
/// in-game reference). Only TEXT-FORMAT modifiers (FontSize, AutoFontSize,
/// FillColor, StrokeColor, LetterSpacing, LineSpacing, font record) apply via
/// this route, and only to textfields.
#[test]
fn parent_wrapped_entry_styles_text_format_of_tagged_textfield() {
    let mut scene = make_test_scene();
    {
        let node = scene.nodes.get_mut(&1).unwrap();
        node.ty = BbNodeType::WidgetTextField;
    }
    let entry = json!({
        "name": "FontSizeSmall",
        "conditionsList": [
            {
                "_Type_": "BuildingBlocks_StyleConditionList",
                "conditions": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionParent",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": {"_RecordId_": "tag-uuid-1"}
                            }
                        ]
                    }
                ]
            }
        ],
        "modifiers": [
            {"_Type_": "BuildingBlocks_FieldModifierBoolean", "field": "AutoFontSize", "value": false},
            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 40.0},
            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeX", "value": 123.0}
        ]
    });
    let brand = BrandStyle {
        identifier: "s_test_hud".to_string(),
        entries: std::slice::from_ref(&entry),
        raw: &json!({}),
    };
    apply_brand_modifiers(&mut scene, &brand, None);
    let node = scene.nodes.get(&1).unwrap();
    assert_eq!(
        node.raw.get("FontSize").and_then(|v| v.as_f64()),
        Some(40.0),
        "text-format FontSize applies to the tagged textfield"
    );
    assert_eq!(
        node.raw.get("AutoFontSize").and_then(|v| v.as_bool()),
        Some(false),
        "AutoFontSize applies to the tagged textfield"
    );
    assert!(
        node.raw.get("SizeX").is_none(),
        "widget-geometry modifiers must NOT apply via the text-format route"
    );
}

/// The text-format route is textfield-only: a non-text widget carrying the
/// same tag keeps literal Parent semantics (no match — its parent lacks the
/// tag), so geometry/colour entries on containers are unaffected.
#[test]
fn parent_wrapped_entry_does_not_match_non_textfield_via_text_format() {
    let mut scene = make_test_scene();
    let entry = json!({
        "conditionsList": [
            {
                "conditions": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionParent",
                        "conditions": [
                            {"_Type_": "BuildingBlocks_StyleSelectorConditionTag", "tag": {"_RecordId_": "tag-uuid-1"}}
                        ]
                    }
                ]
            }
        ],
        "modifiers": [
            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 40.0}
        ]
    });
    let brand = BrandStyle {
        identifier: "s_test_hud".to_string(),
        entries: std::slice::from_ref(&entry),
        raw: &json!({}),
    };
    apply_brand_modifiers(&mut scene, &brand, None);
    let node = scene.nodes.get(&1).unwrap();
    assert!(node.raw.get("FontSize").is_none(), "WidgetImage must not take the text-format route");
}

/// A `Type(Text)` selector INSIDE a `Parent(...)`-wrapped entry styles the
/// text format: a WidgetTextField renders its text via an implicit text-format
/// CHILD that is itself a `Text` node, so `Type(Text)` matches that child. DRAK
/// velocity-num authors its readouts as `Type(Text) + Parent[(Not)Tag(fontnumber)]`
/// → FontSize 500/420 (verified against the in-game `ship_velocity_num_master`
/// reference: cap heights 21%/16% of screen height). The `Parent` wrapper is
/// the "this field's text" anchor; see the Ancestor counterexample below.
#[test]
fn parent_wrapped_type_text_entry_styles_text_format() {
    let mut scene = make_test_scene();
    {
        let node = scene.nodes.get_mut(&1).unwrap();
        node.ty = BbNodeType::WidgetTextField;
    }
    let entry = json!({
        "conditionsList": [
            {
                "conditions": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionParent",
                        "conditions": [
                            {"_Type_": "BuildingBlocks_StyleSelectorConditionTag", "tag": {"_RecordId_": "tag-uuid-1"}}
                        ]
                    },
                    {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Text"}
                ]
            }
        ],
        "modifiers": [
            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 40.0}
        ]
    });
    let brand = BrandStyle {
        identifier: "s_test_hud".to_string(),
        entries: std::slice::from_ref(&entry),
        raw: &json!({}),
    };
    apply_brand_modifiers(&mut scene, &brand, None);
    let node = scene.nodes.get(&1).unwrap();
    assert_eq!(
        node.raw.get("FontSize").and_then(|v| v.as_f64()),
        Some(40.0),
        "Type(Text) inside a Parent wrapper sizes the field's text format (velocity-num readouts)"
    );
}

/// The DISCRIMINATOR for the MFD-footer counterexample: a `Type(Text)` selector
/// wrapped by `Ancestor(...)` (NOT `Parent`) does NOT style a WidgetTextField's
/// text format — it targets a real `WidgetText` widget via the normal route.
/// The MFD footer's `SelectedName`/`UnSelectedName` entries are
/// `Type(Text) + Ancestor[(Not)Tag(selected)]` and must NOT restyle the
/// screen-name WidgetTextField (its colour/tracking come from the brand H1
/// table in the in-game reference). Only a direct `Parent` wrapper anchors the
/// text-format route.
#[test]
fn ancestor_wrapped_type_text_entry_does_not_style_text_format() {
    let mut scene = make_test_scene();
    // node 1 = the screen-name WidgetTextField (untagged); its ancestor (node 2)
    // carries the "selected" tag, so the Ancestor condition is genuinely
    // satisfied — the entry is rejected by the missing Parent wrapper, not by a
    // failed tag test.
    let mut parent = scene.nodes.get(&1).cloned().unwrap();
    parent.id = 2;
    parent.name = "header".to_string();
    parent.parent = None;
    parent.children = vec![1];
    parent.ty = BbNodeType::DisplayWidget;
    parent.style_tag_uuids = vec!["tag-uuid-1".to_string()];
    parent.raw = json!({});
    scene.nodes.insert(2, parent);
    {
        let node = scene.nodes.get_mut(&1).unwrap();
        node.ty = BbNodeType::WidgetTextField;
        node.parent = Some(2);
        node.style_tag_uuids = vec![];
    }
    scene.roots = vec![2];
    let entry = json!({
        "conditionsList": [
            {
                "conditions": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Text"
                    },
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAncestor",
                        "breakConditions": [],
                        "conditions": [
                            {"_Type_": "BuildingBlocks_StyleSelectorConditionTag", "tag": {"_RecordId_": "tag-uuid-1"}}
                        ]
                    }
                ]
            }
        ],
        "modifiers": [
            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 40.0}
        ]
    });
    let brand = BrandStyle {
        identifier: "s_test_hud".to_string(),
        entries: std::slice::from_ref(&entry),
        raw: &json!({}),
    };
    apply_brand_modifiers(&mut scene, &brand, None);
    let node = scene.nodes.get(&1).unwrap();
    assert!(
        node.raw.get("FontSize").is_none(),
        "Ancestor-wrapped Type(Text) targets a WidgetText widget, not the field's text format (MFD footer)"
    );
}

/// A BARE `Type(Text)` selector (no `Parent`, no `Ancestor`) also styles a
/// WidgetTextField's implicit text-format child. The DRAK master-mode display
/// (`drak_hc_hud_cutlass_masterm_display`) authors its `defaultStyles` "New
/// Style" as a bare `Type(Text)` with FontSize 350 + white FillColor; the
/// in-game `master_mode_display_master` reference shows large white SCM/GUN
/// text, confirming the entry must reach the field's text format. Only an
/// `Ancestor(...)`-wrapped `Type(Text)` is excluded (the MFD-footer case
/// above); bare and `Parent`-wrapped both route.
#[test]
fn bare_type_text_entry_styles_text_format() {
    let mut scene = make_test_scene();
    {
        let node = scene.nodes.get_mut(&1).unwrap();
        node.ty = BbNodeType::WidgetTextField;
    }
    let entry = json!({
        "conditionsList": [
            {
                "conditions": [
                    {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Text"}
                ]
            }
        ],
        "modifiers": [
            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 350.0}
        ]
    });
    let brand = BrandStyle {
        identifier: "s_test_hud".to_string(),
        entries: std::slice::from_ref(&entry),
        raw: &json!({}),
    };
    apply_brand_modifiers(&mut scene, &brand, None);
    let node = scene.nodes.get(&1).unwrap();
    assert_eq!(
        node.raw.get("FontSize").and_then(|v| v.as_f64()),
        Some(350.0),
        "bare Type(Text) sizes the field's text format (master-mode SCM/GUN, FontSize 350)"
    );
}
