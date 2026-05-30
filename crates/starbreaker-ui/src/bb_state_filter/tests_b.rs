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
    fn direct_variable_scene_order_picks_first_as_cold_default() {
        // ptr:3 = Attract, ptr:4 = MainMenu, ptr:7 = Heal
        // ptr:5 (AttractCanvas) bound to ptr:3
        // ptr:6 (MainMenuCanvas) bound to ptr:4
        // ptr:8 (HealCanvas) bound to ptr:7
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.BaseScreens.Attract"),
                variable_op(4, "state.BaseScreens.MainMenu"),
                variable_op(7, "state.BaseScreens.Heal"),
                boolean_field_op(5, 3),
                boolean_field_op(6, 4),
                boolean_field_op(8, 7),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "AttractCanvas (ptr:5) shown — first direct-variable in group is cold-default");
        assert!(result.contains(&6), "MainMenuCanvas (ptr:6) hidden — not the cold-default");
        assert!(result.contains(&8), "HealCanvas (ptr:8) hidden — not the cold-default");
    }

    /// Bed base-screen canvases should prefer MainMenu as cold-default when no
    /// explicit static override exists.
    #[test]
    fn bed_base_screens_prefers_mainmenu() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "Bed/state.BaseScreens.Attract"),
                variable_op(4, "Bed/state.BaseScreens.MainMenu"),
                variable_op(7, "Bed/state.BaseScreens.Heal"),
                boolean_field_op(5, 3), // AttractCanvas
                boolean_field_op(6, 4), // MainMenuCanvas
                boolean_field_op(8, 7), // HealCanvas
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "AttractCanvas (ptr:5) hidden for Bed default");
        assert!(
            !result.contains(&6),
            "MainMenuCanvas (ptr:6) shown as Bed cold-default"
        );
        assert!(result.contains(&8), "HealCanvas (ptr:8) hidden");
    }

    /// Direct-variable rule requires a group of ≥2 same-prefix variables.
    /// A single-member group (just `state.X` with no siblings) must NOT be
    /// elected — it's likely a single hide/show flag, not a state-machine.
    #[test]
    fn direct_variable_singleton_group_not_promoted() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.LonelyFlag"),
                boolean_field_op(5, 3),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&5),
            "Singleton-group canvas (ptr:5) must remain filtered — no other group members to make it a state-machine"
        );
    }


    /// If an `Invert(Or(...))` gate names directly-gated sibling canvases,
    /// the first Or operand remains the idle overlay and the framing canvas
    /// follows its evaluated `Instantiated` value (hidden when false).
    #[test]
    fn idle_gate_or_filters_framing_canvas_when_false() {
        // Mirrors the wall medbay shape:
        // ptr:3 = Attract, ptr:4 = LogIn, ptr:19 = Or(3, 4), ptr:6 = NOT(19)
        // ptr:5 (Header) bound to ptr:6 → hidden when Attract is cold-default
        // ptr:11 (LogInCanvas) bound to ptr:4 → hidden
        // ptr:8 (AttractCanvas) bound to ptr:3 → shown as first Or operand
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(4, "state.LogIn"),
                json!({
                    "_Pointer_": "ptr:19",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:3", "_PointsTo_:ptr:4"]
                }),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:19"
                }),
                boolean_field_op(5, 6),
                // Direct-field order in the real wall canvas is LogIn, then Attract.
                boolean_field_op(11, 4),
                boolean_field_op(8, 3),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Header (ptr:5) hidden when Invert(Or(...)) evaluates false");
        assert!(result.contains(&11), "LogInCanvas (ptr:11) hidden (LogIn=false)");
        assert!(!result.contains(&8), "AttractCanvas (ptr:8) shown (first Or operand)");
    }

    /// `Invert(Or(...))` still falls back to the first Or operand when the Or
    /// operands do not have directly-gated sibling canvases.
    #[test]
    fn idle_gate_or_without_direct_siblings_uses_first_operand() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(4, "state.LogIn"),
                json!({
                    "_Pointer_": "ptr:19",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:3", "_PointsTo_:ptr:4"]
                }),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:19"
                }),
                boolean_field_op(5, 6),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Header (ptr:5) hidden under first-operand idle default");
    }

    /// Explicit static-true override of any group member SUPPRESSES the
    /// idle-default rule: the idle-gate variable stays false and the
    /// framing widget is shown.
    #[test]
    fn explicit_group_override_suppresses_idle_default() {
        // ptr:3 = Attract, ptr:7 = Admin, ptr:6 = NOT(3)
        // staticVariables[]: state.Admin=true (explicit override → suppresses
        // Attract idle-default)
        let rv = make_record_value(
            vec![static_var("state.Admin", true)],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(7, "state.Admin"),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
                boolean_field_op(8, 7),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "Header (ptr:5) shown — Admin override suppresses Attract idle-default");
        assert!(!result.contains(&8), "AdminCanvas (ptr:8) shown via explicit static-true");
    }

    /// Old test 6 kept as a separate scenario: when the inverted variable is
    /// NOT a member of an idle-gate group (no shared dotted prefix at all),
    /// idle-default does not kick in and `NOT(false) → true`.
    #[test]
    fn ungrouped_invert_does_not_trigger_idle_default() {
        // Variable binding is a single segment with no dotted prefix.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "Attract"), // no `.` → no group
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "ungrouped variable: NOT(false)=true, Header shown");
    }

    #[test]
    fn is_active_false_widget_is_filtered() {
        // `IsActive` is treated as a visibility field just like `Instantiated`.
        // Variable `state.Foo` has no static default → false → widget ptr:5 hidden.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Foo"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:5",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "IsActive=false widget (ptr:5) must be filtered");
    }

    #[test]
    fn visible_false_widget_is_filtered() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.ActorIsInBed"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:7",
                    "field": "Visible",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&7), "Visible=false widget (ptr:7) must be filtered");
    }

    #[test]
    fn unknown_field_is_not_filtered() {
        // Bindings to non-visibility fields (e.g. `Text`, `Color`) must not
        // cause widget filtering.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.SomeFlag"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:9",
                    "field": "Text",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&9),
            "Bindings to non-visibility fields must not filter the widget"
        );
    }

    #[test]
    fn boolean_component_parameter_default_false_hides_is_active_without_override() {
        let rv = make_record_value(
            vec![],
            vec![serde_json::json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:5",
                "field": "IsActive",
                "input": {
                    "_Pointer_": "ptr:9",
                    "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                    "parameter": "ParamInput0",
                    "defaultValue": false
                }
            })],
        );

        let no_override = instantiated_false_widgets_with_param_inputs(&rv, &[]);
        assert!(
            no_override.contains(&5),
            "without paramInput override, explicit defaultValue=false should hide ptr:5"
        );

        let with_false_override = instantiated_false_widgets_with_param_inputs(
            &rv,
            &[serde_json::json!({
                "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
                "parameter": "ParamInput0",
                "value": false
            })],
        );
        assert!(
            with_false_override.contains(&5),
            "explicit false paramInput override should hide ptr:5"
        );

        let with_override = instantiated_false_widgets_with_param_inputs(
            &rv,
            &[serde_json::json!({
                "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
                "parameter": "ParamInput0",
                "value": true
            })],
        );
        assert!(
            !with_override.contains(&5),
            "paramInput override true should show ptr:5"
        );
    }

    #[test]
    fn boolean_component_parameter_missing_override_without_default_stays_visible() {
        let rv = make_record_value(
            vec![],
            vec![serde_json::json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:5",
                "field": "IsActive",
                "input": {
                    "_Pointer_": "ptr:9",
                    "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                    "parameter": "ParamInput0"
                }
            })],
        );

        let no_override = instantiated_false_widgets_with_param_inputs(&rv, &[]);
        assert!(
            !no_override.contains(&5),
            "without paramInput override and no defaultValue, IsActive should remain visible"
        );
    }

    #[test]
    fn non_state_variables_without_static_values_do_not_hide_widgets() {
        let rv = make_record_value(
            vec![],
            vec![
                json!({
                    "_Pointer_": "ptr:12",
                    "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                    "binding": "CloneLocationInfo/UserOwnsLocation"
                }),
                json!({
                    "_Pointer_": "ptr:13",
                    "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                    "binding": "Bed/MedBed/MedBedStatus/CanRespawnHere"
                }),
                json!({
                    "_Pointer_": "ptr:8",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateAnd",
                    "inputs": ["_PointsTo_:ptr:12", "_PointsTo_:ptr:13"]
                }),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:7",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:8"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&7),
            "non-state sensor variables without static defaults should not hide ptr:7"
        );
    }
