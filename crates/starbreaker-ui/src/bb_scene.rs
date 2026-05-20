//! BuildingBlocks scene parser — typed IR for BB canvas JSON records.
//!
//! Converts a `serde_json::Value` that represents a DataCore
//! `BuildingBlocks_Canvas` record into a structured [`BbScene`] with a flat
//! node map and explicit parent/child links.
//!
//! # Record shape
//! ```text
//! {
//!   "_RecordName_": "BuildingBlocks_Canvas.<name>",
//!   "_RecordId_":   "<uuid>",
//!   "_RecordValue_": {
//!     "_Type_": "BuildingBlocks_Canvas",
//!     "size": { "_Type_": "Vec3", "x": 1920, "y": 1080, "z": 0 },
//!     "scene": [ … ]
//!   }
//! }
//! ```
//! Each `scene` element has `_Pointer_: "ptr:N"`, an optional
//! `parent: "_PointsTo_:ptr:M"` (or null / absent for roots), and a
//! `_Type_` discriminant starting with `BuildingBlocks_`.
//!
//! Children are derived by inverting the parent pointers — the JSON itself is
//! a flat array, not a tree.
//!
//! # Tolerance
//! Unknown fields are silently ignored.  Unknown `_Type_` strings produce
//! [`BbNodeType::Other`].  Missing optional fields return sensible defaults.
//! This module never panics on well-formed JSON.

use std::collections::BTreeMap;

// ─────────────────────────────────────────────────────────────────────────────
// Public type aliases
// ─────────────────────────────────────────────────────────────────────────────

/// The integer N extracted from a `"ptr:N"` pointer string.
pub type BbNodeId = u32;

// ─────────────────────────────────────────────────────────────────────────────
// Top-level scene
// ─────────────────────────────────────────────────────────────────────────────

/// A fully-parsed BuildingBlocks canvas scene ready for rendering.
#[derive(Debug, Clone)]
pub struct BbScene {
    /// Canvas width and height in canvas units (from `size.x` / `size.y`).
    pub canvas_size: (f32, f32),
    /// IDs of all root nodes (nodes with no parent).
    pub roots: Vec<BbNodeId>,
    /// All nodes keyed by their pointer ID, in stable insertion order.
    pub nodes: BTreeMap<BbNodeId, BbNode>,
    /// Raw BuildingBlocks operations array used for runtime bindings.
    pub operations: Vec<serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Node
// ─────────────────────────────────────────────────────────────────────────────

/// One widget node from the `scene` array.
#[derive(Debug, Clone)]
pub struct BbNode {
    pub id: BbNodeId,
    /// Parent node id, or `None` for root nodes.
    pub parent: Option<BbNodeId>,
    /// Ordered list of child node ids (derived from parent back-references).
    pub children: Vec<BbNodeId>,
    /// Discriminated widget type.
    pub ty: BbNodeType,
    pub name: String,
    /// UUIDs extracted from the `_RecordId_` fields of `styleTags[]`.
    pub style_tag_uuids: Vec<String>,
    pub is_active: bool,
    pub layer: i32,
    pub alpha: f32,
    pub position: Vec3,
    pub position_offset: Vec3,
    pub sizing: BbSizing,
    pub padding: BbTrbl,
    pub margin: BbTrbl,
    pub pivot: Vec2,
    pub anchor: Vec2,
    pub background: Option<BbBackground>,
    pub border: Option<BbBorder>,
    pub radial: Option<BbRadialTransform>,
    /// Populated for `WidgetTextField` nodes.
    pub text: Option<BbText>,
    /// Populated for `WidgetIcon` nodes.
    pub icon: Option<BbIcon>,
    /// The original JSON object, preserved so later renderers can access
    /// fields not yet promoted to typed fields.
    pub raw: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
// Node type
// ─────────────────────────────────────────────────────────────────────────────

/// Discriminant derived from the `_Type_` field of each scene element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BbNodeType {
    DisplayWidget,
    WidgetCanvas,
    WidgetIcon,
    WidgetCard,
    WidgetTextField,
    ComponentGeneralButton,
    ComponentGeneralButtonSecondary,
    WidgetImage,
    WidgetText,
    WidgetCustomShape,
    WidgetBodyBackground,
    /// Any `_Type_` not in the list above; the inner string is the full
    /// `BuildingBlocks_*` type name.
    Other(String),
}

impl BbNodeType {
    fn from_type_str(s: &str) -> Self {
        // Strip the "BuildingBlocks_" prefix for matching.
        let tail = s.strip_prefix("BuildingBlocks_").unwrap_or(s);
        match tail {
            "DisplayWidget" => Self::DisplayWidget,
            "WidgetCanvas" => Self::WidgetCanvas,
            "WidgetIcon" => Self::WidgetIcon,
            "WidgetCard" => Self::WidgetCard,
            "WidgetTextField" => Self::WidgetTextField,
            "ComponentGeneralButton" => Self::ComponentGeneralButton,
            "ComponentGeneralButtonSecondary" => Self::ComponentGeneralButtonSecondary,
            "WidgetImage" => Self::WidgetImage,
            "WidgetText" => Self::WidgetText,
            "WidgetCustomShape" => Self::WidgetCustomShape,
            "WidgetBodyBackground" => Self::WidgetBodyBackground,
            _ => Self::Other(s.to_owned()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Geometry helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Three-component vector (maps to `Vec3` / `Deg3` in the JSON).
#[derive(Debug, Clone, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for Vec3 {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0 }
    }
}

/// Two-component vector (x / y of Vec3 nodes used for pivot and anchor).
#[derive(Debug, Clone, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Default for Vec2 {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sizing
// ─────────────────────────────────────────────────────────────────────────────

/// A `BuildingBlocks_FixedOrRelativeValue`.
#[derive(Debug, Clone, PartialEq)]
pub enum BbValue {
    /// `behavior == "Fixed"` — pixel / canvas-unit measurement.
    Fixed(f32),
    /// `behavior == "Percent"` — fraction of the parent dimension (0–1).
    Percent(f32),
    /// Any other `behavior` string (e.g. `"Auto"`, `"PercentOfY"`).
    Other { value: f32, behavior: String },
}

impl Default for BbValue {
    fn default() -> Self {
        Self::Fixed(0.0)
    }
}

/// Width and height from `BuildingBlocks_Size`.
#[derive(Debug, Clone, Default)]
pub struct BbSizing {
    pub width: BbValue,
    pub height: BbValue,
}

/// Four-sided spacing from `BuildingBlocks_TRBL`.
///
/// TRBL values in the JSON are plain numbers, not `FixedOrRelativeValue`.
#[derive(Debug, Clone, Default)]
pub struct BbTrbl {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Visual properties
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregated background/fill information from the three fill fields
/// (`background`, `svgFill`, `segmentedFill`).
#[derive(Debug, Clone, Default)]
pub struct BbBackground {
    /// RGBA colour [r, g, b, a] in 0–1 range, if the `background.color`
    /// field contains parseable numeric components.  Named theme colours
    /// (e.g. `BuildingBlocks_ColorStyle { color: "Accent1" }`) are not
    /// represented here; see `raw` for full data.
    pub fill_colour: Option<[f32; 4]>,
    /// SVG asset path from `svgFill.svgPath`, if non-empty.
    pub svg_fill_path: Option<String>,
    /// Segmented-fill data, present when `segmentedFill.enable` is `true`.
    pub segmented_fill: Option<BbSegmentedFill>,
}

/// Minimal capture of `BuildingBlocks_SegmentedFill`.
#[derive(Debug, Clone)]
pub struct BbSegmentedFill {
    pub angle: f32,
}

/// One side of a `BuildingBlocks_Border`.
#[derive(Debug, Clone, Default)]
pub struct BbBorderSide {
    pub width: f32,
    pub colour: Option<[f32; 4]>,
}

/// `BuildingBlocks_Border` — four sides with width and optional colour.
#[derive(Debug, Clone, Default)]
pub struct BbBorder {
    pub top: BbBorderSide,
    pub right: BbBorderSide,
    pub bottom: BbBorderSide,
    pub left: BbBorderSide,
}

/// `BuildingBlocks_RadialTransform` — radial layout parameters.
#[derive(Debug, Clone)]
pub struct BbRadialTransform {
    pub transform_multiplier: f32,
    pub curvature_axis: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Type-specific payloads
// ─────────────────────────────────────────────────────────────────────────────

/// Text / label properties for `BuildingBlocks_WidgetTextField`.
///
/// The runtime text string is typically bound at run time; `string` defaults
/// to `""` for static-export purposes.
#[derive(Debug, Clone, Default)]
pub struct BbText {
    /// Static default text (empty in most cases — set via data bindings).
    pub string: String,
    /// Font record reference, if present.
    pub font_record: Option<String>,
    /// Nominal font size (defaults to `Fixed(12.0)` when absent).
    pub font_size: BbValue,
    /// Horizontal text alignment string (e.g. `"Center"`, `"Left"`).
    pub alignment: String,
    /// Text colour [r, g, b, a] in 0–1 range, if parseable.
    pub colour: Option<[f32; 4]>,
}

/// Icon properties for `BuildingBlocks_WidgetIcon`.
#[derive(Debug, Clone, Default)]
pub struct BbIcon {
    /// Custom icon path from `iconProperties.customIcon`, if non-empty.
    pub image_record: Option<String>,
    /// Tint colour [r, g, b, a] in 0–1 range, if parseable.
    pub tint_colour: Option<[f32; 4]>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry-point
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a DataCore `BuildingBlocks_Canvas` record JSON into a [`BbScene`].
///
/// `json` should be the full record object (containing `_RecordName_`,
/// `_RecordId_`, and `_RecordValue_`).
///
/// # Errors
/// Returns `Err(String)` if the JSON is missing mandatory structural fields
/// (`_RecordValue_`, `size`, or `scene`).  Individual node parse failures are
/// silently skipped with a warning rather than aborting the whole scene.
pub fn parse_bb_canvas(json: &serde_json::Value) -> Result<BbScene, String> {
    let record_value = json
        .get("_RecordValue_")
        .ok_or("missing _RecordValue_")?;

    let size = record_value
        .get("size")
        .ok_or("missing _RecordValue_.size")?;
    let canvas_w = f32_field(size, "x");
    let canvas_h = f32_field(size, "y");

    let scene_arr = record_value
        .get("scene")
        .and_then(|v| v.as_array())
        .ok_or("missing or non-array _RecordValue_.scene")?;
    let empty_library = Vec::new();
    let library_arr = record_value
        .get("library")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_library);
    let operations = record_value
        .get("operations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // ── First pass: parse each node (without children populated yet). ────────
    // Nodes without a `_Pointer_` field get a synthetic ID derived from their
    // array index so they are still represented in the scene.  Synthetic IDs
    // start at 0x8000_0000 to avoid collision with real ptr:N values (which are
    // always small positive integers in practice).
    let mut nodes: BTreeMap<BbNodeId, BbNode> = BTreeMap::new();
    let mut library_ids = std::collections::BTreeSet::new();
    let mut synthetic_base: u32 = 0x8000_0000;

    for raw_item in scene_arr {
        let _ = parse_node_into(raw_item, &mut nodes, &mut synthetic_base, "scene");
    }
    for raw_item in library_arr {
        if let Some(id) = parse_node_into(raw_item, &mut nodes, &mut synthetic_base, "library") {
            library_ids.insert(id);
        }
    }

    // WidgetList nodes reference a reusable library template through `target`.
    // Static renders do not have runtime array data, so attach one template
    // instance under the list. This mirrors the authored structure without
    // inventing list rows or canvas-specific defaults.
    let list_targets: Vec<(BbNodeId, BbNodeId)> = nodes
        .values()
        .filter_map(|n| {
            let target = n.raw.get("target")?.as_str().and_then(parse_points_to)?;
            Some((n.id, target))
        })
        .collect();
    for (list_id, target_id) in list_targets {
        if let Some(target) = nodes.get_mut(&target_id) {
            if target.parent.is_none() && library_ids.contains(&target_id) {
                target.parent = Some(list_id);
            }
        }
    }

    // ── Second pass: wire up children. ──────────────────────────────────────
    // Collect (parent_id, child_id) pairs first to avoid borrow conflicts.
    let parent_child_pairs: Vec<(BbNodeId, BbNodeId)> = nodes
        .values()
        .filter_map(|n| n.parent.map(|p| (p, n.id)))
        .collect();

    for (parent_id, child_id) in parent_child_pairs {
        if let Some(parent_node) = nodes.get_mut(&parent_id) {
            parent_node.children.push(child_id);
        } else {
            log::warn!(
                "bb_scene: node ptr:{child_id} references unknown parent ptr:{parent_id}"
            );
        }
    }

    // Sort children by insertion order (by child id for determinism).
    for node in nodes.values_mut() {
        node.children.sort_unstable();
    }

    // ── Collect roots. ───────────────────────────────────────────────────────
    let mut roots: Vec<BbNodeId> = nodes
        .values()
        .filter(|n| n.parent.is_none() && !library_ids.contains(&n.id))
        .map(|n| n.id)
        .collect();
    roots.sort_unstable();

    Ok(BbScene { canvas_size: (canvas_w, canvas_h), roots, nodes, operations })
}

// ─────────────────────────────────────────────────────────────────────────────
// Node parser
// ─────────────────────────────────────────────────────────────────────────────

fn parse_node_into(
    raw_item: &serde_json::Value,
    nodes: &mut BTreeMap<BbNodeId, BbNode>,
    synthetic_base: &mut u32,
    source: &str,
) -> Option<BbNodeId> {
    let needs_synthetic = raw_item
        .get("_Pointer_")
        .and_then(|v| v.as_str())
        .is_none();

    let node_result = if needs_synthetic {
        let synthetic_id = *synthetic_base;
        *synthetic_base += 1;
        parse_node_with_id(raw_item, synthetic_id)
    } else {
        parse_node(raw_item)
    };

    match node_result {
        Ok(node) => {
            let id = node.id;
            nodes.insert(id, node);
            Some(id)
        }
        Err(e) => {
            log::warn!("bb_scene: skipping {source} item: {e}");
            None
        }
    }
}

fn parse_node(raw: &serde_json::Value) -> Result<BbNode, String> {
    let pointer_str = raw
        .get("_Pointer_")
        .and_then(|v| v.as_str())
        .ok_or("scene item missing _Pointer_")?;
    let id = parse_ptr(pointer_str)
        .ok_or_else(|| format!("invalid _Pointer_ value: {pointer_str}"))?;
    parse_node_with_id(raw, id)
}

fn parse_node_with_id(raw: &serde_json::Value, id: BbNodeId) -> Result<BbNode, String> {
    let type_str = raw
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let ty = BbNodeType::from_type_str(type_str);

    let parent = raw
        .get("parent")
        .and_then(|v| v.as_str())
        .and_then(parse_points_to);

    let name = raw
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let style_tag_uuids = parse_style_tags(raw.get("styleTags"));
    let is_active = raw.get("isActive").and_then(|v| v.as_bool()).unwrap_or(true);
    let layer = raw.get("layer").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    // BB JSON convention: missing `alpha` means fully opaque (1.0), not transparent (0.0).
    let alpha_raw = raw.get("alpha").and_then(|v| v.as_f64());
    let alpha = alpha_raw.map(|v| v as f32).unwrap_or(1.0);

    let position = parse_vec3(raw.get("position")).unwrap_or_default();
    let position_offset = parse_vec3(raw.get("positionOffset")).unwrap_or_default();
    let sizing = parse_sizing(raw.get("sizing"));
    let padding = parse_trbl(raw.get("padding"));
    let margin = parse_trbl(raw.get("margin"));
    let pivot = parse_vec2_from_vec3(raw.get("pivot"));
    let anchor = parse_vec2_from_vec3(raw.get("anchor"));

    let background = parse_background(raw);
    let border = parse_border(raw.get("border"));
    let radial = parse_radial(raw.get("radialTransform"));

    let text = if matches!(ty, BbNodeType::WidgetTextField) {
        Some(parse_text(raw))
    } else {
        None
    };

    let icon = if matches!(ty, BbNodeType::WidgetIcon) {
        Some(parse_icon(raw))
    } else {
        None
    };

    Ok(BbNode {
        id,
        parent,
        children: Vec::new(),
        ty,
        name,
        style_tag_uuids,
        is_active,
        layer,
        alpha,
        position,
        position_offset,
        sizing,
        padding,
        margin,
        pivot,
        anchor,
        background,
        border,
        radial,
        text,
        icon,
        raw: raw.clone(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Field parsers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse `"ptr:N"` → `Some(N)`.
fn parse_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

/// Parse `"_PointsTo_:ptr:N"` → `Some(N)`.
fn parse_points_to(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:").and_then(parse_ptr)
}

fn f32_field(obj: &serde_json::Value, key: &str) -> f32 {
    obj.get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32
}

fn parse_vec3(v: Option<&serde_json::Value>) -> Option<Vec3> {
    let obj = v?;
    Some(Vec3 { x: f32_field(obj, "x"), y: f32_field(obj, "y"), z: f32_field(obj, "z") })
}

/// Extract a Vec2 from a field that may be a Vec3 or Vec2 JSON object.
fn parse_vec2_from_vec3(v: Option<&serde_json::Value>) -> Vec2 {
    match v {
        Some(obj) => Vec2 { x: f32_field(obj, "x"), y: f32_field(obj, "y") },
        None => Vec2::default(),
    }
}

fn parse_bb_value(v: Option<&serde_json::Value>) -> BbValue {
    let obj = match v {
        Some(o) if o.is_object() => o,
        _ => return BbValue::default(),
    };
    let value = f32_field(obj, "value");
    let behavior = obj.get("behavior").and_then(|b| b.as_str()).unwrap_or("Fixed");
    match behavior {
        "Fixed" => BbValue::Fixed(value),
        "Percent" => BbValue::Percent(value),
        other => BbValue::Other { value, behavior: other.to_owned() },
    }
}

fn parse_sizing(v: Option<&serde_json::Value>) -> BbSizing {
    let obj = match v {
        Some(o) => o,
        None => return BbSizing::default(),
    };
    BbSizing {
        width: parse_bb_value(obj.get("width")),
        height: parse_bb_value(obj.get("height")),
    }
}

fn parse_trbl(v: Option<&serde_json::Value>) -> BbTrbl {
    let obj = match v {
        Some(o) => o,
        None => return BbTrbl::default(),
    };
    BbTrbl {
        top: f32_field(obj, "top"),
        right: f32_field(obj, "right"),
        bottom: f32_field(obj, "bottom"),
        left: f32_field(obj, "left"),
    }
}

fn parse_style_tags(v: Option<&serde_json::Value>) -> Vec<String> {
    let arr = match v.and_then(|a| a.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|tag| tag.get("_RecordId_").and_then(|r| r.as_str()).map(str::to_owned))
        .collect()
}

/// Try to parse a colour value from a JSON value into `[r, g, b, a]` (0–1).
///
/// Handles `{ "r": …, "g": …, "b": …, "a": … }` with either 0–255 or 0–1
/// component ranges (heuristic: if any component > 1.0, assume 0–255).
/// Returns `None` for null, strings, or unrecognised shapes.
fn parse_colour(v: Option<&serde_json::Value>) -> Option<[f32; 4]> {
    let obj = v?;
    if obj.is_null() {
        return None;
    }
    let r = obj.get("r").and_then(|c| c.as_f64())? as f32;
    let g = obj.get("g").and_then(|c| c.as_f64())? as f32;
    let b = obj.get("b").and_then(|c| c.as_f64())? as f32;
    let a = obj.get("a").and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        Some([r / 255.0, g / 255.0, b / 255.0, a / 255.0])
    } else {
        Some([r, g, b, a])
    }
}

fn parse_background(node: &serde_json::Value) -> Option<BbBackground> {
    let bg = node.get("background")?;
    if bg.is_null() {
        return None;
    }

    // Honour the BuildingBlocks `enable` flag: when an authored background has
    // `enable: false`, the engine skips its fill_colour entirely and the panel
    // contributes only via children. Some scenes carry a non-null `color`
    // (often a debug yellow RGBA 255,255,0,255) gated behind `enable: false`;
    // rendering that fill produces spurious opaque rectangles. The svgFill
    // and segmentedFill sub-features are independent and remain available.
    let bg_enable = bg
        .get("enable")
        .and_then(|e| e.as_bool())
        .unwrap_or(true);

    let fill_colour = if bg_enable {
        bg.get("color").and_then(|c| parse_colour(Some(c)))
    } else {
        None
    };

    let svg_fill_path = node
        .get("svgFill")
        .and_then(|s| s.get("svgPath"))
        .and_then(|p| p.as_str())
        .filter(|p| !p.is_empty())
        .map(str::to_owned);

    let segmented_fill = node
        .get("segmentedFill")
        .and_then(|sf| {
            let enabled = sf.get("enable").and_then(|e| e.as_bool()).unwrap_or(false);
            if enabled {
                let angle = f32_field(sf, "angle");
                Some(BbSegmentedFill { angle })
            } else {
                None
            }
        });

    Some(BbBackground { fill_colour, svg_fill_path, segmented_fill })
}

fn parse_border_side(v: Option<&serde_json::Value>) -> BbBorderSide {
    let obj = match v {
        Some(o) => o,
        None => return BbBorderSide::default(),
    };
    BbBorderSide {
        width: f32_field(obj, "width"),
        colour: parse_colour(obj.get("color")),
    }
}

fn parse_border(v: Option<&serde_json::Value>) -> Option<BbBorder> {
    let obj = v?;
    if obj.is_null() {
        return None;
    }
    Some(BbBorder {
        top: parse_border_side(obj.get("top")),
        right: parse_border_side(obj.get("right")),
        bottom: parse_border_side(obj.get("bottom")),
        left: parse_border_side(obj.get("left")),
    })
}

fn parse_radial(v: Option<&serde_json::Value>) -> Option<BbRadialTransform> {
    let obj = v?;
    if obj.is_null() {
        return None;
    }
    let transform_multiplier = f32_field(obj, "transformMultiplier");
    let curvature_axis = obj
        .get("curvatureAxis")
        .and_then(|a| a.as_str())
        .unwrap_or("Z")
        .to_owned();
    Some(BbRadialTransform { transform_multiplier, curvature_axis })
}

fn parse_text(node: &serde_json::Value) -> BbText {
    let alignment = node
        .get("textAlignment")
        .and_then(|v| v.as_str())
        .unwrap_or("Left")
        .to_owned();

    // Static text string — typically bound at runtime; default to empty.
    let string = node
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    // Font record — not present in current fixtures but captured if available.
    let font_record = node
        .get("fontRecord")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // Font size — use a sensible default when absent.
    let font_size = node
        .get("fontSize")
        .map(|v| parse_bb_value(Some(v)))
        .unwrap_or(BbValue::Fixed(12.0));

    let colour = node
        .get("textColor")
        .or_else(|| node.get("textColour"))
        .and_then(|c| parse_colour(Some(c)));

    BbText { string, font_record, font_size, alignment, colour }
}

fn parse_icon(node: &serde_json::Value) -> BbIcon {
    let image_record = node
        .get("iconProperties")
        .and_then(|ip| ip.get("customIcon"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // Tint colour — not present in current fixtures but captured if available.
    let tint_colour = node
        .get("iconProperties")
        .and_then(|ip| ip.get("color"))
        .and_then(|c| parse_colour(Some(c)));

    BbIcon { image_record, tint_colour }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/canvas/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse fixture {name} as JSON: {e}"))
    }

    fn count_type(scene: &BbScene, ty: &BbNodeType) -> usize {
        scene.nodes.values().filter(|n| &n.ty == ty).count()
    }

    // ── MC_S_Target_Master ───────────────────────────────────────────────────

    #[test]
    fn target_master_node_count_and_types() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 2, "expected 2 nodes");
        assert_eq!(count_type(&scene, &BbNodeType::DisplayWidget), 1);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 1);
    }

    #[test]
    fn target_master_root_and_parent() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.roots.len(), 1);
        let root_id = scene.roots[0];
        let root = &scene.nodes[&root_id];
        assert!(root.parent.is_none(), "root should have no parent");

        // The non-root node's parent must equal the root id.
        let child = scene.nodes.values().find(|n| n.parent.is_some()).expect("no child found");
        assert_eq!(child.parent, Some(root_id));
    }

    #[test]
    fn target_master_canvas_size() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!(scene.canvas_size.0 > 0.0, "canvas width should be positive");
        assert!(scene.canvas_size.1 > 0.0, "canvas height should be positive");
    }

    #[test]
    fn target_master_root_children_wired() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let root_id = scene.roots[0];
        let root = &scene.nodes[&root_id];
        assert_eq!(root.children.len(), 1, "root should have exactly 1 child");
    }

    // ── MC_S_Self_Master ─────────────────────────────────────────────────────

    #[test]
    fn self_master_node_count_and_types() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 7, "expected 7 nodes");
        assert_eq!(count_type(&scene, &BbNodeType::DisplayWidget), 1);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 6);
    }

    #[test]
    fn self_master_single_root() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert_eq!(scene.roots.len(), 1);
        let root = &scene.nodes[&scene.roots[0]];
        assert!(root.parent.is_none());
        assert_eq!(root.ty, BbNodeType::DisplayWidget);
    }

    #[test]
    fn self_master_canvas_size_1920x1080() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!((scene.canvas_size.0 - 1920.0).abs() < f32::EPSILON);
        assert!((scene.canvas_size.1 - 1080.0).abs() < f32::EPSILON);
    }

    #[test]
    fn self_master_root_has_six_children() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let root = &scene.nodes[&scene.roots[0]];
        assert_eq!(root.children.len(), 6);
    }

    // ── BB_ScreenRadar ───────────────────────────────────────────────────────

    #[test]
    fn radar_node_count_and_types() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 25, "expected 25 nodes");
        assert_eq!(count_type(&scene, &BbNodeType::DisplayWidget), 6);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 5);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetIcon), 5);
        assert_eq!(count_type(&scene, &BbNodeType::ComponentGeneralButtonSecondary), 4);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCard), 3);
        assert_eq!(count_type(&scene, &BbNodeType::ComponentGeneralButton), 1);
        assert_eq!(count_type(&scene, &BbNodeType::WidgetTextField), 1);
    }

    #[test]
    fn radar_canvas_size_positive() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!(scene.canvas_size.0 > 0.0);
        assert!(scene.canvas_size.1 > 0.0);
    }

    #[test]
    fn radar_text_field_alignment() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let tf = scene
            .nodes
            .values()
            .find(|n| n.ty == BbNodeType::WidgetTextField)
            .expect("no WidgetTextField found");
        // In the fixture the textAlignment is "Center".
        assert!(!tf.text.as_ref().unwrap().alignment.is_empty());
    }

    #[test]
    fn radar_icon_nodes_parsed() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        // All WidgetIcon nodes should have their icon field populated.
        for node in scene.nodes.values().filter(|n| n.ty == BbNodeType::WidgetIcon) {
            assert!(node.icon.is_some(), "WidgetIcon node should have icon field");
        }
    }

    #[test]
    fn radar_style_tags_parsed() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        // Verify that at least one node has style tags — confirms the parsing
        // logic actually extracts UUIDs from the styleTags array.
        let any_with_tags = scene.nodes.values().any(|n| !n.style_tag_uuids.is_empty());
        assert!(any_with_tags, "expected at least one node with style tags");
    }

    // ── EC_PowerManagement ───────────────────────────────────────────────────

    #[test]
    fn power_management_single_widget_canvas() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.nodes.len(), 1, "expected 1 node");
        assert_eq!(count_type(&scene, &BbNodeType::WidgetCanvas), 1);
    }

    #[test]
    fn power_management_root_no_parent() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");

        assert_eq!(scene.roots.len(), 1);
        let root = &scene.nodes[&scene.roots[0]];
        assert!(root.parent.is_none());
        assert_eq!(root.ty, BbNodeType::WidgetCanvas);
    }

    #[test]
    fn power_management_canvas_size_positive() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        assert!(scene.canvas_size.0 > 0.0);
        assert!(scene.canvas_size.1 > 0.0);
    }

    #[test]
    fn power_management_node_is_active() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse failed");
        let root = &scene.nodes[&scene.roots[0]];
        assert!(root.is_active, "root node should be active");
    }
}
