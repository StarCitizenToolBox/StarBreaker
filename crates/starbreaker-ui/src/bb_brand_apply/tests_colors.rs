use super::tests_support::make_test_scene;
use super::*;
use crate::bb_scene::BbBackground;
use serde_json::json;

    #[test]
    fn named_fill_color_preserves_token_in_raw() {
        let palette = json!({
            "colorStyles": [
                {"color": {"r": 1.0, "g": 0.5, "b": 0.25, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.25, "b": 0.75, "a": 1.0}}
            ]
        });

        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierColor",
            "field": "FillColor",
            "color": {
                "_Type_": "BuildingBlocks_ColorStyle",
                "color": "Accent1",
                "alpha": 1.0
            }
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &palette, None);

        assert_eq!(
            node.raw.get("FillColorToken").and_then(|value| value.as_str()),
            Some("Accent1")
        );
        assert!(node.raw.get("FillColor").is_some(), "resolved rgba should still be present");
    }

    #[test]
    fn record_ref_font_style_object_field_maps_to_font_style_record() {
        let palette = json!({});
        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierRecordRef",
            "field": {
                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                "value": "file://./../../fontstyles/blenderpro-bold.json"
            }
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &palette, None);

        assert_eq!(
            node.raw
                .get("FontStyleRecord")
                .and_then(|value| value.as_str()),
            Some("file://./../../fontstyles/blenderpro-bold.json")
        );
    }

    #[test]
    fn test_mixed_type_and_tag_condition_tag_mismatch() {
        // Mixed AllOf: type matches but tag doesn't → should NOT apply.
        let mut scene = make_test_scene(); // WidgetImage, tag = "tag-uuid-1"
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
                                "tag": { "_RecordId_": "wrong-tag" }
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
            "Mixed type+tag condition must NOT match when tag is wrong"
        );
    }

    #[test]
    fn test_condition_ancestor_matches_grandparent_tag() {
        let mut scene = make_test_scene();
        // Re-parent node 1 under parent 2 under grandparent 3.
        let mut parent = scene.nodes.get(&1).cloned().unwrap();
        parent.id = 2;
        parent.name = "parent".to_string();
        parent.parent = Some(3);
        parent.children = vec![1];
        parent.style_tag_uuids = vec!["parent-tag".to_string()];
        parent.raw = json!({});
        scene.nodes.insert(2, parent);

        let mut grandparent = scene.nodes.get(&1).cloned().unwrap();
        grandparent.id = 3;
        grandparent.name = "grandparent".to_string();
        grandparent.parent = None;
        grandparent.children = vec![2];
        grandparent.style_tag_uuids = vec!["ancestor-tag".to_string()];
        grandparent.raw = json!({});
        scene.nodes.insert(3, grandparent);

        let child = scene.nodes.get_mut(&1).unwrap();
        child.parent = Some(2);
        child.children.clear();
        child.background = Some(BbBackground::default());
        scene.roots = vec![3];

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAncestor",
                        "conditions": [{
                            "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                            "tag": { "_RecordId_": "ancestor-tag" }
                        }]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierColor",
                    "field": "FillColor",
                    "color": { "r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0 }
                }]
            })],
            raw: &json!({}),
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
        assert!((fill[0] - 0.25).abs() < 0.001);
        assert!((fill[1] - 0.5).abs() < 0.001);
        assert!((fill[2] - 0.75).abs() < 0.001);
        assert!((fill[3] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_condition_any_of_tag_matches_when_any_tag_matches() {
        let mut scene = make_test_scene();

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAnyOfTag",
                        "tags": [
                            { "_RecordId_": "wrong-tag" },
                            { "_RecordId_": "tag-uuid-1" }
                        ]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierString",
                    "field": "ImagePath",
                    "value": "UI/Textures/any_of_tag_hit.tif"
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).expect("test node");
        assert_eq!(
            node.raw.get("ImagePath").and_then(|value| value.as_str()),
            Some("UI/Textures/any_of_tag_hit.tif")
        );
    }

    #[test]
    fn test_condition_any_of_tag_no_match_when_no_tags_match() {
        let mut scene = make_test_scene();

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAnyOfTag",
                        "tags": [
                            { "_RecordId_": "wrong-tag-a" },
                            { "_RecordId_": "wrong-tag-b" }
                        ]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierString",
                    "field": "ImagePath",
                    "value": "UI/Textures/any_of_tag_should_not_apply.tif"
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).expect("test node");
        assert!(
            node.raw.get("ImagePath").is_none(),
            "ConditionAnyOfTag should not match when node has none of the tags"
        );
    }
