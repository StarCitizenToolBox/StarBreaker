use std::collections::HashMap;

use crate::ui_snapshot::{
    UI_SNAPSHOT_SCHEMA_VERSION, UiScreenSnapshot, UiSnapshotElement, UiSnapshotElementCategory,
};

use super::{
    UI_REGRESSION_MANIFEST_SCHEMA_VERSION, UiRegressionCategory, UiRegressionManifest,
    UiRegressionTarget, UiRegressionTier, compare_manifest_targets_with_loader,
};

#[test]
fn manifest_parses_phase2_target_shape() {
    let raw = r#"{
            "schema_version": 1,
            "targets": [
                {
                    "id": "ui_target_a",
                    "category": "image",
                    "baseline_path": "tests/fixtures/gold/ui_target_a.snapshot.json",
                    "current_path": "target/ui/ui_target_a.snapshot.json",
                    "tier": "platinum",
                    "roi": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 }
                }
            ]
        }"#;

    let manifest = UiRegressionManifest::from_json_str(raw).expect("manifest must parse");
    assert_eq!(manifest.schema_version, UI_REGRESSION_MANIFEST_SCHEMA_VERSION);
    assert_eq!(manifest.targets.len(), 1);
    assert_eq!(manifest.targets[0].id, "ui_target_a");
    assert_eq!(manifest.targets[0].category, UiRegressionCategory::Image);
    assert_eq!(manifest.targets[0].tier, UiRegressionTier::Platinum);
}

#[test]
fn tier_tolerances_match_phase3_contract() {
    let platinum = UiRegressionTier::Platinum.snapshot_tolerance();
    assert_eq!(platinum.numeric_relative, 0.01);
    assert_eq!(platinum.font_size_relative, 0.05);
    assert_eq!(platinum.rgba_channel_abs, 0.05);

    let gold = UiRegressionTier::Gold.snapshot_tolerance();
    assert_eq!(gold.numeric_relative, 0.05);
    assert_eq!(gold.font_size_relative, 0.10);
    assert_eq!(gold.rgba_channel_abs, 0.10);
}

#[test]
fn comparator_runner_executes_manifest_targets() {
    let manifest = UiRegressionManifest {
        schema_version: UI_REGRESSION_MANIFEST_SCHEMA_VERSION,
        targets: vec![UiRegressionTarget {
            id: "ui_target_a".to_string(),
            category: UiRegressionCategory::Text,
            baseline_path: "baseline".to_string(),
            current_path: "current".to_string(),
            tier: UiRegressionTier::Platinum,
            roi: None,
        }],
    };

    let mut snapshots = HashMap::new();
    snapshots.insert("baseline".to_string(), sample_snapshot(0.0));
    snapshots.insert("current".to_string(), sample_snapshot(0.0));

    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot: {path}"))
    })
    .expect("comparison should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "ui_target_a");
    assert!(results[0].comparison.passed);
}

#[test]
fn comparator_runner_surfaces_loader_errors_with_target_context() {
    let manifest = UiRegressionManifest {
        schema_version: UI_REGRESSION_MANIFEST_SCHEMA_VERSION,
        targets: vec![UiRegressionTarget {
            id: "ui_target_b".to_string(),
            category: UiRegressionCategory::Image,
            baseline_path: "missing".to_string(),
            current_path: "current".to_string(),
            tier: UiRegressionTier::Gold,
            roi: None,
        }],
    };

    let error = compare_manifest_targets_with_loader::<_, String>(&manifest, |_path| {
        Err("not found".to_string())
    })
    .expect_err("loader error should bubble up");

    assert_eq!(error.target_id, "ui_target_b");
    assert_eq!(error.path, "missing");
    assert_eq!(error.message, "not found");
}

#[test]
fn category_runner_requires_expected_image_elements() {
    let manifest = UiRegressionManifest {
        schema_version: UI_REGRESSION_MANIFEST_SCHEMA_VERSION,
        targets: vec![UiRegressionTarget {
            id: "sample-image".to_string(),
            category: UiRegressionCategory::Image,
            baseline_path: "baseline".to_string(),
            current_path: "current".to_string(),
            tier: UiRegressionTier::Platinum,
            roi: None,
        }],
    };

    let mut snapshots = HashMap::new();
    snapshots.insert("baseline".to_string(), sample_snapshot(0.0));
    snapshots.insert("current".to_string(), sample_snapshot(0.0));

    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot: {path}"))
    })
    .expect("comparison should succeed");

    assert!(!results[0].comparison.passed);
    assert!(results[0]
        .comparison
        .failures
        .iter()
        .any(|line| line.contains("target category Image missing")));
}

#[test]
fn font_category_runner_flags_font_identity_drift() {
    let manifest = UiRegressionManifest {
        schema_version: UI_REGRESSION_MANIFEST_SCHEMA_VERSION,
        targets: vec![UiRegressionTarget {
            id: "sample-font".to_string(),
            category: UiRegressionCategory::Font,
            baseline_path: "baseline".to_string(),
            current_path: "current".to_string(),
            tier: UiRegressionTier::Platinum,
            roi: None,
        }],
    };

    let mut baseline = sample_snapshot(0.0);
    let mut current = sample_snapshot(0.0);
    baseline.elements[0].text_font_identity = Some("font:regular".to_string());
    current.elements[0].text_font_identity = Some("font:bold".to_string());

    let mut snapshots = HashMap::new();
    snapshots.insert("baseline".to_string(), baseline);
    snapshots.insert("current".to_string(), current);

    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot: {path}"))
    })
    .expect("comparison should succeed");

    assert!(!results[0].comparison.passed);
    assert!(results[0]
        .comparison
        .failures
        .iter()
        .any(|line| line.contains("font category identity drift")));
}

fn sample_snapshot(x: f32) -> UiScreenSnapshot {
    UiScreenSnapshot {
        schema_version: UI_SNAPSHOT_SCHEMA_VERSION,
        canvas_guid: "canvas_guid".to_string(),
        canvas_name: Some("canvas_name".to_string()),
        target_width: 1920,
        target_height: 1080,
        elements: vec![UiSnapshotElement {
            identity: "1:textfield".to_string(),
            node_id: 1,
            category: UiSnapshotElementCategory::Text,
            draw_order_index: 0,
            node_type: "text_field".to_string(),
            visible: true,
            x,
            y: 0.0,
            w: 100.0,
            h: 20.0,
            alpha: 1.0,
            blend_mode: None,
            asset_identity: None,
            alignment: None,
            vertical_alignment: None,
            overflow_mode: None,
            background_rgba: None,
            stroke_rgba: None,
            text_rgba: None,
            icon_tint_rgba: None,
            stroke_extent: None,
            text_payload: Some("TEST".to_string()),
            text_font_identity: Some("font:baseline".to_string()),
            line_spacing: Some(18.0),
        }],
    }
}
