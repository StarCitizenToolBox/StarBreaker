//! Manifest comparison runner.

use crate::ui_snapshot::{
    UiScreenSnapshot, UiSnapshotComparison, UiSnapshotElementCategory, compare_snapshots,
};

use super::types::{
    UiRegressionCategory, UiRegressionManifest, UiRegressionRunError, UiRegressionTarget,
    UiRegressionTargetResult,
};

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
        let baseline =
            snapshot_loader(&target.baseline_path).map_err(|error| UiRegressionRunError {
                target_id: target.id.clone(),
                path: target.baseline_path.clone(),
                message: error.to_string(),
            })?;
        let current = snapshot_loader(&target.current_path).map_err(|error| UiRegressionRunError {
            target_id: target.id.clone(),
            path: target.current_path.clone(),
            message: error.to_string(),
        })?;
        let mut comparison =
            compare_snapshots(&baseline, &current, target.tier.snapshot_tolerance());
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
