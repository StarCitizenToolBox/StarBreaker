//! Manifest schema for generic UI regression targets.
//!
//! Phase 2 introduces config-driven structural checks so adding new targets
//! does not require new hard-coded test logic.

use serde::{Deserialize, Serialize};

use crate::ui_snapshot::UiSnapshotTolerance;

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
                numeric_screen_floor_ratio: 0.001,
                rgba_channel_abs: 0.05,
            },
            Self::Gold => UiSnapshotTolerance {
                numeric_relative: 0.05,
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn tier_tolerances_match_phase2_contract() {
        let platinum = UiRegressionTier::Platinum.snapshot_tolerance();
        assert_eq!(platinum.numeric_relative, 0.01);
        assert_eq!(platinum.rgba_channel_abs, 0.05);

        let gold = UiRegressionTier::Gold.snapshot_tolerance();
        assert_eq!(gold.numeric_relative, 0.05);
        assert_eq!(gold.rgba_channel_abs, 0.10);
    }
}
