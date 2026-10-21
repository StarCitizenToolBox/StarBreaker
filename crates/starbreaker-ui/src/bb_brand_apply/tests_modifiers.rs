use super::tests_support::make_test_scene;
use super::*;
use crate::bb_scene::BbBackground;
use serde_json::json;

    #[test]
    fn color_modifier_accepts_color_solid_wrapper() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().background = Some(BbBackground::default());
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "_Type_": "BuildingBlocks_FieldModifierColor",
                        "field": "BackgroundColor",
                        "color": {
                            "_Type_": "BuildingBlocks_ColorSolid",
                            "color": {"_Type_": "SRGBA8", "r": 7, "g": 100, "b": 161, "a": 255}
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.background.as_ref().and_then(|bg| bg.fill_colour),
            Some([7.0 / 255.0, 100.0 / 255.0, 161.0 / 255.0, 1.0])
        );
    }

    /// A `FieldModifierColor` whose palette container carries NO `colorStyles`
    /// (an entries-only container like the power screen's defaultStyles /
    /// brandStyles) cannot resolve the RGBA — but its TOKEN must still flow
    /// (the render-time colour resolver has the effective palette). Dropping
    /// the token left the power icons' 'System Icon Color' Accent2 unapplied.
    #[test]
    fn color_modifier_without_palette_still_writes_token() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "_Type_": "BuildingBlocks_FieldModifierColor",
                        "field": "FillColor",
                        "color": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Accent2",
                            "alpha": 1.0
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("FillColorToken").and_then(|v| v.as_str()),
            Some("Accent2"),
            "the colour token must survive an unresolvable palette"
        );
    }

    /// When a HIGHER-priority entry resolves token-only (entries-only
    /// palette), a stale RGBA written by an earlier pass for the same field
    /// must not shadow it at draw time: the medical close-button X kept its
    /// Base light-blue RGBA (icon overlay default) while the bioc ghost
    /// entry's `Bright` token landed beside it — the renderer drew the stale
    /// RGBA and the wanted white X never appeared. The fallback clears the
    /// field's RGBA so the render-time resolver honours the token (the same
    /// overwrite semantics as the resolved-colour path).
    #[test]
    fn token_only_color_modifier_clears_stale_rgba() {
        let mut scene = make_test_scene();
        {
            let node = scene.nodes.get_mut(&1).unwrap();
            let obj = node.raw.as_object_mut().unwrap();
            obj.insert(
                "FillColor".to_string(),
                json!({"r": 0.45, "g": 0.78, "b": 1.0, "a": 1.0}),
            );
            obj.insert("FillColorToken".to_string(), json!("Base"));
        }
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "_Type_": "BuildingBlocks_FieldModifierColor",
                        "field": "FillColor",
                        "color": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Bright",
                            "alpha": 1.0
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("FillColorToken").and_then(|v| v.as_str()),
            Some("Bright"),
            "the later entry's token wins"
        );
        assert!(
            node.raw.get("FillColor").is_none(),
            "the stale RGBA from the earlier pass is cleared, got {:?}",
            node.raw.get("FillColor")
        );
    }

    #[test]
    fn test_type_condition_matches_widget_image() {
        // ConditionType "Image" must match a WidgetImage node.
        let mut scene = make_test_scene(); // node ty = WidgetImage
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleConditionList",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Image"
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/test_image.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("ImagePath").and_then(|v| v.as_str()),
            Some("UI/Textures/test_image.tif"),
            "ConditionType 'Image' must match WidgetImage node"
        );
    }

    #[test]
    fn test_type_condition_no_match_wrong_type() {
        // ConditionType "Text" must NOT match a WidgetImage node.
        let mut scene = make_test_scene(); // node ty = WidgetImage
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleConditionList",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Text"
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/should_not_apply.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert!(
            node.raw.get("ImagePath").is_none(),
            "ConditionType 'Text' must NOT match WidgetImage node"
        );
    }

    /// `ConditionType "Base"` matches a `DisplayWidget` node (the game's base
    /// widget type). This is the footer's `ScreenNameBackground` gate
    /// (`AllOf(Type=Base, Tag …)` on `base_BG`).
    #[test]
    fn type_condition_base_matches_display_widget() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().ty = BbNodeType::DisplayWidget;
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{ "conditions": [
                    { "_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Base" }
                ]}],
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
            "ConditionType 'Base' must match a DisplayWidget node");
    }

    /// `ConditionType "Base"` must NOT match a non-DisplayWidget node (a TextField).
    #[test]
    fn type_condition_base_does_not_match_text_field() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().ty = BbNodeType::WidgetTextField;
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{ "conditions": [
                    { "_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Base" }
                ]}],
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
            "ConditionType 'Base' must not match a WidgetTextField node");
    }

    #[test]
    fn test_mixed_type_and_tag_condition_matches() {
        // Mixed AllOf condition: ConditionType "Image" + ConditionTag must both pass.
        let mut scene = make_test_scene(); // WidgetImage, style_tag_uuids = ["tag-uuid-1"]
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAllOfCondition",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Image"
                            },
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": { "_RecordId_": "tag-uuid-1" }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/DRAK_Background.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("ImagePath").and_then(|v| v.as_str()),
            Some("UI/Textures/DRAK_Background.tif"),
            "Mixed type+tag condition must match WidgetImage with matching tag"
        );
    }

    #[test]
    fn test_inline_color_overlay_resolves_named_svg_tint() {
        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).unwrap();
        node.ty = BbNodeType::WidgetCustomShape;
        node.background = Some(BbBackground::default());
        node.raw = json!({
            "enableColorOverlay": true,
            "svgPath": "UI/Textures/Vector/General/FingerPrint.svg",
            "color": {
                "_Type_": "BuildingBlocks_ColorStyle",
                "color": "Accent1",
                "alpha": 1.0
            }
        });
        let style_record = json!({
            "colorStyles": [
                { "color": { "r": 115, "g": 198, "b": 254, "a": 255 } },
                { "color": { "r": 67, "g": 221, "b": 147, "a": 255 } },
                { "color": { "r": 228, "g": 218, "b": 77, "a": 255 } },
                { "color": { "r": 201, "g": 51, "b": 51, "a": 255 } },
                { "color": { "r": 0, "g": 113, "b": 188, "a": 255 } }
            ]
        });
        let brand = BrandStyle {
            identifier: "s_bioc".to_string(),
            entries: &[],
            raw: &style_record,
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let fill = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        // A custom-shape fill overlay (the FingerPrint) resolves `Accent1` with
        // SURFACE semantics → slot 4 = (0,113,188) darker blue, not the light slot 0.
        assert!((fill[0] - 0.0 / 255.0).abs() < 0.001);
        assert!((fill[1] - 113.0 / 255.0).abs() < 0.001);
        assert!((fill[2] - 188.0 / 255.0).abs() < 0.001);
        assert_eq!(fill[3], 1.0);
    }

    #[test]
    fn test_border_width_aliases_apply_to_typed_border() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "BorderTopWidth",
                        "value": 1.0
                    },
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "BorderRightWidth",
                        "value": 2.0
                    },
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "BorderBottomWidth",
                        "value": 3.0
                    },
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "BorderLeftWidth",
                        "value": 4.0
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        let border = node.border.as_ref().expect("border should be present");
        assert_eq!(border.top.width, 1.0);
        assert_eq!(border.right.width, 2.0);
        assert_eq!(border.bottom.width, 3.0);
        assert_eq!(border.left.width, 4.0);
    }




    #[test]
    fn test_embedded_parent_child_bright_fill_tints_svg_node() {
        let mut scene = make_test_scene();
        let parent = BbNode {
            id: 2,
            parent: None,
            children: vec![1],
            ty: BbNodeType::WidgetCanvas,
            name: "parent".to_string(),
            style_tag_uuids: vec!["parent-tag".to_string()],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Default::default(),
            position_offset: Default::default(),
            sizing: Default::default(),
            padding: Default::default(),
            margin: Default::default(),
            pivot: Default::default(),
            anchor: Default::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: json!({}),
        };
        scene.nodes.insert(2, parent);
        let child = scene.nodes.get_mut(&1).unwrap();
        child.parent = Some(2);
        child.children.clear();
        child.style_tag_uuids = vec!["fingerprint-child-tag".to_string()];
        child.background = Some(BbBackground::default());
        child.raw = json!({ "svgPath": "UI/Textures/Vector/General/FingerPrint.svg" });
        scene.roots = vec![2];

        let style_record = json!({
            "colorStyles": [
                { "color": { "r": 115, "g": 198, "b": 254, "a": 255 } }
            ]
        });
        let brand = BrandStyle {
            identifier: "embeddedStyles".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": { "_RecordId_": "fingerprint-child-tag" }
                            },
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionParent",
                                "conditions": [
                                    {
                                        "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                        "tag": { "_RecordId_": "parent-tag" }
                                    }
                                ]
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "_Type_": "BuildingBlocks_FieldModifierColor",
                        "field": "FillColor",
                        "color": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Bright",
                            "alpha": 1.0
                        }
                    }
                ]
            })],
            raw: &style_record,
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let fill = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        assert!((fill[0] - 115.0 / 255.0).abs() < 0.001);
        assert!((fill[1] - 198.0 / 255.0).abs() < 0.001);
        assert!((fill[2] - 254.0 / 255.0).abs() < 0.001);
        assert_eq!(fill[3], 1.0);
    }


/// `enableColorOverlay: true` is the universal editor default on images, so
/// an overlay-enabled IMAGE with no authored colour must NOT be default
/// tinted (a Base default blue-cast the medical card photos and brown-washed
/// the power screen) — only WidgetIcon glyphs get the Base default.
#[test]
fn overlay_enabled_image_without_colour_keeps_own_colours() {
    let mut scene = make_test_scene(); // node ty = WidgetImage
    {
        let node = scene.nodes.get_mut(&1).unwrap();
        let obj = node.raw.as_object_mut().unwrap();
        obj.insert(
            "svgFill".to_string(),
            json!({"enableColorOverlay": true, "color": null}),
        );
    }
    let brand = BrandStyle {
        identifier: "test_brand".to_string(),
        entries: &[],
        raw: &json!({}),
    };
    apply_brand_modifiers(&mut scene, &brand, None);
    assert!(
        scene.nodes.get(&1).unwrap().raw.get("FillColorToken").is_none(),
        "an overlay-enabled colourless image must not be default-tinted"
    );
}

/// A plain image (no colour overlay) keeps its own colours — no default tint.
#[test]
fn image_without_overlay_keeps_own_colours() {
    let mut scene = make_test_scene();
    let brand = BrandStyle {
        identifier: "test_brand".to_string(),
        entries: &[],
        raw: &json!({}),
    };
    apply_brand_modifiers(&mut scene, &brand, None);
    assert!(
        scene.nodes.get(&1).unwrap().raw.get("FillColorToken").is_none(),
        "a non-overlay image must not be default-tinted"
    );
}

#[test]
fn style_provenance_helpers_track_the_winning_field_source() {
    // `stamp_style_provenance` is last-writer-wins (mirrors the value
    // resolution — recorded only for APPLIED modifiers, so a suppressed shared
    // override is never credited); `provenance_field` flags only colour /
    // visibility / geometry modifiers (ledger item A / SB_UI_STYLE_PROVENANCE).
    let mut scene = make_test_scene();
    let node = scene.nodes.get_mut(&1).unwrap();
    stamp_style_provenance(node, "BackgroundColor", "mfd_g_emissions/New Style");
    stamp_style_provenance(node, "BackgroundColor", "s_drak_hud/Brand Accent");
    let provenance = node
        .raw
        .get("__StyleProvenance")
        .and_then(|map| map.get("BackgroundColor"))
        .and_then(|value| value.as_str());
    assert_eq!(provenance, Some("s_drak_hud/Brand Accent"), "last writer wins");

    let colour = json!({"_Type_": "BuildingBlocks_FieldModifierColor", "field": "BackgroundColor"});
    let padding = json!({"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PaddingTop", "value": 0.0});
    assert_eq!(provenance_field(&colour), Some("BackgroundColor"));
    assert_eq!(provenance_field(&padding), None, "non-colour/visibility fields are not tracked");
}
