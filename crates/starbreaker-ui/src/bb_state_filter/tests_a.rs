    use super::*;
    use serde_json::json;

    fn boolean_field_op(widget_ptr: u32, input_ptr: u32) -> serde_json::Value {
        json!({
            "_Type_": "BuildingBlocks_BindingsBooleanField",
            "widget": format!("_PointsTo_:ptr:{widget_ptr}"),
            "field": "Instantiated",
            "input": format!("_PointsTo_:ptr:{input_ptr}")
        })
    }

    fn variable_op(ptr: u32, binding: &str) -> serde_json::Value {
        json!({
            "_Pointer_": format!("ptr:{ptr}"),
            "_Type_": "BuildingBlocks_BindingsBooleanVariable",
            "binding": binding
        })
    }

    fn scene_widget(ptr: u32, params: Vec<serde_json::Value>) -> serde_json::Value {
        json!({
            "_Pointer_": format!("ptr:{ptr}"),
            "_Type_": "BuildingBlocks_WidgetCanvas",
            "paramInputValues": params,
        })
    }

    fn static_var(name: &str, val: bool) -> serde_json::Value {
        json!({ "name": name, "value": val })
    }

    fn make_record_value(
        static_vars: Vec<serde_json::Value>,
        ops: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        json!({
            "_Type_": "BuildingBlocks_Canvas",
            "staticVariables": static_vars,
            "operations": ops
        })
    }

    // ── test 1 ──────────────────────────────────────────────────────────────

    /// Canvas with no operations produces an empty false set.
    #[test]
    fn no_operations_returns_empty_set() {
        let rv = make_record_value(vec![], vec![]);
        let result = instantiated_false_widgets(&rv);
        assert!(result.is_empty(), "expected empty set, got {result:?}");
    }

    // ── test 2 ──────────────────────────────────────────────────────────────

    /// A WidgetCanvas whose Instantiated is bound to a variable with a direct
    /// `staticVariables` entry of `true` (no `_SV` suffix) must NOT appear in
    /// the false set.
    #[test]
    fn static_true_variable_widget_is_not_filtered() {
        let rv = make_record_value(
            vec![static_var("state.Admin", true)],
            vec![
                variable_op(10, "state.Admin"),
                boolean_field_op(5, 10), // ptr:5 (WidgetCanvas) bound to ptr:10 (Admin=true)
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&5),
            "Admin=true canvas (ptr:5) must not be filtered"
        );
    }

    /// A `_SV`-suffixed capability flag in `staticVariables` must NOT activate
    /// the same-named active-mode bool.  This is the wall-medbay case:
    /// `state.Admin_SV=true` is a capability flag, NOT an Admin-state activator.
    ///
    /// Scaffold: include an Invert(EvaluateOr) framing-widget pattern that
    /// names Attract as the cold-default (matches real wall medbay shape) so
    /// the direct-variable scene-order rule does NOT promote Admin.
    #[test]
    fn sv_capability_flag_does_not_activate_active_mode() {
        let rv = make_record_value(
            vec![static_var("state.Admin_SV", true)],
            vec![
                // Cold-default chain: Invert(Or(Attract, LogIn)) → Attract is
                // first Or operand → cold-default = Attract.
                variable_op(20, "state.Attract"),
                variable_op(30, "state.LogIn"),
                json!({
                    "_Pointer_": "ptr:23",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:20", "_PointsTo_:ptr:30"]
                }),
                json!({
                    "_Pointer_": "ptr:22",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:23"
                }),
                boolean_field_op(21, 22), // Header hidden when Or(Attract, LogIn) is true
                // Admin sub-canvas gated by direct variable; should remain
                // false (Attract is the cold-default for the group).
                variable_op(10, "state.Admin"),
                boolean_field_op(5, 10),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&5),
            "Admin canvas (ptr:5) must be filtered — _SV capability flag must NOT activate the matching active-mode bool"
        );
    }

    /// Boolean parameter slots should be detected anywhere under the candidate
    /// widget subtree, not just on the candidate node itself.
    #[test]
    fn scene_widget_boolean_param_count_tracks_descendants() {
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "scene": [
                scene_widget(9, vec![]),
                json!({
                    "_Pointer_": "ptr:10",
                    "_Type_": "BuildingBlocks_WidgetCanvas",
                    "parent": "_PointsTo_:ptr:9",
                    "paramInputValues": [
                        json!({
                            "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
                            "parameter": "ParamInput0",
                            "value": false
                        })
                    ]
                }),
            ]
        });
        let items = scene_widget_map(&rv);
        assert_eq!(scene_widget_boolean_param_count(&items, 9), 1);
    }

    #[test]
    fn scene_widget_without_boolean_param_counts_zero() {
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "scene": [scene_widget(9, vec![])]
        });
        let items = scene_widget_map(&rv);
        assert_eq!(scene_widget_boolean_param_count(&items, 9), 0);
    }

    // ── test 3 ──────────────────────────────────────────────────────────────

    /// A WidgetCanvas whose Instantiated is bound to a variable with no static
    /// override (defaults to `false`) must appear in the false set.
    #[test]
    fn no_static_var_defaults_to_false_and_is_filtered() {
        let rv = make_record_value(
            vec![], // no static variables → all default false
            vec![
                variable_op(10, "state.Attract"),
                boolean_field_op(3, 10), // ptr:3 bound to Attract=false
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&3),
            "Attract=false canvas (ptr:3) must be in false set"
        );
    }

    #[test]
    fn inline_widget_and_input_refs_are_evaluated() {
        let rv = make_record_value(
            vec![],
            vec![json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "field": "Instantiated",
                "widget": {"_Type_":"BuildingBlocks_WidgetCanvas","_Pointer_":"ptr:3"},
                "input": {"_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"state.Attract"}
            })],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&3),
            "inline object refs should evaluate like pointer refs"
        );
    }

    #[test]
    fn bed_mainmenu_default_applies_with_inline_boolean_variable_inputs() {
        let rv = make_record_value(
            vec![],
            vec![
                json!({
                    "_Type_":"BuildingBlocks_BindingsBooleanField",
                    "field":"Instantiated",
                    "widget":{"_Type_":"BuildingBlocks_WidgetCanvas","_Pointer_":"ptr:5"},
                    "input":{"_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"Bed/state.BaseScreens.Attract"}
                }),
                json!({
                    "_Type_":"BuildingBlocks_BindingsBooleanField",
                    "field":"Instantiated",
                    "widget":{"_Type_":"BuildingBlocks_WidgetCanvas","_Pointer_":"ptr:6"},
                    "input":{"_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"Bed/state.BaseScreens.MainMenu"}
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Attract should be hidden for bed cold-default");
        assert!(!result.contains(&6), "MainMenu should be shown for bed cold-default");
    }

    // ── test 4 ──────────────────────────────────────────────────────────────

    /// Compound expression: `Instantiated = OR(false, false)` must produce
    /// `false` → canvas is in the false set.
    #[test]
    fn compound_or_false_false_is_filtered() {
        // ptr:20 = ConfirmMore (no static → false)
        // ptr:21 = ConfirmNone (no static → false)
        // ptr:23 = OR(ptr:20, ptr:21) → false
        // ptr:7 (WidgetCanvas) bound to ptr:23 → filtered
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(20, "state.ConfirmMore"),
                variable_op(21, "state.ConfirmNone"),
                json!({
                    "_Pointer_": "ptr:23",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:20", "_PointsTo_:ptr:21"]
                }),
                boolean_field_op(7, 23),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&7),
            "OR(false, false) canvas (ptr:7) must be in false set"
        );
    }

    // ── test 5 ──────────────────────────────────────────────────────────────

    /// A WidgetCanvas with no Instantiated binding at all is never filtered.
    #[test]
    fn canvas_without_instantiated_binding_is_not_filtered() {
        // Only a non-Instantiated field op exists for ptr:9.
        let rv = make_record_value(
            vec![],
            vec![json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:9",
                "field": "IsActive",
                "input": "_PointsTo_:ptr:10"
            })],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&9),
            "canvas with no Instantiated binding must not be filtered"
        );
    }

    // ── test 6 ──────────────────────────────────────────────────────────────

    /// Plain `Invert(SingleVariable)` is NOT an idle-default pattern.  This
    /// is a single-flag hide gate: the framing widget is hidden only when
    /// the flag is explicitly true.  At cold-default the flag is false so
    /// the framing widget remains visible.
    ///
    /// This protects Header/Footer widgets bound via
    /// `Instantiated = NOT(SomeFlag)` from being incorrectly hidden when
    /// `SomeFlag` is the only thing being inverted.
    #[test]
    fn invert_single_variable_does_not_trigger_idle_default() {
        // ptr:3 = SomeFlag (no static value)
        // ptr:6 = NOT(ptr:3)
        // ptr:5 (Header WidgetCanvas) bound to ptr:6 → visible while flag is false
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.SomeFlag"),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&5),
            "Header (ptr:5) must NOT be filtered — Invert(SingleVariable) is not an idle-default pattern"
        );
    }

    /// Integer-state op-types resolve to their at-rest values: an event check
    /// (`Equal value`) is false (overlay hidden), while content gated by
    /// `Invert(Equal off_state)` stays visible (the rendered screen is on), and
    /// a `BooleanFromIntegerSwitch` yields its `defaultValue`.
    #[test]
    fn integer_state_ops_resolve_to_at_rest_values() {
        fn isactive_op(widget: u32, input: u32) -> serde_json::Value {
            json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": format!("_PointsTo_:ptr:{widget}"),
                "field": "IsActive",
                "input": format!("_PointsTo_:ptr:{input}")
            })
        }
        let rv = make_record_value(
            vec![],
            vec![
                // widget 5: IsActive = FromInteger(Equal 5)  → event → hidden.
                json!({"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Equal","value":5,"inputL":"_PointsTo_:ptr:99"}),
                isactive_op(5, 10),
                // widget 6: IsActive = Invert(FromInteger(Equal 0)) → "show unless off" → visible.
                json!({"_Pointer_":"ptr:21","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Equal","value":0,"inputL":"_PointsTo_:ptr:99"}),
                json!({"_Pointer_":"ptr:20","_Type_":"BuildingBlocks_BindingsBooleanInvert","input":"_PointsTo_:ptr:21"}),
                isactive_op(6, 20),
                // widget 7: IsActive = IntegerSwitch{defaultValue:false} → hidden.
                json!({"_Pointer_":"ptr:30","_Type_":"BuildingBlocks_BindingsBooleanFromIntegerSwitch","defaultValue":false,"exceptions":[1],"input":"_PointsTo_:ptr:99"}),
                isactive_op(7, 30),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "FromInteger(Equal 5) event must hide widget 5: {result:?}");
        assert!(!result.contains(&6), "Invert(Equal 0) (on at rest) must keep widget 6 visible: {result:?}");
        assert!(result.contains(&7), "IntegerSwitch default-false must hide widget 7: {result:?}");
    }

    /// An `EvaluateAnd` with a determining `false` operand resolves to `false`
    /// (overlay hidden) even when a sibling operand is unresolved — rather than
    /// bailing to "unknown" and falling to the conservative keep-visible default.
    #[test]
    fn and_short_circuits_on_a_false_operand() {
        fn isactive_op(widget: u32, input: u32) -> serde_json::Value {
            json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": format!("_PointsTo_:ptr:{widget}"),
                "field": "IsActive",
                "input": format!("_PointsTo_:ptr:{input}")
            })
        }
        let rv = make_record_value(
            vec![],
            vec![
                // ptr:10 = FromInteger(Equal 5) → false (event not active).
                json!({"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Equal","value":5,"inputL":"_PointsTo_:ptr:99"}),
                // ptr:11 = And(ptr:10=false, ptr:98=unresolvable) → false via short-circuit.
                json!({"_Pointer_":"ptr:11","_Type_":"BuildingBlocks_BindingsBooleanEvaluateAnd","inputs":["_PointsTo_:ptr:10","_PointsTo_:ptr:98"]}),
                isactive_op(5, 11),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&5),
            "And(false, unknown) must short-circuit to false and hide widget 5: {result:?}"
        );
    }

    /// When a `BooleanFromInteger`'s operands are statically resolvable — an
    /// `IntegerComponentParameter` carries an authored `defaultValue` — the real
    /// comparison is computed instead of the blunt at-rest `Equal→false`
    /// heuristic. A wired `inputR` operand takes precedence over the inline
    /// `value` literal (the engine uses `value` only for an unwired right
    /// operand). Runtime `IntegerVariable` operands have no static default and
    /// still take the heuristic path (covered by
    /// `integer_state_ops_resolve_to_at_rest_values`).
    #[test]
    fn from_integer_with_component_parameter_resolves_real_comparison() {
        fn isactive_op(widget: u32, input: u32) -> serde_json::Value {
            json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": format!("_PointsTo_:ptr:{widget}"),
                "field": "IsActive",
                "input": format!("_PointsTo_:ptr:{input}")
            })
        }
        fn int_param(ptr: u32, parameter: &str, default: i64) -> serde_json::Value {
            json!({
                "_Pointer_": format!("ptr:{ptr}"),
                "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                "parameter": parameter,
                "defaultValue": default
            })
        }
        let rv = make_record_value(
            vec![],
            vec![
                // widget 5: IsActive = (param[5] Equal 5) → TRUE → shown. The
                // blunt heuristic would wrongly hide this (Equal→false).
                int_param(40, "ParamInput0", 5),
                json!({"_Pointer_":"ptr:41","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Equal","value":5,"inputL":"_PointsTo_:ptr:40"}),
                isactive_op(5, 41),
                // widget 6: IsActive = (param[5] Equal 0) → FALSE → hidden.
                int_param(50, "ParamInput1", 5),
                json!({"_Pointer_":"ptr:51","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Equal","value":0,"inputL":"_PointsTo_:ptr:50"}),
                isactive_op(6, 51),
                // widget 7: IsActive = (param[7] Equal inputR=param[7]) → TRUE.
                // A wired inputR overrides the inline `value` literal (0).
                int_param(60, "ParamInput2", 7),
                int_param(61, "ParamInput3", 7),
                json!({"_Pointer_":"ptr:62","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Equal","value":0,"inputL":"_PointsTo_:ptr:60","inputR":"_PointsTo_:ptr:61"}),
                isactive_op(7, 62),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&5),
            "param(5)==5 is TRUE → widget 5 shown (heuristic would wrongly hide it): {result:?}"
        );
        assert!(
            result.contains(&6),
            "param(5)==0 is FALSE → widget 6 hidden: {result:?}"
        );
        assert!(
            !result.contains(&7),
            "param(7)==inputR param(7) is TRUE (inputR overrides value literal) → widget 7 shown: {result:?}"
        );
    }

    /// A canvas whose ONLY top-level scene node is a `WidgetCanvas` gated
    /// `Instantiated = Or(stateVar, capabilityVar)` — both false at static rest —
    /// must NOT be deactivated. It is the entire screen: its content is delivered
    /// by a `CanvasReferenceRecord` style modifier (no `url`) and there is no
    /// sibling state-canvas to fall back to, so deactivating it blanks the whole
    /// render. The static export renders the screen's switched-on state (the same
    /// principle behind the `default_state_is_off` fallback-canvas selection).
    /// Mirrors the HUD-component masters `HC_HUD_Ship_*_Master` (g-force ball,
    /// velocity ball, countermeasures, bars/nums/alerts), whose root canvas binds
    /// `Instantiated = Or(screen, FlightController/AccelerationBallEnabled)`.
    #[test]
    fn sole_top_level_widget_canvas_is_not_deactivated() {
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "staticVariables": [
                static_var("screen", false),
                static_var("FlightController/AccelerationBallEnabled", false),
            ],
            "operations": [
                boolean_field_op(1, 8),
                json!({
                    "_Pointer_": "ptr:8",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:9", "_PointsTo_:ptr:11"]
                }),
                variable_op(9, "screen"),
                variable_op(11, "FlightController/AccelerationBallEnabled"),
            ],
            "scene": [ scene_widget(1, vec![]) ],
        });
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&1),
            "sole top-level WidgetCanvas (the whole screen) must stay instantiated for the static export: {result:?}"
        );
    }

    /// Guard against over-reaching: when a gated `WidgetCanvas` has a SIBLING
    /// top-level `WidgetCanvas`, it belongs to a multi-state mutual-exclusion set
    /// (e.g. medical's Attract / MainMenu / HealMe selection) where deactivating a
    /// false state is correct — another sibling is the cold-default. The sole-root
    /// exemption must NOT leak into that case, so the gated widget stays filtered.
    #[test]
    fn gated_widget_with_sibling_top_level_canvas_is_still_deactivated() {
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "staticVariables": [ static_var("Bed/state.BaseScreens.HealMe", false) ],
            "operations": [
                boolean_field_op(1, 9),
                variable_op(9, "Bed/state.BaseScreens.HealMe"),
            ],
            "scene": [ scene_widget(1, vec![]), scene_widget(2, vec![]) ],
        });
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&1),
            "a gated widget with a sibling top-level canvas (an alternative state) stays filtered: {result:?}"
        );
    }

