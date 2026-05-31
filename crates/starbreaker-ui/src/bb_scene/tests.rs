use super::parse_bb_canvas;
use super::types::{BbNodeType, BbScene};

// ── helpers ──────────────────────────────────────────────────────────────

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/canvas/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse fixture {name} as JSON: {e}"))
    }

    fn count_type(scene: &BbScene, ty: &BbNodeType) -> usize {
        scene.nodes.values().filter(|n| &n.ty == ty).count()
    }

    // ── MC_S_Target_Master ───────────────────────────────────────────────────

    #[test]
    fn target_master_node_count_and_types() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 2, "expected 2 nodes");
        assert_eq!(count_type(&scene, &BbNodeType::DisplayWidget), 1);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 1);
    }

    #[test]
    fn target_master_root_and_parent() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.roots.len(), 1);
        let root_id = scene.roots[0];
        let root = &scene.nodes[&root_id];
        assert!(root.parent.is_none(), "root should have no parent");

        // The non-root node's parent must equal the root id.
        let child = scene.nodes.values().find(|n| n.parent.is_some()).expect("no child found");
        assert_eq!(child.parent, Some(root_id));
    }

    #[test]
    fn target_master_canvas_size() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!(scene.canvas_size.0 > 0.0, "canvas width should be positive");
        assert!(scene.canvas_size.1 > 0.0, "canvas height should be positive");
    }

    #[test]
    fn target_master_root_children_wired() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let root_id = scene.roots[0];
        let root = &scene.nodes[&root_id];
        assert_eq!(root.children.len(), 1, "root should have exactly 1 child");
    }

    #[test]
    fn parse_background_accepts_color_solid_wrapper() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "solid_background",
                        "isActive": true,
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorSolid",
                                "color": {"_Type_": "SRGBA8", "r": 17, "g": 19, "b": 36, "a": 64}
                            }
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = parse_bb_canvas(&canvas).expect("parse failed");
        let node = scene.nodes.values().find(|node| node.name == "solid_background").unwrap();
        assert_eq!(
            node.background.as_ref().and_then(|bg| bg.fill_colour),
            Some([17.0 / 255.0, 19.0 / 255.0, 36.0 / 255.0, 64.0 / 255.0])
        );
    }

    #[test]
    fn widget_clone_copies_field_operations_for_cloned_widgets() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "root",
                        "isActive": true
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetClone",
                        "name": "clone",
                        "isActive": true,
                        "parent": "_PointsTo_:ptr:1",
                        "target": "_PointsTo_:ptr:4"
                    }
                ],
                "library": [
                    {
                        "_Pointer_": "ptr:4",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "template",
                        "isActive": true
                    },
                    {
                        "_Pointer_": "ptr:5",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "template_child",
                        "isActive": false,
                        "parent": "_PointsTo_:ptr:4"
                    }
                ],
                "operations": [
                    {
                        "_Pointer_": "ptr:10",
                        "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                        "binding": "EnableBackground"
                    },
                    {
                        "_Type_": "BuildingBlocks_BindingsBooleanField",
                        "widget": "_PointsTo_:ptr:5",
                        "field": "IsActive",
                        "input": "_PointsTo_:ptr:10"
                    }
                ]
            }
        });

        let scene = parse_bb_canvas(&canvas).expect("parse failed");
        let cloned_child = scene
            .nodes
            .values()
            .find(|node| node.name == "template_child" && node.parent == Some(3))
            .expect("expected cloned child node");
        let cloned_widget_ref = format!("_PointsTo_:ptr:{}", cloned_child.id);
        let bool_field_count = scene
            .operations
            .iter()
            .filter(|op| {
                op.get("_Type_").and_then(|v| v.as_str())
                    == Some("BuildingBlocks_BindingsBooleanField")
            })
            .count();
        let cloned_field_exists = scene.operations.iter().any(|op| {
            op.get("_Type_").and_then(|v| v.as_str())
                == Some("BuildingBlocks_BindingsBooleanField")
                && op.get("widget").and_then(|v| v.as_str()) == Some(cloned_widget_ref.as_str())
        });

        assert_eq!(bool_field_count, 2, "expected the clone to duplicate the field op");
        assert!(cloned_field_exists, "expected a remapped field op for the cloned widget");
    }

    // ── MC_S_Self_Master ─────────────────────────────────────────────────────

    #[test]
    fn self_master_node_count_and_types() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 7, "expected 7 nodes");
        assert_eq!(count_type(&scene, &BbNodeType::DisplayWidget), 1);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 6);
    }

    #[test]
    fn self_master_single_root() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert_eq!(scene.roots.len(), 1);
        let root = &scene.nodes[&scene.roots[0]];
        assert!(root.parent.is_none());
        assert_eq!(root.ty, BbNodeType::DisplayWidget);
    }

    #[test]
    fn self_master_canvas_size_1920x1080() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!((scene.canvas_size.0 - 1920.0).abs() < f32::EPSILON);
        assert!((scene.canvas_size.1 - 1080.0).abs() < f32::EPSILON);
    }

    #[test]
    fn self_master_root_has_six_children() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let root = &scene.nodes[&scene.roots[0]];
        assert_eq!(root.children.len(), 6);
    }

    // ── BB_ScreenRadar ───────────────────────────────────────────────────────

    #[test]
    fn radar_node_count_and_types() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 25, "expected 25 nodes");
        assert_eq!(count_type(&scene, &BbNodeType::DisplayWidget), 6);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 5);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetIcon), 5);
        assert_eq!(count_type(&scene, &BbNodeType::ComponentGeneralButtonSecondary), 4);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCard), 3);
        assert_eq!(count_type(&scene, &BbNodeType::ComponentGeneralButton), 1);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetTextField), 1);
    }

    #[test]
    fn radar_canvas_size_positive() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!(scene.canvas_size.0 > 0.0);
        assert!(scene.canvas_size.1 > 0.0);
    }

    #[test]
    fn radar_text_field_alignment() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let tf = scene
            .nodes
            .values()
            .find(|n| n.ty == BbNodeType::WidgetTextField)
            .expect("no WidgetTextField found");
        // In the fixture the textAlignment is "Center".
        assert!(!tf.text.as_ref().unwrap().alignment.is_empty());
    }

    #[test]
    fn radar_icon_nodes_parsed() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        // All WidgetIcon nodes should have their icon field populated.
        for node in scene.nodes.values().filter(|n| n.ty == BbNodeType::WidgetIcon) {
            assert!(node.icon.is_some(), "WidgetIcon node should have icon field");
        }
    }

    #[test]
    fn radar_style_tags_parsed() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        // Verify that at least one node has style tags — confirms the parsing
        // logic actually extracts UUIDs from the styleTags array.
        let any_with_tags = scene.nodes.values().any(|n| !n.style_tag_uuids.is_empty());
        assert!(any_with_tags, "expected at least one node with style tags");
    }

    // ── EC_PowerManagement ───────────────────────────────────────────────────

    #[test]
    fn power_management_single_widget_canvas() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 1, "expected 1 node");
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 1);
    }

    #[test]
    fn power_management_root_no_parent() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.roots.len(), 1);
        let root = &scene.nodes[&scene.roots[0]];
        assert!(root.parent.is_none());
        assert_eq!(root.ty, BbNodeType::WidgetCanvas);
    }

    #[test]
    fn power_management_canvas_size_positive() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!(scene.canvas_size.0 > 0.0);
        assert!(scene.canvas_size.1 > 0.0);
    }

    #[test]
    fn power_management_node_is_active() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let root = &scene.nodes[&scene.roots[0]];
        assert!(root.is_active, "root node should be active");
    }
