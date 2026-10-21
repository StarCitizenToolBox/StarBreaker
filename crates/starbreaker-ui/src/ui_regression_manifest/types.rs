//! UI regression manifest schema types.

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
    pub comparison: crate::ui_snapshot::UiSnapshotComparison,
}

/// Errors raised while loading baseline/current snapshots for a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiRegressionRunError {
    pub target_id: String,
    pub path: String,
    pub message: String,
}
