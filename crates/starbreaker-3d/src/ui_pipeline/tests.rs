//! Tests for the ui_pipeline bridge module.

use crate::ui_pipeline::{
    authored_canvas_size, binding_target_size, datacore_ui_lookup_type_names,
};
use crate::ui_pipeline::p4k_fetchers::{p4k_swf_candidates, swf_immediate_subdirs};
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

#[test]
fn swf_immediate_subdirs_lists_ship_dirs_case_insensitively() {
    // P4K entry names are native (`\`-separated). Their casing is not guaranteed
    // to match the uppercase-brand prefix the resolver builds, so enumeration
    // must be case-insensitive (mirroring `fetch_swf_bytes`'s eq_ignore_ascii_case).
    let names = [
        r"Data\UI\ShipInterface\assets\SWF\DRA\DRAK_Buccaneer\AnnunciatorScreen\AnnunciatorHalve1.swf",
        r"Data\UI\ShipInterface\assets\SWF\DRA\DRAK_Buccaneer\AnnunciatorScreen\AnnunciatorHalve2.swf",
        r"Data\UI\ShipInterface\assets\SWF\DRA\DRAK_Dragonfly\Support_Bespoke_2\TargetStatus.swf",
        r"Data\UI\ShipInterface\assets\SWF\DRA\RadarScreen\Radar.swf",
        r"Data\UI\ShipInterface\assets\SWF\DRA\BrandLevelFile.swf", // direct file → not a subdir
        r"Data\UI\ShipInterface\assets\SWF\AEG\AEGS_Avenger\Foo.swf", // different brand
    ];
    // Prefix built by `p4k_ship_subdirs` (uppercase brand, trailing `\`),
    // deliberately lower-cased here to prove case-insensitive matching.
    let prefix = r"data\ui\shipinterface\assets\swf\dra\";
    let dirs = swf_immediate_subdirs(names.iter().copied(), prefix);
    assert_eq!(
        dirs,
        vec![
            "DRAK_Buccaneer".to_string(),
            "DRAK_Dragonfly".to_string(),
            "RadarScreen".to_string(),
        ],
        "must return deduped, sorted immediate subdirs, excluding brand-level files and other brands"
    );
}

#[test]
fn swf_immediate_subdirs_empty_when_no_match() {
    let names = [r"Data\UI\ShipInterface\assets\SWF\AEG\AEGS_Avenger\Foo.swf"];
    let dirs = swf_immediate_subdirs(names.iter().copied(), r"Data\UI\ShipInterface\assets\SWF\DRA\");
    assert!(dirs.is_empty(), "no DRA entries → empty");
}

#[test]
fn swf_immediate_subdirs_empty_prefix_returns_nothing() {
    // A degenerate empty prefix would otherwise match every entry's leading
    // path segment and enumerate the whole archive's top-level directories.
    let names = [
        r"Data\UI\ShipInterface\assets\SWF\DRA\Radar.swf",
        r"Engine\Fonts\Arial.swf",
    ];
    let dirs = swf_immediate_subdirs(names.iter().copied(), "");
    assert!(
        dirs.is_empty(),
        "empty prefix must enumerate nothing, not every top-level dir: {dirs:?}"
    );
}
