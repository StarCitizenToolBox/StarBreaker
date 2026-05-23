//! Brand modifier application for BuildingBlocks scenes.
//!
//! Applies brand-style modifiers (SvgPath, ImagePath, FillColor, BorderColor, etc.)
//! to nodes in a `BbScene` by matching `conditionsList` tags against node
//! `style_tag_uuids`.
//!
//! # Condition-matching algorithm
//! An entry matches a node when there exists at least one `conditionsList[i]` such
//! that **every** `conditions[j]` item passes. Each condition item is one of:
//! - `BuildingBlocks_StyleSelectorConditionTag` — `tag._RecordId_` must be in
//!   `node.style_tag_uuids`.
//! - `BuildingBlocks_StyleSelectorConditionType` — `type` string must match the
//!   node widget type (e.g. `"Image"` → `WidgetImage`).
//! An entry with EMPTY or ABSENT `conditionsList` matches **every** node
//! (unconditional defaults).

use crate::bb_brand_style::BrandStyle;
use crate::bb_loc::LocFetcher;
use crate::bb_scene::{BbBorder, BbNode, BbNodeId, BbNodeType, BbScene, BbValue};

/// Apply brand-style modifiers to a scene.
///
/// For each node in `scene.nodes`, tests whether any `brand.entries[]` match the
/// node's `style_tag_uuids`, then applies all non-canvas-reference modifiers from
/// matching entries to the node.
///
/// Canvas-reference modifiers (those whose `field._Type_` ends with
/// `CanvasReferenceRecord`) are skipped — these are already handled by the
/// resolve pass in `bb_resolve.rs`.
///
/// When `loc_fetcher` is `Some`, string modifier values that start with `@` are
/// resolved through the localization fetcher before being written to the node.
pub fn apply_brand_modifiers(
    scene: &mut BbScene,
    brand: &BrandStyle<'_>,
    loc_fetcher: Option<&dyn LocFetcher>,
) {
    let style_probe = std::env::var("BB_A3_STYLE_PROBE").as_deref() == Ok("1");
    let node_ids: Vec<_> = scene.nodes.keys().copied().collect();
    for node_id in node_ids {
        let matching_entries: Vec<&serde_json::Value> = {
            let Some(node) = scene.nodes.get(&node_id) else {
                continue;
            };
            let matches: Vec<&serde_json::Value> = brand
                .entries
                .iter()
                .filter(|entry| entry_matches_scene(entry, node_id, node, scene))
                .collect();
            if style_probe {
                let matched_names: Vec<&str> = matches
                    .iter()
                    .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
                    .collect();
                log::info!(
                    "A3-style-probe: id=ptr:{} name={:?} tags={:?} matches={:?}",
                    node_id,
                    node.name,
                    node.style_tag_uuids,
                    matched_names
                );
            }
            matches
        };

        let Some(node) = scene.nodes.get_mut(&node_id) else {
            continue;
        };
        if looks_like_style_brand_identifier(&brand.identifier) {
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    "__BrandIdentifier".to_string(),
                    serde_json::Value::String(brand.identifier.clone()),
                )
            });
        }
        apply_inline_color_overlay(node, brand.raw);
        resolve_node_background_color(node, brand.raw);
        for entry in &matching_entries {
            apply_entry_modifiers(entry, node, brand.raw, loc_fetcher);
            record_applied_style_entry(node, entry);
        }
    }
}

fn record_applied_style_entry(node: &mut BbNode, entry: &serde_json::Value) {
    let Some(obj) = node.raw.as_object_mut() else {
        return;
    };
    let slot = obj
        .entry("__AppliedStyleEntries".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let serde_json::Value::Array(items) = slot {
        items.push(entry.clone());
    }
}

fn looks_like_style_brand_identifier(identifier: &str) -> bool {
    let lower = identifier.to_ascii_lowercase();
    lower.starts_with("s_") || lower.starts_with("gen_")
}

fn resolve_node_background_color(node: &mut BbNode, palette_source: &serde_json::Value) {
    if node
        .background
        .as_ref()
        .and_then(|bg| bg.fill_colour)
        .is_some()
    {
        return;
    }

    let background_color_value = node.raw.get("BackgroundColor");
    let style_color_value = node
        .raw
        .get("background")
        .and_then(|bg| bg.get("color"))
        .filter(|v| {
            v.get("_Type_")
                .and_then(|ty| ty.as_str())
                .is_some_and(|ty| ty == "BuildingBlocks_ColorStyle")
        });

    let Some(color_value) = background_color_value.or(style_color_value) else {
        return;
    };
    let Some(color) = parse_color_value(color_value, palette_source) else {
        return;
    };

    if node.background.is_none() {
        node.background = Some(Default::default());
    }
    if let Some(bg) = node.background.as_mut() {
        bg.fill_colour = Some(color);
    }
}

/// Test whether a brand-style entry matches a node within a scene.
///
/// An entry matches when:
/// - Its `conditionsList` is absent or empty (unconditional), OR
/// - There exists at least one `conditionsList[i]` such that **all**
///   `conditions[j]` items pass. Conditions may be nested (`AllOf`, `AnyOf`,
///   `Parent`), and parent conditions are evaluated against the node's direct
///   parent in the parsed BB scene hierarchy.
fn entry_matches_scene(
    entry: &serde_json::Value,
    node_id: BbNodeId,
    node: &BbNode,
    scene: &BbScene,
) -> bool {
    let conditions_list = match entry.get("conditionsList").and_then(|v| v.as_array()) {
        Some(cl) => cl,
        None => return true,
    };

    if conditions_list.is_empty() {
        return true;
    }

    conditions_list.iter().any(|conditions_block| {
        let Some(conditions) = conditions_block.get("conditions").and_then(|v| v.as_array()) else {
            return false;
        };
        conditions
            .iter()
            .all(|condition| condition_matches_node(condition, node_id, node, scene))
    })
}

fn condition_matches_node(
    condition: &serde_json::Value,
    node_id: BbNodeId,
    node: &BbNode,
    scene: &BbScene,
) -> bool {
    let cond_type = condition
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if cond_type.ends_with("ConditionAllOfCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions
                    .iter()
                    .all(|child| condition_matches_node(child, node_id, node, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionAnyOfCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions
                    .iter()
                    .any(|child| condition_matches_node(child, node_id, node, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionNotCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                !conditions
                    .iter()
                    .any(|child| condition_matches_node(child, node_id, node, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionParent") {
        let Some(parent_id) = node.parent else {
            return false;
        };
        let Some(parent) = scene.nodes.get(&parent_id) else {
            return false;
        };
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions
                    .iter()
                    .all(|child| condition_matches_node(child, parent_id, parent, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionAncestor") {
        let Some(conditions) = condition.get("conditions").and_then(|v| v.as_array()) else {
            return false;
        };
        let mut current = node.parent;
        while let Some(ancestor_id) = current {
            let Some(ancestor) = scene.nodes.get(&ancestor_id) else {
                break;
            };
            if conditions
                .iter()
                .all(|child| condition_matches_node(child, ancestor_id, ancestor, scene))
            {
                return true;
            }
            current = ancestor.parent;
        }
        return false;
    }

    if cond_type.ends_with("ConditionAnyOfTag") {
        let Some(tags) = condition.get("tags").and_then(|v| v.as_array()) else {
            return false;
        };
        return tags.iter().filter_map(tag_ref_id).any(|tag_id| {
            node.style_tag_uuids
                .iter()
                .any(|node_tag| node_tag == tag_id)
        });
    }

    if cond_type.ends_with("ConditionTag") || condition.get("tag").is_some() {
        return condition_tag_id(condition)
            .map(|tag_id| node.style_tag_uuids.iter().any(|tag| tag == tag_id))
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionType") {
        return condition
            .get("type")
            .and_then(|v| v.as_str())
            .map(|type_str| node_type_matches(type_str, &node.ty))
            .unwrap_or(true);
    }

    false
}

fn condition_tag_id(condition: &serde_json::Value) -> Option<&str> {
    let tag = condition.get("tag")?;
    tag_ref_id(tag)
}

fn tag_ref_id(tag: &serde_json::Value) -> Option<&str> {
    tag.get("_RecordId_")
        .and_then(|v| v.as_str())
        .or_else(|| tag.as_str())
}

/// Return `true` when `type_str` from a `ConditionType` entry matches the node type.
///
/// Maps the game's short widget-family names (e.g. `"Image"`) to our `BbNodeType`
/// variants.  Unknown type strings return `false`.
fn node_type_matches(type_str: &str, ty: &BbNodeType) -> bool {
    match type_str {
        "Image" => matches!(ty, BbNodeType::WidgetImage),
        "Text" => matches!(ty, BbNodeType::WidgetText | BbNodeType::WidgetTextField),
        "Canvas" => matches!(ty, BbNodeType::WidgetCanvas),
        "Icon" => matches!(ty, BbNodeType::WidgetIcon),
        "Card" => matches!(ty, BbNodeType::WidgetCard),
        "DisplayWidget" => matches!(ty, BbNodeType::DisplayWidget),
        _ => false,
    }
}

/// Apply all modifiers from a matching entry to a node.
fn apply_entry_modifiers(
    entry: &serde_json::Value,
    node: &mut BbNode,
    palette_source: &serde_json::Value,
    loc_fetcher: Option<&dyn LocFetcher>,
) {
    let modifiers = match entry.get("modifiers").and_then(|v| v.as_array()) {
        Some(m) => m,
        None => return,
    };

    for modifier in modifiers {
        apply_modifier(modifier, node, palette_source, loc_fetcher);
    }
}

fn apply_inline_color_overlay(node: &mut BbNode, palette_source: &serde_json::Value) {
    if node.raw.get("FillColor").is_some() {
        return;
    }
    let overlay_enabled = node
        .raw
        .get("enableColorOverlay")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || node
            .raw
            .get("svgFill")
            .and_then(|v| v.get("enableColorOverlay"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if !overlay_enabled {
        return;
    }

    let color_value = node
        .raw
        .get("color")
        .or_else(|| node.raw.get("svgFill").and_then(|v| v.get("color")));
    if let Some(color) = color_value.and_then(|value| parse_color_value(value, palette_source)) {
        let token = color_value.and_then(color_style_token).map(str::to_owned);
        apply_color_field("FillColor", color, token.as_deref(), node);
    }
}

/// Apply a single modifier to a node.
///
/// Parses the `field._Type_` discriminator and `field.field` name, then updates
/// the appropriate typed field on `node` (or writes to `node.raw` as a fallback).
fn apply_modifier(
    modifier: &serde_json::Value,
    node: &mut BbNode,
    palette_source: &serde_json::Value,
    loc_fetcher: Option<&dyn LocFetcher>,
) {
    let Some((type_str, field_name, value)) = modifier_parts(modifier) else {
        return;
    };

    // Skip canvas-reference modifiers (already handled by bb_resolve).
    if type_str.ends_with("CanvasReferenceRecord") {
        return;
    }

    match type_str {
        "BuildingBlocks_FieldModifierString" => {
            if let Some(value) = value.and_then(|v| v.as_str()) {
                apply_string_field(field_name, value, node, loc_fetcher);
            }
        }
        "BuildingBlocks_FieldModifierNumber" => {
            if let Some(value) = value.and_then(|v| v.as_f64()) {
                apply_number_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierColor" => {
            if let Some(value) = value {
                let token = color_style_token(value).map(str::to_owned);
                if let Some(color) = parse_color_value(value, palette_source) {
                    apply_color_field(field_name, color, token.as_deref(), node);
                }
            }
        }
        "BuildingBlocks_FieldModifierBoolean" => {
            if let Some(value) = value.and_then(|v| v.as_bool()) {
                apply_boolean_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierEnumerated"
        | "BuildingBlocks_FieldModifierEnumeratedTypeImageScalingBehavior"
        | "BuildingBlocks_FieldModifierEnumeratedTypeWidthBehavior"
        | "BuildingBlocks_FieldModifierEnumeratedTypeHeightBehavior" => {
            if let Some(value) = value.and_then(|v| v.as_str()) {
                apply_enum_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierRecordRef"
        | "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord" => {
            if let Some(value) = value.and_then(|v| v.as_str()) {
                apply_record_ref_field(field_name, value, node);
            }
        }
        _ => {
            log::debug!(
                "bb_brand_apply: unrecognised modifier type '{}' for field '{}'",
                type_str,
                field_name
            );
            if let Some(value) = value {
                node.raw
                    .as_object_mut()
                    .and_then(|obj| obj.insert(field_name.to_string(), value.clone()));
            }
        }
    }
}

fn modifier_parts(modifier: &serde_json::Value) -> Option<(&str, &str, Option<&serde_json::Value>)> {
    let modifier_type = modifier.get("_Type_").and_then(|v| v.as_str());

    match modifier.get("field")? {
        serde_json::Value::String(field_name) => {
            let type_str = modifier_type?;
            let value = if type_str == "BuildingBlocks_FieldModifierColor" {
                modifier.get("color").or_else(|| modifier.get("value"))
            } else {
                modifier.get("value")
            };
            Some((type_str, field_name.as_str(), value))
        }
        serde_json::Value::Object(field) => {
            let type_str = field
                .get("_Type_")
                .and_then(|v| v.as_str())
                .or(modifier_type)?;
            let field_name = field
                .get("field")
                .and_then(|v| v.as_str())
                .or_else(|| match type_str {
                    "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord" => Some("FontStyleRecord"),
                    _ => None,
                })
                .unwrap_or("");
            let value = field
                .get("value")
                .or_else(|| field.get("color"))
                .or_else(|| modifier.get("value"))
                .or_else(|| modifier.get("color"));
            Some((type_str, field_name, value))
        }
        _ => None,
    }
}

/// Apply a string-typed modifier field.
///
/// When `loc_fetcher` is provided and `value` starts with `@`, it is resolved
/// through the localization fetcher.  Asset-path fields (`SvgPath`, `ImagePath`)
/// are intentionally NOT resolved — they reference files, not localized strings.
fn apply_string_field(field_name: &str, value: &str, node: &mut BbNode, loc_fetcher: Option<&dyn LocFetcher>) {
    match field_name {
        "SvgPath" | "ImagePath" => {
            // Write to node.raw for the renderer to pick up.
            node.raw
                .as_object_mut()
                .and_then(|obj| obj.insert(field_name.to_string(), serde_json::Value::String(value.to_string())));
        }
        _ => {
            // Resolve @KEY localization references if a fetcher is available.
            let resolved = if value.starts_with('@') {
                if let Some(fetcher) = loc_fetcher {
                    crate::bb_loc::resolve_loc_string(value, &[], fetcher)
                } else {
                    value.to_string()
                }
            } else {
                value.to_string()
            };
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised string field '{}' = '{}'",
                field_name,
                resolved
            );
            node.raw
                .as_object_mut()
                .and_then(|obj| obj.insert(field_name.to_string(), serde_json::Value::String(resolved)));
        }
    }
}

/// Apply a number-typed modifier field.
fn apply_number_field(field_name: &str, value: f64, node: &mut BbNode) {
    match field_name {
        "SizeX" => node.sizing.width = BbValue::Fixed(value as f32),
        "SizeY" => node.sizing.height = BbValue::Fixed(value as f32),
        "AnchorX" => node.anchor.x = value as f32,
        "AnchorY" => node.anchor.y = value as f32,
        "PivotX" => node.pivot.x = value as f32,
        "PivotY" => node.pivot.y = value as f32,
        "Alpha" => node.alpha = (value as f32).clamp(0.0, 1.0),
        "BorderWidth" => {
            let width = value as f32;
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.width = width;
                border.right.width = width;
                border.bottom.width = width;
                border.left.width = width;
            }
        }
        "BorderWidthTop" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.width = value as f32;
            }
        }
        "BorderWidthRight" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.right.width = value as f32;
            }
        }
        "BorderWidthBottom" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.bottom.width = value as f32;
            }
        }
        "BorderWidthLeft" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.left.width = value as f32;
            }
        }
        "NineSliceTop" | "NineSliceBottom" | "NineSliceLeft" | "NineSliceRight" => {
            // Write to raw for renderer.
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap()),
                )
            });
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised number field '{}' = {}",
                field_name,
                value
            );
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap()),
                )
            });
        }
    }
}

/// Apply a color-typed modifier field.
fn apply_color_field(field_name: &str, color: [f32; 4], token: Option<&str>, node: &mut BbNode) {
    match field_name {
        "FillColor" | "StrokeColor" | "BackgroundColor" => {
            // Update node.background if it exists.
            if let Some(bg) = &mut node.background {
                bg.fill_colour = Some(color);
            }
            // Also write to raw for non-typed cases.
            write_color_to_raw(field_name, color, node);
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColor" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.colour = Some(color);
                border.right.colour = Some(color);
                border.bottom.colour = Some(color);
                border.left.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorTop" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorRight" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.right.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorBottom" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.bottom.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorLeft" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.left.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised color field '{}' = {:?}",
                field_name,
                color
            );
            write_color_to_raw(field_name, color, node);
            write_color_token_to_raw(field_name, token, node);
        }
    }
}

/// Apply a boolean-typed modifier field.
fn apply_boolean_field(field_name: &str, value: bool, node: &mut BbNode) {
    match field_name {
        "IsActive" => node.is_active = value,
        "EnableBackground" | "EnableColorOverlay" | "EnableNineSliceRect" => {
            // Write to raw for renderer.
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Bool(value),
                )
            });
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised boolean field '{}' = {}",
                field_name,
                value
            );
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Bool(value),
                )
            });
        }
    }
}

/// Apply an enumerated-typed modifier field.
fn apply_enum_field(field_name: &str, value: &str, node: &mut BbNode) {
    match field_name {
        "ImageScalingBehavior" | "WidthBehavior" | "HeightBehavior" => {
            // Write to raw for renderer.
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::String(value.to_string()),
                )
            });
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised enum field '{}' = '{}'",
                field_name,
                value
            );
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::String(value.to_string()),
                )
            });
        }
    }
}

/// Apply a record-ref-typed modifier field.
fn apply_record_ref_field(field_name: &str, value: &str, node: &mut BbNode) {
    // Treat as a string and store in node.raw.
    log::debug!(
        "bb_brand_apply: record-ref field '{}' = '{}'",
        field_name,
        value
    );
    node.raw.as_object_mut().and_then(|obj| {
        obj.insert(
            field_name.to_string(),
            serde_json::Value::String(value.to_string()),
        )
    });
}

/// Parse a color value from JSON.
///
/// Supports literal `{r,g,b,a}` values and `BuildingBlocks_ColorStyle` named
/// palette slots. Named slots are resolved against the style record's
/// `colorStyles[]` array using the standard BuildingBlocks slot ordering.
fn parse_color_value(value: &serde_json::Value, palette_source: &serde_json::Value) -> Option<[f32; 4]> {
    if value.get("color").and_then(|v| v.as_str()).is_some() && value.get("r").is_none() {
        return parse_named_color(value, palette_source);
    }
    value.as_object().map(parse_literal_color)
}

fn parse_literal_color(color_obj: &serde_json::Map<String, serde_json::Value>) -> [f32; 4] {
    let r = color_obj
        .get("r")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let g = color_obj
        .get("g")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let b = color_obj
        .get("b")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let a = color_obj
        .get("a")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0) as f32;

    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        [r / 255.0, g / 255.0, b / 255.0, a / 255.0]
    } else {
        [r, g, b, a]
    }
}

fn parse_named_color(value: &serde_json::Value, palette_source: &serde_json::Value) -> Option<[f32; 4]> {
    let name = color_style_token(value)?;
    let alpha = value.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    let slot = color_style_slot_index(name)?;
    let color_styles = palette_source
        .get("colorStyles")
        .or_else(|| palette_source.get("_RecordValue_").and_then(|v| v.get("colorStyles")))?
        .as_array()?;
    let color_obj = color_styles.get(slot)?.get("color")?.as_object()?;
    let mut color = parse_literal_color(color_obj);
    color[3] *= alpha.clamp(0.0, 1.0);
    Some(color)
}

fn color_style_slot_index(name: &str) -> Option<usize> {
    match name {
        "Accent1" | "Bright" | "Base" => Some(0),
        "Accent2" | "Positive" | "Success" => Some(1),
        "Accent3" | "Warning" => Some(2),
        "Accent4" | "Critical" | "Negative" => Some(3),
        "Accent5" | "ContactUnknown" => Some(4),
        "Mid" | "ContactNeutral" => Some(5),
        "Light" | "Disabled" => Some(6),
        "Highlight" | "ContactPositiveRep" => Some(7),
        "Surface" => Some(8),
        "BG" | "Background" => Some(9),
        "FG" | "Foreground" => Some(10),
        "Backlight" | "ContactAgressive" | "ContactAggressive" => Some(11),
        "PositiveState" => Some(12),
        "WarningState" => Some(13),
        "CriticalState" => Some(14),
        "Text" => Some(15),
        "Gold" | "Special" => Some(16),
        _ => None,
    }
}

fn color_style_token(value: &serde_json::Value) -> Option<&str> {
    value
        .get("color")
        .and_then(|v| v.as_str())
        .filter(|name| !name.trim().is_empty())
}

/// Ensure `node.border` is `Some(…)`, initializing to default if `None`.
fn ensure_border(node: &mut BbNode) {
    if node.border.is_none() {
        node.border = Some(BbBorder::default());
    }
}

/// Write a color to `node.raw` as an object `{r, g, b, a}`.
fn write_color_to_raw(field_name: &str, color: [f32; 4], node: &mut BbNode) {
    let color_obj = serde_json::json!({
        "r": color[0],
        "g": color[1],
        "b": color[2],
        "a": color[3],
    });
    node.raw
        .as_object_mut()
        .and_then(|obj| obj.insert(field_name.to_string(), color_obj));
}

fn write_color_token_to_raw(field_name: &str, token: Option<&str>, node: &mut BbNode) {
    let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    node.raw.as_object_mut().and_then(|obj| {
        obj.insert(
            format!("{field_name}Token"),
            serde_json::Value::String(token.to_string()),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bb_scene::{BbBackground, BbNode, BbNodeType, BbScene};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn make_test_scene() -> BbScene {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            1,
            BbNode {
                id: 1,
                parent: None,
                children: vec![],
                ty: BbNodeType::WidgetImage,
                name: "test_node".to_string(),
                style_tag_uuids: vec!["tag-uuid-1".to_string()],
                is_active: true,
                layer: 0,
                alpha: 1.0,
                position: Default::default(),
                position_offset: Default::default(),
                sizing: Default::default(),
                padding: Default::default(),
                margin: Default::default(),
                pivot: Default::default(),
                anchor: Default::default(),
                background: None,
                border: None,
                radial: None,
                text: None,
                icon: None,
                raw: json!({}),
            },
        );

        BbScene {
            canvas_size: (1920.0, 1080.0),
            roots: vec![1],
            nodes,
            operations: vec![],
        }
    }

    #[test]
    fn test_unconditional_entry_applies_to_all() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "Alpha",
                            "value": 0.5
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        assert_eq!(scene.nodes.get(&1).unwrap().alpha, 0.5);
    }

    #[test]
    fn test_conditional_entry_matches_tag() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "conditions": [
                            {
                                "tag": {
                                    "_RecordId_": "tag-uuid-1"
                                }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "Alpha",
                            "value": 0.75
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        assert_eq!(scene.nodes.get(&1).unwrap().alpha, 0.75);
    }

    #[test]
    fn test_conditional_entry_no_match() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "conditions": [
                            {
                                "tag": {
                                    "_RecordId_": "nonexistent-tag"
                                }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "Alpha",
                            "value": 0.25
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        assert_eq!(scene.nodes.get(&1).unwrap().alpha, 1.0); // Unchanged
    }

    #[test]
    fn test_string_modifier() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "SvgPath",
                            "value": "UI/Textures/test.svg"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("SvgPath").and_then(|v| v.as_str()),
            Some("UI/Textures/test.svg")
        );
    }

    #[test]
    fn test_color_modifier_0_to_1() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().background = Some(Default::default());

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierColor",
                            "field": "FillColor",
                            "value": {
                                "r": 0.5,
                                "g": 0.75,
                                "b": 1.0,
                                "a": 1.0
                            }
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        let color = node.background.as_ref().unwrap().fill_colour.unwrap();
        assert_eq!(color, [0.5, 0.75, 1.0, 1.0]);
    }

    #[test]
    fn test_color_modifier_0_to_255() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().background = Some(Default::default());

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierColor",
                            "field": "BackgroundColor",
                            "value": {
                                "r": 128.0,
                                "g": 192.0,
                                "b": 255.0,
                                "a": 255.0
                            }
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        let color = node.background.as_ref().unwrap().fill_colour.unwrap();
        // Should be normalized to 0..1
        assert!((color[0] - 128.0 / 255.0).abs() < 0.01);
        assert!((color[1] - 192.0 / 255.0).abs() < 0.01);
        assert!((color[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_named_base_color_maps_to_slot_zero() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().background = Some(Default::default());
        let palette = json!({
            "colorStyles": [
                { "color": { "r": 115, "g": 198, "b": 254, "a": 255 } }
            ]
        });
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierColor",
                    "field": "FillColor",
                    "color": {
                        "_Type_": "BuildingBlocks_ColorStyle",
                        "color": "Base",
                        "alpha": 1.0
                    }
                }]
            })],
            raw: &palette,
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let color = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        assert!((color[0] - 115.0 / 255.0).abs() < 0.001);
        assert!((color[1] - 198.0 / 255.0).abs() < 0.001);
        assert!((color[2] - 254.0 / 255.0).abs() < 0.001);
        assert_eq!(color[3], 1.0);
    }

    #[test]
    fn test_boolean_modifier_is_active() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierBoolean",
                            "field": "IsActive",
                            "value": false
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        assert_eq!(scene.nodes.get(&1).unwrap().is_active, false);
    }

    #[test]
    fn test_border_color_modifier() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierColor",
                            "field": "BorderColorTop",
                            "value": {
                                "r": 1.0,
                                "g": 0.0,
                                "b": 0.0,
                                "a": 1.0
                            }
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert!(node.border.is_some());
        let color = node.border.as_ref().unwrap().top.colour.unwrap();
        assert_eq!(color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_size_modifier() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "SizeX",
                            "value": 640.0
                        }
                    },
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierNumber",
                            "field": "SizeY",
                            "value": 480.0
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(node.sizing.width, BbValue::Fixed(640.0));
        assert_eq!(node.sizing.height, BbValue::Fixed(480.0));
    }

    #[test]
    fn test_type_condition_matches_widget_image() {
        // ConditionType "Image" must match a WidgetImage node.
        let mut scene = make_test_scene(); // node ty = WidgetImage
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleConditionList",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Image"
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/test_image.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("ImagePath").and_then(|v| v.as_str()),
            Some("UI/Textures/test_image.tif"),
            "ConditionType 'Image' must match WidgetImage node"
        );
    }

    #[test]
    fn test_type_condition_no_match_wrong_type() {
        // ConditionType "Text" must NOT match a WidgetImage node.
        let mut scene = make_test_scene(); // node ty = WidgetImage
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleConditionList",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Text"
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/should_not_apply.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert!(
            node.raw.get("ImagePath").is_none(),
            "ConditionType 'Text' must NOT match WidgetImage node"
        );
    }

    #[test]
    fn test_mixed_type_and_tag_condition_matches() {
        // Mixed AllOf condition: ConditionType "Image" + ConditionTag must both pass.
        let mut scene = make_test_scene(); // WidgetImage, style_tag_uuids = ["tag-uuid-1"]
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAllOfCondition",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Image"
                            },
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": { "_RecordId_": "tag-uuid-1" }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/DRAK_Background.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw.get("ImagePath").and_then(|v| v.as_str()),
            Some("UI/Textures/DRAK_Background.tif"),
            "Mixed type+tag condition must match WidgetImage with matching tag"
        );
    }

    #[test]
    fn test_inline_color_overlay_resolves_named_svg_tint() {
        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).unwrap();
        node.ty = BbNodeType::WidgetCustomShape;
        node.background = Some(BbBackground::default());
        node.raw = json!({
            "enableColorOverlay": true,
            "svgPath": "UI/Textures/Vector/General/FingerPrint.svg",
            "color": {
                "_Type_": "BuildingBlocks_ColorStyle",
                "color": "Accent1",
                "alpha": 1.0
            }
        });
        let style_record = json!({
            "colorStyles": [
                { "color": { "r": 115, "g": 198, "b": 254, "a": 255 } }
            ]
        });
        let brand = BrandStyle {
            identifier: "s_bioc".to_string(),
            entries: &[],
            raw: &style_record,
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let fill = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        assert!((fill[0] - 115.0 / 255.0).abs() < 0.001);
        assert!((fill[1] - 198.0 / 255.0).abs() < 0.001);
        assert!((fill[2] - 254.0 / 255.0).abs() < 0.001);
        assert_eq!(fill[3], 1.0);
    }

    #[test]
    fn test_embedded_parent_child_bright_fill_tints_svg_node() {
        let mut scene = make_test_scene();
        let parent = BbNode {
            id: 2,
            parent: None,
            children: vec![1],
            ty: BbNodeType::WidgetCanvas,
            name: "parent".to_string(),
            style_tag_uuids: vec!["parent-tag".to_string()],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Default::default(),
            position_offset: Default::default(),
            sizing: Default::default(),
            padding: Default::default(),
            margin: Default::default(),
            pivot: Default::default(),
            anchor: Default::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: json!({}),
        };
        scene.nodes.insert(2, parent);
        let child = scene.nodes.get_mut(&1).unwrap();
        child.parent = Some(2);
        child.children.clear();
        child.style_tag_uuids = vec!["fingerprint-child-tag".to_string()];
        child.background = Some(BbBackground::default());
        child.raw = json!({ "svgPath": "UI/Textures/Vector/General/FingerPrint.svg" });
        scene.roots = vec![2];

        let style_record = json!({
            "colorStyles": [
                { "color": { "r": 115, "g": 198, "b": 254, "a": 255 } }
            ]
        });
        let brand = BrandStyle {
            identifier: "embeddedStyles".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": { "_RecordId_": "fingerprint-child-tag" }
                            },
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionParent",
                                "conditions": [
                                    {
                                        "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                        "tag": { "_RecordId_": "parent-tag" }
                                    }
                                ]
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "_Type_": "BuildingBlocks_FieldModifierColor",
                        "field": "FillColor",
                        "color": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Bright",
                            "alpha": 1.0
                        }
                    }
                ]
            })],
            raw: &style_record,
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let fill = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        assert!((fill[0] - 115.0 / 255.0).abs() < 0.001);
        assert!((fill[1] - 198.0 / 255.0).abs() < 0.001);
        assert!((fill[2] - 254.0 / 255.0).abs() < 0.001);
        assert_eq!(fill[3], 1.0);
    }

    #[test]
    fn named_fill_color_preserves_token_in_raw() {
        let palette = json!({
            "colorStyles": [
                {"color": {"r": 1.0, "g": 0.5, "b": 0.25, "a": 1.0}}
            ]
        });

        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierColor",
            "field": "FillColor",
            "color": {
                "_Type_": "BuildingBlocks_ColorStyle",
                "color": "Accent1",
                "alpha": 1.0
            }
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &palette, None);

        assert_eq!(
            node.raw.get("FillColorToken").and_then(|value| value.as_str()),
            Some("Accent1")
        );
        assert!(node.raw.get("FillColor").is_some(), "resolved rgba should still be present");
    }

    #[test]
    fn record_ref_font_style_object_field_maps_to_font_style_record() {
        let palette = json!({});
        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierRecordRef",
            "field": {
                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                "value": "file://./../../fontstyles/blenderpro-bold.json"
            }
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &palette, None);

        assert_eq!(
            node.raw
                .get("FontStyleRecord")
                .and_then(|value| value.as_str()),
            Some("file://./../../fontstyles/blenderpro-bold.json")
        );
    }

    #[test]
    fn test_mixed_type_and_tag_condition_tag_mismatch() {
        // Mixed AllOf: type matches but tag doesn't → should NOT apply.
        let mut scene = make_test_scene(); // WidgetImage, tag = "tag-uuid-1"
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAllOfCondition",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Image"
                            },
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": { "_RecordId_": "wrong-tag" }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/should_not_apply.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert!(
            node.raw.get("ImagePath").is_none(),
            "Mixed type+tag condition must NOT match when tag is wrong"
        );
    }

    #[test]
    fn test_condition_ancestor_matches_grandparent_tag() {
        let mut scene = make_test_scene();
        // Re-parent node 1 under parent 2 under grandparent 3.
        let mut parent = scene.nodes.get(&1).cloned().unwrap();
        parent.id = 2;
        parent.name = "parent".to_string();
        parent.parent = Some(3);
        parent.children = vec![1];
        parent.style_tag_uuids = vec!["parent-tag".to_string()];
        parent.raw = json!({});
        scene.nodes.insert(2, parent);

        let mut grandparent = scene.nodes.get(&1).cloned().unwrap();
        grandparent.id = 3;
        grandparent.name = "grandparent".to_string();
        grandparent.parent = None;
        grandparent.children = vec![2];
        grandparent.style_tag_uuids = vec!["ancestor-tag".to_string()];
        grandparent.raw = json!({});
        scene.nodes.insert(3, grandparent);

        let child = scene.nodes.get_mut(&1).unwrap();
        child.parent = Some(2);
        child.children.clear();
        child.background = Some(BbBackground::default());
        scene.roots = vec![3];

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAncestor",
                        "conditions": [{
                            "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                            "tag": { "_RecordId_": "ancestor-tag" }
                        }]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierColor",
                    "field": "FillColor",
                    "color": { "r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0 }
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);
        let fill = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        assert!((fill[0] - 0.25).abs() < 0.001);
        assert!((fill[1] - 0.5).abs() < 0.001);
        assert!((fill[2] - 0.75).abs() < 0.001);
        assert!((fill[3] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_condition_any_of_tag_matches_when_any_tag_matches() {
        let mut scene = make_test_scene();

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAnyOfTag",
                        "tags": [
                            { "_RecordId_": "wrong-tag" },
                            { "_RecordId_": "tag-uuid-1" }
                        ]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierString",
                    "field": "ImagePath",
                    "value": "UI/Textures/any_of_tag_hit.tif"
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).expect("test node");
        assert_eq!(
            node.raw.get("ImagePath").and_then(|value| value.as_str()),
            Some("UI/Textures/any_of_tag_hit.tif")
        );
    }

    #[test]
    fn test_condition_any_of_tag_no_match_when_no_tags_match() {
        let mut scene = make_test_scene();

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAnyOfTag",
                        "tags": [
                            { "_RecordId_": "wrong-tag-a" },
                            { "_RecordId_": "wrong-tag-b" }
                        ]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierString",
                    "field": "ImagePath",
                    "value": "UI/Textures/any_of_tag_should_not_apply.tif"
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).expect("test node");
        assert!(
            node.raw.get("ImagePath").is_none(),
            "ConditionAnyOfTag should not match when node has none of the tags"
        );
    }
}
