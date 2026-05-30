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
        assert!((fill[0] - 115.0 / 255.0).abs() < 0.001);
        assert!((fill[1] - 198.0 / 255.0).abs() < 0.001);
        assert!((fill[2] - 254.0 / 255.0).abs() < 0.001);
        assert_eq!(fill[3], 1.0);
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

