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

    /// A `WidgetClone`'s `urlPostfix` namespaces the CLONED subtree's
    /// inheriting variable bindings (the emissions header's clone_IR/EM/CS
    /// carry `Signatures/[000i]` so each clone's `Emitted`/`Ambient` resolve
    /// per signature channel). Non-inheriting bindings address absolute
    /// engine paths and stay untouched.
    #[test]
    fn widget_clone_url_postfix_namespaces_cloned_variable_bindings() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetClone",
                     "name": "clone_IR", "isActive": true,
                     "parent": "_PointsTo_:ptr:1",
                     "urlPostfix": "Signatures/[0000]",
                     "target": "_PointsTo_:ptr:4"}
                ],
                "library": [
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "template", "isActive": true},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "text_Emitted", "isActive": true,
                     "parent": "_PointsTo_:ptr:4"}
                ],
                "operations": [
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                     "binding": "Emitted", "inheritsNamespace": true},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                     "binding": "Absolute/Path", "inheritsNamespace": false},
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:5", "field": "Alpha",
                     "input": "_PointsTo_:ptr:10"},
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:5", "field": "SizeX",
                     "input": "_PointsTo_:ptr:11"}
                ]
            }
        });

        let scene = parse_bb_canvas(&canvas).expect("parse failed");
        let bindings: Vec<String> = scene
            .operations
            .iter()
            .filter(|op| {
                op.get("_Type_").and_then(|v| v.as_str())
                    == Some("BuildingBlocks_BindingsNumberVariable")
            })
            .filter_map(|op| op.get("binding").and_then(|v| v.as_str()).map(str::to_owned))
            .collect();
        assert!(
            bindings.iter().any(|b| b == "Signatures/[0000]/Emitted"),
            "cloned inheriting binding is namespaced, got {bindings:?}"
        );
        assert!(
            bindings.iter().filter(|b| *b == "Absolute/Path").count() == 2,
            "non-inheriting binding stays absolute in both copies, got {bindings:?}"
        );
        assert!(
            bindings.iter().any(|b| b == "Emitted"),
            "the library original keeps its un-namespaced binding"
        );
    }

    /// A `WidgetClone`'s `modifiers` (FieldModifierPair) apply to the CLONED
    /// instance: a `FieldModifierLocalization` on a `ParamInputN` field
    /// supplies the cloned target widget's localized parameter (the emissions
    /// clone_IR/EM/CS set text_Abbreviation's `ParamInput0` to
    /// `@hud_Label_IR/EM/CS`). The synthesized `_SynthLocalizedWidget_` op is
    /// the same vehicle `inject_param_overrides` uses; the library original
    /// keeps its placeholder.
    #[test]
    fn widget_clone_localization_modifiers_apply_to_cloned_targets() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetClone",
                     "name": "clone_IR", "isActive": true,
                     "parent": "_PointsTo_:ptr:1",
                     "target": "_PointsTo_:ptr:4",
                     "modifiers": [
                        {"_Type_": "BuildingBlocks_FieldModifierPair",
                         "target": "_PointsTo_:ptr:5",
                         "modifier": {
                            "_Type_": "BuildingBlocks_FieldModifierLocalization",
                            "field": "ParamInput0",
                            "value": "@hud_Label_IR"
                         }}
                     ]}
                ],
                "library": [
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "template", "isActive": true},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "text_Abbreviation", "isActive": true,
                     "parent": "_PointsTo_:ptr:4"}
                ],
                "operations": []
            }
        });

        let scene = parse_bb_canvas(&canvas).expect("parse failed");
        let cloned_label = scene
            .nodes
            .values()
            .find(|node| node.name == "text_Abbreviation" && node.id != 5)
            .expect("expected cloned label node");
        let synth = scene
            .operations
            .iter()
            .find(|op| op.get("_Type_").and_then(|v| v.as_str()) == Some("_SynthLocalizedWidget_"))
            .expect("expected a _SynthLocalizedWidget_ op for the clone modifier");
        assert_eq!(
            synth.get("widget").and_then(|v| v.as_str()),
            Some(format!("_PointsTo_:ptr:{}", cloned_label.id).as_str()),
            "the modifier applies to the CLONED label, not the library original"
        );
        assert_eq!(
            synth.get("resolvedLocKey").and_then(|v| v.as_str()),
            Some("@hud_Label_IR")
        );
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

    /// A `WidgetIcon` selecting its glyph by `iconProperties.iconPreset` (with an
    /// empty `customIcon`/`svgPath`) must resolve the preset to its vector-icon
    /// asset path so it renders — e.g. the MFD footer's `<`/`>` nav carats.
    #[test]
    fn icon_preset_without_custom_icon_resolves_to_svg_asset() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetIcon",
                        "name": "icon_Previous",
                        "isActive": true,
                        "svgFill": {"_Type_": "BuildingBlocks_SvgFill", "svgPath": ""},
                        "iconProperties": {
                            "_Type_": "BuildingBlocks_ComponentIconProperties",
                            "iconPreset": "ArrowCaratLeft",
                            "customIcon": ""
                        }
                    }
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("parse failed");
        let node = &scene.nodes[&scene.roots[0]];
        let icon = node.icon.as_ref().expect("WidgetIcon should have icon field");
        assert_eq!(
            icon.image_record.as_deref(),
            Some("UI/Textures/Vector/General/ModularKit/Widgets/IconWidget/arrow_carat_left.svg"),
            "an empty-customIcon WidgetIcon must resolve its iconPreset to an SVG asset"
        );
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

    // ── page-in start-state settling ─────────────────────────────────────────

    /// A scene root authored `alpha == 0` but `isActive` with a page-in
    /// `animation` block is the engine's page-in container; a settled static
    /// capture must use the end state (1.0). Both the parsed `alpha` field **and**
    /// the backing `raw["alpha"]` must be settled so any later reader that
    /// re-derives alpha from `raw` sees the end-state, not the page-in start.
    #[test]
    fn pagein_start_root_settles_alpha_in_field_and_raw() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "base_Root",
                        "isActive": true,
                        "alpha": 0.0,
                        "animation": {"_Type_": "BuildingBlocks_AnimationPlayer", "playOnLoad": true}
                    }
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("parse failed");
        let root = &scene.nodes[&scene.roots[0]];
        assert_eq!(root.alpha, 1.0, "parsed alpha field must be settled to 1.0");
        assert_eq!(
            root.raw.get("alpha").and_then(|v| v.as_f64()),
            Some(1.0),
            "raw[\"alpha\"] must be settled to 1.0 to stay consistent with the parsed field"
        );
    }
