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
    /// Canvas coordinate scaling mode from `_RecordValue_.coordinateMethod`.
    pub coordinate_method: BbCoordinateMethod,
    /// IDs of all root nodes (nodes with no parent).
    pub roots: Vec<BbNodeId>,
    /// All nodes keyed by their pointer ID, in stable insertion order.
    pub nodes: BTreeMap<BbNodeId, BbNode>,
    /// Raw BuildingBlocks operations array used for runtime bindings.
    pub operations: Vec<serde_json::Value>,
}

/// Authoring rule for mapping canvas units into the render target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BbCoordinateMethod {
    UseRaw,
    Auto,
    AspectOverridesWidth,
    AspectOverridesHeight,
}

impl Default for BbCoordinateMethod {
    fn default() -> Self {
        Self::UseRaw
    }
}

impl BbCoordinateMethod {
    pub(super) fn from_raw(value: Option<&str>) -> Self {
        match value.unwrap_or("useRaw") {
            "auto" => Self::Auto,
            "aspectOverridesWidth" => Self::AspectOverridesWidth,
            "aspectOverridesHeight" => Self::AspectOverridesHeight,
            _ => Self::UseRaw,
        }
    }
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
    pub(super) fn from_type_str(s: &str) -> Self {
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
    /// Built-in icon preset from `iconProperties.iconPreset`.
    pub icon_preset: Option<String>,
    /// Tint colour [r, g, b, a] in 0–1 range, if parseable.
    pub tint_colour: Option<[f32; 4]>,
}

