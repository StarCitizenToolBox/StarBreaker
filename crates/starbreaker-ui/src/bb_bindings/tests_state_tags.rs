//! State-tag binding resolution tests: `TagFromBoolean`,
//! `TagFromIntegerSwitch`, `IntegerFromBoolean` chains, and variable-driven
//! `PrimaryStateTag`/`SecondaryStateTag` resolution. Split from `tests.rs`
//! (line-cap); text/label resolution tests stay there.

use serde_json::json;

use super::*;
use crate::canvas::Value;
use crate::defaults::DefaultValueRegistry;

    /// `TagFromBoolean` carries each branch tag as a Tag record-reference OBJECT
    /// (`isTrue`/`isFalse` with `_RecordName_`/`_RecordId_`), not just an inline
    /// string. A `PrimaryStateTag` whose boolean is true must resolve to the
    /// `isTrue` tag (the footer's selected-screen state tag).
    #[test]
    fn tag_from_boolean_resolves_is_true_record_ref_object() {
        let resolver = BindingResolver::from_operations(&[
            json!({
                "_Type_": "BuildingBlocks_BindingsStringField",
                "widget": "ptr:5", "field": "PrimaryStateTag", "input": "ptr:11"
            }),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsTagFromBoolean",
                "isTrue": { "_RecordName_": "Tag.deadbeef-0000-0000-0000-000000000000",
                            "_RecordId_": "deadbeef-0000-0000-0000-000000000000" },
                "isFalse": null,
                "input": "ptr:12"
            }),
            json!({
                "_Pointer_": "ptr:12",
                "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                "parameter": "P", "defaultValue": true
            }),
        ]);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(
            resolver.resolve_field_text(5, "PrimaryStateTag", &defaults).as_deref(),
            Some("Tag.deadbeef-0000-0000-0000-000000000000"),
            "TagFromBoolean must resolve the isTrue record-ref object when enabled"
        );
    }

    /// When the boolean is false and `isFalse` is null, the tag resolves to none.
    #[test]
    fn tag_from_boolean_false_with_null_is_false_resolves_none() {
        let resolver = BindingResolver::from_operations(&[
            json!({
                "_Type_": "BuildingBlocks_BindingsStringField",
                "widget": "ptr:5", "field": "PrimaryStateTag", "input": "ptr:11"
            }),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsTagFromBoolean",
                "isTrue": { "_RecordName_": "Tag.deadbeef-0000-0000-0000-000000000000" },
                "isFalse": null,
                "input": "ptr:12"
            }),
            json!({
                "_Pointer_": "ptr:12",
                "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                "parameter": "P", "defaultValue": false
            }),
        ]);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(resolver.resolve_field_text(5, "PrimaryStateTag", &defaults), None);
    }

    /// `BooleanFromInteger` comparing two *unbound* `IntegerComponentParameter`s
    /// (both falling back to their `defaultValue`) is not a real runtime state:
    /// the footer's `bindingid(ParamInput0, -1) == selectedmfd(ParamInput1, -1)`
    /// gate is only true at runtime when this screen is the selected MFD, which it
    /// is not at rest. Comparing the two sentinel defaults (`-1 == -1`) would
    /// spuriously fire the selected-screen tag, so the evaluator must fall to the
    /// at-rest heuristic (`Equal` → false) and resolve no `isTrue` tag — leaving
    /// the unselected footer styling (dark box, orange text) the reference shows.
    #[test]
    fn tag_from_boolean_via_two_unbound_params_equal_resolves_none() {
        let resolver = BindingResolver::from_operations(&[
            json!({
                "_Type_": "BuildingBlocks_BindingsStringField",
                "widget": "ptr:5", "field": "PrimaryStateTag", "input": "ptr:11"
            }),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsTagFromBoolean",
                "isTrue": { "_RecordName_": "Tag.fef243b7-0000-0000-0000-000000000000" },
                "isFalse": null,
                "input": "ptr:12"
            }),
            json!({
                "_Pointer_": "ptr:12",
                "_Type_": "BuildingBlocks_BindingsBooleanFromInteger",
                "type": "Equal", "value": 0,
                "inputL": "ptr:13", "inputR": "ptr:14"
            }),
            json!({ "_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                    "parameter": "ParamInput0", "defaultValue": -1 }),
            json!({ "_Pointer_": "ptr:14", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                    "parameter": "ParamInput1", "defaultValue": -1 }),
        ]);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(
            resolver.resolve_field_text(5, "PrimaryStateTag", &defaults),
            None,
            "two unbound-param sentinels (-1 == -1) must NOT fire the selected tag"
        );
    }

    /// A `BooleanFromInteger` comparing one unbound parameter against an *authored
    /// literal* (`value`/`inputR` constant) is a real comparison the designer
    /// wrote, so it must evaluate literally — not fall to the heuristic. Here a
    /// parameter defaulting to 5 `Equal` the authored literal 5 is genuinely true.
    #[test]
    fn boolean_from_integer_param_equals_authored_literal_uses_real_comparison() {
        let resolver = BindingResolver::from_operations(&[
            json!({
                "_Type_": "BuildingBlocks_BindingsStringField",
                "widget": "ptr:5", "field": "PrimaryStateTag", "input": "ptr:11"
            }),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsTagFromBoolean",
                "isTrue": { "_RecordName_": "Tag.abcd1234-0000-0000-0000-000000000000" },
                "isFalse": null,
                "input": "ptr:12"
            }),
            json!({
                "_Pointer_": "ptr:12",
                "_Type_": "BuildingBlocks_BindingsBooleanFromInteger",
                "type": "Equal", "value": 5, "inputR": null,
                "inputL": "ptr:13"
            }),
            json!({ "_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                    "parameter": "ParamInput0", "defaultValue": 5 }),
        ]);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(
            resolver.resolve_field_text(5, "PrimaryStateTag", &defaults).as_deref(),
            Some("Tag.abcd1234-0000-0000-0000-000000000000"),
            "param(5) Equal authored-literal 5 must be a real comparison → true"
        );
    }

    #[test]
    fn integer_component_parameter_override_cycle_terminates() {
        // A multi-hop `IntegerComponentParameter` override cycle (ParamA→ptr:2,
        // ParamB→ptr:1) must NOT recurse forever. The per-call `input == current`
        // check only catches direct self-refs; the `seen` guard breaks longer
        // cycles. Observed overflowing the stack while exporting the Drake Clipper's
        // power MFD. With the cycle broken, ptr:1's override resolves ptr:2, whose
        // override loops back to ptr:1 (now seen) → ptr:2 falls to its default 9.
        let resolver = BindingResolver::from_operations(&[
            json!({ "_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                    "parameter": "ParamA", "defaultValue": 7 }),
            json!({ "_Type_": "BuildingBlocks_BindingsIntegerField",
                    "widget": "ptr:100", "field": "ParamA", "input": "ptr:2" }),
            json!({ "_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                    "parameter": "ParamB", "defaultValue": 9 }),
            json!({ "_Type_": "BuildingBlocks_BindingsIntegerField",
                    "widget": "ptr:101", "field": "ParamB", "input": "ptr:1" }),
        ]);
        let defaults = DefaultValueRegistry::default();
        let mut seen = std::collections::HashSet::new();
        assert_eq!(resolver.eval_integer_ptr(1, &defaults, &mut seen), Some(9));
    }

    #[test]
    fn synth_string_widget_ptr_string_maps_to_resolved_string() {
        let resolver = BindingResolver::from_operations(&[json!({
            "_Type_": "_SynthStringWidget_",
            "widget": "ptr:4",
            "resolvedString": "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_menuoption_a.tif"
        })]);
        assert_eq!(
            resolver.resolve_string_binding(4),
            Some("UI/Textures/I_InteractiveScreens/Med/i_med_bioc_menuoption_a.tif")
        );
    }

    #[test]
    fn integer_component_parameter_uses_field_override_before_default() {
        let resolver = BindingResolver::from_operations(&[
            json!({
                "_Pointer_": "ptr:1",
                "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                "parameter": "ParamInput0",
                "defaultValue": 0
            }),
            json!({
                "_Pointer_": "ptr:2",
                "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                "binding": "/AnnunciatorProvider/Issues/Issue1/Severity"
            }),
            json!({
                "_Type_": "BuildingBlocks_BindingsIntegerField",
                "widget": "ptr:100",
                "field": "ParamInput0",
                "input": "ptr:2"
            }),
            json!({
                "_Pointer_": "ptr:3",
                "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                "values": [
                    {
                        "first": 1,
                        "second": {
                            "_RecordName_": "Tag.SeverityLow"
                        }
                    },
                    {
                        "first": 2,
                        "second": {
                            "_RecordName_": "Tag.SeverityMed"
                        }
                    }
                ],
                "defaultValue": {
                    "_RecordName_": "Tag.None"
                },
                "input": "ptr:1"
            }),
            json!({
                "_Type_": "BuildingBlocks_BindingsStringField",
                "widget": "ptr:200",
                "field": "PrimaryStateTag",
                "input": "ptr:3"
            }),
        ]);

        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path(
            "/AnnunciatorProvider/Issues/Issue1/Severity",
            Value::Int(2),
        );

        let tag = resolver
            .resolve_field_text(200, "PrimaryStateTag", &defaults)
            .expect("state tag should resolve from variable override");
        assert_eq!(tag, "Tag.SeverityMed");
    }

    /// `IntegerFromBoolean` over at-rest boolean variables selects `isFalse`.
    /// Mirrors the medical header's screen-state encoder: three
    /// `IntegerFromBoolean` ops over unbound `state.BaseScreens.*` variables
    /// sum to 0, and the `TagFromIntegerSwitch` maps 0 onto the
    /// hide-back-button state tag. An unresolved boolean input is the
    /// variable's engine type default (`false`), not "no value".
    #[test]
    fn integer_from_boolean_at_rest_chain_resolves_state_tag() {
        let ops = vec![
            json!({
                "_Type_": "BuildingBlocks_BindingsStringField",
                "widget": "ptr:4", "field": "SecondaryStateTag", "input": "ptr:19"
            }),
            json!({
                "_Pointer_": "ptr:19",
                "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                "values": [
                    {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 0,
                     "second": {"_RecordName_": "Tag.2fcd20ff-0138-44a5-a0fe-8f429f0aa8f6"}}
                ],
                "defaultValue": null,
                "input": "_PointsTo_:ptr:26"
            }),
            json!({
                "_Pointer_": "ptr:26",
                "_Type_": "BuildingBlocks_BindingsIntegerArithmatic",
                "type": "Add", "amount": 1,
                "inputL": "_PointsTo_:ptr:29", "inputR": "_PointsTo_:ptr:30"
            }),
            json!({
                "_Pointer_": "ptr:29",
                "_Type_": "BuildingBlocks_BindingsIntegerFromBoolean",
                "isTrue": 1, "isFalse": 0, "inputTrue": null, "inputFalse": null,
                "input": "_PointsTo_:ptr:31"
            }),
            json!({
                "_Pointer_": "ptr:30",
                "_Type_": "BuildingBlocks_BindingsIntegerFromBoolean",
                "isTrue": 3, "isFalse": 0, "inputTrue": null, "inputFalse": null,
                "input": "_PointsTo_:ptr:32"
            }),
            json!({
                "_Pointer_": "ptr:31",
                "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                "path": [], "binding": "state.BaseScreens.Departures"
            }),
            json!({
                "_Pointer_": "ptr:32",
                "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                "path": [], "binding": "state.BaseScreens.Admin"
            }),
        ];
        let resolver = BindingResolver::from_operations(&ops);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(
            resolver.resolve_field_text(4, "SecondaryStateTag", &defaults).as_deref(),
            Some("Tag.2fcd20ff-0138-44a5-a0fe-8f429f0aa8f6"),
            "at-rest boolean variables must encode integer 0 and select the 0-pair tag"
        );

        // A bound-true variable selects `isTrue` and the sum leaves the
        // 0-pair, so no tag resolves (the switch has no default).
        let mut bound = DefaultValueRegistry::default();
        bound.insert_path("state.BaseScreens.Admin", Value::Bool(true));
        let resolver_bound = BindingResolver::from_operations(&ops);
        assert_eq!(
            resolver_bound.resolve_field_text(4, "SecondaryStateTag", &bound),
            None,
            "an active screen state must leave the hide tag unset"
        );
    }

    /// `TagFromIntegerSwitch` selects a tag by integer input — the button
    /// component standard's `FillStyle` wiring (0 → fill-style-filled,
    /// 1 → fill-style-ghost) resolves the at-rest fill-style state tag.
    #[test]
    fn tag_from_integer_switch_selects_tag_for_resolved_value() {
        let ops = vec![
            json!({
                "_Type_": "BuildingBlocks_BindingsStringField",
                "widget": "ptr:5", "field": "PrimaryStateTag", "input": "ptr:11"
            }),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                "values": [
                    {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 0,
                     "second": {"_RecordName_": "Tag.308bf0ed-0000-0000-0000-000000000000"}},
                    {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 1,
                     "second": {"_RecordName_": "Tag.7f42e80d-0000-0000-0000-000000000000"}}
                ],
                "input": "ptr:12"
            }),
            json!({ "_Pointer_": "ptr:12", "_Type_": "_SynthIntegerParam_", "resolvedInt": 1 }),
        ];
        let resolver = BindingResolver::from_operations(&ops);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(
            resolver.resolve_field_text(5, "PrimaryStateTag", &defaults).as_deref(),
            Some("Tag.7f42e80d-0000-0000-0000-000000000000"),
            "switch must select the pair matching the resolved integer"
        );

        let ops0: Vec<serde_json::Value> = ops
            .iter()
            .map(|op| {
                let mut op = op.clone();
                if op.get("_Pointer_").and_then(|v| v.as_str()) == Some("ptr:12") {
                    op["resolvedInt"] = json!(0);
                }
                op
            })
            .collect();
        let resolver0 = BindingResolver::from_operations(&ops0);
        assert_eq!(
            resolver0.resolve_field_text(5, "PrimaryStateTag", &defaults).as_deref(),
            Some("Tag.308bf0ed-0000-0000-0000-000000000000"),
        );
    }

    /// A component parameter WIRED to an engine variable that is unbound at
    /// rest resolves to the variable's type default, not the authored editor
    /// `defaultValue`. The power screen's notification overlays
    /// (`engineeringoverride`/`presetnotification`, authored default `true`)
    /// are bound from unbound `BooleanVariable`s and must be hidden at rest.
    #[test]
    fn wired_unbound_variable_param_resolves_type_default_not_authored() {
        let ops = vec![
            // gen canvas: overlay IsActive <- engineeringoverride param.
            json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "ptr:4", "field": "IsActive", "input": "_PointsTo_:ptr:5"
            }),
            json!({
                "_Pointer_": "ptr:5",
                "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                "name": "engineeringoverride", "parameter": "ParamInput3", "defaultValue": true
            }),
            // master canvas: ParamInput3 <- BooleanVariable "engineeringoverride".
            json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "ptr:1", "field": "ParamInput3", "input": "_PointsTo_:ptr:9"
            }),
            json!({
                "_Pointer_": "ptr:9",
                "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                "path": [], "binding": "engineeringoverride"
            }),
        ];
        let resolver = BindingResolver::from_operations(&ops);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(
            resolver.resolve_field_bool(4, "IsActive", &defaults),
            Some(false),
            "wired-to-unbound-variable must be the type default (false), not authored true"
        );

        // A bound variable still wins.
        let mut bound = DefaultValueRegistry::default();
        bound.insert_path("engineeringoverride", Value::Bool(true));
        assert_eq!(
            resolver.resolve_field_bool(4, "IsActive", &bound),
            Some(true),
            "a bound variable value must resolve normally"
        );
    }

    /// An UNWIRED parameter (no host ParamInput ops at all) keeps the authored
    /// `defaultValue` — editor semantics (e.g. `iscast` defaulting true).
    #[test]
    fn unwired_param_keeps_authored_default() {
        let ops = vec![
            json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "ptr:4", "field": "IsActive", "input": "_PointsTo_:ptr:5"
            }),
            json!({
                "_Pointer_": "ptr:5",
                "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                "name": "iscast", "parameter": "ParamInput0", "defaultValue": true
            }),
        ];
        let resolver = BindingResolver::from_operations(&ops);
        let defaults = DefaultValueRegistry::default();
        assert_eq!(
            resolver.resolve_field_bool(4, "IsActive", &defaults),
            Some(true),
            "no wiring at all keeps the authored default"
        );
    }

    /// Authored `staticVariables[]` are the engine's per-canvas at-rest
    /// values; the resolver consults them (after the defaults registry) when
    /// a `Bindings*Variable` is otherwise unbound. The power screen's
    /// `_pipHeight`/`_pipSpacing` (0.0667) and `PresetNotification=false`
    /// flow this way. They are carried as `_SynthStaticVariable_` ops so
    /// child-canvas merges inherit them; the FIRST occurrence (outermost
    /// canvas) wins.
    #[test]
    fn static_variables_back_unbound_variable_evaluation() {
        let ops = vec![
            json!({"_Type_": "_SynthStaticVariable_", "name": "_pipHeight", "value": 0.06666667}),
            json!({"_Type_": "_SynthStaticVariable_", "name": "PresetNotification", "value": false}),
            json!({
                "_Type_": "BuildingBlocks_BindingsNumberField",
                "widget": "ptr:8", "field": "ParamInput1", "input": "_PointsTo_:ptr:5"
            }),
            json!({
                "_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                "path": [], "binding": "_pipHeight"
            }),
            json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "ptr:6", "field": "Instantiated", "input": "_PointsTo_:ptr:7"
            }),
            json!({
                "_Pointer_": "ptr:7", "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                "path": [], "binding": "PresetNotification"
            }),
        ];
        let resolver = BindingResolver::from_operations(&ops);
        let defaults = DefaultValueRegistry::default();
        let height = resolver
            .resolve_field_number(8, "ParamInput1", &defaults)
            .expect("static number must resolve");
        assert!((height - 0.06666667).abs() < 1e-6, "got {height}");
        assert_eq!(
            resolver.resolve_field_bool(6, "Instantiated", &defaults),
            Some(false),
            "static boolean must resolve"
        );

        // The defaults registry still wins over the authored static.
        let mut bound = DefaultValueRegistry::default();
        bound.insert_path("PresetNotification", Value::Bool(true));
        assert_eq!(
            resolver.resolve_field_bool(6, "Instantiated", &bound),
            Some(true),
            "a bound engine value overrides the authored static"
        );
    }
