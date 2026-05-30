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
