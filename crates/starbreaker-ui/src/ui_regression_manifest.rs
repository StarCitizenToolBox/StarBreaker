//! Manifest schema for generic UI regression targets.
//!
//! Phase 2 introduces config-driven structural checks so adding new targets
//! does not require new hard-coded test logic.

use serde::{Deserialize, Serialize};

use crate::ui_snapshot::{
    UiScreenSnapshot, UiSnapshotComparison, UiSnapshotElementCategory, UiSnapshotTolerance,
    compare_snapshots,
};

/// Manifest schema version for generic UI regression target lists.
pub const UI_REGRESSION_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// One target registered for snapshot/visual regression checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRegressionTarget {
    pub id: String,
    pub category: UiRegressionCategory,
    pub baseline_path: String,
    pub current_path: String,
    pub tier: UiRegressionTier,
    pub roi: Option<UiRegressionRoi>,
}

/// Target category used by generic regression policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRegressionCategory {
    Image,
    Shape,
    Text,
    Font,
}

/// Quality tier for drift tolerances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRegressionTier {
    Platinum,
    Gold,
}

impl UiRegressionTier {
    /// Convert policy tier to structural snapshot comparator tolerance.
    pub fn snapshot_tolerance(self) -> UiSnapshotTolerance {
        match self {
            Self::Platinum => UiSnapshotTolerance {
                numeric_relative: 0.01,
                font_size_relative: 0.05,
                numeric_screen_floor_ratio: 0.001,
                rgba_channel_abs: 0.05,
            },
            Self::Gold => UiSnapshotTolerance {
                numeric_relative: 0.05,
                font_size_relative: 0.10,
                numeric_screen_floor_ratio: 0.002,
                rgba_channel_abs: 0.10,
            },
        }
    }
}

/// Optional normalized region-of-interest used to scope checks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiRegressionRoi {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Root document for generic UI regression checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRegressionManifest {
    pub schema_version: u32,
    pub targets: Vec<UiRegressionTarget>,
}

impl UiRegressionManifest {
    pub fn from_json_str(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

/// Per-target result from a manifest-driven comparison run.
#[derive(Debug, Clone, PartialEq)]
pub struct UiRegressionTargetResult {
    pub id: String,
    pub comparison: UiSnapshotComparison,
}

/// Errors raised while loading baseline/current snapshots for a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiRegressionRunError {
    pub target_id: String,
    pub path: String,
    pub message: String,
}

/// Execute generic snapshot comparisons for every target in a manifest.
pub fn compare_manifest_targets_with_loader<F, E>(
    manifest: &UiRegressionManifest,
    mut snapshot_loader: F,
) -> Result<Vec<UiRegressionTargetResult>, UiRegressionRunError>
where
    F: FnMut(&str) -> Result<UiScreenSnapshot, E>,
    E: std::fmt::Display,
{
    let mut results = Vec::with_capacity(manifest.targets.len());
    for target in &manifest.targets {
        let baseline = snapshot_loader(&target.baseline_path).map_err(|error| UiRegressionRunError {
            target_id: target.id.clone(),
            path: target.baseline_path.clone(),
            message: error.to_string(),
        })?;
        let current = snapshot_loader(&target.current_path).map_err(|error| UiRegressionRunError {
            target_id: target.id.clone(),
            path: target.current_path.clone(),
            message: error.to_string(),
        })?;
        let mut comparison = compare_snapshots(&baseline, &current, target.tier.snapshot_tolerance());
        apply_target_category_checks(target, &baseline, &current, &mut comparison);
        results.push(UiRegressionTargetResult {
            id: target.id.clone(),
            comparison,
        });
    }
    Ok(results)
}

fn apply_target_category_checks(
    target: &UiRegressionTarget,
    baseline: &UiScreenSnapshot,
    current: &UiScreenSnapshot,
    comparison: &mut UiSnapshotComparison,
) {
    match target.category {
        UiRegressionCategory::Image => {
            require_category_present(
                target,
                baseline,
                current,
                UiSnapshotElementCategory::Image,
                comparison,
            );
        }
        UiRegressionCategory::Shape => {
            require_category_present(
                target,
                baseline,
                current,
                UiSnapshotElementCategory::Shape,
                comparison,
            );
        }
        UiRegressionCategory::Text => {
            require_category_present(
                target,
                baseline,
                current,
                UiSnapshotElementCategory::Text,
                comparison,
            );
        }
        UiRegressionCategory::Font => {
            require_category_present(
                target,
                baseline,
                current,
                UiSnapshotElementCategory::Text,
                comparison,
            );
            for baseline_element in baseline
                .elements
                .iter()
                .filter(|element| element.visible && element.category == UiSnapshotElementCategory::Text)
            {
                if let Some(current_element) = current
                    .elements
                    .iter()
                    .find(|element| element.visible && element.identity == baseline_element.identity)
                {
                    if baseline_element.text_font_identity != current_element.text_font_identity {
                        comparison.failures.push(format!(
                            "{}: font category identity drift baseline={:?} current={:?}",
                            baseline_element.identity,
                            baseline_element.text_font_identity,
                            current_element.text_font_identity
                        ));
                    }
                }
            }
        }
    }

    comparison.passed = comparison.failures.is_empty();
}

fn require_category_present(
    target: &UiRegressionTarget,
    baseline: &UiScreenSnapshot,
    current: &UiScreenSnapshot,
    category: UiSnapshotElementCategory,
    comparison: &mut UiSnapshotComparison,
) {
    let baseline_has = baseline
        .elements
        .iter()
        .any(|element| element.visible && element.category == category);
    let current_has = current
        .elements
        .iter()
        .any(|element| element.visible && element.category == category);
    if !baseline_has || !current_has {
        comparison.failures.push(format!(
            "{}: target category {:?} missing in baseline/current snapshot",
            target.id, category
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_snapshot::{UI_SNAPSHOT_SCHEMA_VERSION, UiSnapshotElement, UiSnapshotElementCategory};
    use std::collections::HashMap;

    #[test]
    fn manifest_parses_phase2_target_shape() {
        let raw = r#"{
            "schema_version": 1,
            "targets": [
                {
                    "id": "medical1",
                    "category": "image",
                    "baseline_path": "tests/fixtures/gold/medical1.snapshot.json",
                    "current_path": "target/ui/medical1.snapshot.json",
                    "tier": "platinum",
                    "roi": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 }
                }
            ]
        }"#;

        let manifest = UiRegressionManifest::from_json_str(raw).expect("manifest must parse");
        assert_eq!(manifest.schema_version, UI_REGRESSION_MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.targets.len(), 1);
        assert_eq!(manifest.targets[0].id, "medical1");
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
                id: "medical1".to_string(),
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
        assert_eq!(results[0].id, "medical1");
        assert!(results[0].comparison.passed);
    }

    #[test]
    fn comparator_runner_surfaces_loader_errors_with_target_context() {
        let manifest = UiRegressionManifest {
            schema_version: UI_REGRESSION_MANIFEST_SCHEMA_VERSION,
            targets: vec![UiRegressionTarget {
                id: "medical2".to_string(),
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

        assert_eq!(error.target_id, "medical2");
        assert_eq!(error.path, "missing");
        assert_eq!(error.message, "not found");
    }

    #[test]
    fn category_runner_requires_expected_image_elements() {
        let manifest = UiRegressionManifest {
            schema_version: UI_REGRESSION_MANIFEST_SCHEMA_VERSION,
            targets: vec![UiRegressionTarget {
                id: "medical-image".to_string(),
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
                id: "medical-font".to_string(),
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
}
