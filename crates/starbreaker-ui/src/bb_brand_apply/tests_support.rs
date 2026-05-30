use crate::bb_scene::{BbNode, BbNodeType, BbScene};
use serde_json::json;
use std::collections::BTreeMap;

pub(super) fn make_test_scene() -> BbScene {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        1,
        BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetImage,
            name: "test_node".to_string(),
            style_tag_uuids: vec!["tag-uuid-1".to_string()],
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

    BbScene {
        canvas_size: (1920.0, 1080.0),
        roots: vec![1],
        nodes,
        operations: vec![],
    }
}

