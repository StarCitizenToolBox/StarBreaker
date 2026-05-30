//! Tests for the ui_pipeline bridge module.

use crate::ui_pipeline::{
    authored_canvas_size, binding_target_size, datacore_ui_lookup_type_names, p4k_swf_candidates,
};
use serde_json::json;

#[test]
fn authored_canvas_size_reads_record_value_size() {
    let canvas = json!({
        "_RecordValue_": {
            "size": { "x": 1920.0, "y": 1080.0, "z": 0.0 }
        }
    });
    assert_eq!(authored_canvas_size(&canvas), Some((1920, 1080)));
}

#[test]
fn authored_canvas_size_ignores_invalid_size() {
    let canvas = json!({
        "_RecordValue_": {
            "size": { "x": 0.0, "y": 1080.0, "z": 0.0 }
        }
    });
    assert_eq!(authored_canvas_size(&canvas), None);
}

#[test]
fn non_mfd_prefers_authored_canvas_size() {
    assert_eq!(binding_target_size("physical", Some((1920, 1080))), (1920, 1080));
    assert_eq!(binding_target_size("physical", None), (2048, 1024));
}

#[test]
fn mfd_and_radar_keep_fixed_sizes() {
    assert_eq!(binding_target_size("mfd", Some((1920, 1080))), (1600, 900));
    assert_eq!(binding_target_size("radar", Some((1920, 1080))), (1024, 1024));
}

#[test]
fn live_ui_lookup_includes_referenced_ui_support_records() {
    assert!(datacore_ui_lookup_type_names().contains(&"TagDatabase"));
    assert!(datacore_ui_lookup_type_names().contains(&"BuildingBlocks_Timeline"));
}

#[test]
fn swf_candidates_normalize_forward_slashes_to_p4k_paths() {
    assert_eq!(
        p4k_swf_candidates("Data/UI/fonts/Shared/fonts_en.gfx"),
        vec!["Data\\UI\\fonts\\Shared\\fonts_en.gfx".to_string()]
    );
}

#[test]
fn swf_candidates_add_data_prefix_for_relative_paths() {
    assert_eq!(
        p4k_swf_candidates("UI/BuildingBlocks/assets/SWF/Canvas.swf"),
        vec![
            "UI\\BuildingBlocks\\assets\\SWF\\Canvas.swf".to_string(),
            "Data\\UI\\BuildingBlocks\\assets\\SWF\\Canvas.swf".to_string(),
        ]
    );
}
