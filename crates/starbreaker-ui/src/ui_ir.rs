//! Canonical UI intermediate representation (IR) schema and compiler.
//!
//! This module defines a versioned, renderer-agnostic IR document that captures
//! fidelity-critical UI data from a resolved BuildingBlocks scene.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use crate::bb_layout;
use crate::bb_scene::{BbNodeType, BbScene, BbValue};

/// Current IR schema version.
pub const UI_IR_SCHEMA_VERSION: u32 = 1;

/// Renderer backend hint derived from scene content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRendererHint {
    Bb,
    Swf,
    Hybrid,
}

/// Canonical UI IR document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrDocument {
    pub schema_version: u32,
    pub canvas_guid: String,
    pub canvas_name: Option<String>,
    pub target_width: u32,
    pub target_height: u32,
    pub selected_style_source: Option<String>,
    pub renderer_hint: UiRendererHint,
    pub confidence: u8,
    pub warnings: Vec<String>,
    pub unresolved_references: Vec<String>,
    pub resolved_asset_refs: Vec<String>,
    pub missing_asset_refs: Vec<String>,
    pub nodes: Vec<UiIrNode>,
}

/// One scene node in the canonical IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub children: Vec<u32>,
    pub node_type: String,
    pub name: String,
    pub is_active: bool,
    pub layer: i32,
    pub alpha: f32,
    pub anchor: [f32; 2],
    pub pivot: [f32; 2],
    pub authored_position: [f32; 2],
    pub authored_size: [UiIrValue; 2],
    pub padding: [f32; 4],
    pub margin: [f32; 4],
    pub computed_rect: UiIrRect,
    pub background_fill_colour: Option<[f32; 4]>,
    pub icon_tint_colour: Option<[f32; 4]>,
    pub text_payload: Option<UiIrTextPayload>,
    pub text_style: Option<UiIrTextStyle>,
    pub asset_ref: Option<String>,
    pub style_tag_uuids: Vec<String>,
}

/// Typed representation of authored fixed/relative values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiIrValue {
    Fixed { value: f32 },
    Percent { value: f32 },
    Other { value: f32, behavior: String },
}

/// Pixel-space computed rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiIrRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Text payload status carried by the IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UiIrTextPayload {
    Resolved { text: String },
    UnresolvedKey { key: String },
    Empty,
}

/// Typography/style attributes for text widgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrTextStyle {
    pub font_record: Option<String>,
    pub font_size: UiIrValue,
    pub alignment: String,
    pub colour: Option<[f32; 4]>,
}

/// Validate required schema invariants for a UI IR document.
pub fn validate_ui_ir_document(document: &UiIrDocument) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let mut seen_ids = HashSet::new();

    if document.schema_version != UI_IR_SCHEMA_VERSION {
        errors.push(format!(
            "schema_version mismatch: expected {}, got {}",
            UI_IR_SCHEMA_VERSION, document.schema_version
        ));
    }

    if document.canvas_guid.trim().is_empty() {
        errors.push("canvas_guid must not be empty".to_string());
    }

    if document.target_width == 0 || document.target_height == 0 {
        errors.push("target dimensions must be non-zero".to_string());
    }

    if document.confidence > 100 {
        errors.push("confidence must be in 0..=100".to_string());
    }

    for node in &document.nodes {
        if !seen_ids.insert(node.id) {
            errors.push(format!("duplicate node id {}", node.id));
        }
        if node.name.trim().is_empty() {
            errors.push(format!("node {} has empty name", node.id));
        }
        if node.node_type.trim().is_empty() {
            errors.push(format!("node {} has empty node_type", node.id));
        }
        if node.computed_rect.w < 0.0 || node.computed_rect.h < 0.0 {
            errors.push(format!(
                "node {} has negative computed size ({}, {})",
                node.id, node.computed_rect.w, node.computed_rect.h
            ));
        }
    }

    for node in &document.nodes {
        if let Some(parent_id) = node.parent_id {
            if !seen_ids.contains(&parent_id) {
                errors.push(format!("node {} references missing parent {}", node.id, parent_id));
            }
        }
        for child_id in &node.children {
            if !seen_ids.contains(child_id) {
                errors.push(format!("node {} references missing child {}", node.id, child_id));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Stable SHA-256 hash of a UI IR document.
///
/// The hash is computed from canonical JSON serialization (`serde_json` output
/// from typed structs and BTreeMap-backed sources) to support deterministic
/// fixture comparisons across reruns.
pub fn stable_hash_ui_ir(document: &UiIrDocument) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(document)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{:x}", digest))
}

/// Compile a canonical UI IR document from a resolved scene.
pub fn compile_ui_ir_from_scene(
    scene: &BbScene,
    canvas_guid: &str,
    canvas_name: Option<&str>,
    target_size: (u32, u32),
    selected_style_source: Option<String>,
    unresolved_references: &[String],
    resolved_asset_refs: Vec<String>,
    missing_asset_refs: Vec<String>,
    confidence: u8,
) -> UiIrDocument {
    let layout = bb_layout::layout(scene, target_size.0, target_size.1);

    let has_text = scene.nodes.values().any(|n| n.text.is_some());
    let has_custom_shape = scene
        .nodes
        .values()
        .any(|n| n.ty == BbNodeType::WidgetCustomShape);
    let renderer_hint = match (has_text, has_custom_shape) {
        (true, true) => UiRendererHint::Hybrid,
        (false, true) => UiRendererHint::Swf,
        _ => UiRendererHint::Bb,
    };

    let unresolved_count = scene
        .nodes
        .values()
        .filter_map(|n| n.text.as_ref())
        .map(|t| t.string.trim())
        .filter(|s| s.starts_with('@'))
        .count() as u8;
    let computed_confidence = confidence
        .saturating_sub(unresolved_count.saturating_mul(10))
        .min(100);

    let mut warnings = Vec::new();
    if scene.roots.is_empty() {
        warnings.push("scene has no root nodes".to_string());
    }
    if unresolved_count > 0 {
        warnings.push(format!(
            "{} unresolved text key(s) present in scene",
            unresolved_count
        ));
    }
    if !missing_asset_refs.is_empty() {
        warnings.push(format!(
            "{} asset reference(s) could not be resolved",
            missing_asset_refs.len()
        ));
    }

    let mut nodes = Vec::with_capacity(scene.nodes.len());
    for (&id, node) in &scene.nodes {
        let rect = layout.rects.get(&id).copied().unwrap_or_default();
        let text_payload = node.text.as_ref().map(|text| {
            let trimmed = text.string.trim();
            if trimmed.is_empty() {
                UiIrTextPayload::Empty
            } else if trimmed.starts_with('@') {
                UiIrTextPayload::UnresolvedKey {
                    key: trimmed.to_string(),
                }
            } else {
                UiIrTextPayload::Resolved {
                    text: trimmed.to_string(),
                }
            }
        });
        let text_style = node.text.as_ref().map(|text| UiIrTextStyle {
            font_record: text.font_record.clone(),
            font_size: convert_bb_value(&text.font_size),
            alignment: text.alignment.clone(),
            colour: text.colour,
        });

        let asset_ref = node
            .icon
            .as_ref()
            .and_then(|i| i.image_record.clone())
            .or_else(|| {
                node.background
                    .as_ref()
                    .and_then(|bg| bg.svg_fill_path.clone())
            });

        nodes.push(UiIrNode {
            id,
            parent_id: node.parent,
            children: node.children.clone(),
            node_type: node_type_name(&node.ty).to_string(),
            name: node.name.clone(),
            is_active: node.is_active,
            layer: node.layer,
            alpha: node.alpha,
            anchor: [node.anchor.x, node.anchor.y],
            pivot: [node.pivot.x, node.pivot.y],
            authored_position: [node.position.x, node.position.y],
            authored_size: [
                convert_bb_value(&node.sizing.width),
                convert_bb_value(&node.sizing.height),
            ],
            padding: [
                node.padding.top,
                node.padding.right,
                node.padding.bottom,
                node.padding.left,
            ],
            margin: [
                node.margin.top,
                node.margin.right,
                node.margin.bottom,
                node.margin.left,
            ],
            computed_rect: UiIrRect {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
            },
            background_fill_colour: node
                .background
                .as_ref()
                .and_then(|bg| bg.fill_colour),
            icon_tint_colour: node.icon.as_ref().and_then(|i| i.tint_colour),
            text_payload,
            text_style,
            asset_ref,
            style_tag_uuids: node.style_tag_uuids.clone(),
        });
    }

    UiIrDocument {
        schema_version: UI_IR_SCHEMA_VERSION,
        canvas_guid: canvas_guid.to_string(),
        canvas_name: canvas_name.map(str::to_string),
        target_width: target_size.0,
        target_height: target_size.1,
        selected_style_source,
        renderer_hint,
        confidence: computed_confidence,
        warnings,
        unresolved_references: unresolved_references.to_vec(),
        resolved_asset_refs,
        missing_asset_refs,
        nodes,
    }
}

fn convert_bb_value(value: &BbValue) -> UiIrValue {
    match value {
        BbValue::Fixed(v) => UiIrValue::Fixed { value: *v },
        BbValue::Percent(v) => UiIrValue::Percent { value: *v },
        BbValue::Other { value, behavior } => UiIrValue::Other {
            value: *value,
            behavior: behavior.clone(),
        },
    }
}

fn node_type_name(node_type: &BbNodeType) -> &str {
    match node_type {
        BbNodeType::DisplayWidget => "display_widget",
        BbNodeType::WidgetCanvas => "widget_canvas",
        BbNodeType::WidgetIcon => "widget_icon",
        BbNodeType::WidgetCard => "widget_card",
        BbNodeType::WidgetTextField => "widget_text_field",
        BbNodeType::ComponentGeneralButton => "component_general_button",
        BbNodeType::ComponentGeneralButtonSecondary => "component_general_button_secondary",
        BbNodeType::WidgetImage => "widget_image",
        BbNodeType::WidgetText => "widget_text",
        BbNodeType::WidgetCustomShape => "widget_custom_shape",
        BbNodeType::WidgetBodyBackground => "widget_body_background",
        BbNodeType::Other(s) => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_ir_emits_schema_and_nodes() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "isActive": true,
                        "position": {"x": 5.0, "y": 7.0},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        },
                        "text": "HELLO"
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            "guid-1",
            Some("BuildingBlocks_Canvas.Test"),
            (200, 100),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            90,
        );

        assert_eq!(ir.schema_version, UI_IR_SCHEMA_VERSION);
        assert_eq!(ir.canvas_guid, "guid-1");
        assert_eq!(ir.target_width, 200);
        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.nodes[0].name, "label");
        assert_eq!(ir.nodes[0].node_type, "widget_text_field");
        assert_eq!(ir.nodes[0].text_payload, Some(UiIrTextPayload::Resolved { text: "HELLO".into() }));
    }

    #[test]
    fn compile_ir_is_deterministic_for_same_input() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test2",
            "_RecordValue_": {
                "size": {"x": 50, "y": 50},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "root",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Percent", "value": 1.0},
                            "height": {"behavior": "Percent", "value": 1.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir1 = compile_ui_ir_from_scene(
            &scene,
            "guid-2",
            None,
            (128, 128),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );
        let ir2 = compile_ui_ir_from_scene(
            &scene,
            "guid-2",
            None,
            (128, 128),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let s1 = serde_json::to_string(&ir1).expect("serialize ir1");
        let s2 = serde_json::to_string(&ir2).expect("serialize ir2");
        assert_eq!(s1, s2);

        let h1 = stable_hash_ui_ir(&ir1).expect("hash ir1");
        let h2 = stable_hash_ui_ir(&ir2).expect("hash ir2");
        assert_eq!(h1, h2);
    }

    #[test]
    fn validate_ir_rejects_invalid_document() {
        let invalid = UiIrDocument {
            schema_version: 999,
            canvas_guid: String::new(),
            canvas_name: None,
            target_width: 0,
            target_height: 0,
            selected_style_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 101,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: vec![UiIrNode {
                id: 1,
                parent_id: None,
                children: Vec::new(),
                node_type: String::new(),
                name: String::new(),
                is_active: true,
                layer: 0,
                alpha: 1.0,
                anchor: [0.0, 0.0],
                pivot: [0.0, 0.0],
                authored_position: [0.0, 0.0],
                authored_size: [
                    UiIrValue::Fixed { value: 1.0 },
                    UiIrValue::Fixed { value: 1.0 },
                ],
                padding: [0.0, 0.0, 0.0, 0.0],
                margin: [0.0, 0.0, 0.0, 0.0],
                computed_rect: UiIrRect {
                    x: 0.0,
                    y: 0.0,
                    w: -1.0,
                    h: -1.0,
                },
                background_fill_colour: None,
                icon_tint_colour: None,
                text_payload: None,
                text_style: None,
                asset_ref: None,
                style_tag_uuids: Vec::new(),
            }],
        };

        let result = validate_ui_ir_document(&invalid);
        assert!(result.is_err());
    }

    #[test]
    fn compile_ir_populates_anchor_pivot_alpha_and_style_tags() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test3",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "alpha": 0.5,
                        "anchor": {"x": 0.25, "y": 0.75},
                        "pivot": {"x": 0.5, "y": 0.5},
                        "styleTags": [{"_RecordId_": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"}],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            "guid-3",
            None,
            (100, 100),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );
        let node = &ir.nodes[0];
        assert_eq!(node.alpha, 0.5);
        assert_eq!(node.anchor, [0.25, 0.75]);
        assert_eq!(node.pivot, [0.5, 0.5]);
        assert_eq!(node.style_tag_uuids.len(), 1);
    }
}
