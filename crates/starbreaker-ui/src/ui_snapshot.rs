//! Medical UI snapshot schema, extraction, and drift comparison.
//!
//! This module provides a deterministic, metadata-first snapshot representation
//! derived from [`crate::ui_ir::UiIrDocument`]. It is intended for regression
//! protection where structural UI stability matters more than pixel identity.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ui_ir::{UiIrColourBlendMode, UiIrDocument, UiIrNode};

/// Snapshot schema version for medical structural snapshots.
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

/// Extract a deterministic structural snapshot from canonical UI IR.
pub fn snapshot_from_ui_ir(document: &UiIrDocument) -> UiScreenSnapshot {
    let mut draw_nodes: Vec<&UiIrNode> = document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .collect();
    draw_nodes.sort_by_key(|node| (node.layer, node.id));

    let mut elements = Vec::new();
    for (draw_order_index, node) in draw_nodes.into_iter().enumerate() {
        let Some(category) = classify_node(node) else {
            continue;
        };
        elements.push(UiSnapshotElement {
            identity: format!("{}:{}", node.id, node.node_type),
            node_id: node.id,
            category,
            draw_order_index: draw_order_index as u32,
            node_type: node.node_type.clone(),
            visible: node.is_active,
            x: node.computed_rect.x,
            y: node.computed_rect.y,
            w: node.computed_rect.w,
            h: node.computed_rect.h,
            alpha: node.alpha,
            blend_mode: node.colour_blend_mode.as_ref().map(blend_mode_name),
            asset_identity: node
                .asset_ref
                .clone()
                .or_else(|| node.custom_shape.as_ref().and_then(|shape| shape.svg_path.clone())),
            alignment: node.text_style.as_ref().map(|style| style.alignment.clone()),
            vertical_alignment: node
                .text_style
                .as_ref()
                .map(|style| style.vertical_alignment.clone()),
            overflow_mode: node.overflow_mode.clone(),
            background_rgba: node.background_fill_colour,
            stroke_rgba: node.stroke_colour,
            text_rgba: node.text_style.as_ref().and_then(|style| style.colour),
            icon_tint_rgba: node.icon_tint_colour,
            stroke_extent: node.stroke_extent,
            text_payload: node.text_payload.as_ref().and_then(|payload| match payload {
                crate::ui_ir::UiIrTextPayload::Resolved { text } => Some(text.clone()),
                crate::ui_ir::UiIrTextPayload::UnresolvedKey { key } => Some(key.clone()),
                crate::ui_ir::UiIrTextPayload::IntentionallyEmpty { key } => key.clone(),
                crate::ui_ir::UiIrTextPayload::Empty => None,
            }),
            text_font_identity: node.text_style.as_ref().and_then(|style| {
                style
                    .font_record
                    .clone()
                    .or_else(|| style.resolved_font_record.as_ref().map(|record| record.to_string()))
            }),
            line_spacing: node.text_style.as_ref().and_then(|style| style.line_spacing),
        });
    }

    UiScreenSnapshot {
        schema_version: UI_SNAPSHOT_SCHEMA_VERSION,
        canvas_guid: document.canvas_guid.clone(),
        canvas_name: document.canvas_name.clone(),
        target_width: document.target_width,
        target_height: document.target_height,
        elements,
    }
}

/// Compare two snapshots with hybrid tolerance and strict category/identity checks.
pub fn compare_snapshots(
    baseline: &UiScreenSnapshot,
    current: &UiScreenSnapshot,
    tolerance: UiSnapshotTolerance,
) -> UiSnapshotComparison {
    let mut failures = Vec::new();
    let baseline_max_dim = baseline.target_width.max(baseline.target_height) as f32;

    let baseline_visible: HashMap<&str, &UiSnapshotElement> = baseline
        .elements
        .iter()
        .filter(|element| element.visible)
        .map(|element| (element.identity.as_str(), element))
        .collect();
    let current_visible: HashMap<&str, &UiSnapshotElement> = current
        .elements
        .iter()
        .filter(|element| element.visible)
        .map(|element| (element.identity.as_str(), element))
        .collect();

    for (identity, _) in &baseline_visible {
        if !current_visible.contains_key(identity) {
            failures.push(format!("missing visible element: {identity}"));
        }
    }
    for (identity, element) in &current_visible {
        if !baseline_visible.contains_key(identity) {
            failures.push(format!(
                "unexpected new visible element: {identity} ({:?})",
                element.category
            ));
        }
    }

    for (identity, baseline_element) in &baseline_visible {
        let Some(current_element) = current_visible.get(identity) else {
            continue;
        };

        // Category-targeted strict checks.
        if baseline_element.category != current_element.category {
            failures.push(format!(
                "{identity}: category drift baseline={:?} current={:?}",
                baseline_element.category, current_element.category
            ));
        }
        if baseline_element.draw_order_index != current_element.draw_order_index {
            failures.push(format!(
                "{identity}: draw-order drift baseline={} current={}",
                baseline_element.draw_order_index, current_element.draw_order_index
            ));
        }
        if baseline_element.blend_mode != current_element.blend_mode {
            failures.push(format!(
                "{identity}: blend mode drift baseline={:?} current={:?}",
                baseline_element.blend_mode, current_element.blend_mode
            ));
        }
        if baseline_element.asset_identity != current_element.asset_identity {
            failures.push(format!(
                "{identity}: asset identity drift baseline={:?} current={:?}",
                baseline_element.asset_identity, current_element.asset_identity
            ));
        }
        if baseline_element.alignment != current_element.alignment {
            failures.push(format!(
                "{identity}: alignment drift baseline={:?} current={:?}",
                baseline_element.alignment, current_element.alignment
            ));
        }
        if baseline_element.vertical_alignment != current_element.vertical_alignment {
            failures.push(format!(
                "{identity}: vertical alignment drift baseline={:?} current={:?}",
                baseline_element.vertical_alignment, current_element.vertical_alignment
            ));
        }
        if baseline_element.overflow_mode != current_element.overflow_mode {
            failures.push(format!(
                "{identity}: overflow drift baseline={:?} current={:?}",
                baseline_element.overflow_mode, current_element.overflow_mode
            ));
        }
        if baseline_element.text_payload != current_element.text_payload {
            failures.push(format!(
                "{identity}: text payload/case drift baseline={:?} current={:?}",
                baseline_element.text_payload, current_element.text_payload
            ));
        }
        if baseline_element.text_font_identity != current_element.text_font_identity {
            failures.push(format!(
                "{identity}: font identity drift baseline={:?} current={:?}",
                baseline_element.text_font_identity, current_element.text_font_identity
            ));
        }

        // Tolerance-based numeric checks.
        compare_geometry_numeric(
            &mut failures,
            identity,
            "x",
            baseline_element.x,
            current_element.x,
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "y",
            baseline_element.y,
            current_element.y,
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "w",
            baseline_element.w,
            current_element.w,
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "h",
            baseline_element.h,
            current_element.h,
            baseline_max_dim,
            tolerance,
        );
        compare_unit_numeric(
            &mut failures,
            identity,
            "alpha",
            baseline_element.alpha,
            current_element.alpha,
            tolerance,
        );

        compare_optional_numeric(
            &mut failures,
            identity,
            "stroke_extent",
            baseline_element.stroke_extent,
            current_element.stroke_extent,
            baseline_max_dim,
            tolerance,
        );
        compare_optional_font_numeric(
            &mut failures,
            identity,
            "line_spacing",
            baseline_element.line_spacing,
            current_element.line_spacing,
            baseline_max_dim,
            tolerance,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "background_rgba",
            baseline_element.background_rgba,
            current_element.background_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "stroke_rgba",
            baseline_element.stroke_rgba,
            current_element.stroke_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "text_rgba",
            baseline_element.text_rgba,
            current_element.text_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "icon_tint_rgba",
            baseline_element.icon_tint_rgba,
            current_element.icon_tint_rgba,
            tolerance.rgba_channel_abs,
        );
    }

    UiSnapshotComparison {
        passed: failures.is_empty(),
        failures,
    }
}

fn classify_node(node: &UiIrNode) -> Option<UiSnapshotElementCategory> {
    if node.text_payload.is_some() {
        Some(UiSnapshotElementCategory::Text)
    } else if node.asset_ref.is_some() {
        Some(UiSnapshotElementCategory::Image)
    } else if node.custom_shape.is_some() {
        Some(UiSnapshotElementCategory::Shape)
    } else {
        None
    }
}

fn blend_mode_name(mode: &UiIrColourBlendMode) -> String {
    match mode {
        UiIrColourBlendMode::SourceOver => "source_over".to_string(),
        UiIrColourBlendMode::Additive => "additive".to_string(),
    }
}

fn compare_geometry_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: f32,
    current: f32,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) {
    let threshold = hybrid_threshold(baseline, baseline_max_dim, tolerance);
    let abs_delta = (baseline - current).abs();
    if abs_delta > threshold {
        failures.push(format!(
            "{identity}: {field} drift baseline={baseline:.4} current={current:.4} delta={abs_delta:.4} threshold={threshold:.4}"
        ));
    }
}

fn compare_unit_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: f32,
    current: f32,
    tolerance: UiSnapshotTolerance,
) {
    let threshold = (baseline.abs() * tolerance.numeric_relative).max(tolerance.rgba_channel_abs);
    let abs_delta = (baseline - current).abs();
    if abs_delta > threshold {
        failures.push(format!(
            "{identity}: {field} drift baseline={baseline:.4} current={current:.4} delta={abs_delta:.4} threshold={threshold:.4}"
        ));
    }
}

fn compare_optional_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: Option<f32>,
    current: Option<f32>,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) {
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            let threshold = hybrid_threshold(base, baseline_max_dim, tolerance);
            let abs_delta = (base - cur).abs();
            if abs_delta > threshold {
                failures.push(format!(
                    "{identity}: {field} drift baseline={base:.4} current={cur:.4} delta={abs_delta:.4} threshold={threshold:.4}"
                ));
            }
        }
        (None, None) => {}
        _ => failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn compare_optional_font_numeric(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: Option<f32>,
    current: Option<f32>,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) {
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            let threshold = hybrid_threshold_with_relative(
                base,
                baseline_max_dim,
                tolerance.font_size_relative,
                tolerance.numeric_screen_floor_ratio,
            );
            let abs_delta = (base - cur).abs();
            if abs_delta > threshold {
                failures.push(format!(
                    "{identity}: {field} drift baseline={base:.4} current={cur:.4} delta={abs_delta:.4} threshold={threshold:.4}"
                ));
            }
        }
        (None, None) => {}
        _ => failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn compare_optional_rgba(
    failures: &mut Vec<String>,
    identity: &str,
    field: &str,
    baseline: Option<[f32; 4]>,
    current: Option<[f32; 4]>,
    channel_tolerance: f32,
) {
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            for (index, (base_ch, cur_ch)) in base.iter().zip(cur.iter()).enumerate() {
                let abs_delta = (base_ch - cur_ch).abs();
                if abs_delta > channel_tolerance {
                    failures.push(format!(
                        "{identity}: {field}[{index}] drift baseline={base_ch:.4} current={cur_ch:.4} delta={abs_delta:.4} threshold={channel_tolerance:.4}"
                    ));
                }
            }
        }
        (None, None) => {}
        _ => failures.push(format!(
            "{identity}: {field} presence drift baseline={baseline:?} current={current:?}"
        )),
    }
}

fn hybrid_threshold(
    baseline: f32,
    baseline_max_dim: f32,
    tolerance: UiSnapshotTolerance,
) -> f32 {
    hybrid_threshold_with_relative(
        baseline,
        baseline_max_dim,
        tolerance.numeric_relative,
        tolerance.numeric_screen_floor_ratio,
    )
}

fn hybrid_threshold_with_relative(
    baseline: f32,
    baseline_max_dim: f32,
    numeric_relative: f32,
    numeric_screen_floor_ratio: f32,
) -> f32 {
    let relative_limit = baseline.abs() * numeric_relative;
    let screen_floor_limit = baseline_max_dim * numeric_screen_floor_ratio;
    relative_limit.max(screen_floor_limit)
}

/// Renderer-focused metadata snapshot for low-cost regression protection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRendererMetadataSnapshot {
    pub schema_version: u32,
    pub canvas_guid: String,
    pub target_width: u32,
    pub target_height: u32,
    pub elements: Vec<UiRendererMetadataElement>,
}

/// One visible renderer element in draw order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRendererMetadataElement {
    pub identity: String,
    pub node_id: u32,
    pub node_type: String,
    pub draw_order_index: u32,
    pub blend_mode: Option<String>,
    pub asset_identity: Option<String>,
    pub rect: [f32; 4],
    pub background_rgba: Option<[f32; 4]>,
    pub stroke_rgba: Option<[f32; 4]>,
    pub text_rgba: Option<[f32; 4]>,
    pub icon_tint_rgba: Option<[f32; 4]>,
}

/// Extract low-cost renderer metadata without pixel snapshots.
pub fn renderer_metadata_snapshot_from_ui_ir(document: &UiIrDocument) -> UiRendererMetadataSnapshot {
    let mut draw_nodes: Vec<&UiIrNode> = document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .collect();
    draw_nodes.sort_by_key(|node| (node.layer, node.id));

    let mut elements = Vec::new();
    for (draw_order_index, node) in draw_nodes.into_iter().enumerate() {
        let rect = crate::ir_compose::debug_node_draw_rect(node, document);
        elements.push(UiRendererMetadataElement {
            identity: format!("{}:{}", node.id, node.node_type),
            node_id: node.id,
            node_type: node.node_type.clone(),
            draw_order_index: draw_order_index as u32,
            blend_mode: node.colour_blend_mode.as_ref().map(blend_mode_name),
            asset_identity: node
                .asset_ref
                .clone()
                .or_else(|| node.custom_shape.as_ref().and_then(|shape| shape.svg_path.clone())),
            rect: [rect.x, rect.y, rect.w, rect.h],
            background_rgba: node.background_fill_colour,
            stroke_rgba: node.stroke_colour,
            text_rgba: node.text_style.as_ref().and_then(|style| style.colour),
            icon_tint_rgba: node.icon_tint_colour,
        });
    }

    UiRendererMetadataSnapshot {
        schema_version: UI_SNAPSHOT_SCHEMA_VERSION,
        canvas_guid: document.canvas_guid.clone(),
        target_width: document.target_width,
        target_height: document.target_height,
        elements,
    }
}

/// Compare renderer metadata snapshots using Phase 7 tolerance policy.
pub fn compare_renderer_metadata_snapshots(
    baseline: &UiRendererMetadataSnapshot,
    current: &UiRendererMetadataSnapshot,
    tolerance: UiSnapshotTolerance,
) -> UiSnapshotComparison {
    let mut failures = Vec::new();
    let baseline_max_dim = baseline.target_width.max(baseline.target_height) as f32;

    let baseline_by_identity: HashMap<&str, &UiRendererMetadataElement> = baseline
        .elements
        .iter()
        .map(|element| (element.identity.as_str(), element))
        .collect();
    let current_by_identity: HashMap<&str, &UiRendererMetadataElement> = current
        .elements
        .iter()
        .map(|element| (element.identity.as_str(), element))
        .collect();

    for (identity, _) in &baseline_by_identity {
        if !current_by_identity.contains_key(identity) {
            failures.push(format!("missing renderer metadata element: {identity}"));
        }
    }
    for (identity, element) in &current_by_identity {
        if !baseline_by_identity.contains_key(identity) {
            failures.push(format!(
                "unexpected renderer metadata element: {identity} ({})",
                element.node_type
            ));
        }
    }

    for (identity, baseline_element) in &baseline_by_identity {
        let Some(current_element) = current_by_identity.get(identity) else {
            continue;
        };
        if baseline_element.draw_order_index != current_element.draw_order_index {
            failures.push(format!(
                "{identity}: renderer draw-order drift baseline={} current={}",
                baseline_element.draw_order_index, current_element.draw_order_index
            ));
        }
        if baseline_element.blend_mode != current_element.blend_mode {
            failures.push(format!(
                "{identity}: renderer blend drift baseline={:?} current={:?}",
                baseline_element.blend_mode, current_element.blend_mode
            ));
        }
        if baseline_element.asset_identity != current_element.asset_identity {
            failures.push(format!(
                "{identity}: renderer asset identity drift baseline={:?} current={:?}",
                baseline_element.asset_identity, current_element.asset_identity
            ));
        }

        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_x",
            baseline_element.rect[0],
            current_element.rect[0],
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_y",
            baseline_element.rect[1],
            current_element.rect[1],
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_w",
            baseline_element.rect[2],
            current_element.rect[2],
            baseline_max_dim,
            tolerance,
        );
        compare_geometry_numeric(
            &mut failures,
            identity,
            "renderer_rect_h",
            baseline_element.rect[3],
            current_element.rect[3],
            baseline_max_dim,
            tolerance,
        );

        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_background_rgba",
            baseline_element.background_rgba,
            current_element.background_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_stroke_rgba",
            baseline_element.stroke_rgba,
            current_element.stroke_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_text_rgba",
            baseline_element.text_rgba,
            current_element.text_rgba,
            tolerance.rgba_channel_abs,
        );
        compare_optional_rgba(
            &mut failures,
            identity,
            "renderer_icon_tint_rgba",
            baseline_element.icon_tint_rgba,
            current_element.icon_tint_rgba,
            tolerance.rgba_channel_abs,
        );
    }

    UiSnapshotComparison {
        passed: failures.is_empty(),
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_ir::{
        UI_IR_SCHEMA_VERSION, UiIrDocument, UiIrNode, UiIrRect, UiIrTextPayload, UiIrTextStyle,
        UiIrValue, UiRendererHint,
    };

    fn base_node(id: u32, layer: i32) -> UiIrNode {
        UiIrNode {
            id,
            parent_id: None,
            children: vec![],
            node_type: "widget_text_field".to_string(),
            name: format!("node-{id}"),
            is_active: true,
            layer,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            authored_position: [0.0, 0.0],
            authored_size: [UiIrValue::Fixed { value: 100.0 }, UiIrValue::Fixed { value: 40.0 }],
            padding: [0.0; 4],
            margin: [0.0; 4],
            overflow_mode: Some("visible".to_string()),
            computed_rect: UiIrRect {
                x: 10.0,
                y: 20.0,
                w: 100.0,
                h: 40.0,
            },
            background_fill_colour: Some([0.1, 0.1, 0.1, 0.5]),
            corner_radius: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            segmented_fill: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: Some(1.0),
            colour_blend_mode: Some(crate::ui_ir::UiIrColourBlendMode::SourceOver),
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: Some(UiIrTextPayload::Resolved {
                text: "HELLO".to_string(),
            }),
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: Some(UiIrTextStyle {
                font_record: Some("$Text1Book".to_string()),
                resolved_font_record: None,
                font_size: UiIrValue::Fixed { value: 30.0 },
                line_spacing: Some(2.0),
                alignment: "Center".to_string(),
                vertical_alignment: "Center".to_string(),
                anchor_to_parent_x: None,
                anchor_to_parent_y: None,
                colour: Some([0.8, 0.8, 0.8, 1.0]),
                colour_token: None,
                label_style: Some("Heading1".to_string()),
            }),
            asset_ref: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: vec![],
            resolved_style_tags: vec![],
        }
    }

    fn base_document(nodes: Vec<UiIrNode>) -> UiIrDocument {
        UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "test-guid".to_string(),
            canvas_name: Some("test-canvas".to_string()),
            target_width: 1920,
            target_height: 1080,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: vec![],
            unresolved_references: vec![],
            resolved_asset_refs: vec![],
            missing_asset_refs: vec![],
            nodes,
        }
    }

    #[test]
    fn snapshot_extracts_visible_text_elements_with_draw_order() {
        let low_layer = base_node(2, 1);
        let high_layer = base_node(1, 9);
        let document = base_document(vec![high_layer, low_layer]);

        let snapshot = snapshot_from_ui_ir(&document);
        assert_eq!(snapshot.schema_version, UI_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snapshot.elements.len(), 2);
        assert_eq!(snapshot.elements[0].node_id, 2);
        assert_eq!(snapshot.elements[0].draw_order_index, 0);
        assert_eq!(snapshot.elements[1].node_id, 1);
        assert_eq!(snapshot.elements[1].draw_order_index, 1);
    }

    #[test]
    fn compare_snapshots_fails_on_draw_order_drift() {
        let baseline = base_document(vec![base_node(1, 1), base_node(2, 2)]);
        let current = base_document(vec![base_node(1, 2), base_node(2, 1)]);

        let baseline_snapshot = snapshot_from_ui_ir(&baseline);
        let current_snapshot = snapshot_from_ui_ir(&current);
        let comparison = compare_snapshots(&baseline_snapshot, &current_snapshot, UiSnapshotTolerance::default());

        assert!(!comparison.passed);
        assert!(
            comparison
                .failures
                .iter()
                .any(|failure| failure.contains("draw-order drift"))
        );
    }

    #[test]
    fn compare_snapshots_fails_on_new_visible_shape_element() {
        let baseline_node = base_node(1, 1);
        let baseline = base_document(vec![baseline_node]);

        let mut shape_node = base_node(2, 2);
        shape_node.text_payload = None;
        shape_node.text_style = None;
        shape_node.node_type = "widget_custom_shape".to_string();
        shape_node.custom_shape = Some(crate::ui_ir::UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        let current = base_document(vec![base_node(1, 1), shape_node]);

        let baseline_snapshot = snapshot_from_ui_ir(&baseline);
        let current_snapshot = snapshot_from_ui_ir(&current);
        let comparison = compare_snapshots(&baseline_snapshot, &current_snapshot, UiSnapshotTolerance::default());

        assert!(!comparison.passed);
        assert!(
            comparison
                .failures
                .iter()
                .any(|failure| failure.contains("unexpected new visible element"))
        );
    }

    #[test]
    fn compare_snapshots_uses_font_size_relative_for_line_spacing() {
        let baseline = base_document(vec![base_node(1, 1)]);
        let mut current = baseline.clone();
        current.nodes[0]
            .text_style
            .as_mut()
            .expect("text style should exist")
            .line_spacing = Some(2.08);

        let baseline_snapshot = snapshot_from_ui_ir(&baseline);
        let current_snapshot = snapshot_from_ui_ir(&current);
        let comparison = compare_snapshots(
            &baseline_snapshot,
            &current_snapshot,
            UiSnapshotTolerance {
                numeric_relative: 0.01,
                font_size_relative: 0.05,
                numeric_screen_floor_ratio: 0.001,
                rgba_channel_abs: 0.05,
            },
        );

        assert!(
            comparison.passed,
            "line_spacing drift within font-size tolerance should pass: {:?}",
            comparison.failures
        );
    }

    #[test]
    fn renderer_metadata_snapshot_is_deterministic() {
        let document = base_document(vec![base_node(1, 1), base_node(2, 2)]);
        let first = renderer_metadata_snapshot_from_ui_ir(&document);
        let second = renderer_metadata_snapshot_from_ui_ir(&document);
        assert_eq!(first, second);
    }

    #[test]
    fn renderer_metadata_comparator_fails_on_blend_drift() {
        let baseline = base_document(vec![base_node(1, 1)]);
        let mut current = baseline.clone();
        current.nodes[0].colour_blend_mode = Some(crate::ui_ir::UiIrColourBlendMode::Additive);

        let baseline_snapshot = renderer_metadata_snapshot_from_ui_ir(&baseline);
        let current_snapshot = renderer_metadata_snapshot_from_ui_ir(&current);
        let comparison = compare_renderer_metadata_snapshots(
            &baseline_snapshot,
            &current_snapshot,
            UiSnapshotTolerance::default(),
        );

        assert!(!comparison.passed);
        assert!(
            comparison
                .failures
                .iter()
                .any(|failure| failure.contains("renderer blend drift"))
        );
    }
}
