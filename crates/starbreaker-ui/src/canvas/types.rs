//! Typed data models for BuildingBlocks canvas records.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// A 2-D affine transform captured from widget positioning data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub tx: f32,
    pub ty: f32,
    pub sx: f32,
    pub sy: f32,
    pub angle: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            sx: 1.0,
            sy: 1.0,
            angle: 0.0,
        }
    }
}

/// An RGBA colour value stored as 8-bit components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RgbaColor {
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
}

/// A typed default value for a binding slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Guid(String),
}

/// A typed widget component parsed from a `scene[]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ViewComponent {
    WidgetCanvas {
        canvas_path: Option<String>,
        url_postfix: Option<String>,
        url_optional: Option<String>,
        sub_guid: Option<String>,
    },
    Sprite {
        swf_path: String,
        linkage_name: String,
    },
    TextField {
        binding_path: Option<String>,
        default_text: Option<String>,
        font_id: Option<String>,
        color: Option<u32>,
    },
    Shape {
        shape_id: Option<u16>,
        fill: Option<u32>,
    },
    Image {
        texture_path: String,
    },
    Container {
        children: Vec<ViewComponent>,
    },
}

/// A named view definition within a canvas record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasView {
    pub name: String,
    pub ordinal: u32,
    pub components: Vec<ViewComponent>,
    pub default: bool,
}

/// A flat scene-graph item from the `scene[]` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneItem {
    pub kind: String,
    pub guid: Option<String>,
    pub url_postfix: Option<String>,
    pub url_optional: Option<String>,
    pub transform: Transform2D,
    pub color: Option<RgbaColor>,
    pub properties: HashMap<String, JsonValue>,
}

/// A data-binding operation from the `operations[]` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub kind: String,
    pub binding_path: Option<String>,
    pub target_property: Option<String>,
    pub default_value: Option<Value>,
}

/// The parsed in-memory representation of a BuildingBlocks canvas record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasRecord {
    pub guid: String,
    pub name: String,
    pub views: Vec<CanvasView>,
    pub scene: Vec<SceneItem>,
    pub operations: Vec<Operation>,
}
