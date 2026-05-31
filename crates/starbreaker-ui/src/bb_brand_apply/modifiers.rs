use crate::bb_loc::LocFetcher;
use crate::bb_scene::{BbNode, BbValue};
use super::colors::{
    color_style_role_for_field,
    color_style_token,
    ensure_border,
    parse_color_value,
    write_color_to_raw,
    write_color_token_to_raw,
    ColorStyleRole,
};
pub(super) fn apply_inline_color_overlay(node: &mut BbNode, palette_source: &serde_json::Value) {
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
    if let Some(color) = color_value.and_then(|value| parse_color_value(value, palette_source, ColorStyleRole::Foreground)) {
        let token = color_value.and_then(color_style_token).map(str::to_owned);
        apply_color_field("FillColor", color, token.as_deref(), node);
    }
}

/// Apply a single modifier to a node.
///
/// Parses the `field._Type_` discriminator and `field.field` name, then updates
/// the appropriate typed field on `node` (or writes to `node.raw` as a fallback).
pub(super) fn apply_modifier(
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
                let role = color_style_role_for_field(field_name, node);
                if let Some(color) = parse_color_value(value, palette_source, role) {
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
                    let param_input_values = node
                        .raw
                        .get("paramInputValues")
                        .and_then(|value| value.as_array())
                        .map(|values| values.as_slice())
                        .unwrap_or(&[]);
                    crate::bb_loc::resolve_loc_string(value, param_input_values, fetcher)
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
        "BorderWidthTop" | "BorderTopWidth" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.width = value as f32;
            }
        }
        "BorderWidthRight" | "BorderRightWidth" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.right.width = value as f32;
            }
        }
        "BorderWidthBottom" | "BorderBottomWidth" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.bottom.width = value as f32;
            }
        }
        "BorderWidthLeft" | "BorderLeftWidth" => {
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
