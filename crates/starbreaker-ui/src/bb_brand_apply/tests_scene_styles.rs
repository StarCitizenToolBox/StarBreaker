//! Regression tests for projecting authored canvas style entries onto scenes.

use super::apply_scene_style_entries;
use crate::bb_scene::{BbCoordinateMethod, BbNode, BbNodeType, BbScene};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn scene_style_entries_apply_enable_background_to_matching_node() {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        1,
        BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
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
        },
    );
    nodes.insert(
        2,
        BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetImage,
            name: "child".to_string(),
            style_tag_uuids: vec!["child-tag".to_string()],
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
        },
    );

    let mut scene = BbScene {
        coordinate_method: BbCoordinateMethod::UseRaw,
        canvas_size: (100.0, 100.0),
        roots: vec![1],
        nodes,
        operations: vec![],
    };
    let entries = vec![json!({
        "conditionsList": [{
            "conditions": [
                {"_Type_": "BuildingBlocks_StyleSelectorConditionTag", "tag": {"_RecordId_": "child-tag"}},
                {"_Type_": "BuildingBlocks_StyleSelectorConditionAncestor", "conditions": [
                    {"_Type_": "BuildingBlocks_StyleSelectorConditionTag", "tag": {"_RecordId_": "parent-tag"}}
                ]}
            ]
        }],
        "modifiers": [
            {"_Type_": "BuildingBlocks_FieldModifierBoolean", "field": "EnableBackground", "value": true}
        ]
    })];

    apply_scene_style_entries(&mut scene, &entries, &json!({}), None);

    assert_eq!(
        scene.nodes.get(&2).and_then(|node| node.raw.get("EnableBackground")).and_then(|value| value.as_bool()),
        Some(true)
    );
}