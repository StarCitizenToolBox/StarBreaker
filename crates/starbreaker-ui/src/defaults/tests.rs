use std::collections::HashMap;

use crate::canvas::{RgbaColor, Value};

use super::DefaultValueRegistry;

#[test]
fn well_known_defaults_has_at_least_nine_entries() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert!(
        reg.path_count() >= 9,
        "expected >=9 well-known path defaults, got {}",
        reg.path_count()
    );
}

#[test]
fn well_known_defaults_targetname() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(
        reg.lookup_path("/vehicle/targetname"),
        Some(&Value::Str("NO TARGET".into()))
    );
}

#[test]
fn well_known_defaults_target_distance() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(
        reg.lookup_path("/vehicle/target/distance"),
        Some(&Value::Str("0.0km".into()))
    );
}

#[test]
fn well_known_defaults_target_bearing() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(
        reg.lookup_path("/vehicle/target/bearing"),
        Some(&Value::Str("0°".into()))
    );
}

#[test]
fn well_known_defaults_hp() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(reg.lookup_path("/ship/hp/current"), Some(&Value::Str("MAX".into())));
    assert_eq!(reg.lookup_path("/ship/hp/max"), Some(&Value::Str("MAX".into())));
}

#[test]
fn well_known_defaults_power() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(
        reg.lookup_path("/seatdashboard/powerstate"),
        Some(&Value::Str("OFFLINE".into()))
    );
    assert_eq!(reg.lookup_path("/seatdashboard/powercurrent"), Some(&Value::Int(2)));
    assert_eq!(reg.lookup_path("/seatdashboard/powermax"), Some(&Value::Int(16)));
}

#[test]
fn well_known_defaults_gungroup() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(
        reg.lookup_path("/vehicle/gungroup"),
        Some(&Value::Str("(ALL)".into()))
    );
}

#[test]
fn well_known_defaults_unknown_path_returns_none() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(reg.lookup_path("/nonexistent/binding"), None);
}

#[test]
fn mfd_slot_roundtrip() {
    let mut reg = DefaultValueRegistry::new();
    reg.insert_mfd_slot(4, 6, "3228e5cc-d0b7-49f7-9765-20cbad1b61b0");
    assert_eq!(
        reg.lookup_mfd_slot(4, 6),
        Some("3228e5cc-d0b7-49f7-9765-20cbad1b61b0")
    );
    assert_eq!(reg.lookup_mfd_slot(4, 7), None);
}

#[test]
fn ingest_screen_states_parses_normal_state() {
    let fixture = serde_json::json!([
        {
            "name": "Normal",
            "lightOn": true,
            "color": { "r": 102, "g": 214, "b": 255, "a": 255 },
            "intensity": 0.025
        }
    ]);
    let mut reg = DefaultValueRegistry::new();
    reg.ingest_screen_states(&fixture);
    assert_eq!(
        reg.screen_state_color("Normal"),
        Some(RgbaColor {
            r: 102,
            g: 214,
            b: 255,
            a: 255
        })
    );
    assert_eq!(reg.screen_state_color("Unknown"), None);
}

#[test]
fn ingest_screen_states_skips_malformed_entries() {
    let fixture = serde_json::json!([
        { "name": "Broken" },
        { "name": "Good", "color": { "r": 10, "g": 20, "b": 30, "a": 255 } }
    ]);
    let mut reg = DefaultValueRegistry::new();
    reg.ingest_screen_states(&fixture);
    assert_eq!(reg.screen_state_color("Broken"), None);
    assert_eq!(
        reg.screen_state_color("Good"),
        Some(RgbaColor {
            r: 10,
            g: 20,
            b: 30,
            a: 255
        })
    );
}

#[test]
fn ingest_screen_states_non_array_is_no_op() {
    let mut reg = DefaultValueRegistry::new();
    reg.ingest_screen_states(&serde_json::json!("not an array"));
    assert_eq!(reg.path_count(), 0);
}

#[test]
fn localization_lookup_with_at_prefix() {
    let mut reg = DefaultValueRegistry::new();
    let mut map = HashMap::new();
    map.insert("hud_notarget".to_owned(), "NO TARGET".to_owned());
    map.insert("dfm_ui_target".to_owned(), "TARGET".to_owned());
    reg.set_localization(map);

    assert_eq!(reg.lookup_localization("@hud_NoTarget"), Some("NO TARGET"));
    assert_eq!(reg.lookup_localization("@hud_notarget"), Some("NO TARGET"));
    assert_eq!(reg.lookup_localization("dfm_ui_target"), Some("TARGET"));
    assert_eq!(reg.lookup_localization("@nonexistent"), None);
}

#[test]
fn localization_lookup_empty_table_returns_none() {
    let reg = DefaultValueRegistry::new();
    assert_eq!(reg.lookup_localization("@hud_NoTarget"), None);
}

#[test]
fn well_known_defaults_has_localization_fallbacks() {
    let reg = DefaultValueRegistry::with_well_known_path_defaults();
    assert_eq!(reg.lookup_localization("@hud_NoTarget"), Some("NO TARGET"));
    assert_eq!(reg.lookup_localization("@hud_GimbalMode"), Some("GIMBAL MODE"));
    assert_eq!(reg.lookup_localization("@hud_ActiveGroup"), Some("ACTIVE GROUP"));
    assert_eq!(reg.lookup_localization("@LOC_EMPTY"), Some(""));
    assert_eq!(reg.lookup_localization("@nonexistent"), None);
}

#[test]
fn pipeline_defaults_merge_localization_without_losing_sentinels() {
    let reg = DefaultValueRegistry::with_pipeline_defaults(Some(HashMap::from([
        ("hud_custom".to_string(), "CUSTOM".to_string()),
        ("loc_placeholder".to_string(), "<= PLACEHOLDER =>".to_string()),
        ("loc_empty".to_string(), "SHOULD_NOT_OVERRIDE".to_string()),
    ])));

    assert_eq!(reg.lookup_localization("@hud_custom"), Some("CUSTOM"));
    assert_eq!(reg.lookup_localization("@LOC_PLACEHOLDER"), Some(""));
    assert_eq!(reg.lookup_localization("@LOC_EMPTY"), Some(""));
}
