// UI snapshot schema, extraction, and drift comparison.
//
// This module provides a deterministic, metadata-first snapshot representation
// derived from [`crate::ui_ir::UiIrDocument`]. It is intended for regression
// protection where structural UI stability matters more than pixel identity.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ui_ir::{UiIrColourBlendMode, UiIrDocument, UiIrNode};

/// Snapshot schema version for structural UI snapshots.
///
/// v2 adds the rendered glyph cap-top/left of a node's primary (and, for
/// caption pairs, secondary) text band (`primary_text_top` etc.). These capture
/// the *visible* text position — directly comparable to a measured in-game
/// reference — so known-outlier overrides can be anchored on what the eye sees,
/// not just the container rect.
pub const UI_SNAPSHOT_SCHEMA_VERSION: u32 = 2;

/// Category tracked by the Phase 7 structural snapshot checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiSnapshotElementCategory {
    Text,
    Image,
    Shape,
}

/// One visible element captured in the snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSnapshotElement {
    pub identity: String,
    pub node_id: u32,
    pub category: UiSnapshotElementCategory,
    pub draw_order_index: u32,
    pub node_type: String,
    pub visible: bool,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub alpha: f32,
    pub blend_mode: Option<String>,
    pub asset_identity: Option<String>,
    pub alignment: Option<String>,
    pub vertical_alignment: Option<String>,
    pub overflow_mode: Option<String>,
    pub background_rgba: Option<[f32; 4]>,
    pub background_tint_token: Option<String>,
    pub stroke_rgba: Option<[f32; 4]>,
    pub stroke_tint_token: Option<String>,
    pub text_rgba: Option<[f32; 4]>,
    pub text_tint_token: Option<String>,
    pub icon_tint_rgba: Option<[f32; 4]>,
    pub icon_tint_token: Option<String>,
    pub stroke_extent: Option<f32>,
    pub text_payload: Option<String>,
    pub text_font_identity: Option<String>,
    pub text_font_size: Option<f32>,
    pub line_spacing: Option<f32>,
    /// Rendered cap-top / left edge of the primary text band's drawn glyphs
    /// (screen px). `None` for non-text nodes. Directly comparable to a measured
    /// in-game reference cap-top — the anchor for visible-position outliers.
    #[serde(default)]
    pub primary_text_top: Option<f32>,
    #[serde(default)]
    pub primary_text_left: Option<f32>,
    /// Rendered cap-top / left of the secondary (caption) band, for
    /// `BuildingBlocks_ComponentLabelCaptionPair` nodes; `None` otherwise.
    #[serde(default)]
    pub secondary_text_top: Option<f32>,
    #[serde(default)]
    pub secondary_text_left: Option<f32>,
}

/// Root snapshot document for one UI screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiScreenSnapshot {
    pub schema_version: u32,
    pub canvas_guid: String,
    pub canvas_name: Option<String>,
    pub target_width: u32,
    pub target_height: u32,
    pub elements: Vec<UiSnapshotElement>,
}

/// Comparator tolerances for structural drift checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiSnapshotTolerance {
    pub numeric_relative: f32,
    pub font_size_relative: f32,
    pub numeric_screen_floor_ratio: f32,
    pub rgba_channel_abs: f32,
}

impl Default for UiSnapshotTolerance {
    fn default() -> Self {
        Self {
            numeric_relative: 0.10,
            font_size_relative: 0.10,
            numeric_screen_floor_ratio: 0.002,
            rgba_channel_abs: 0.10,
        }
    }
}

/// Drift comparison result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSnapshotComparison {
    pub passed: bool,
    pub failures: Vec<String>,
    /// Positive-reinforcement notes: a known-outlier field moved *closer* to its
    /// in-game reference target than the frozen baseline. Never fails the run;
    /// signals "this is better than frozen — consider re-freezing".
    pub improvements: Vec<String>,
}

/// Schema version for the known-outlier registry.
pub const UI_KNOWN_OUTLIER_SCHEMA_VERSION: u32 = 1;

/// A reference target value for a known-outlier field. The variant is chosen by
/// the captured field's type; `#[serde(untagged)]` lets the registry JSON write
/// a bare number, a 4-element RGBA array, or a string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UiOutlierValue {
    Number(f32),
    Rgba([f32; 4]),
    Text(String),
}

/// One known-outlier override.
///
/// Records that the frozen value of `(identity, field)` is a *known* miss versus
/// the in-game reference. The comparator then treats this field one-sided:
/// movement toward `reference_target` is an improvement (passes + positive
/// note), movement away is a regression (fails). `frozen_value` records the
/// value at registration time and is validated against the live baseline to
/// catch a stale entry. The mechanism is field-generic — any captured numeric,
/// RGBA, or discrete field can be registered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiKnownOutlier {
    /// Manifest target id this entry applies to (e.g. `ui_target_b`).
    pub target: String,
    /// `UiSnapshotElement.identity` of the element.
    pub identity: String,
    /// Captured field name: `x`, `y`, `w`, `h`, `alpha`, `primary_text_top`,
    /// `secondary_text_top`, `text_rgba`, `text_font_size`, ... .
    pub field: String,
    /// Value at registration time (validated against the live baseline).
    pub frozen_value: UiOutlierValue,
    /// Measured in-game reference target — the direction of "better".
    pub reference_target: UiOutlierValue,
    /// Graduation tolerance (px / unit / channel). Within this of the target,
    /// the field is ready to graduate to a strict freeze.
    #[serde(default)]
    pub confidence: f32,
    /// Why this is a known outlier (root cause / tracking note).
    #[serde(default)]
    pub reason: String,
    /// Provenance: which reference image and how the target was measured.
    #[serde(default)]
    pub source: String,
}

/// Root document for the committed known-outlier registry (numbers only — no
/// reference image is required at test time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiKnownOutlierRegistry {
    pub schema_version: u32,
    pub outliers: Vec<UiKnownOutlier>,
}

impl UiKnownOutlierRegistry {
    pub fn from_json_str(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }

    /// Borrow the outliers registered for one manifest target.
    pub fn for_target<'a>(&'a self, target_id: &str) -> Vec<&'a UiKnownOutlier> {
        self.outliers
            .iter()
            .filter(|outlier| outlier.target == target_id)
            .collect()
    }
}

