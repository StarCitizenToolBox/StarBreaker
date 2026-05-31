use serde_json::json;

use super::*;
use crate::canvas::Value;
use crate::defaults::DefaultValueRegistry;

fn resolver() -> BindingResolver {
    BindingResolver {
        widget_to_path: Default::default(),
        widget_to_loc_key: Default::default(),
        widget_to_input_ptrs: Default::default(),
        widget_field_to_input_ptrs: Default::default(),
        field_name_to_input_ptrs: Default::default(),
        ptr_to_op: Default::default(),
        ptr_to_path: Default::default(),
        widget_to_string: Default::default(),
    }
}

    #[test]
    fn text_at_key_resolves_via_loc_map() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("foo".to_string(), "POWER MANAGEMENT".to_string())].into());

        let raw = json!({"text": "@foo"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "POWER MANAGEMENT");
    }

    #[test]
    fn text_literal_returned_as_is() {
        let resolver = resolver();
        let defaults = DefaultValueRegistry::default();

        let raw = json!({"text": "Hello World"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "Hello World");
    }

    #[test]
    fn loc_string_field_resolved() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("mykey".to_string(), "My Label".to_string())].into());

        // `locString` field carries the loc key; `text` is absent.
        let raw = json!({"locString": "@mykey"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "My Label");
    }

    #[test]
    fn loc_string_field_respects_case_modifier() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("mykey".to_string(), "My Label".to_string())].into());

        let raw = json!({
            "locString": "@mykey",
            "labelProperties": {
                "caseModifier": "Upper"
            }
        });
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "MY LABEL");
    }

    #[test]
    fn top_level_case_modifier_applies_to_loc_string_field() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("mykey".to_string(), "My Label".to_string())].into());

        let raw = json!({
            "locString": "@mykey",
            "caseModifier": "Upper"
        });
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "MY LABEL");
    }

    #[test]
    fn label_properties_case_modifier_upper_applied() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("info_kiosks_logoscreen_001".to_string(), "Touch to start".to_string())].into());

        let raw = json!({
            "labelProperties": {
                "label": "@Info_Kiosks_LogoScreen_001",
                "caseModifier": "Upper"
            }
        });
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "TOUCH TO START");
    }

    #[test]
    fn loc_empty_sentinel_skipped() {
        let resolver = resolver();
        let defaults = DefaultValueRegistry::default();

        // @LOC_EMPTY resolves to "" (suppressed sentinel) — must not emit that.
        let raw = json!({"locString": "@LOC_EMPTY"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "");
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
