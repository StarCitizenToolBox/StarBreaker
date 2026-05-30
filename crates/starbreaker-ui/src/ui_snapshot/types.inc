// UI snapshot schema, extraction, and drift comparison.
//
// This module provides a deterministic, metadata-first snapshot representation
// derived from [`crate::ui_ir::UiIrDocument`]. It is intended for regression
// protection where structural UI stability matters more than pixel identity.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ui_ir::{UiIrColourBlendMode, UiIrDocument, UiIrNode};

/// Snapshot schema version for structural UI snapshots.
pub const UI_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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
    pub stroke_rgba: Option<[f32; 4]>,
    pub text_rgba: Option<[f32; 4]>,
    pub icon_tint_rgba: Option<[f32; 4]>,
    pub stroke_extent: Option<f32>,
    pub text_payload: Option<String>,
    pub text_font_identity: Option<String>,
    pub line_spacing: Option<f32>,
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
}

