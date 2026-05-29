//! Canvas JSON parser.

use std::collections::HashMap;

use log::warn;
use serde_json::Value as JsonValue;

use crate::error::UiError;

use super::types::{
    CanvasRecord, CanvasView, Operation, RgbaColor, SceneItem, Transform2D, Value, ViewComponent,
};

/// Parses a `serde_json::Value` into a [`CanvasRecord`].
pub struct CanvasParser;

impl CanvasParser {
    /// Parse a canvas record JSON into a [`CanvasRecord`].
    pub fn parse(
        guid: impl Into<String>,
        name: impl Into<String>,
        json: &JsonValue,
    ) -> Result<CanvasRecord, UiError> {
        let obj = json
            .as_object()
            .ok_or_else(|| UiError::ParseError("canvas record JSON is not an object".to_string()))?;

        let views = parse_views(obj.get("views"));
        let scene = parse_scene(obj.get("scene"));
        let operations = parse_operations(obj.get("operations"));

        Ok(CanvasRecord {
            guid: guid.into(),
            name: name.into(),
            views,
            scene,
            operations,
        })
    }

    /// Classify a single scene-item JSON object into a [`ViewComponent`].
    pub fn parse_view_component(item: &JsonValue) -> ViewComponent {
        let type_str = str_field(item, "_Type_").unwrap_or_default();
        classify_component(&type_str, item)
    }
}

fn parse_views(value: Option<&JsonValue>) -> Vec<CanvasView> {
    let arr = match value.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .enumerate()
        .map(|(idx, view_val)| {
            let name = str_field(view_val, "name").unwrap_or_default();
            let default_flag = bool_field(view_val, "default")
                .or_else(|| bool_field(view_val, "defaultView"))
                .unwrap_or(idx == 0);

            let components = match view_val.get("screens").and_then(|s| s.as_array()) {
                Some(screens) => screens.iter().map(parse_screen_ref).collect(),
                None => Vec::new(),
            };

            CanvasView {
                name,
                ordinal: idx as u32,
                components,
                default: default_flag,
            }
        })
        .collect()
}

fn parse_screen_ref(screen: &JsonValue) -> ViewComponent {
    let sub_guid = match screen {
        JsonValue::String(s) if !s.is_empty() && s != "null" => Some(s.clone()),
        JsonValue::Object(_) => str_field(screen, "canvas").filter(|s| !s.is_empty()),
        _ => None,
    };
    let url_postfix = str_field(screen, "urlPostfix");
    let url_optional = str_field(screen, "urlOptional");
    let canvas_path = str_field(screen, "canvasPath").or_else(|| str_field(screen, "canvas"));

    ViewComponent::WidgetCanvas {
        canvas_path,
        url_postfix,
        url_optional,
        sub_guid,
    }
}

fn parse_scene(value: Option<&JsonValue>) -> Vec<SceneItem> {
    let arr = match value.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter().map(parse_scene_item).collect()
}

fn parse_scene_item(item: &JsonValue) -> SceneItem {
    let kind = str_field(item, "_Type_").unwrap_or_default();
    let guid = str_field(item, "canvas").filter(|s| !s.is_empty() && s != "null");

    let url_postfix = str_field(item, "urlPostfix").filter(|s| !s.is_empty());
    let url_optional = str_field(item, "urlOptional").filter(|s| !s.is_empty());

    let transform = parse_transform(item.get("transform"));
    let color = parse_rgba_color(item.get("color"));

    let skip_keys = [
        "_Type_",
        "canvas",
        "urlPostfix",
        "urlOptional",
        "transform",
        "color",
    ];
    let properties: HashMap<String, JsonValue> = item
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| !skip_keys.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();

    SceneItem {
        kind,
        guid,
        url_postfix,
        url_optional,
        transform,
        color,
        properties,
    }
}

fn parse_operations(value: Option<&JsonValue>) -> Vec<Operation> {
    let arr = match value.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter().map(parse_operation).collect()
}

fn parse_operation(item: &JsonValue) -> Operation {
    let kind = str_field(item, "_Type_").unwrap_or_default();

    let binding_path = str_field(item, "binding")
        .or_else(|| str_field(item, "bindingPath"))
        .filter(|s| !s.is_empty());

    let target_property = str_field(item, "property")
        .or_else(|| str_field(item, "targetProperty"))
        .filter(|s| !s.is_empty());

    let default_value = extract_default_value(&kind, item);

    Operation {
        kind,
        binding_path,
        target_property,
        default_value,
    }
}

fn extract_default_value(kind: &str, item: &JsonValue) -> Option<Value> {
    let raw = item
        .get("defaultValue")
        .or_else(|| item.get("value"))
        .or_else(|| item.get("default"))?;

    if kind.contains("Boolean") || kind.contains("Bool") {
        return raw.as_bool().map(Value::Bool);
    }
    if (kind.contains("Integer") || kind.contains("Int")) && let Some(n) = raw.as_i64() {
        return Some(Value::Int(n));
    }
    if (kind.contains("Float") || kind.contains("Number")) && let Some(n) = raw.as_f64() {
        return Some(Value::Float(n));
    }

    match raw {
        JsonValue::Bool(b) => Some(Value::Bool(*b)),
        JsonValue::Number(n) if n.is_i64() => n.as_i64().map(Value::Int),
        JsonValue::Number(n) => n.as_f64().map(Value::Float),
        JsonValue::String(s) => {
            if looks_like_guid(s) {
                Some(Value::Guid(s.clone()))
            } else {
                Some(Value::Str(s.clone()))
            }
        }
        _ => None,
    }
}

fn classify_component(type_str: &str, item: &JsonValue) -> ViewComponent {
    let suffix = type_str.strip_prefix("BuildingBlocks_").unwrap_or(type_str);

    match suffix {
        "WidgetCanvas" | "WidgetCanvasURL" => {
            let canvas_path = str_field(item, "canvas").filter(|s| !s.is_empty() && s != "null");
            let sub_guid = canvas_path.clone().filter(|s| looks_like_guid(s));
            let canvas_path = if sub_guid.is_some() { None } else { canvas_path };
            ViewComponent::WidgetCanvas {
                canvas_path,
                url_postfix: str_field(item, "urlPostfix").filter(|s| !s.is_empty()),
                url_optional: str_field(item, "urlOptional").filter(|s| !s.is_empty()),
                sub_guid,
            }
        }
        "Text" | "TextField" | "Label" | "TextWidget" | "WidgetText" | "BindingsText"
        | "DynamicText" | "SpriteText" => {
            let binding_path = str_field(item, "binding").or_else(|| str_field(item, "bindingPath"));
            let default_text = str_field(item, "text").or_else(|| str_field(item, "defaultText"));
            let font_id = str_field(item, "fontId").or_else(|| str_field(item, "font"));
            let color = u32_field(item, "color");
            ViewComponent::TextField {
                binding_path,
                default_text,
                font_id,
                color,
            }
        }
        "Shape" | "Rectangle" | "Rect" | "Circle" | "Line" | "WidgetShape" => {
            let shape_id = item
                .get("shapeId")
                .or_else(|| item.get("characterId"))
                .and_then(|v| v.as_u64())
                .map(|n| n as u16);
            let fill = u32_field(item, "fill").or_else(|| u32_field(item, "fillColor"));
            ViewComponent::Shape { shape_id, fill }
        }
        "Sprite" | "WidgetSprite" | "SpriteInstance" | "MovieClip" => {
            let swf_path = str_field(item, "swfPath")
                .or_else(|| str_field(item, "swf"))
                .unwrap_or_default();
            let linkage_name = str_field(item, "linkageName")
                .or_else(|| str_field(item, "name"))
                .unwrap_or_default();
            ViewComponent::Sprite {
                swf_path,
                linkage_name,
            }
        }
        "Image" | "Bitmap" | "Texture" | "WidgetImage" => {
            let texture_path = str_field(item, "texturePath")
                .or_else(|| str_field(item, "texture"))
                .or_else(|| str_field(item, "path"))
                .unwrap_or_default();
            ViewComponent::Image { texture_path }
        }
        "Group" | "Container" | "Panel" | "WidgetGroup" | "Canvas" => {
            let children = item
                .get("children")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|c| classify_component(&str_field(c, "_Type_").unwrap_or_default(), c))
                        .collect()
                })
                .unwrap_or_default();
            ViewComponent::Container { children }
        }
        other => {
            warn!(
                "starbreaker-ui: unknown BuildingBlocks component type '{}'; treating as empty Container",
                other
            );
            ViewComponent::Container {
                children: Vec::new(),
            }
        }
    }
}

fn str_field(value: &JsonValue, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_owned)
}

fn bool_field(value: &JsonValue, key: &str) -> Option<bool> {
    value.get(key).and_then(|v| v.as_bool())
}

fn u32_field(value: &JsonValue, key: &str) -> Option<u32> {
    value.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

fn parse_transform(value: Option<&JsonValue>) -> Transform2D {
    let v = match value {
        Some(v) => v,
        None => return Transform2D::default(),
    };
    Transform2D {
        tx: f32_field(v, "tx").or_else(|| f32_field(v, "x")).unwrap_or(0.0),
        ty: f32_field(v, "ty").or_else(|| f32_field(v, "y")).unwrap_or(0.0),
        sx: f32_field(v, "sx")
            .or_else(|| f32_field(v, "scaleX"))
            .unwrap_or(1.0),
        sy: f32_field(v, "sy")
            .or_else(|| f32_field(v, "scaleY"))
            .unwrap_or(1.0),
        angle: f32_field(v, "angle")
            .or_else(|| f32_field(v, "rotation"))
            .unwrap_or(0.0),
    }
}

fn f32_field(value: &JsonValue, key: &str) -> Option<f32> {
    value.get(key).and_then(|v| v.as_f64()).map(|n| n as f32)
}

fn parse_rgba_color(value: Option<&JsonValue>) -> Option<RgbaColor> {
    let v = value?;
    match v {
        JsonValue::Number(n) => {
            let packed = n.as_u64()? as u32;
            Some(RgbaColor {
                r: ((packed >> 16) & 0xFF) as u8,
                g: ((packed >> 8) & 0xFF) as u8,
                b: (packed & 0xFF) as u8,
                a: ((packed >> 24) & 0xFF) as u8,
            })
        }
        JsonValue::Object(_) => {
            let r = v.get("r").and_then(|c| c.as_u64()).unwrap_or(255) as u8;
            let g = v.get("g").and_then(|c| c.as_u64()).unwrap_or(255) as u8;
            let b = v.get("b").and_then(|c| c.as_u64()).unwrap_or(255) as u8;
            let a = v.get("a").and_then(|c| c.as_u64()).unwrap_or(255) as u8;
            Some(RgbaColor { r, g, b, a })
        }
        _ => None,
    }
}

fn looks_like_guid(s: &str) -> bool {
    s.len() == 36
        && s.as_bytes().get(8) == Some(&b'-')
        && s.as_bytes().get(13) == Some(&b'-')
        && s.as_bytes().get(18) == Some(&b'-')
        && s.as_bytes().get(23) == Some(&b'-')
}
