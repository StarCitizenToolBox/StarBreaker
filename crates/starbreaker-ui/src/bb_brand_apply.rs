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
use crate::bb_scene::{BbBorder, BbNode, BbNodeType, BbScene, BbValue};

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
    for node in scene.nodes.values_mut() {
        for entry in brand.entries {
            if entry_matches_node(entry, node) {
                apply_entry_modifiers(entry, node, loc_fetcher);
            }
        }
    }
}

/// Test whether a brand-style entry matches a node.
///
/// An entry matches when:
/// - Its `conditionsList` is absent or empty (unconditional), OR
/// - There exists at least one `conditionsList[i]` such that **all**
///   `conditions[j]` items pass (tag UUIDs present AND widget-type matches).
fn entry_matches_node(entry: &serde_json::Value, node: &BbNode) -> bool {
    let conditions_list = match entry.get("conditionsList").and_then(|v| v.as_array()) {
        Some(cl) => cl,
        None => return true, // No conditionsList → matches all nodes.
    };

    if conditions_list.is_empty() {
        return true; // Empty array → matches all nodes.
    }

    // For each conditions block, check if all conditions pass for this node.
    for conditions_block in conditions_list {
        let conditions = match conditions_block
            .get("conditions")
            .and_then(|v| v.as_array())
        {
            Some(c) => c,
            None => continue,
        };

        let all_conditions_pass = conditions.iter().all(|cond| {
            let cond_type = cond.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if cond_type.ends_with("ConditionTag") || cond.get("tag").is_some() {
                // Tag condition: the tag UUID must be in node.style_tag_uuids.
                if let Some(tag_id) = cond
                    .get("tag")
                    .and_then(|t| t.get("_RecordId_"))
                    .and_then(|r| r.as_str())
                {
                    node.style_tag_uuids.contains(&tag_id.to_string())
                } else {
                    false
                }
            } else if cond_type.ends_with("ConditionType") {
                // Type condition: node widget type must match the type string.
                if let Some(type_str) = cond.get("type").and_then(|v| v.as_str()) {
                    node_type_matches(type_str, &node.ty)
                } else {
                    true // No type specified → pass.
                }
            } else {
                // Unknown condition kind → conservative fail.
                false
            }
        });

        if all_conditions_pass {
            return true; // At least one conditions block fully matched.
        }
    }

    false
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
fn apply_entry_modifiers(entry: &serde_json::Value, node: &mut BbNode, loc_fetcher: Option<&dyn LocFetcher>) {
    let modifiers = match entry.get("modifiers").and_then(|v| v.as_array()) {
        Some(m) => m,
        None => return,
    };

    for modifier in modifiers {
        apply_modifier(modifier, node, loc_fetcher);
    }
}

/// Apply a single modifier to a node.
///
/// Parses the `field._Type_` discriminator and `field.field` name, then updates
/// the appropriate typed field on `node` (or writes to `node.raw` as a fallback).
fn apply_modifier(modifier: &serde_json::Value, node: &mut BbNode, loc_fetcher: Option<&dyn LocFetcher>) {
    let field = match modifier.get("field") {
        Some(f) => f,
        None => return,
    };

    let type_str = field
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let field_name = field.get("field").and_then(|v| v.as_str()).unwrap_or("");

    // Skip canvas-reference modifiers (already handled by bb_resolve).
    if type_str.ends_with("CanvasReferenceRecord") {
        return;
    }

    // Extract the value based on the modifier type.
    match type_str {
        "BuildingBlocks_FieldModifierString" => {
            if let Some(value) = field.get("value").and_then(|v| v.as_str()) {
                apply_string_field(field_name, value, node, loc_fetcher);
            }
        }
        "BuildingBlocks_FieldModifierNumber" => {
            if let Some(value) = field.get("value").and_then(|v| v.as_f64()) {
                apply_number_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierColor" => {
            if let Some(color_obj) = field.get("value").and_then(|v| v.as_object()) {
                let color = parse_color(color_obj);
                apply_color_field(field_name, color, node);
            }
        }
        "BuildingBlocks_FieldModifierBoolean" => {
            if let Some(value) = field.get("value").and_then(|v| v.as_bool()) {
                apply_boolean_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierEnumerated"
        | "BuildingBlocks_FieldModifierEnumeratedTypeImageScalingBehavior"
        | "BuildingBlocks_FieldModifierEnumeratedTypeWidthBehavior"
        | "BuildingBlocks_FieldModifierEnumeratedTypeHeightBehavior" => {
            if let Some(value) = field.get("value").and_then(|v| v.as_str()) {
                apply_enum_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierRecordRef"
        | "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord" => {
            if let Some(value) = field.get("value").and_then(|v| v.as_str()) {
                apply_record_ref_field(field_name, value, node);
            }
        }
        _ => {
            // Unrecognised modifier type → log and passthrough to raw.
            log::debug!(
                "bb_brand_apply: unrecognised modifier type '{}' for field '{}'",
                type_str,
                field_name
            );
            if let Some(value) = field.get("value") {
                node.raw
                    .as_object_mut()
                    .and_then(|obj| obj.insert(field_name.to_string(), value.clone()));
            }
        }
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
fn apply_color_field(field_name: &str, color: [f32; 4], node: &mut BbNode) {
    match field_name {
        "FillColor" | "StrokeColor" | "BackgroundColor" => {
            // Update node.background if it exists.
            if let Some(bg) = &mut node.background {
                bg.fill_colour = Some(color);
            }
            // Also write to raw for non-typed cases.
            write_color_to_raw(field_name, color, node);
        }
        "BorderColor" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.colour = Some(color);
                border.right.colour = Some(color);
                border.bottom.colour = Some(color);
                border.left.colour = Some(color);
            }
        }
        "BorderColorTop" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.colour = Some(color);
            }
        }
        "BorderColorRight" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.right.colour = Some(color);
            }
        }
        "BorderColorBottom" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.bottom.colour = Some(color);
            }
        }
        "BorderColorLeft" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.left.colour = Some(color);
            }
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised color field '{}' = {:?}",
                field_name,
                color
            );
            write_color_to_raw(field_name, color, node);
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

/// Parse a color object from JSON.
///
/// Detects whether the values are in 0..1 range (f32) or 0..255 range (u8)
/// and normalizes to [f32; 4] in 0..1.
fn parse_color(color_obj: &serde_json::Map<String, serde_json::Value>) -> [f32; 4] {
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

    // Detect range: if any component > 1.0, assume 0..255 and normalize.
    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        [r / 255.0, g / 255.0, b / 255.0, a / 255.0]
    } else {
        [r, g, b, a]
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bb_scene::{BbNode, BbNodeType, BbScene};
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
}
