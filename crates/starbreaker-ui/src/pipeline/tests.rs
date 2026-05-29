use super::*;

#[test]
fn pipeline_defaults_seeds_placeholder_keys_and_merges_localization() {
    let defaults = DefaultValueRegistry::with_pipeline_defaults(Some(std::collections::HashMap::from([(
        "hud_custom".to_string(),
        "CUSTOM".to_string(),
    )])));

    assert_eq!(defaults.lookup_localization("@hud_custom"), Some("CUSTOM"));
    assert_eq!(defaults.lookup_localization("@loc_placeholder"), Some(""));
    assert_eq!(defaults.lookup_localization("@loc_empty"), Some(""));
}

#[test]
fn fallback_counter_warnings_emit_human_readable_messages() {
    let warnings = fallback_counter_warnings([
        ("swf_candidate_miss", 1),
        ("manufacturer_style_fallback_drak", 2),
        ("ignored_zero", 0),
    ]);

    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0], "fallback path used: swf_candidate_miss=1");
    assert_eq!(warnings[1], "fallback path used: manufacturer_style_fallback_drak=2");
}
