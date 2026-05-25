//! Canonical UI intermediate representation (IR) schema and compiler.
//!
//! This module defines a versioned, renderer-agnostic IR document that captures
//! fidelity-critical UI data from a resolved BuildingBlocks scene.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use crate::bb_bindings::BindingResolver;
use crate::bb_layout;
use crate::bb_layout::{LayoutResult, Rect};
use crate::bb_scene::{BbNode, BbNodeId, BbNodeType, BbScene, BbValue};
use crate::defaults::DefaultValueRegistry;
use crate::pipeline::CanvasFetcher;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_swf_source: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_fill_colour_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segmented_fill: Option<UiIrSegmentedFill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<UiIrBorder>,
    pub stroke_colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_colour_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_extent: Option<f32>,
    pub icon_tint_colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_tint_colour_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_preset: Option<String>,
    pub text_payload: Option<UiIrTextPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_text_payload: Option<UiIrTextPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_text_style: Option<UiIrTextStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meter_progress: Option<f32>,
    pub text_style: Option<UiIrTextStyle>,
    pub asset_ref: Option<String>,
    pub custom_shape: Option<UiIrCustomShape>,
    pub style_tag_uuids: Vec<String>,
    pub resolved_style_tags: Vec<UiIrStyleTag>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrBorder {
    pub top: UiIrBorderSide,
    pub right: UiIrBorderSide,
    pub bottom: UiIrBorderSide,
    pub left: UiIrBorderSide,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrBorderSide {
    pub width: f32,
    pub colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colour_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrSegmentedFill {
    pub enabled: bool,
    pub angle: f32,
    pub segment_size: f32,
    pub segment_spacing_size: f32,
    pub segment_x_offset: f32,
    pub segmented_bar_fill: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_colour_token: Option<String>,
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
    pub resolved_font_record: Option<serde_json::Value>,
    pub font_size: UiIrValue,
    pub alignment: String,
    #[serde(default = "default_vertical_alignment")]
    pub vertical_alignment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_to_parent_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_to_parent_y: Option<f32>,
    pub colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colour_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_style: Option<String>,
}

fn default_vertical_alignment() -> String {
    "Center".to_string()
}

fn effective_font_record(node: &BbNode) -> Option<String> {
    node.raw
    .get("FontStyleRecord")
    .or_else(|| node.raw.get("fontRecord"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| node.text.as_ref().and_then(|text| text.font_record.clone()))
}

/// Shape metadata for custom-shape widgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrCustomShape {
    pub shape_type: Option<String>,
    pub shape: Option<String>,
    pub svg_path: Option<String>,
    pub render_shape: Option<bool>,
    pub enable_nine_slice_rect: Option<bool>,
    pub nine_slice_rect: Option<[f32; 4]>,
    pub nine_slice_scale: Option<f32>,
}

/// Resolved style-tag metadata from source `styleTags[]` entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrStyleTag {
    pub uuid: String,
    pub tag_name: Option<String>,
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
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    canvas_guid: &str,
    canvas_name: Option<&str>,
    target_size: (u32, u32),
    defaults: &DefaultValueRegistry,
    selected_style_source: Option<String>,
    selected_swf_source: Option<String>,
    unresolved_references: &[String],
    resolved_asset_refs: Vec<String>,
    missing_asset_refs: Vec<String>,
    confidence: u8,
) -> UiIrDocument {
    compile_ui_ir_from_scene_with_animation_sample(
        scene,
        canvas_fetcher,
        canvas_guid,
        canvas_name,
        target_size,
        defaults,
        selected_style_source,
        selected_swf_source,
        unresolved_references,
        resolved_asset_refs,
        missing_asset_refs,
        None,
        confidence,
    )
}

pub fn compile_ui_ir_from_scene_with_animation_sample(
    scene: &BbScene,
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    canvas_guid: &str,
    canvas_name: Option<&str>,
    target_size: (u32, u32),
    defaults: &DefaultValueRegistry,
    selected_style_source: Option<String>,
    selected_swf_source: Option<String>,
    unresolved_references: &[String],
    resolved_asset_refs: Vec<String>,
    missing_asset_refs: Vec<String>,
    animation_sample_percent: Option<f32>,
    confidence: u8,
) -> UiIrDocument {
    let layout = bb_layout::layout_with_animation_sample(
        scene,
        target_size.0,
        target_size.1,
        animation_sample_percent,
    );
    let binding_resolver = BindingResolver::from_operations(&scene.operations);

    let has_text = scene.nodes.values().any(|n| n.text.is_some());
    let has_custom_shape = scene
        .nodes
        .values()
        .any(|n| n.ty == BbNodeType::WidgetCustomShape);
    let has_selected_swf_source = selected_swf_source.is_some();
    let renderer_hint = match (has_selected_swf_source, has_text, has_custom_shape) {
        (true, true, true) => UiRendererHint::Hybrid,
        (true, false, true) => UiRendererHint::Swf,
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
    let style_font_sizes = collect_style_font_sizes(scene);
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
    if has_custom_shape && !has_selected_swf_source {
        warnings.push(
            "custom-shape content present but no SWF source was resolved; using BB renderer"
                .to_string(),
        );
    }

    let mut nodes = Vec::with_capacity(scene.nodes.len());
    for (&id, node) in &scene.nodes {
        let layout_rect = layout.rects.get(&id).copied().unwrap_or_default();
        let has_text_intent = node_has_text_intent(node);
        let resolved_text = has_text_intent
            .then(|| binding_resolver.resolve_text_detailed(id, &node.raw, defaults));
        let text_payload = resolved_text.as_ref().map(|resolved| {
            let trimmed = resolved.text.trim();
            if !trimmed.is_empty() {
                if trimmed.starts_with('@') {
                    if let Some(localized) = defaults.lookup_localization(trimmed) {
                        if !localized.trim().is_empty() {
                            UiIrTextPayload::Resolved {
                                text: localized.trim().to_string(),
                            }
                        } else {
                            UiIrTextPayload::UnresolvedKey {
                                key: trimmed.to_string(),
                            }
                        }
                    } else {
                        UiIrTextPayload::UnresolvedKey {
                            key: trimmed.to_string(),
                        }
                    }
                } else {
                    UiIrTextPayload::Resolved {
                        text: trimmed.to_string(),
                    }
                }
            } else if let Some(unresolved_key) = unresolved_text_key_from_raw(&node.raw) {
                UiIrTextPayload::UnresolvedKey {
                    key: unresolved_key,
                }
            } else {
                UiIrTextPayload::Empty
            }
        });
        let font_record = effective_font_record(node);
        let text_style = if let Some(text) = node.text.as_ref() {
            Some(UiIrTextStyle {
                font_record: font_record.clone(),
                resolved_font_record: font_record
                    .as_deref()
                    .and_then(|record_ref| resolve_record(canvas_fetcher, &[record_ref])),
                font_size: resolve_effective_font_size(
                    id,
                    node,
                    text,
                    layout_rect.h,
                    text_payload.as_ref().and_then(resolved_text_from_payload),
                    scene,
                    &binding_resolver,
                    defaults,
                    &style_font_sizes,
                ),
                alignment: text.alignment.clone(),
                vertical_alignment: node
                    .raw
                    .get("verticalTextAlignment")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Center")
                    .to_string(),
                anchor_to_parent_x: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentX"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                anchor_to_parent_y: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentY"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                colour: text.colour.or_else(|| fill_colour_from_raw_for_text(&node.raw)),
                colour_token: text_colour_token_from_raw(&node.raw).or_else(|| {
                    default_style_text_colour_token_from_raw(&node.raw, &node.ty, false)
                }),
                label_style: label_style_name_from_raw(node),
            })
        } else if text_payload.is_some() {
            let alignment = node
                .raw
                .get("textAlignment")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    node.raw
                        .get("labelProperties")
                        .and_then(|lp| lp.get("textAlignment"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("Left")
                .to_string();

            let font_size = textfield_fallback_font_size_from_signals(
                node,
                layout_rect.h,
                text_payload.as_ref().and_then(resolved_text_from_payload),
            )
                .or_else(|| {
                    node.raw
                        .get("fontSize")
                        .or_else(|| node.raw.get("FontSize"))
                        .and_then(|v| v.as_f64())
                        .map(|value| value as f32)
                })
                .unwrap_or_else(|| {
                    if node_type_name(&node.ty)
                        .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
                    {
                        21.0
                    } else {
                        18.0
                    }
                });

            Some(UiIrTextStyle {
                font_record: font_record.clone(),
                resolved_font_record: font_record
                    .as_deref()
                    .and_then(|record_ref| resolve_record(canvas_fetcher, &[record_ref])),
                font_size: UiIrValue::Fixed { value: font_size },
                alignment,
                vertical_alignment: node
                    .raw
                    .get("verticalTextAlignment")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Center")
                    .to_string(),
                anchor_to_parent_x: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentX"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                anchor_to_parent_y: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentY"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                colour: fill_colour_from_raw_for_text(&node.raw),
                colour_token: text_colour_token_from_raw(&node.raw).or_else(|| {
                    default_style_text_colour_token_from_raw(&node.raw, &node.ty, false)
                }),
                label_style: label_style_name_from_raw(node),
            })
        } else {
            None
        };

        let secondary_text_payload = if node_type_name(&node.ty)
            .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        {
            binding_resolver
                .resolve_field_text(id, "ParamInput1", defaults)
                .map(|text| {
                    let trimmed = text.trim();
                    if trimmed.starts_with('@') {
                        if let Some(localized) = defaults.lookup_localization(trimmed) {
                            UiIrTextPayload::Resolved {
                                text: localized.trim().to_string(),
                            }
                        } else {
                            UiIrTextPayload::UnresolvedKey {
                                key: trimmed.to_string(),
                            }
                        }
                    } else {
                        UiIrTextPayload::Resolved {
                            text: trimmed.to_string(),
                        }
                    }
                })
        } else {
            None
        };

        let secondary_text_style = if secondary_text_payload.is_some() {
            let alignment = node
                .raw
                .get("captionProperties")
                .and_then(|cp| cp.get("textAlignment"))
                .and_then(|v| v.as_str())
                .unwrap_or("Left")
                .to_string();
            let font_size = node
                .raw
                .get("fontSize")
                .or_else(|| node.raw.get("FontSize"))
                .and_then(|v| v.as_f64())
                .map(|value| value as f32)
                .unwrap_or(18.0);

            Some(UiIrTextStyle {
                font_record: None,
                resolved_font_record: None,
                font_size: UiIrValue::Fixed { value: font_size },
                alignment,
                vertical_alignment: node
                    .raw
                    .get("verticalTextAlignment")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Center")
                    .to_string(),
                anchor_to_parent_x: node
                    .raw
                    .get("captionProperties")
                    .and_then(|cp| cp.get("anchorToParentX"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                anchor_to_parent_y: node
                    .raw
                    .get("captionProperties")
                    .and_then(|cp| cp.get("anchorToParentY"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                colour: fill_colour_from_raw_for_text(&node.raw),
                colour_token: text_colour_token_from_raw(&node.raw).or_else(|| {
                    default_style_text_colour_token_from_raw(&node.raw, &node.ty, true)
                }),
                label_style: node
                    .raw
                    .get("captionProperties")
                    .and_then(|cp| cp.get("style"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
            })
        } else {
            None
        };

        let suppress_placeholder_only_label_caption_pair = node_type_name(&node.ty)
            .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
            && secondary_text_payload
                .as_ref()
                .is_none_or(is_placeholder_or_empty_secondary_text_payload)
            && node
                .raw
                .get("captionProperties")
                .and_then(|cp| cp.get("caption"))
                .and_then(|value| value.as_str())
                .is_some_and(|caption| caption.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER"));

        let suppress_duplicate_function_title = node
            .name
            .eq_ignore_ascii_case("FunctionTitle")
            && text_payload
                .as_ref()
                .is_some_and(|payload| matches!(payload, UiIrTextPayload::Resolved { text } if !text.trim().is_empty()));

        let rect = if suppress_placeholder_only_label_caption_pair {
            layout_rect
        } else {
            maybe_reanchor_active_label_caption_pair_rect(scene, &layout, id, node, layout_rect)
        };

        let meter_progress = if node_type_name(&node.ty)
            .eq_ignore_ascii_case("BuildingBlocks_WidgetLinearProgressMeter")
        {
            binding_resolver
                .resolve_field_number(id, "ParamInput0", defaults)
                .map(|v| v.clamp(0.0, 1.0) as f32)
                .or_else(|| {
                    node.raw
                        .get("progress")
                        .and_then(|value| value.as_f64())
                        .map(|value| value.clamp(0.0, 1.0) as f32)
                })
        } else {
            None
        };

        let asset_ref = collect_node_asset_refs(node)
            .into_iter()
            .next()
            .or_else(|| {
                binding_resolver
                    .resolve_field_text(id, "SvgPath", defaults)
                    .or_else(|| binding_resolver.resolve_field_text(id, "svgPath", defaults))
                    .or_else(|| binding_resolver.resolve_field_text(id, "ImagePath", defaults))
                    .or_else(|| binding_resolver.resolve_field_text(id, "imagePath", defaults))
            })
            .or_else(|| {
                binding_resolver
                    .resolve_string_binding(id)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            });

        let custom_shape = if node.ty == BbNodeType::WidgetCustomShape {
            let shape_type = node
                .raw
                .get("shapeType")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let shape = node
                .raw
                .get("shape")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let svg_path = node
                .raw
                .get("svgPath")
                .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("svgPath")))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let render_shape = node
                .raw
                .get("renderShape")
                .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("renderShape")))
                .and_then(|v| v.as_bool());
            let enable_nine_slice_rect = node
                .raw
                .get("enableNineSliceRect")
                .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("enableNineSliceRect")))
                .and_then(|v| v.as_bool());
            let nine_slice_rect = node
                .raw
                .get("nineSliceRect")
                .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("nineSliceRect")))
                .and_then(parse_nine_slice_rect);
            let nine_slice_scale = node
                .raw
                .get("nineSliceScale")
                .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("nineSliceScale")))
                .and_then(|v| v.as_f64())
                .map(|value| value as f32);
            Some(UiIrCustomShape {
                shape_type,
                shape,
                svg_path,
                render_shape,
                enable_nine_slice_rect,
                nine_slice_rect,
                nine_slice_scale,
            })
        } else {
            None
        };

        let resolved_style_tags = node
            .raw
            .get("styleTags")
            .and_then(|v| v.as_array())
            .map(|tags| {
                tags
                    .iter()
                    .filter_map(|tag| {
                        let uuid = tag
                            .get("_RecordId_")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)?;

                        let resolved_record = resolve_style_tag_record(
                            canvas_fetcher,
                            tag,
                            tag.get("_RecordPath_")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .unwrap_or(""),
                            tag.get("_RecordName_")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .unwrap_or(""),
                            &uuid,
                        );

                        let tag_name = resolved_record
                            .as_ref()
                            .and_then(|record| record.get("_RecordValue_"))
                            .and_then(|v| v.get("tagName"))
                            .and_then(|v| v.as_str())
                            .map(str::to_owned);

                        Some(UiIrStyleTag {
                            uuid,
                            tag_name,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let alpha = if is_transient_static_pulse_node(node) {
            0.0
        } else if let Some(sample_percent) = animation_sample_percent {
            sampled_animation_alpha(&node.raw, sample_percent).unwrap_or_else(|| {
                representative_animation_alpha(&node.raw).unwrap_or(node.alpha)
            })
        } else {
            representative_animation_alpha(&node.raw).unwrap_or(node.alpha)
        };
        let stroke_extent = node
            .raw
            .get("strokeExtent")
            .or_else(|| node.raw.get("svgFill").and_then(|svg_fill| svg_fill.get("strokeExtent")))
            .and_then(|value| value.as_f64())
            .map(|value| value as f32);

        let allow_background_fill = raw_background_enabled(&node.raw);

        nodes.push(UiIrNode {
            id,
            parent_id: node.parent,
            children: node.children.clone(),
            node_type: node_type_name(&node.ty).to_string(),
            name: node.name.clone(),
            is_active: node.is_active
                && !suppress_placeholder_only_label_caption_pair
                && !suppress_duplicate_function_title,
            layer: node.layer,
            alpha,
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
            background_fill_colour: allow_background_fill
                .then(|| {
                    node.background
                        .as_ref()
                        .and_then(|bg| bg.fill_colour)
                        .or_else(|| node.raw.get("BackgroundColor").and_then(parse_raw_colour))
                })
                .flatten(),
            corner_radius: node_corner_radius(node),
            background_fill_colour_token: background_fill_colour_token_from_raw(&node.raw, allow_background_fill),
            segmented_fill: segmented_fill_from_raw(node),
            border: border_from_node(node),
            stroke_colour: stroke_colour_from_raw(&node.raw),
            stroke_colour_token: stroke_colour_token_from_raw(&node.raw),
            stroke_extent,
            icon_tint_colour: node
                .icon
                .as_ref()
                .and_then(|i| i.tint_colour)
                .or_else(|| custom_shape.as_ref().and_then(|_| fill_colour_from_raw_for_text(&node.raw))),
            icon_tint_colour_token: icon_tint_colour_token_from_raw(
                &node.raw,
                true,
            ),
            icon_preset: node.icon.as_ref().and_then(|i| i.icon_preset.clone()),
            text_payload,
            secondary_text_payload,
            secondary_text_style,
            meter_progress,
            text_style,
            asset_ref,
            custom_shape,
            style_tag_uuids: node.style_tag_uuids.clone(),
            resolved_style_tags,
        });
    }

    UiIrDocument {
        schema_version: UI_IR_SCHEMA_VERSION,
        canvas_guid: canvas_guid.to_string(),
        canvas_name: canvas_name.map(str::to_string),
        target_width: target_size.0,
        target_height: target_size.1,
        selected_style_source,
        selected_swf_source,
        renderer_hint,
        confidence: computed_confidence,
        warnings,
        unresolved_references: unresolved_references.to_vec(),
        resolved_asset_refs,
        missing_asset_refs,
        nodes,
    }
}

pub(crate) fn collect_node_asset_refs(node: &BbNode) -> Vec<String> {
    let mut asset_refs = Vec::new();

    push_asset_ref(
        &mut asset_refs,
        node.icon
            .as_ref()
            .and_then(|icon| icon.image_record.as_deref()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.background
            .as_ref()
            .and_then(|background| background.svg_fill_path.as_deref()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("ImagePath")
            .and_then(|value| value.as_str()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("imagePath")
            .and_then(|value| value.as_str()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("SvgPath")
            .and_then(|value| value.as_str()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("svgPath")
            .and_then(|value| value.as_str()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("svgFill")
            .and_then(|value| value.get("svgPath"))
            .and_then(|value| value.as_str()),
    );

    asset_refs
}

fn push_asset_ref(asset_refs: &mut Vec<String>, candidate: Option<&str>) {
    let Some(candidate) = candidate.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if asset_refs.iter().all(|existing| existing != candidate) {
        asset_refs.push(candidate.to_string());
    }
}

fn unresolved_text_key_from_raw(raw: &serde_json::Value) -> Option<String> {
    let direct = raw.get("text").and_then(|v| v.as_str()).map(str::trim);
    if let Some(key) = direct.filter(|s| s.starts_with('@') && !s.is_empty()) {
        return Some(key.to_string());
    }

    let loc_string = raw
        .get("locString")
        .and_then(|v| v.as_str())
        .map(str::trim);
    if let Some(key) = loc_string.filter(|s| s.starts_with('@') && !s.is_empty()) {
        return Some(key.to_string());
    }

    let label = raw
        .get("labelProperties")
        .and_then(|lp| lp.get("label"))
        .and_then(|v| v.as_str())
        .map(str::trim);
    label
        .filter(|s| s.starts_with('@') && !s.is_empty())
        .map(ToString::to_string)
}

fn is_placeholder_or_empty_secondary_text_payload(payload: &UiIrTextPayload) -> bool {
    match payload {
        UiIrTextPayload::Empty => true,
        UiIrTextPayload::Resolved { text } => text.trim().is_empty(),
        UiIrTextPayload::UnresolvedKey { key } => key.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER"),
    }
}

fn node_has_text_intent(node: &BbNode) -> bool {
    node.text.is_some()
        || node.raw.get("text").is_some()
        || node.raw.get("locString").is_some()
        || node.raw.get("labelProperties").is_some()
}

fn maybe_reanchor_active_label_caption_pair_rect(
    scene: &BbScene,
    layout: &LayoutResult,
    node_id: BbNodeId,
    node: &BbNode,
    rect: Rect,
) -> Rect {
    if !node_type_name(&node.ty).eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        || node.anchor.x > 0.01
        || node.pivot.x > 0.01
    {
        return rect;
    }

    let Some(parent_id) = node.parent else {
        return rect;
    };

    if !is_footer_brand_label_context(scene, parent_id) {
        return rect;
    }

    let Some(parent) = scene.nodes.get(&parent_id) else {
        return rect;
    };

    let mut has_placeholder_sibling = false;
    let mut leftmost_x = rect.x;
    for sibling_id in &parent.children {
        let Some(sibling) = scene.nodes.get(sibling_id) else {
            continue;
        };
        if !node_type_name(&sibling.ty)
            .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        {
            continue;
        }

        if *sibling_id != node_id
            && sibling
                .raw
                .get("captionProperties")
                .and_then(|cp| cp.get("caption"))
                .and_then(|value| value.as_str())
                .is_some_and(|caption| caption.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER"))
        {
            has_placeholder_sibling = true;
        }

        if let Some(sibling_rect) = layout.rects.get(sibling_id)
            && sibling_rect.x < leftmost_x
        {
            leftmost_x = sibling_rect.x;
        }
    }

    if has_placeholder_sibling && leftmost_x < rect.x {
        Rect {
            x: leftmost_x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
        }
    } else {
        rect
    }
}

fn node_corner_radius(node: &BbNode) -> Option<f32> {
    explicit_uniform_corner_radius(node)
}

fn explicit_uniform_corner_radius(node: &BbNode) -> Option<f32> {
    let border = node.raw.get("border")?;
    let radii = ["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"]
        .into_iter()
        .map(|corner| {
            border
                .get(corner)
                .and_then(|value| value.get("radius"))
                .and_then(|value| value.get("value"))
                .and_then(|value| value.as_f64())
                .map(|value| value as f32)
        })
        .collect::<Option<Vec<_>>>()?;

    let first = *radii.first()?;
    if first <= 0.0 || radii.iter().any(|radius| (*radius - first).abs() > f32::EPSILON) {
        return None;
    }

    Some(first)
}

fn is_footer_brand_label_context(scene: &BbScene, mut parent_id: BbNodeId) -> bool {
    loop {
        let Some(parent) = scene.nodes.get(&parent_id) else {
            return false;
        };

        let mut has_logo = false;
        let mut has_bottom_bar = false;
        for child_id in &parent.children {
            let Some(child) = scene.nodes.get(child_id) else {
                continue;
            };
            if node_type_name(&child.ty)
                .eq_ignore_ascii_case("BuildingBlocks_WidgetManufacturerLogo")
            {
                has_logo = true;
            }

            let image_path = child
                .raw
                .get("ImagePath")
                .or_else(|| child.raw.get("imagePath"))
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if image_path.contains("bottom-bar") {
                has_bottom_bar = true;
            }
        }

        if has_logo && has_bottom_bar {
            return true;
        }

        if let Some(next_parent) = parent.parent {
            parent_id = next_parent;
        } else {
            return false;
        }
    }
}

fn background_fill_colour_token_from_raw(raw: &serde_json::Value, allow_fill_colour: bool) -> Option<String> {
    raw.get("BackgroundColorToken")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            if !allow_fill_colour {
                return None;
            }
            raw.get("FillColorToken")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            raw.get("background")
                .and_then(|background| background.get("color"))
                .and_then(colour_style_token)
                .or_else(|| raw.get("BackgroundColor").and_then(colour_style_token))
                .or_else(|| {
                    if allow_fill_colour {
                        raw.get("FillColor").and_then(colour_style_token)
                    } else {
                        None
                    }
                })
        })
}

fn raw_background_enabled(raw: &serde_json::Value) -> bool {
    if raw.get("BackgroundColor").is_some()
        || raw
            .get("BackgroundColorToken")
            .and_then(|token| token.as_str())
            .is_some_and(|token| !token.trim().is_empty())
    {
        return true;
    }

    if raw
        .get("EnableBackground")
        .and_then(|enable| enable.as_bool())
        .unwrap_or(false)
    {
        return true;
    }

    raw.get("background")
        .and_then(|background| background.get("enable"))
        .and_then(|enable| enable.as_bool())
        .unwrap_or(false)
}

fn border_from_node(node: &BbNode) -> Option<UiIrBorder> {
    let border = node.border.as_ref()?;
    Some(UiIrBorder {
        top: UiIrBorderSide {
            width: border.top.width,
            colour: border.top.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Top"),
        },
        right: UiIrBorderSide {
            width: border.right.width,
            colour: border.right.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Right"),
        },
        bottom: UiIrBorderSide {
            width: border.bottom.width,
            colour: border.bottom.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Bottom"),
        },
        left: UiIrBorderSide {
            width: border.left.width,
            colour: border.left.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Left"),
        },
    })
}

fn border_colour_token_from_raw(raw: &serde_json::Value, side: &str) -> Option<String> {
    raw.get(format!("BorderColor{side}Token"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            raw.get("BorderColorToken")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            raw.get("border")
                .and_then(|border| border.get(side.to_ascii_lowercase()))
                .and_then(|value| value.get("color"))
                .and_then(colour_style_token)
        })
}

fn stroke_colour_from_raw(raw: &serde_json::Value) -> Option<[f32; 4]> {
    let obj = raw.get("StrokeColor")?.as_object()?;
    let r = obj.get("r").and_then(|v| v.as_f64())? as f32;
    let g = obj.get("g").and_then(|v| v.as_f64())? as f32;
    let b = obj.get("b").and_then(|v| v.as_f64())? as f32;
    let a = obj.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some([r, g, b, a])
}

fn segmented_fill_from_raw(node: &crate::bb_scene::BbNode) -> Option<UiIrSegmentedFill> {
    let raw = &node.raw;
    let segmented_raw = raw.get("segmentedFill");

    let enabled = raw
        .get("EnableSegmentedFill")
        .and_then(|v| v.as_bool())
        .or_else(|| segmented_raw.and_then(|sf| sf.get("enable")).and_then(|v| v.as_bool()))
        .unwrap_or(false);

    if !enabled {
        return None;
    }

    let angle = raw
        .get("SegmentAngle")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("angle"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .or_else(|| {
            node.background
                .as_ref()
                .and_then(|bg| bg.segmented_fill.as_ref())
                .map(|fill| fill.angle)
        })
        .unwrap_or(0.0);

    let segment_size = raw
        .get("SegmentSize")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("segmentSize"))
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .unwrap_or(64.0);

    let segment_spacing_size = raw
        .get("SegmentSpacingSize")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("spaceSize"))
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .unwrap_or(64.0);

    let segment_x_offset = raw
        .get("SegmentXOffset")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("xOffset"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .unwrap_or(0.0);

    let segmented_bar_fill = raw
        .get("EnableSegmentedBarFill")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("barFill"))
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);

    let segment_colour = raw
        .get("SegmentColor")
        .and_then(parse_raw_colour)
        .or_else(|| segmented_raw.and_then(|sf| sf.get("segmentColor")).and_then(parse_raw_colour));

    let segment_colour_token = raw
        .get("SegmentColor")
        .and_then(|v| v.get("color"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("segmentColor"))
                .and_then(|v| v.get("color"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        });

    if segment_size <= 0.0 {
        return None;
    }

    Some(UiIrSegmentedFill {
        enabled,
        angle,
        segment_size,
        segment_spacing_size,
        segment_x_offset,
        segmented_bar_fill,
        segment_colour,
        segment_colour_token,
    })
}

fn parse_raw_colour(value: &serde_json::Value) -> Option<[f32; 4]> {
    let r = value.get("r").and_then(|v| v.as_f64())? as f32;
    let g = value.get("g").and_then(|v| v.as_f64())? as f32;
    let b = value.get("b").and_then(|v| v.as_f64())? as f32;
    let a = value.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        Some([r / 255.0, g / 255.0, b / 255.0, a / 255.0])
    } else {
        Some([r, g, b, a])
    }
}

fn parse_nine_slice_rect(value: &serde_json::Value) -> Option<[f32; 4]> {
    let left = value.get("left")?.as_f64()? as f32;
    let top = value.get("top")?.as_f64()? as f32;
    let right = value.get("right")?.as_f64()? as f32;
    let bottom = value.get("bottom")?.as_f64()? as f32;
    Some([left, top, right, bottom])
}

fn stroke_colour_token_from_raw(raw: &serde_json::Value) -> Option<String> {
    raw.get("StrokeColorToken")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| raw.get("StrokeColor").and_then(colour_style_token))
}

fn icon_tint_colour_token_from_raw(raw: &serde_json::Value, allow_fill_colour: bool) -> Option<String> {
    raw.get("iconProperties")
        .and_then(|properties| properties.get("color"))
        .and_then(colour_style_token)
        .or_else(|| {
            allow_fill_colour.then(|| {
                raw.get("FillColorToken")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(str::to_owned)
            }).flatten()
        })
        .or_else(|| {
            allow_fill_colour.then(|| raw.get("FillColor").and_then(colour_style_token)).flatten()
        })
}

fn text_colour_token_from_raw(raw: &serde_json::Value) -> Option<String> {
    raw.get("FillColorToken")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| raw.get("textColor").and_then(colour_style_token))
        .or_else(|| raw.get("textColour").and_then(colour_style_token))
        .or_else(|| raw.get("FillColor").and_then(colour_style_token))
}

fn default_style_text_colour_token_from_raw(
    raw: &serde_json::Value,
    node_type: &BbNodeType,
    is_secondary: bool,
) -> Option<String> {
    if is_secondary {
        return None;
    }
    if !node_type_name(node_type).eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair") {
        return None;
    }

    raw.get("__BrandIdentifier")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|identifier| {
            let lower = identifier.to_ascii_lowercase();
            lower.starts_with("s_") || lower.starts_with("gen_")
        })
        .map(|_| "Base".to_string())
}

fn colour_style_token(value: &serde_json::Value) -> Option<String> {
    value
        .get("_Type_")
        .and_then(|v| v.as_str())
        .filter(|ty| *ty == "BuildingBlocks_ColorStyle")?;

    value
        .get("color")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

/// Extract `FillColor` from `node.raw` as a text foreground colour fallback.
///
/// `bb_brand_apply` stores brand-applied colours as `{r, g, b, a}` float objects
/// in `node.raw["FillColor"]`. For text nodes, `FillColor` represents the text
/// foreground colour. `parse_text` reads only `textColor`/`textColour` and misses
/// brand-applied colours, so this provides the fallback.
fn fill_colour_from_raw_for_text(raw: &serde_json::Value) -> Option<[f32; 4]> {
    let obj = raw.get("FillColor")?.as_object()?;
    let r = obj.get("r").and_then(|v| v.as_f64())? as f32;
    let g = obj.get("g").and_then(|v| v.as_f64())? as f32;
    let b = obj.get("b").and_then(|v| v.as_f64())? as f32;
    let a = obj.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some([r, g, b, a])
}

fn resolve_effective_font_size(
    node_id: BbNodeId,
    node: &crate::bb_scene::BbNode,
    text: &crate::bb_scene::BbText,
    node_rect_h: f32,
    resolved_text: Option<&str>,
    scene: &BbScene,
    binding_resolver: &BindingResolver,
    defaults: &DefaultValueRegistry,
    style_font_sizes: &HashMap<String, f32>,
) -> UiIrValue {
    let label_style = label_style_name_from_node_or_ancestors(node_id, node, scene);

    // Prefer the latest raw font size because style/brand modifiers are applied
    // after parse_text and can update FontSize without mutating BbText.
    if let Some(raw_font_size) = font_size_from_raw(node) {
        return apply_label_style_font_scale(raw_font_size, label_style.as_deref());
    }

    // Numeric binding operations can drive FontSize without mutating node.raw.
    if let Some(bound_font_size) = binding_resolver.resolve_field_number(node_id, "FontSize", defaults) {
        if bound_font_size.is_finite() && bound_font_size > 0.0 {
            return apply_label_style_font_scale(
                UiIrValue::Fixed {
                value: bound_font_size as f32,
                },
                label_style.as_deref(),
            );
        }
    }

    // If this node has no explicit size, borrow the scene-derived size for its
    // label style (when any sibling with the same style has an authored FontSize).
    if let Some(style_name) = label_style.as_deref() {
        if let Some(size) = style_font_sizes.get(style_name) {
            return apply_label_style_font_scale(
                UiIrValue::Fixed { value: *size },
                Some(style_name),
            );
        }
    }

    if let Some(size) = textfield_fallback_font_size_from_signals(node, node_rect_h, resolved_text) {
        return apply_label_style_font_scale(
            UiIrValue::Fixed { value: size },
            label_style.as_deref(),
        );
    }

    // Fall through to whatever parse_text stored.
    apply_label_style_font_scale(convert_bb_value(&text.font_size), label_style.as_deref())
}

fn apply_label_style_font_scale(value: UiIrValue, label_style: Option<&str>) -> UiIrValue {
    let scale: f32 = match label_style {
        Some("Title3") => 1.15,
        Some("Heading2") => 0.8,
        _ => 1.0,
    };

    if (scale - 1.0).abs() <= f32::EPSILON {
        return value;
    }

    match value {
        UiIrValue::Fixed { value } => UiIrValue::Fixed {
            value: value * scale,
        },
        other => other,
    }
}

fn font_size_from_raw(node: &crate::bb_scene::BbNode) -> Option<UiIrValue> {
    let value = node
        .raw
        .get("fontSize")
        .or_else(|| node.raw.get("FontSize"))
        .or_else(|| {
            node.raw
                .get("modifiers")
                .and_then(|mods| mods.get("fontSize").or_else(|| mods.get("FontSize")))
        })?;

    if let Some(number) = value.as_f64() {
        return Some(UiIrValue::Fixed {
            value: number as f32,
        });
    }

    let obj = value.as_object()?;
    let raw_value = obj.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let behavior = obj
        .get("behavior")
        .and_then(|b| b.as_str())
        .unwrap_or("Fixed");
    Some(match behavior {
        "Fixed" => UiIrValue::Fixed { value: raw_value },
        "Percent" => UiIrValue::Percent { value: raw_value },
        other => UiIrValue::Other {
            value: raw_value,
            behavior: other.to_owned(),
        },
    })
}

fn font_size_fixed_value_from_raw(node: &crate::bb_scene::BbNode) -> Option<f32> {
    match font_size_from_raw(node)? {
        UiIrValue::Fixed { value } => Some(value),
        _ => None,
    }
}

fn label_style_name_from_raw(node: &crate::bb_scene::BbNode) -> Option<String> {
    node.raw
        .get("labelProperties")
        .and_then(|v| v.get("style"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn resolved_text_from_payload(payload: &UiIrTextPayload) -> Option<&str> {
    match payload {
        UiIrTextPayload::Resolved { text } => Some(text.as_str()),
        _ => None,
    }
}

fn textfield_fallback_font_size_from_signals(
    node: &crate::bb_scene::BbNode,
    node_rect_h: f32,
    resolved_text: Option<&str>,
) -> Option<f32> {
    if !matches!(node.ty, BbNodeType::WidgetTextField) {
        return None;
    }

    let style = label_style_name_from_raw(node)?;
    let text_len = resolved_text
        .map(|value| value.trim().chars().count())
        .unwrap_or(0);

    match style.as_str() {
        "Title4" => Some(56.0),
        "Title3" => Some(90.0),
        "Heading3" => Some(21.0),
        "Heading2" if node_rect_h >= 220.0 && text_len > 0 && text_len <= 4 => Some(90.0),
        "Heading2" if node_rect_h <= 80.0 => Some(28.0),
        "Heading2" => Some(18.0),
        "Heading6" if node_rect_h >= 48.0 => Some(18.0),
        "Heading1" if text_len >= 32 && node_rect_h <= 120.0 => Some(27.9),
        "Heading1" if node_rect_h <= 80.0 => Some(28.0),
        "Heading1" => Some(40.0),
        _ => None,
    }
}

fn label_style_name_from_node_or_ancestors(
    node_id: BbNodeId,
    node: &crate::bb_scene::BbNode,
    scene: &BbScene,
) -> Option<String> {
    if let Some(style) = label_style_name_from_raw(node) {
        return Some(style);
    }

    let mut current = scene.nodes.get(&node_id).and_then(|n| n.parent);
    while let Some(parent_id) = current {
        let parent = scene.nodes.get(&parent_id)?;
        if let Some(style) = label_style_name_from_raw(parent) {
            return Some(style);
        }
        current = parent.parent;
    }
    None
}

fn collect_style_font_sizes(scene: &BbScene) -> HashMap<String, f32> {
    let mut values_by_style: HashMap<String, Vec<f32>> = HashMap::new();

    for node in scene.nodes.values() {
        let Some(style_name) = label_style_name_from_raw(node) else {
            continue;
        };
        let Some(value) = font_size_fixed_value_from_raw(node) else {
            continue;
        };
        if value > 0.0 {
            values_by_style.entry(style_name).or_default().push(value);
        }
    }

    values_by_style
        .into_iter()
        .map(|(style, mut values)| {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = values[values.len() / 2];
            (style, median)
        })
        .collect()
}

fn resolve_record(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    candidates: &[&str],
) -> Option<serde_json::Value> {
    let fetcher = canvas_fetcher?;
    for candidate in candidates {
        let key = candidate.trim();
        if key.is_empty() {
            continue;
        }
        if let Ok(record) = fetcher.fetch_canvas_by_path(key) {
            return Some(record);
        }
        if let Ok(record) = fetcher.fetch_canvas_by_name(key) {
            return Some(record);
        }
        if let Ok(record) = fetcher.fetch_canvas_json(key) {
            return Some(record);
        }
    }
    None
}

/// Resolve a style-tag record for IR emission.
///
/// Resolution order:
/// 1. Direct record fetch by `_RecordPath_`, `_RecordName_`, and UUID.
/// 2. If those fail and `_RecordPath_` points to `tagdatabase`, fetch the
///    tag-database record and resolve the UUID from its nested `tags[]` tree.
///
/// This keeps tag resolution in the core IR pipeline instead of requiring
/// dump-specific flattening/indexing steps.
fn resolve_style_tag_record(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    tag_reference: &serde_json::Value,
    record_path: &str,
    record_name: &str,
    tag_uuid: &str,
) -> Option<serde_json::Value> {
    let tag_db_path = record_path.trim();
    let is_tag_database_path = !tag_db_path.is_empty()
        && tag_db_path.to_ascii_lowercase().contains("tagdatabase");

    if !is_tag_database_path {
        if let Some(record) = resolve_record(canvas_fetcher, &[record_path, record_name, tag_uuid]) {
            return Some(record);
        }
    }

    let fetcher = canvas_fetcher?;
    if !is_tag_database_path {
        if let Some(record) = resolve_record(Some(fetcher), &[tag_uuid]) {
            return Some(record);
        }
    }

    if !is_tag_database_path {
        return None;
    }

    let tag_db = fetcher.fetch_canvas_by_path(tag_db_path).ok()?;
    let tags = tag_db
        .get("_RecordValue_")
        .and_then(|rv| rv.get("tags"))
        .and_then(|v| v.as_array())?;

    let tag_value = tags.iter().find_map(|tag| find_tag_in_tree(tag, tag_uuid))?;
    let resolved_record_name = tag_reference
        .get("_RecordName_")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Tag.{tag_uuid}"));

    Some(serde_json::json!({
        "_RecordId_": tag_uuid,
        "_RecordName_": resolved_record_name,
        "_RecordPath_": tag_db_path,
        "_Type_": "Tag",
        "_RecordValue_": trim_tag_tree_to_matched_tag(tag_value),
    }))
}

fn find_tag_in_tree<'a>(value: &'a serde_json::Value, tag_uuid: &str) -> Option<&'a serde_json::Value> {
    let object = value.as_object()?;
    let matches = object
        .get("_RecordId_")
        .and_then(|v| v.as_str())
        .is_some_and(|id| id == tag_uuid);
    if matches {
        return Some(value);
    }

    object
        .get("children")
        .and_then(|v| v.as_array())
        .and_then(|children| children.iter().find_map(|child| find_tag_in_tree(child, tag_uuid)))
}

fn trim_tag_tree_to_matched_tag(tag_value: &serde_json::Value) -> serde_json::Value {
    let Some(object) = tag_value.as_object() else {
        return tag_value.clone();
    };

    let mut trimmed = serde_json::Map::with_capacity(object.len());
    for (key, value) in object {
        if key != "children" {
            trimmed.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(trimmed)
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

fn animation_number_keyframes(raw: &serde_json::Value, field_name: &str) -> Vec<(f64, f32)> {
    let Some(keyframes) = raw.get("animation")
        .and_then(|animation| animation.get("animationTimeline"))
        .and_then(|timeline| timeline.get("keyframes"))
        .and_then(|keyframes| keyframes.as_array())
    else {
        return Vec::new();
    };

    keyframes
        .iter()
        .flat_map(|keyframe| {
            let percent = keyframe
                .get("percent")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0);
            keyframe
                .get("modifiers")
                .and_then(|modifiers| modifiers.as_array())
                .into_iter()
                .flatten()
                .filter_map(move |modifier_data| {
                    let modifier = modifier_data.get("modifier").unwrap_or(modifier_data);
                    let is_number = modifier
                        .get("_Type_")
                        .and_then(|value| value.as_str())
                        .is_some_and(|ty| ty == "BuildingBlocks_FieldModifierNumber");
                    if !is_number {
                        return None;
                    }
                    let matches_field = modifier
                        .get("field")
                        .and_then(|value| value.as_str())
                        .is_some_and(|field| field == field_name);
                    if !matches_field {
                        return None;
                    }
                    modifier
                        .get("value")
                        .and_then(|value| value.as_f64())
                        .map(|value| (percent, value as f32))
                })
        })
        .collect()
}

fn representative_animation_alpha(raw: &serde_json::Value) -> Option<f32> {
    animation_number_keyframes(raw, "Alpha")
        .into_iter()
        .map(|(_, value)| value.clamp(0.0, 1.0))
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn sampled_animation_alpha(raw: &serde_json::Value, sample_percent: f32) -> Option<f32> {
    sampled_animation_number(raw, "Alpha", sample_percent).map(|value| value.clamp(0.0, 1.0))
}

fn sampled_animation_number(raw: &serde_json::Value, field_name: &str, sample_percent: f32) -> Option<f32> {
    let mut keyframes = animation_number_keyframes(raw, field_name);
    if keyframes.is_empty() {
        return None;
    }
    keyframes.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap_or(std::cmp::Ordering::Equal));

    let timeline_max = keyframes.last().map(|(percent, _)| *percent).unwrap_or(1.0);
    let sample = if timeline_max <= 1.0 {
        sample_percent as f64 / 100.0
    } else {
        sample_percent as f64
    };

    let first = keyframes[0];
    if sample <= first.0 {
        return Some(first.1);
    }

    for window in keyframes.windows(2) {
        let (left_percent, left_value) = window[0];
        let (right_percent, right_value) = window[1];
        if sample <= right_percent {
            let span = right_percent - left_percent;
            if span <= f64::EPSILON {
                return Some(right_value);
            }
            let t = ((sample - left_percent) / span) as f32;
            return Some(left_value + (right_value - left_value) * t);
        }
    }

    keyframes.last().map(|(_, value)| *value)
}

fn is_transient_static_pulse_node(node: &BbNode) -> bool {
    if !node_type_name(&node.ty).eq_ignore_ascii_case("BuildingBlocks_WidgetCircle") {
        return false;
    }

    let looping = node
        .raw
        .get("animation")
        .and_then(|animation| animation.get("loopIndefinitely"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !looping {
        return false;
    }

    let mut alpha_keyframes = animation_number_keyframes(&node.raw, "Alpha");
    if alpha_keyframes.len() < 2 {
        return false;
    }
    alpha_keyframes.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap_or(std::cmp::Ordering::Equal));

    let starts_hidden = alpha_keyframes.first().is_some_and(|(_, value)| *value <= 0.001);
    let ends_hidden = alpha_keyframes.last().is_some_and(|(_, value)| *value <= 0.001);
    let scales_over_time = !animation_number_keyframes(&node.raw, "SizeX").is_empty()
        || !animation_number_keyframes(&node.raw, "SizeY").is_empty();

    starts_hidden && ends_hidden && scales_over_time
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

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }

    #[test]
    fn resolve_style_tag_record_uses_matched_tag_instead_of_full_database() {
        let tag_db_path = "libs/foundry/records/tagdatabase/tagdatabase.tagdatabase.json";
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(tag_db_path.to_string(), serde_json::json!({
                "_RecordId_": "66ee5bfc-d90b-41bd-ad2e-e0a2b3efe359",
                "_RecordName_": "TagDatabase.TagDatabase",
                "_RecordValue_": {
                    "_Type_": "TagDatabase",
                    "tags": [
                        {
                            "_RecordId_": "parent-tag",
                            "tagName": "Parent",
                            "children": [
                                {
                                    "_RecordId_": "target-tag",
                                    "tagName": "Target",
                                    "children": [
                                        {
                                            "_RecordId_": "descendant-tag",
                                            "tagName": "Descendant"
                                        }
                                    ]
                                }
                            ]
                        }
                    ]
                }
            }))]),
        };

        let tag_reference = serde_json::json!({
            "_RecordId_": "target-tag",
            "_RecordName_": "Tag.target-tag",
            "_RecordPath_": tag_db_path,
        });

        let resolved = resolve_style_tag_record(
            Some(&fetcher),
            &tag_reference,
            tag_db_path,
            "Tag.target-tag",
            "target-tag",
        )
        .expect("tag should resolve");

        assert_eq!(resolved.get("_Type_").and_then(|v| v.as_str()), Some("Tag"));
        assert_eq!(resolved.get("_RecordId_").and_then(|v| v.as_str()), Some("target-tag"));

        let record_value = resolved.get("_RecordValue_").expect("record value");
        assert_eq!(record_value.get("_RecordId_").and_then(|v| v.as_str()), Some("target-tag"));
        assert_eq!(record_value.get("tagName").and_then(|v| v.as_str()), Some("Target"));
        assert!(record_value.get("tags").is_none(), "full tag database leaked into record");
        assert!(record_value.get("children").is_none(), "descendant tags leaked into record");
    }

    #[test]
    fn compile_ir_style_tags_are_compact() {
        let tag_db_path = "libs/foundry/records/tagdatabase/tagdatabase.tagdatabase.json";
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(tag_db_path.to_string(), serde_json::json!({
                "_RecordId_": "66ee5bfc-d90b-41bd-ad2e-e0a2b3efe359",
                "_RecordName_": "TagDatabase.TagDatabase",
                "_RecordValue_": {
                    "_Type_": "TagDatabase",
                    "tags": [
                        {
                            "_RecordId_": "target-tag",
                            "tagName": "Target"
                        }
                    ]
                }
            }))]),
        };

        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestTags",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "root",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        },
                        "styleTags": [
                            {
                                "_RecordId_": "target-tag",
                                "_RecordName_": "Tag.target-tag",
                                "_RecordPath_": tag_db_path
                            }
                        ]
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-tags",
            Some("BuildingBlocks_Canvas.TestTags"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let actual = serde_json::to_value(ir).expect("serialize ir");
        let style_tag = actual
            .get("nodes")
            .and_then(|v| v.as_array())
            .and_then(|nodes| nodes.first())
            .and_then(|node| node.get("resolved_style_tags"))
            .and_then(|v| v.as_array())
            .and_then(|tags| tags.first())
            .and_then(|v| v.as_object())
            .expect("resolved style tag object");

        assert_eq!(style_tag.get("uuid").and_then(|v| v.as_str()), Some("target-tag"));
        assert_eq!(style_tag.get("tag_name").and_then(|v| v.as_str()), Some("Target"));
        assert_eq!(style_tag.len(), 2, "resolved style tags should serialize only essential fields");
    }

    #[test]
    fn collect_node_asset_refs_includes_brand_applied_paths() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAssets",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "asset_node",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.values_mut().next().expect("node");
        node.raw
            .as_object_mut()
            .expect("raw object")
            .insert(
                "ImagePath".to_string(),
                serde_json::Value::String(
                    "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_bottom-bar.tif".to_string(),
                ),
            );
        node.raw
            .as_object_mut()
            .expect("raw object")
            .insert(
                "SvgPath".to_string(),
                serde_json::Value::String(
                    "UI/Textures/Vector/General/BrandLogos/logo_bioticorp_a.svg".to_string(),
                ),
            );

        let asset_refs = collect_node_asset_refs(node);
        assert_eq!(
            asset_refs,
            vec![
                "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_bottom-bar.tif".to_string(),
                "UI/Textures/Vector/General/BrandLogos/logo_bioticorp_a.svg".to_string(),
            ]
        );
    }

    #[test]
    fn compile_ir_emits_semantic_colour_tokens() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestColours",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "isActive": true,
                        "text": "HELLO",
                        "textColor": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Bright",
                            "alpha": 1.0
                        },
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Accent2",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetIcon",
                        "name": "icon",
                        "isActive": true,
                        "iconProperties": {
                            "customIcon": "UI/Textures/icon.svg",
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Accent3",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape",
                        "isActive": true,
                        "renderShape": true,
                        "svgPath": "UI/Textures/Vector/General/FingerPrint.svg",
                        "FillColor": {"r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-colours",
            Some("BuildingBlocks_Canvas.TestColours"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let label = ir.nodes.iter().find(|node| node.name == "label").expect("label node");
        assert_eq!(label.background_fill_colour, None);
        assert_eq!(label.background_fill_colour_token.as_deref(), Some("Accent2"));
        assert_eq!(
            label.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            Some("Bright")
        );

        let icon = ir.nodes.iter().find(|node| node.name == "icon").expect("icon node");
        assert_eq!(icon.icon_tint_colour, None);
        assert_eq!(icon.icon_tint_colour_token.as_deref(), Some("Accent3"));

        let shape = ir.nodes.iter().find(|node| node.name == "shape").expect("shape node");
        assert_eq!(shape.icon_tint_colour, Some([0.25, 0.5, 0.75, 1.0]));
    }

    #[test]
    fn compile_ir_limits_style_palette_text_default_to_label_caption() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestStylePaletteTextColour",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "palette_text",
                        "isActive": true,
                        "text": "PATIENT NAME",
                        "__BrandIdentifier": "s_bioc",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_ComponentLabelCaptionPair",
                        "name": "palette_pair",
                        "isActive": true,
                        "__BrandIdentifier": "s_bioc",
                        "labelProperties": {
                            "label": "PATIENT NAME",
                            "style": "Heading3"
                        },
                        "alignment": "Left",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-style-palette-text-colour",
            Some("BuildingBlocks_Canvas.TestStylePaletteTextColour"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "palette_text").expect("palette text node");
        assert_eq!(
            node.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            None
        );

        let pair = ir.nodes.iter().find(|node| node.name == "palette_pair").expect("palette pair node");
        assert_eq!(
            pair.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            Some("Base")
        );
    }

    #[test]
    fn compile_ir_keeps_asset_fill_colour_as_tint_not_background() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAssetFillTint",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "bottom_bar",
                        "isActive": true,
                        "ImagePath": "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_bottom-bar.tif",
                        "FillColor": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Base",
                            "alpha": 1.0
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-asset-fill-tint",
            Some("BuildingBlocks_Canvas.TestAssetFillTint"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "bottom_bar").expect("bottom bar node");
        assert_eq!(node.background_fill_colour, None);
        assert_eq!(node.background_fill_colour_token.as_deref(), None);
        assert_eq!(node.icon_tint_colour_token.as_deref(), Some("Base"));
    }

    #[test]
    fn compile_ir_does_not_draw_fill_colour_on_disabled_background_containers() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestDisabledContainerFillTint",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "card_root",
                        "isActive": true,
                        "background": {
                            "enable": false,
                            "color": null
                        },
                        "FillColor": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Bright",
                            "alpha": 1.0
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "enabled_panel",
                        "isActive": true,
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Bright",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-disabled-container-fill-tint",
            Some("BuildingBlocks_Canvas.TestDisabledContainerFillTint"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let card_root = ir.nodes.iter().find(|node| node.name == "card_root").expect("card root node");
        assert_eq!(card_root.background_fill_colour, None);
        assert_eq!(card_root.background_fill_colour_token, None);

        let enabled_panel = ir.nodes.iter().find(|node| node.name == "enabled_panel").expect("enabled panel node");
        assert_eq!(enabled_panel.background_fill_colour, None);
        assert_eq!(enabled_panel.background_fill_colour_token.as_deref(), Some("Bright"));
    }

    #[test]
    fn compile_ir_draws_explicit_background_color_on_disabled_base_background() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestExplicitBackgroundColor",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "style_background_panel",
                        "isActive": true,
                        "background": {
                            "enable": false,
                            "color": null
                        },
                        "BackgroundColor": {"r": 0.015, "g": 0.031, "b": 0.09, "a": 0.5},
                        "BackgroundColorToken": "Background",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-explicit-background-color",
            Some("BuildingBlocks_Canvas.TestExplicitBackgroundColor"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let panel = ir.nodes.iter().find(|node| node.name == "style_background_panel").expect("panel node");
        assert_eq!(panel.background_fill_colour, Some([0.015, 0.031, 0.09, 0.5]));
        assert_eq!(panel.background_fill_colour_token.as_deref(), Some("Background"));
    }

    #[test]
    fn compile_ir_uses_representative_animation_alpha_for_static_render() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAnimationStart",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCircle",
                        "name": "pulse",
                        "isActive": true,
                        "alpha": 1.0,
                        "strokeExtent": 3.0,
                        "animation": {
                            "animationTimeline": {
                                "keyframes": [
                                    {
                                        "percent": 0.0,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            },
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "SizeX",
                                                    "value": 0.44
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.4,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 1.0
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.8,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            },
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "SizeX",
                                                    "value": 1.33
                                                }
                                            }
                                        ]
                                    }
                                ]
                            },
                            "loopIndefinitely": true
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "isActive": true,
                        "alpha": 1.0,
                        "text": "TOUCH TO START",
                        "animation": {
                            "animationTimeline": {
                                "keyframes": [
                                    {
                                        "percent": 0.0,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.33,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.66
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.99,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-animation-start",
            Some("BuildingBlocks_Canvas.TestAnimationStart"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let pulse = ir.nodes.iter().find(|node| node.name == "pulse").expect("pulse node");
        assert_eq!(pulse.alpha, 0.0);
        assert_eq!(pulse.stroke_extent, Some(3.0));

        let label = ir.nodes.iter().find(|node| node.name == "label").expect("label node");
        assert_eq!(label.alpha, 0.66);
    }

    #[test]
    fn compile_ir_samples_animation_alpha_at_requested_percent() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAnimationSample",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "fingerprint_like",
                        "isActive": true,
                        "alpha": 1.0,
                        "text": "SCAN",
                        "animation": {
                            "animationTimeline": {
                                "keyframes": [
                                    {
                                        "percent": 0.0,
                                        "modifiers": [{"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "Alpha", "value": 0.0}}]
                                    },
                                    {
                                        "percent": 1.0,
                                        "modifiers": [{"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "Alpha", "value": 1.0}}]
                                    }
                                ]
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene_with_animation_sample(
            &scene,
            None,
            "guid-animation-sample",
            Some("BuildingBlocks_Canvas.TestAnimationSample"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            Some(50.0),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "fingerprint_like").expect("sample node");
        assert!((node.alpha - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn compile_ir_emits_border_and_stroke_metadata() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestBorderStroke",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "bordered",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        },
                        "border": {
                            "top": {"width": 2.0, "color": {"r": 255.0, "g": 0.0, "b": 0.0, "a": 255.0}},
                            "right": {"width": 3.0},
                            "bottom": {"width": 4.0},
                            "left": {"width": 5.0}
                        },
                        "StrokeColor": {"r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0},
                        "StrokeColorToken": "Accent4",
                        "strokeExtent": 1.5,
                        "BorderColorToken": "Accent1",
                        "BorderColorRightToken": "Accent2"
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-border-stroke",
            Some("BuildingBlocks_Canvas.TestBorderStroke"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "bordered").expect("bordered node");
        let border = node.border.as_ref().expect("border metadata");
        assert_eq!(border.top.width, 2.0);
        assert_eq!(border.top.colour, Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(border.top.colour_token.as_deref(), Some("Accent1"));
        assert_eq!(border.right.colour_token.as_deref(), Some("Accent2"));
        assert_eq!(node.stroke_colour, Some([0.25, 0.5, 0.75, 1.0]));
        assert_eq!(node.stroke_colour_token.as_deref(), Some("Accent4"));
        assert_eq!(node.stroke_extent, Some(1.5));
    }

    #[test]
    fn compile_ir_reads_svg_fill_stroke_extent() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestSvgFillStrokeExtent",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "separator",
                        "isActive": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        },
                        "svgFill": {
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "strokeExtent": 1.0
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-svg-fill-stroke-extent",
            Some("BuildingBlocks_Canvas.TestSvgFillStrokeExtent"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "separator").expect("separator node");
        assert_eq!(node.stroke_extent, Some(1.0));
    }

    #[test]
    fn compile_ir_emits_segmented_fill_metadata() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestSegmentedFill",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "segmented",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        },
                        "background": {
                            "enable": false,
                            "color": null
                        },
                        "segmentedFill": {
                            "enable": true,
                            "angle": 22.5
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-segmented",
            Some("BuildingBlocks_Canvas.TestSegmentedFill"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "segmented").expect("segmented node");
        let segmented = node.segmented_fill.as_ref().expect("segmented fill metadata");
        assert!(segmented.enabled);
        assert_eq!(segmented.angle, 22.5);
        assert_eq!(segmented.segment_size, 64.0);
        assert_eq!(segmented.segment_spacing_size, 64.0);
    }

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
            None,
            "guid-1",
            Some("BuildingBlocks_Canvas.Test"),
            (200, 100),
            &defaults(),
            None,
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
            None,
            "guid-2",
            None,
            (128, 128),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );
        let ir2 = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-2",
            None,
            (128, 128),
            &defaults(),
            None,
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
            selected_swf_source: None,
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
                corner_radius: None,
                background_fill_colour_token: None,
                segmented_fill: None,
                border: None,
                stroke_colour: None,
                stroke_colour_token: None,
                stroke_extent: None,
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None,
                asset_ref: None,
                custom_shape: None,
                style_tag_uuids: Vec::new(),
                resolved_style_tags: Vec::new(),
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
            None,
            "guid-3",
            None,
            (100, 100),
            &defaults(),
            None,
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

    #[test]
    fn compile_ir_suppresses_placeholder_only_label_caption_pairs() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PlaceholderLabelCaption",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_ComponentLabelCaptionPair",
                        "name": "OperatorName",
                        "isActive": true,
                        "labelProperties": {
                            "label": "@med_Header_OperatorName",
                            "style": "Heading3",
                            "caseModifier": "Upper",
                            "anchorToParentX": 0.5,
                            "anchorToParentY": 0.5
                        },
                        "captionProperties": {
                            "caption": "@LOC_PLACEHOLDER",
                            "style": "Heading6",
                            "caseModifier": "None"
                        },
                        "size": {
                            "width": {"behavior": "Auto", "value": 64.0},
                            "height": {"behavior": "Auto", "value": 64.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("med_header_operatorname", "OPERATOR NAME".to_string());
        defaults.insert_localization("loc_placeholder", String::new());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-placeholder",
            None,
            (100, 100),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert_eq!(ir.nodes.len(), 1);
        let node = &ir.nodes[0];
        assert!(!node.is_active, "placeholder-only label-caption pair should compile inactive");
        assert!(matches!(node.text_payload, Some(UiIrTextPayload::Resolved { .. })));
        assert!(node.secondary_text_payload.is_none());
    }

    #[test]
    fn compile_ir_prefers_raw_font_size_over_stale_parsed_text_size() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.RawFontSize",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.get_mut(&1).expect("node 1");
        node.raw["FontSize"] = serde_json::Value::from(42.0);

        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-raw-font-size",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 42.0 });
    }

    #[test]
    fn compile_ir_prefers_raw_font_record_over_stale_parsed_text_font_record() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.RawFontRecord",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "fontRecord": "file://./old-font.json",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.get_mut(&1).expect("node 1");
        node.raw["FontStyleRecord"] =
            serde_json::Value::from("file://./styled-font.json");

        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-raw-font-record",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(
            style.font_record.as_deref(),
            Some("file://./styled-font.json")
        );
    }

    #[test]
    fn compile_ir_uses_scene_style_font_size_when_node_font_size_is_missing() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.StyleFontSizeFallback",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "reference",
                        "text": "REF",
                        "FontSize": 40.0,
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "target",
                        "text": "TARGET",
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
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
            None,
            "guid-style-font-size-fallback",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let target = ir
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .expect("target node");
        let style = target.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 40.0 });
    }

    #[test]
    fn compile_ir_reads_font_size_from_modifiers_projection() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.ModifierFontSize",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "modifiers": {"FontSize": 50.0},
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
            None,
            "guid-modifier-font-size",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 50.0 });
    }

    #[test]
    fn compile_ir_inherits_label_style_from_ancestor_for_font_size_fallback() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.AncestorStyleFallback",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "reference",
                        "text": "REF",
                        "FontSize": 40.0,
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_ComponentDisplayWidget",
                        "name": "parent",
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "target",
                        "text": "TARGET",
                        "parent": "_PointsTo_:ptr:2",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
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
            None,
            "guid-ancestor-style-fallback",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let target = ir
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .expect("target node");
        let style = target.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 40.0 });
    }

    #[test]
    fn compile_ir_applies_hardcoded_textfield_font_size_exception() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.FontSizeException",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "labelProperties": {"style": "Heading3"},
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
            None,
            "guid-font-size-exception",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 21.0 });
    }

    #[test]
    fn compile_ir_without_selected_swf_source_uses_bb_renderer_hint() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.CustomShapeNoSwf",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape",
                        "shapeType": "line",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-custom-shape-no-swf",
            Some("BuildingBlocks_Canvas.CustomShapeNoSwf"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert_eq!(ir.renderer_hint, UiRendererHint::Bb);
        assert!(ir
            .warnings
            .iter()
            .any(|warning| warning.contains("no SWF source was resolved")));
    }

    #[test]
    fn compile_ir_with_selected_swf_source_preserves_hybrid_renderer_hint() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.CustomShapeWithSwf",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape",
                        "shapeType": "line",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-custom-shape-with-swf",
            Some("BuildingBlocks_Canvas.CustomShapeWithSwf"),
            (100, 100),
            &defaults(),
            None,
            Some("Data\\UI\\ShipInterface\\assets\\SWF\\test.swf".to_string()),
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert_eq!(ir.renderer_hint, UiRendererHint::Hybrid);
    }

    #[test]
    fn ui_ir_source_does_not_reintroduce_forbidden_hardcoded_markers() {
        let source = include_str!("ui_ir.rs");
        let forbidden = [
            ["nominal_font_size_", "from_label_style"].concat(),
            ["BG", "Dots"].concat(),
            ["MainMenu", "Canvas"].concat(),
            ["base_", "animatedelements"].concat(),
            ["apply_medical", "attract_banner_layout"].concat(),
        ];

        for marker in forbidden {
            assert!(
                !source.contains(marker.as_str()),
                "ui_ir hardcoding marker reintroduced: {marker}"
            );
        }
    }
}
