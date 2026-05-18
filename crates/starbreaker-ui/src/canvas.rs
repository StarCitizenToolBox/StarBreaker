//! BuildingBlocks canvas record parser and widget tree resolver.
//!
//! This module converts a `serde_json::Value` (the JSON representation of a
//! BuildingBlocks canvas record, as produced by the DataCore pipeline) into a
//! structured [`CanvasRecord`] with a typed widget tree.  Sub-canvas references
//! (`BuildingBlocks_WidgetCanvas` items whose `canvas` field carries a GUID) are
//! resolved recursively by [`CanvasWidgetTreeResolver`].
//!
//! # Source data assumptions
//! Canvas records in DataCore carry three top-level arrays:
//! - `views[]` – ordered view definitions.  Each view may reference sub-canvas
//!   GUIDs in its `screens[]` field (handled by the resolver, not the parser).
//! - `scene[]` – the widget items for this canvas.  Each item has a `_Type_`
//!   discriminant string starting with `BuildingBlocks_`.
//! - `operations[]` – data-binding declarations.  Each has a `_Type_` starting
//!   with `BuildingBlocks_Bindings`.
//!
//! Unknown `_Type_` strings are accepted; they produce a
//! [`ViewComponent::Container`] with empty children and a `log::warn!` message.
//!
//! # Non-goals
//! This module does **not** depend on `starbreaker-datacore` or any live
//! database connection.  Callers hand in `serde_json::Value` and a fetch
//! callback; the resolver calls back to fetch additional records.

use std::collections::{HashMap, HashSet};

use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::error::UiError;

// ──────────────────────────────────────────────────────────────────────────────
// Public data types
// ──────────────────────────────────────────────────────────────────────────────

/// A 2-D affine transform captured from a canvas widget's positioning data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    /// X translation (pixels / canvas units).
    pub tx: f32,
    /// Y translation (pixels / canvas units).
    pub ty: f32,
    /// X scale factor (1.0 = no scale).
    pub sx: f32,
    /// Y scale factor (1.0 = no scale).
    pub sy: f32,
    /// Rotation angle in degrees.
    pub angle: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self { tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, angle: 0.0 }
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
    /// Fully-opaque white.
    pub const WHITE: Self = Self { r: 255, g: 255, b: 255, a: 255 };
}

/// A typed default value for a binding slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// A DataCore GUID string (format `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
    Guid(String),
}

// ── Widget component enum ────────────────────────────────────────────────────

/// A typed widget component parsed from a `scene[]` entry.
///
/// The discriminant is the `_Type_` field from the JSON.  Variants are derived
/// from the known `BuildingBlocks_*` class names; unknown classes are wrapped in
/// [`ViewComponent::Container`] with an empty child list after logging a warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ViewComponent {
    /// A reference to another canvas (possibly resolved recursively).
    WidgetCanvas {
        /// File-path or record-name reference to the sub-canvas, e.g.
        /// `"gen_ec_powermanagement.json"`.
        canvas_path: Option<String>,
        /// URL postfix used to select a parameterised canvas variant
        /// (e.g. `"mfd"`).
        url_postfix: Option<String>,
        /// Optional URL variant string.
        url_optional: Option<String>,
        /// The DataCore GUID of the referenced sub-canvas when it has been
        /// resolved from the `canvas` field.
        sub_guid: Option<String>,
    },

    /// A reference to a named SWF export symbol (sprite / MC linkage).
    Sprite {
        /// Path to the SWF file that contains the symbol.
        swf_path: String,
        /// ActionScript export linkage name for the symbol.
        linkage_name: String,
    },

    /// A text widget, possibly bound to a runtime variable.
    TextField {
        /// DataCore binding path (e.g. `/vehicle/targetname`).
        binding_path: Option<String>,
        /// Authored default text to display when no binding value is available.
        default_text: Option<String>,
        /// Font record identifier.
        font_id: Option<String>,
        /// Packed ARGB / RGBA colour (format matches source record).
        color: Option<u32>,
    },

    /// A simple filled/stroked shape.
    Shape {
        /// SWF character id of the shape, when sourced from SWF.
        shape_id: Option<u16>,
        /// Packed fill colour.
        fill: Option<u32>,
    },

    /// A static image / texture widget.
    Image {
        /// Path to the texture asset inside the P4K archive.
        texture_path: String,
    },

    /// A generic container that holds child components.
    ///
    /// Used both for explicit group-type widgets and as a fallback for
    /// unknown `_Type_` values.
    Container {
        children: Vec<ViewComponent>,
    },
}

// ── View ────────────────────────────────────────────────────────────────────

/// A named view definition within a canvas record.
///
/// A canvas record may carry multiple views (e.g. a landscape view, a portrait
/// view, and an "Off" view).  One view is marked `default = true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasView {
    /// The view's authored name (e.g. `"_mfd"`, `"_physicalScreen"`, `"Off"`).
    pub name: String,
    /// Position of this view in the `views[]` array (0-based ordinal).
    pub ordinal: u32,
    /// Widget components belonging to this view (parsed from `screens[]`
    /// sub-GUID references — each becomes a [`ViewComponent::WidgetCanvas`]).
    pub components: Vec<ViewComponent>,
    /// `true` when this is the authoritative default view for the canvas.
    pub default: bool,
}

// ── Scene item ───────────────────────────────────────────────────────────────

/// A flat scene-graph item from the `scene[]` array, preserving raw properties.
///
/// Leaf-level widget items are also available as [`ViewComponent`] variants
/// (parsed by [`CanvasParser::parse_view_component`]).  [`SceneItem`] is kept
/// as a parallel representation so callers that only need partial data (e.g.
/// GUID or transform extraction) do not have to match the full enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneItem {
    /// The `_Type_` discriminant, e.g. `"BuildingBlocks_WidgetCanvas"`.
    pub kind: String,
    /// GUID of a referenced sub-canvas (when `_Type_` is `BuildingBlocks_WidgetCanvas`
    /// and the `canvas` field resolves to a GUID).
    pub guid: Option<String>,
    /// `urlPostfix` value, if present.
    pub url_postfix: Option<String>,
    /// `urlOptional` value, if present.
    pub url_optional: Option<String>,
    /// Positional / scale transform extracted from the item's `transform` object.
    pub transform: Transform2D,
    /// Tint / modulate colour, if the item carries a `color` field.
    pub color: Option<RgbaColor>,
    /// All remaining JSON properties, preserved verbatim for downstream phases.
    pub properties: HashMap<String, JsonValue>,
}

// ── Operation ────────────────────────────────────────────────────────────────

/// A data-binding operation from the `operations[]` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    /// The `_Type_` discriminant, e.g. `"BuildingBlocks_BindingsStringVariable"`.
    pub kind: String,
    /// The binding path that supplies the value (e.g. `/vehicle/targetname`).
    pub binding_path: Option<String>,
    /// The target widget property that receives the value.
    pub target_property: Option<String>,
    /// Authored default value (type varies by operation kind).
    pub default_value: Option<Value>,
}

// ── Canvas record ────────────────────────────────────────────────────────────

/// The fully parsed in-memory representation of a BuildingBlocks canvas record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasRecord {
    /// DataCore GUID supplied by the caller (not present in the JSON body).
    pub guid: String,
    /// Record name supplied by the caller (the DataCore `__name` or equivalent).
    pub name: String,
    /// Ordered view definitions.
    pub views: Vec<CanvasView>,
    /// Flat scene-graph items from the `scene[]` array.
    pub scene: Vec<SceneItem>,
    /// Data-binding operations from the `operations[]` array.
    pub operations: Vec<Operation>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Parser
// ──────────────────────────────────────────────────────────────────────────────

/// Parses a `serde_json::Value` (a BuildingBlocks canvas record body) into a
/// [`CanvasRecord`].
///
/// The caller is responsible for supplying the `guid` and `name` strings; these
/// are not encoded in the JSON body itself (they live in the DataCore index).
///
/// ```rust
/// # use starbreaker_ui::canvas::{CanvasParser, CanvasRecord};
/// # use serde_json::json;
/// let json = json!({ "views": [], "scene": [], "operations": [] });
/// let record = CanvasParser::parse("my-guid", "MyCanvas", &json).unwrap();
/// assert_eq!(record.guid, "my-guid");
/// ```
pub struct CanvasParser;

impl CanvasParser {
    /// Parse a canvas record JSON into a [`CanvasRecord`].
    ///
    /// Returns [`UiError::ParseError`] when the top-level JSON value is not an
    /// object.  Missing `views`, `scene`, or `operations` arrays are treated as
    /// empty rather than errors.
    pub fn parse(
        guid: impl Into<String>,
        name: impl Into<String>,
        json: &JsonValue,
    ) -> Result<CanvasRecord, UiError> {
        let obj = json.as_object().ok_or_else(|| {
            UiError::ParseError("canvas record JSON is not an object".to_string())
        })?;

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
    ///
    /// This is exposed publicly so resolver and test code can call it directly.
    pub fn parse_view_component(item: &JsonValue) -> ViewComponent {
        let type_str = str_field(item, "_Type_").unwrap_or_default();
        classify_component(&type_str, item)
    }
}

// ── Internal parsing helpers ─────────────────────────────────────────────────

fn parse_views(value: Option<&JsonValue>) -> Vec<CanvasView> {
    let arr = match value.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .enumerate()
        .map(|(idx, view_val)| {
            let name = str_field(view_val, "name").unwrap_or_default();
            // The "default" flag may be encoded explicitly or implied by `ordinal == 0`.
            let default_flag = bool_field(view_val, "default")
                .or_else(|| bool_field(view_val, "defaultView"))
                .unwrap_or(idx == 0);

            // Parse screens[] as sub-canvas WidgetCanvas references.
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

/// Convert a `screens[]` entry (a GUID string, an object, or a null) into a
/// [`ViewComponent::WidgetCanvas`].
fn parse_screen_ref(screen: &JsonValue) -> ViewComponent {
    // The entry may be a plain GUID string or an object with a `canvas` key.
    let sub_guid = match screen {
        JsonValue::String(s) if !s.is_empty() && s != "null" => Some(s.clone()),
        JsonValue::Object(_) => str_field(screen, "canvas").filter(|s| !s.is_empty()),
        _ => None,
    };
    let url_postfix = str_field(screen, "urlPostfix");
    let url_optional = str_field(screen, "urlOptional");
    let canvas_path = str_field(screen, "canvasPath").or_else(|| str_field(screen, "canvas"));

    ViewComponent::WidgetCanvas { canvas_path, url_postfix, url_optional, sub_guid }
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

    // For WidgetCanvas items, `canvas` may be a GUID string or a path.
    let guid = str_field(item, "canvas").filter(|s| !s.is_empty() && s != "null");

    let url_postfix = str_field(item, "urlPostfix").filter(|s| !s.is_empty());
    let url_optional = str_field(item, "urlOptional").filter(|s| !s.is_empty());

    let transform = parse_transform(item.get("transform"));
    let color = parse_rgba_color(item.get("color"));

    // Preserve remaining properties verbatim.
    let skip_keys = ["_Type_", "canvas", "urlPostfix", "urlOptional", "transform", "color"];
    let properties: HashMap<String, JsonValue> = item
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| !skip_keys.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();

    SceneItem { kind, guid, url_postfix, url_optional, transform, color, properties }
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

    // Known field names across operation types.
    let binding_path = str_field(item, "binding")
        .or_else(|| str_field(item, "bindingPath"))
        .filter(|s| !s.is_empty());

    let target_property = str_field(item, "property")
        .or_else(|| str_field(item, "targetProperty"))
        .filter(|s| !s.is_empty());

    let default_value = extract_default_value(&kind, item);

    Operation { kind, binding_path, target_property, default_value }
}

/// Infer a typed [`Value`] from an operation's `defaultValue` / `value` field,
/// guided by the operation's `_Type_` kind string.
fn extract_default_value(kind: &str, item: &JsonValue) -> Option<Value> {
    // Try common field names for the default.
    let raw = item
        .get("defaultValue")
        .or_else(|| item.get("value"))
        .or_else(|| item.get("default"))?;

    // Use the kind to pick a typed parse.
    if kind.contains("Boolean") || kind.contains("Bool") {
        return raw.as_bool().map(Value::Bool);
    }
    if kind.contains("Integer") || kind.contains("Int") {
        if let Some(n) = raw.as_i64() {
            return Some(Value::Int(n));
        }
    }
    if kind.contains("Float") || kind.contains("Number") {
        if let Some(n) = raw.as_f64() {
            return Some(Value::Float(n));
        }
    }
    // String / GUID fallback.
    match raw {
        JsonValue::Bool(b) => Some(Value::Bool(*b)),
        JsonValue::Number(n) if n.is_i64() => Some(Value::Int(n.as_i64().unwrap())),
        JsonValue::Number(n) => Some(Value::Float(n.as_f64().unwrap())),
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

// ── Component classifier ─────────────────────────────────────────────────────

/// Map a `BuildingBlocks_*` type string to the appropriate [`ViewComponent`]
/// variant.
///
/// The classifier is keyed on the suffix of the `BuildingBlocks_` prefix.
/// Unknown suffixes emit a `warn!` and produce an empty [`ViewComponent::Container`].
fn classify_component(type_str: &str, item: &JsonValue) -> ViewComponent {
    // Strip the standard prefix to get the class suffix.
    let suffix = type_str.strip_prefix("BuildingBlocks_").unwrap_or(type_str);

    match suffix {
        "WidgetCanvas" | "WidgetCanvasURL" => {
            let canvas_path =
                str_field(item, "canvas").filter(|s| !s.is_empty() && s != "null");
            let sub_guid = canvas_path.clone().filter(|s| looks_like_guid(s));
            let canvas_path =
                if sub_guid.is_some() { None } else { canvas_path };
            ViewComponent::WidgetCanvas {
                canvas_path,
                url_postfix: str_field(item, "urlPostfix").filter(|s| !s.is_empty()),
                url_optional: str_field(item, "urlOptional").filter(|s| !s.is_empty()),
                sub_guid,
            }
        }

        // Text / label widgets.
        "Text" | "TextField" | "Label" | "TextWidget" | "WidgetText"
        | "BindingsText" | "DynamicText" | "SpriteText" => {
            let binding_path =
                str_field(item, "binding").or_else(|| str_field(item, "bindingPath"));
            let default_text =
                str_field(item, "text").or_else(|| str_field(item, "defaultText"));
            let font_id = str_field(item, "fontId").or_else(|| str_field(item, "font"));
            let color = u32_field(item, "color");
            ViewComponent::TextField { binding_path, default_text, font_id, color }
        }

        // Shape / vector-graphics widgets.
        "Shape" | "Rectangle" | "Rect" | "Circle" | "Line" | "WidgetShape" => {
            let shape_id = item
                .get("shapeId")
                .or_else(|| item.get("characterId"))
                .and_then(|v| v.as_u64())
                .map(|n| n as u16);
            let fill = u32_field(item, "fill").or_else(|| u32_field(item, "fillColor"));
            ViewComponent::Shape { shape_id, fill }
        }

        // Sprite / SWF symbol references.
        "Sprite" | "WidgetSprite" | "SpriteInstance" | "MovieClip" => {
            let swf_path = str_field(item, "swfPath")
                .or_else(|| str_field(item, "swf"))
                .unwrap_or_default();
            let linkage_name = str_field(item, "linkageName")
                .or_else(|| str_field(item, "name"))
                .unwrap_or_default();
            ViewComponent::Sprite { swf_path, linkage_name }
        }

        // Image / texture references.
        "Image" | "Bitmap" | "Texture" | "WidgetImage" => {
            let texture_path = str_field(item, "texturePath")
                .or_else(|| str_field(item, "texture"))
                .or_else(|| str_field(item, "path"))
                .unwrap_or_default();
            ViewComponent::Image { texture_path }
        }

        // Group / container.
        "Group" | "Container" | "Panel" | "WidgetGroup" | "Canvas" => {
            let children = item
                .get("children")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().map(|c| classify_component(
                    &str_field(c, "_Type_").unwrap_or_default(),
                    c,
                )).collect())
                .unwrap_or_default();
            ViewComponent::Container { children }
        }

        other => {
            warn!(
                "starbreaker-ui: unknown BuildingBlocks component type '{}'; \
                 treating as empty Container",
                other
            );
            ViewComponent::Container { children: Vec::new() }
        }
    }
}

// ── Field extraction helpers ─────────────────────────────────────────────────

fn str_field(value: &JsonValue, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(|s| s.to_owned())
}

fn bool_field(value: &JsonValue, key: &str) -> Option<bool> {
    value.get(key).and_then(|v| v.as_bool())
}

fn u32_field(value: &JsonValue, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
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
        // Packed integer form.
        JsonValue::Number(n) => {
            let packed = n.as_u64()? as u32;
            Some(RgbaColor {
                r: ((packed >> 16) & 0xFF) as u8,
                g: ((packed >> 8) & 0xFF) as u8,
                b: (packed & 0xFF) as u8,
                a: ((packed >> 24) & 0xFF) as u8,
            })
        }
        // Object form `{ r, g, b, a }`.
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

/// Heuristic: a string looks like a DataCore GUID if it is 36 characters long
/// and contains hyphens at the expected positions.
fn looks_like_guid(s: &str) -> bool {
    s.len() == 36
        && s.as_bytes().get(8) == Some(&b'-')
        && s.as_bytes().get(13) == Some(&b'-')
        && s.as_bytes().get(18) == Some(&b'-')
        && s.as_bytes().get(23) == Some(&b'-')
}

// ──────────────────────────────────────────────────────────────────────────────
// Widget tree resolver
// ──────────────────────────────────────────────────────────────────────────────

/// Recursively expands sub-canvas references into a tree of [`CanvasRecord`]s.
///
/// Construct via [`CanvasWidgetTreeResolver::new`], then call
/// [`CanvasWidgetTreeResolver::resolve`] with a root GUID and a fetch callback.
///
/// # Cycle detection
/// The resolver maintains a `HashSet` of visited GUIDs.  Revisiting a GUID
/// returns [`UiError::CycleDetected`].
///
/// # Max depth
/// The resolver aborts with [`UiError::MaxDepthExceeded`] when recursion
/// exceeds `max_depth` (default: 16).
pub struct CanvasWidgetTreeResolver {
    max_depth: usize,
}

impl Default for CanvasWidgetTreeResolver {
    fn default() -> Self {
        Self { max_depth: 16 }
    }
}

impl CanvasWidgetTreeResolver {
    /// Create a resolver with the default maximum expansion depth (16).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a resolver with a custom maximum expansion depth.
    pub fn with_max_depth(max_depth: usize) -> Self {
        Self { max_depth }
    }

    /// Resolve a canvas tree starting from `root_guid`.
    ///
    /// `fetch` is called for every GUID that needs to be loaded.  It receives
    /// the GUID string and must return `Ok(serde_json::Value)` for the canvas
    /// body JSON, or any error wrapped in a `Box<dyn Error + Send + Sync>`.
    ///
    /// The returned [`ResolvedCanvas`] contains the root [`CanvasRecord`] and
    /// all recursively resolved children stored in a flat map keyed by GUID.
    pub fn resolve<F, E>(
        &self,
        root_guid: &str,
        fetch: F,
    ) -> Result<ResolvedCanvas, UiError>
    where
        F: Fn(&str) -> Result<serde_json::Value, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut visited = HashSet::new();
        let mut resolved_map: HashMap<String, CanvasRecord> = HashMap::new();

        let root = self.resolve_one(
            root_guid,
            &fetch,
            &mut visited,
            &mut resolved_map,
            0,
        )?;

        Ok(ResolvedCanvas { root, children: resolved_map })
    }

    fn resolve_one<F, E>(
        &self,
        guid: &str,
        fetch: &F,
        visited: &mut HashSet<String>,
        resolved_map: &mut HashMap<String, CanvasRecord>,
        depth: usize,
    ) -> Result<CanvasRecord, UiError>
    where
        F: Fn(&str) -> Result<serde_json::Value, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        if depth > self.max_depth {
            return Err(UiError::MaxDepthExceeded {
                guid: guid.to_string(),
                max_depth: self.max_depth,
            });
        }

        if !visited.insert(guid.to_string()) {
            return Err(UiError::CycleDetected(guid.to_string()));
        }

        let json = fetch(guid).map_err(|e| UiError::FetchFailed {
            guid: guid.to_string(),
            source: Box::new(e),
        })?;

        // Extract the record name from common field names; fall back to the GUID.
        let name = json
            .get("__name")
            .or_else(|| json.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(guid)
            .to_string();

        let mut record = CanvasParser::parse(guid, &name, &json)?;

        // Recursively resolve sub-canvases referenced in scene items.
        for item in &record.scene {
            if let Some(sub_guid) = &item.guid {
                if !sub_guid.is_empty() && sub_guid != "null" && !resolved_map.contains_key(sub_guid) {
                    match self.resolve_one(sub_guid, fetch, visited, resolved_map, depth + 1) {
                        Ok(child) => {
                            resolved_map.insert(sub_guid.clone(), child);
                        }
                        Err(UiError::CycleDetected(_)) => {
                            return Err(UiError::CycleDetected(sub_guid.clone()));
                        }
                        Err(UiError::MaxDepthExceeded { .. }) => {
                            return Err(UiError::MaxDepthExceeded {
                                guid: sub_guid.clone(),
                                max_depth: self.max_depth,
                            });
                        }
                        Err(e) => {
                            warn!(
                                "starbreaker-ui: failed to resolve sub-canvas {}: {}",
                                sub_guid, e
                            );
                        }
                    }
                }
            }
        }

        // Also resolve sub-canvases referenced in view screens.
        for view in &record.views {
            for component in &view.components {
                if let ViewComponent::WidgetCanvas { sub_guid: Some(sg), .. } = component {
                    if !resolved_map.contains_key(sg) {
                        match self.resolve_one(sg, fetch, visited, resolved_map, depth + 1) {
                            Ok(child) => {
                                resolved_map.insert(sg.clone(), child);
                            }
                            Err(UiError::CycleDetected(_)) => {
                                return Err(UiError::CycleDetected(sg.clone()));
                            }
                            Err(UiError::MaxDepthExceeded { .. }) => {
                                return Err(UiError::MaxDepthExceeded {
                                    guid: sg.clone(),
                                    max_depth: self.max_depth,
                                });
                            }
                            Err(e) => {
                                warn!(
                                    "starbreaker-ui: failed to resolve view sub-canvas {}: {}",
                                    sg, e
                                );
                            }
                        }
                    }
                }
            }
        }

        // Mark this GUID as having been processed by removing it from visited,
        // so sibling branches can re-encounter it without false cycle errors.
        // We do NOT remove it if it was the root call: the root stays in visited
        // for the whole traversal.  Since children are resolved before this
        // return, it is safe to remove the GUID now.
        // Note: we keep `visited` as a true visited-on-current-path set to
        // detect only back-edges (true cycles), not cross-edges (DAG sharing).
        // Remove the current GUID so sibling paths can resolve the same canvas
        // without triggering a false cycle.
        visited.remove(guid);

        // Record the name exposed via the record for completeness.
        record.name = name;
        Ok(record)
    }
}

/// The output of a successful [`CanvasWidgetTreeResolver::resolve`] call.
#[derive(Debug, Clone)]
pub struct ResolvedCanvas {
    /// The root canvas record (for the GUID passed to `resolve`).
    pub root: CanvasRecord,
    /// All recursively resolved child canvases, keyed by GUID.
    pub children: HashMap<String, CanvasRecord>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Fixture helpers ──────────────────────────────────────────────────────

    fn guid_a() -> &'static str {
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    }
    fn guid_b() -> &'static str {
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    }
    fn guid_c() -> &'static str {
        "cccccccc-cccc-cccc-cccc-cccccccccccc"
    }

    // ── 1. Simple canvas with a TextField bound to a string variable ──────────

    #[test]
    fn parse_canvas_with_text_field_operation() {
        let json = json!({
            "views": [],
            "scene": [
                {
                    "_Type_": "BuildingBlocks_TextField",
                    "binding": "/vehicle/targetname",
                    "text": "NO TARGET",
                    "fontId": "font_amber_mono"
                }
            ],
            "operations": [
                {
                    "_Type_": "BuildingBlocks_BindingsStringVariable",
                    "binding": "/vehicle/targetname",
                    "property": "text",
                    "defaultValue": "NO TARGET"
                }
            ]
        });

        let record = CanvasParser::parse(guid_a(), "TestCanvas", &json).unwrap();

        assert_eq!(record.guid, guid_a());
        assert_eq!(record.name, "TestCanvas");
        assert_eq!(record.scene.len(), 1);
        assert_eq!(record.scene[0].kind, "BuildingBlocks_TextField");

        assert_eq!(record.operations.len(), 1);
        let op = &record.operations[0];
        assert_eq!(op.kind, "BuildingBlocks_BindingsStringVariable");
        assert_eq!(op.binding_path.as_deref(), Some("/vehicle/targetname"));
        assert_eq!(op.target_property.as_deref(), Some("text"));
        assert!(matches!(op.default_value, Some(Value::Str(ref s)) if s == "NO TARGET"));
    }

    // ── 2. ViewComponent classifier: TextField ───────────────────────────────

    #[test]
    fn classify_text_field_component() {
        let item = json!({
            "_Type_": "BuildingBlocks_TextField",
            "binding": "/vehicle/targetname",
            "text": ">> NO TARGET <<",
            "fontId": "font_amber",
            "color": 4294956800u64   // 0xFFFFAA00 packed RGBA
        });

        let component = CanvasParser::parse_view_component(&item);
        match component {
            ViewComponent::TextField { binding_path, default_text, font_id, color } => {
                assert_eq!(binding_path.as_deref(), Some("/vehicle/targetname"));
                assert_eq!(default_text.as_deref(), Some(">> NO TARGET <<"));
                assert_eq!(font_id.as_deref(), Some("font_amber"));
                assert!(color.is_some());
            }
            other => panic!("Expected TextField, got {:?}", other),
        }
    }

    // ── 3. Canvas with WidgetCanvas sub-canvas reference ─────────────────────

    #[test]
    fn parse_canvas_with_widget_canvas_scene_item() {
        let json = json!({
            "views": [],
            "scene": [
                {
                    "_Type_": "BuildingBlocks_WidgetCanvas",
                    "canvas": guid_b(),
                    "urlPostfix": "mfd"
                }
            ],
            "operations": []
        });

        let record = CanvasParser::parse(guid_a(), "ContainerCanvas", &json).unwrap();
        assert_eq!(record.scene.len(), 1);
        let item = &record.scene[0];
        assert_eq!(item.kind, "BuildingBlocks_WidgetCanvas");
        assert_eq!(item.guid.as_deref(), Some(guid_b()));
        assert_eq!(item.url_postfix.as_deref(), Some("mfd"));
    }

    // ── 4. Resolver descends into sub-canvas ─────────────────────────────────

    #[test]
    fn resolver_descends_into_sub_canvas() {
        let root_json = json!({
            "scene": [
                {
                    "_Type_": "BuildingBlocks_WidgetCanvas",
                    "canvas": guid_b()
                }
            ],
            "operations": []
        });

        let child_json = json!({
            "__name": "ChildCanvas",
            "scene": [
                {
                    "_Type_": "BuildingBlocks_TextField",
                    "text": "child text"
                }
            ],
            "operations": []
        });

        let resolver = CanvasWidgetTreeResolver::new();
        let result = resolver
            .resolve(guid_a(), |guid| -> Result<_, std::convert::Infallible> {
                if guid == guid_a() {
                    Ok(root_json.clone())
                } else if guid == guid_b() {
                    Ok(child_json.clone())
                } else {
                    panic!("unexpected GUID {}", guid)
                }
            })
            .unwrap();

        // Root canvas is present.
        assert_eq!(result.root.guid, guid_a());
        // Child canvas was resolved and stored in the flat map.
        assert!(result.children.contains_key(guid_b()), "child canvas missing");
        let child = &result.children[guid_b()];
        assert_eq!(child.name, "ChildCanvas");
        assert_eq!(child.scene.len(), 1);
    }

    // ── 5. Cycle detection ────────────────────────────────────────────────────

    #[test]
    fn resolver_detects_cycle() {
        // A → B → A (cycle)
        let a_json = json!({
            "scene": [{ "_Type_": "BuildingBlocks_WidgetCanvas", "canvas": guid_b() }],
            "operations": []
        });
        let b_json = json!({
            "scene": [{ "_Type_": "BuildingBlocks_WidgetCanvas", "canvas": guid_a() }],
            "operations": []
        });

        let resolver = CanvasWidgetTreeResolver::new();
        let result = resolver.resolve(guid_a(), |guid| -> Result<_, std::convert::Infallible> {
            if guid == guid_a() {
                Ok(a_json.clone())
            } else {
                Ok(b_json.clone())
            }
        });

        assert!(
            matches!(result, Err(UiError::CycleDetected(_))),
            "expected CycleDetected, got {:?}",
            result
        );
    }

    // ── 6. Views with screens are parsed into WidgetCanvas components ─────────

    #[test]
    fn parse_views_with_screens() {
        let json = json!({
            "views": [
                {
                    "name": "_mfd",
                    "screens": [ guid_b(), guid_c() ]
                },
                {
                    "name": "Off",
                    "screens": []
                }
            ],
            "scene": [],
            "operations": []
        });

        let record = CanvasParser::parse(guid_a(), "MFDCanvas", &json).unwrap();
        assert_eq!(record.views.len(), 2);

        let first_view = &record.views[0];
        assert_eq!(first_view.name, "_mfd");
        assert_eq!(first_view.ordinal, 0);
        assert!(first_view.default); // first view is default
        assert_eq!(first_view.components.len(), 2);
        // Both screen refs become WidgetCanvas components.
        for comp in &first_view.components {
            assert!(matches!(comp, ViewComponent::WidgetCanvas { .. }));
        }

        let second_view = &record.views[1];
        assert_eq!(second_view.name, "Off");
        assert!(!second_view.default);
    }

    // ── 7. Empty canvas parses without error ─────────────────────────────────

    #[test]
    fn parse_empty_canvas() {
        let json = json!({ "views": [], "scene": [], "operations": [] });
        let record = CanvasParser::parse("00000000-0000-0000-0000-000000000000", "Empty", &json)
            .unwrap();
        assert!(record.views.is_empty());
        assert!(record.scene.is_empty());
        assert!(record.operations.is_empty());
    }

    // ── 8. Non-object JSON is rejected ───────────────────────────────────────

    #[test]
    fn parse_non_object_is_error() {
        let json = json!([1, 2, 3]);
        let result = CanvasParser::parse("guid", "name", &json);
        assert!(matches!(result, Err(UiError::ParseError(_))));
    }

    // ── 9. Transform fields are extracted correctly ───────────────────────────

    #[test]
    fn parse_scene_item_transform() {
        let json = json!({
            "views": [],
            "scene": [
                {
                    "_Type_": "BuildingBlocks_Shape",
                    "transform": { "tx": 100.0, "ty": 50.0, "sx": 2.0, "sy": 0.5, "angle": 45.0 }
                }
            ],
            "operations": []
        });

        let record = CanvasParser::parse(guid_a(), "ShapeCanvas", &json).unwrap();
        let item = &record.scene[0];
        assert!((item.transform.tx - 100.0).abs() < 1e-5);
        assert!((item.transform.ty - 50.0).abs() < 1e-5);
        assert!((item.transform.sx - 2.0).abs() < 1e-5);
        assert!((item.transform.angle - 45.0).abs() < 1e-5);
    }

    // ── 10. Max depth exceeded ────────────────────────────────────────────────

    #[test]
    fn resolver_max_depth_exceeded() {
        // Chain: a(depth=0) → b(depth=1) → c(depth=2) — with max_depth=1,
        // resolving c at depth=2 exceeds the limit.
        let resolver = CanvasWidgetTreeResolver::with_max_depth(1);

        let result = resolver.resolve(guid_a(), |guid| -> Result<_, std::convert::Infallible> {
            let next = match guid {
                g if g == guid_a() => guid_b(),
                g if g == guid_b() => guid_c(),
                _ => return Ok(json!({ "scene": [], "operations": [] })),
            };
            Ok(json!({
                "scene": [{ "_Type_": "BuildingBlocks_WidgetCanvas", "canvas": next }],
                "operations": []
            }))
        });

        assert!(
            matches!(result, Err(UiError::MaxDepthExceeded { .. })),
            "expected MaxDepthExceeded, got {:?}",
            result
        );
    }
}

