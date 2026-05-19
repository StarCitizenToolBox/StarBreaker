//! BuildingBlocks layout engine — pixel-space rect resolver.
//!
//! Turns a merged [`BbScene`] (produced by [`crate::bb_resolve`]) into a
//! [`LayoutResult`] that maps every [`BbNodeId`] to a [`Rect`] in screen-pixel
//! coordinates, plus a deterministic DFS draw order.
//!
//! # Coordinate system
//! Screen-space, +x right, +y down, units = pixels.  The BB authoring system
//! also uses +x right, +y down (verified: in the Clipper reference screenshots
//! a node at `position.y = 100` canvas units appears below the top edge).
//!
//! # Percent sizing
//! `BbValue::Percent(p)` stores the raw `value` from the DataCore JSON.  In
//! every tested fixture `1.0` represents 100 % of the parent dimension (i.e.
//! the value is already a fraction, **not** a 0–100 percentage).  The task
//! specification stated "0–100" but fixture inspection (`MC_S_Target_Master`,
//! `BB_ScreenRadar`, etc.) shows `value: 1` for "fill parent" and `value: 0.08`
//! for an 8 % column.  We therefore compute `parent_inner.w * p` directly.
//!
//! # Margin simplification (Phase A1)
//! For Phase A1, `margin` is applied as a top-left offset only: positive
//! `margin.left` shifts the outer rect rightward, positive `margin.top` shifts
//! it downward.  Full TRBL margin layout (e.g. shrinking the available space
//! for siblings) is deferred to Phase A3.
//!
//! # Stacking
//! BB does not use flexbox by default.  All siblings are laid out using the
//! same parent inner rect as origin (z-order overlay).  If a parent node's
//! `_Type_` is `BuildingBlocks_FlexContainer` (or its raw JSON indicates a flex
//! layout policy), a warning is logged and the same overlay fallback is used.
//! Flex support is deferred to Phase A6.

use std::collections::BTreeMap;

use image::{Rgba, RgbaImage};
use log::warn;

use crate::bb_scene::{BbNodeId, BbNodeType, BbScene, BbValue};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Axis-aligned rectangle in pixel space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge (pixels from canvas left).
    pub x: f32,
    /// Top edge (pixels from canvas top).
    pub y: f32,
    /// Width in pixels.
    pub w: f32,
    /// Height in pixels.
    pub h: f32,
}

impl Rect {
    /// Return a new rect inset by `(top, right, bottom, left)` pixels.
    ///
    /// If the inset would make the dimension negative the result is clamped to
    /// zero size at the centre of the corresponding axis.
    pub fn inset(&self, t: f32, r: f32, b: f32, l: f32) -> Rect {
        let x = self.x + l;
        let y = self.y + t;
        let w = (self.w - l - r).max(0.0);
        let h = (self.h - t - b).max(0.0);
        Rect { x, y, w, h }
    }

    /// Return `true` if `(px, py)` lies inside (or on the edge of) this rect.
    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    /// Return the intersection of this rect with `other`, or `None` when they
    /// do not overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        if x1 > x0 && y1 > y0 {
            Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
        } else {
            None
        }
    }

    /// Centre point of this rect.
    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

/// Output of [`layout`]: pixel-space rects for every node and a DFS draw order.
pub struct LayoutResult {
    /// Canvas rect: always `(0, 0, target_w, target_h)`.
    pub canvas: Rect,
    /// Pixel-space outer rect for every node keyed by [`BbNodeId`].
    ///
    /// Inactive nodes (`is_active == false`) are still present in `rects` —
    /// their geometry may affect parent layout — but they are absent from
    /// [`draw_order`].
    pub rects: BTreeMap<BbNodeId, Rect>,
    /// Uniform authoring-canvas-to-target scale applied to fixed measurements.
    pub canvas_scale: f32,
    /// Render order.
    ///
    /// DFS from each root, parent before children.  Siblings are sorted by
    /// `(layer ascending, node-id ascending)`.  Node id order matches the
    /// declaration order in the source JSON (ptr values increase monotonically
    /// with array position).  Inactive nodes are excluded.
    pub draw_order: Vec<BbNodeId>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Layout entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Compute pixel-space rects for every node in `scene` at `(target_w, target_h)`.
///
/// # Scale and letterboxing
/// The BB canvas declares an authoring coordinate size (`scene.canvas_size`).  A
/// **uniform** scale factor `scale = min(target_w / canvas_w, target_h / canvas_h)`
/// is applied so that `Fixed`-unit node positions / sizes are never stretched or
/// squeezed disproportionately.  The scaled canvas is centred within the target:
///
/// ```text
/// letterbox_x = (target_w − canvas_w × scale) / 2
/// letterbox_y = (target_h − canvas_h × scale) / 2
/// ```
///
/// Roots receive the centred canvas rect as their `parent_inner`; percent-based
/// children fill that rect naturally.
///
/// # Panics
/// Never panics on well-formed input.  Unknown `BbValue` behaviors produce
/// warnings and fall back to filling the parent dimension.
pub fn layout(scene: &BbScene, target_w: u32, target_h: u32) -> LayoutResult {
    let canvas_scale = if scene.canvas_size.0 > 0.0 && scene.canvas_size.1 > 0.0 {
        let sx = target_w as f32 / scene.canvas_size.0;
        let sy = target_h as f32 / scene.canvas_size.1;
        sx.min(sy)
    } else {
        1.0
    };

    // Centred canvas rect inside the target.
    let scaled_w = scene.canvas_size.0 * canvas_scale;
    let scaled_h = scene.canvas_size.1 * canvas_scale;
    let offset_x = ((target_w as f32 - scaled_w) * 0.5).max(0.0);
    let offset_y = ((target_h as f32 - scaled_h) * 0.5).max(0.0);
    let canvas_rect = Rect { x: offset_x, y: offset_y, w: scaled_w, h: scaled_h };

    // The LayoutResult.canvas always spans the full target.
    let canvas = Rect { x: 0.0, y: 0.0, w: target_w as f32, h: target_h as f32 };

    let mut rects: BTreeMap<BbNodeId, Rect> = BTreeMap::new();
    let mut draw_order: Vec<BbNodeId> = Vec::new();

    // Collect roots and sort them for determinism: (layer, id).
    let mut roots: Vec<BbNodeId> = scene.roots.clone();
    roots.sort_by_key(|&id| {
        let layer = scene.nodes.get(&id).map(|n| n.layer).unwrap_or(0);
        (layer, id)
    });

    for root_id in roots {
        layout_node(
            root_id,
            canvas_rect,
            scene,
            canvas_scale,
            canvas_scale,
            &mut rects,
            &mut draw_order,
        );
    }

    LayoutResult { canvas, rects, draw_order, canvas_scale }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recursive layout
// ─────────────────────────────────────────────────────────────────────────────

fn layout_node(
    node_id: BbNodeId,
    parent_inner: Rect,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
    let Some(node) = scene.nodes.get(&node_id) else { return };

    // ── 1. Resolve outer dimensions ─────────────────────────────────────────

    let outer_w = resolve_value(&node.sizing.width, parent_inner.w, parent_inner.h, csx, true);
    let outer_h = resolve_value(&node.sizing.height, parent_inner.h, parent_inner.w, csy, false);

    // ── 2. Anchor / pivot / position ────────────────────────────────────────
    //
    // anchor: normalised point within the *parent inner rect* that the node
    //         "attaches" to.
    // pivot:  normalised point within the *node itself* that lands on the
    //         anchored position.
    // position / positionOffset: additional offset in canvas authoring units.
    //
    // Formula:
    //   anchor_world_x = parent_inner.x + parent_inner.w * anchor.x
    //                    + (position.x + positionOffset.x) * csx
    //   outer.x        = anchor_world_x - outer_w * pivot.x

    let pos_x = (node.position.x + node.position_offset.x) * csx;
    let pos_y = (node.position.y + node.position_offset.y) * csy;

    let anchor_world_x = parent_inner.x + parent_inner.w * node.anchor.x + pos_x;
    let anchor_world_y = parent_inner.y + parent_inner.h * node.anchor.y + pos_y;

    let outer_x = anchor_world_x - outer_w * node.pivot.x;
    let outer_y = anchor_world_y - outer_h * node.pivot.y;

    // ── 3. Margin (Phase A1: top-left offset only) ───────────────────────────
    let outer_x = outer_x + node.margin.left * csx;
    let outer_y = outer_y + node.margin.top * csy;

    let outer_rect = Rect { x: outer_x, y: outer_y, w: outer_w, h: outer_h };

    layout_node_with_rect(node_id, outer_rect, scene, csx, csy, rects, draw_order);
}

/// Register `outer_rect` for `node_id` and recurse into children, bypassing
/// sizing and positioning.  Used by `layout_flex_children` so that flex items
/// fill their allocated slot rather than their intrinsic `sizing` value.
fn layout_node_with_rect(
    node_id: BbNodeId,
    outer_rect: Rect,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
    let Some(node) = scene.nodes.get(&node_id) else { return };

    // ── 4. Inner rect = outer rect inset by padding ──────────────────────────
    let inner_rect = outer_rect.inset(
        node.padding.top * csy,
        node.padding.right * csx,
        node.padding.bottom * csy,
        node.padding.left * csx,
    );

    rects.insert(node_id, outer_rect);

    // Add to draw order only when the node is active.
    if node.is_active {
        draw_order.push(node_id);
    }

    // ── 5. Recurse into children ─────────────────────────────────────────────
    // Sort children by (layer asc, id asc).
    let mut children: Vec<BbNodeId> = node.children.clone();
    children.sort_by_key(|&child_id| {
        let layer = scene.nodes.get(&child_id).map(|n| n.layer).unwrap_or(0);
        (layer, child_id)
    });

    // Detect FlexContainer layout policy — if present, use flex layout instead
    // of overlay for child positioning.
    let flex_policy = node.raw.get("layoutPolicy").filter(|v| {
        v.get("_Type_")
            .and_then(|t| t.as_str())
            .map(|t| t.contains("FlexContainer"))
            .unwrap_or(false)
    });

    if let Some(flex) = flex_policy {
        layout_flex_children(
            &children,
            inner_rect,
            flex,
            scene,
            csx,
            csy,
            rects,
            draw_order,
        );
    } else {
        for child_id in children {
            layout_node(child_id, inner_rect, scene, csx, csy, rects, draw_order);
        }
    }
}

/// Lay out `children` inside `container` according to a `BuildingBlocks_FlexContainer`
/// policy.
///
/// Supports Row and Column directions with `growProportion`-based main-axis
/// sizing and Stretch cross-axis alignment (matches the BB default).
/// Children that lack a `BuildingBlocks_FlexItem` policy are laid out with
/// their own sizing rules as overlay children (non-flex fallback).
fn layout_flex_children(
    children: &[BbNodeId],
    container: Rect,
    flex: &serde_json::Value,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
    let direction = flex
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("Row");
    let is_row = !direction.eq_ignore_ascii_case("Column");

    // Spacing between items (columnSpacing for Row, rowSpacing for Column).
    let item_spacing = if is_row {
        flex.get("columnSpacing")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32
            * csx
    } else {
        flex.get("rowSpacing")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32
            * csy
    };

    // Separate flex children (with growProportion) from non-flex children.
    struct FlexChild {
        id: BbNodeId,
        grow: f32,
    }
    let mut flex_children: Vec<FlexChild> = Vec::new();
    let mut non_flex: Vec<BbNodeId> = Vec::new();

    for &child_id in children {
        let Some(child_node) = scene.nodes.get(&child_id) else { continue };
        let grow = child_node
            .raw
            .get("layoutPolicyItem")
            .and_then(|lpi| lpi.get("growProportion"))
            .and_then(|v| v.as_f64())
            .map(|g| g as f32)
            .unwrap_or(0.0);
        if grow > 0.0 {
            flex_children.push(FlexChild { id: child_id, grow });
        } else {
            non_flex.push(child_id);
        }
    }

    // Lay out non-flex children with overlay fallback.
    for child_id in non_flex {
        layout_node(child_id, container, scene, csx, csy, rects, draw_order);
    }

    if flex_children.is_empty() {
        return;
    }

    let n = flex_children.len();
    let total_spacing = item_spacing * (n as f32 - 1.0).max(0.0);
    let total_grow: f32 = flex_children.iter().map(|c| c.grow).sum();

    // Main axis: available space divided by grow proportions.
    let main_available = if is_row {
        (container.w - total_spacing).max(0.0)
    } else {
        (container.h - total_spacing).max(0.0)
    };

    let mut cursor = if is_row { container.x } else { container.y };

    for FlexChild { id, grow } in &flex_children {
        let main_size = (grow / total_grow) * main_available;
        // Cross axis: stretch to fill container (BB default: itemAlignment=Stretch).
        let child_rect = if is_row {
            Rect {
                x: cursor,
                y: container.y,
                w: main_size,
                h: container.h,
            }
        } else {
            Rect {
                x: container.x,
                y: cursor,
                w: container.w,
                h: main_size,
            }
        };
        cursor += main_size + item_spacing;
        layout_node_with_rect(*id, child_rect, scene, csx, csy, rects, draw_order);
    }
}


///
/// - `Fixed(v)` → `v * canvas_scale`  
/// - `Percent(p)` → `primary_dim * p` (p is a fraction 0–1; value `1.0` = 100 %)
/// - `Other` with a recognised cross-axis behavior (`PercentOfY` for a width
///   dimension, `PercentOfX` for height) → `cross_dim * value`.  All other
///   unknown behaviors log a warning and fall back to `primary_dim` (fill).
fn resolve_value(v: &BbValue, primary_dim: f32, cross_dim: f32, canvas_scale: f32, is_width: bool) -> f32 {
    match v {
        BbValue::Fixed(px) => px * canvas_scale,
        BbValue::Percent(p) => primary_dim * p,
        BbValue::Other { value, behavior } => {
            match behavior.as_str() {
                // Cross-axis percent: width as % of parent height, or vice-versa.
                "PercentOfY" if is_width => cross_dim * value,
                "PercentOfX" if !is_width => cross_dim * value,
                // "Auto": numeric value here is typically a content-fit hint
                // baked at author time, but we do not yet have a proper
                // content-measurement pass. Containers and text widgets both
                // produced more reference-matching results when Auto falls
                // back to "fill parent" — text is then free-positioned by
                // its anchor/pivot/position inside a parent that owns the
                // visible area. Children that needed tighter bounds will be
                // revisited when content-measurement lands.
                "Auto" => primary_dim,
                other => {
                    warn!(
                        "bb_layout: unknown sizing behavior {:?} (value={}) — \
                         falling back to fill",
                        other, value,
                    );
                    primary_dim
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Wireframe renderer
// ─────────────────────────────────────────────────────────────────────────────

/// Render a debug wireframe overlay of `scene` at `(target_w × target_h)`.
///
/// Each laid-out node is drawn as:
/// - A filled rect with a per-type colour at ~30 % alpha.
/// - A 1-pixel white outline at ~80 % alpha.
///
/// The background is `#202020`.  Inactive nodes are not drawn.
///
/// The colour is derived by hashing the node's type-name string to an HSV hue,
/// so no hard-coded type→colour table is needed.
pub fn render_wireframe(scene: &BbScene, target_w: u32, target_h: u32) -> RgbaImage {
    let result = layout(scene, target_w, target_h);

    let mut img = RgbaImage::from_pixel(target_w, target_h, Rgba([0x20, 0x20, 0x20, 0xFF]));

    for &node_id in &result.draw_order {
        let Some(node) = scene.nodes.get(&node_id) else { continue };
        let Some(&outer) = result.rects.get(&node_id) else { continue };

        let type_name = type_name_str(&node.ty);
        let fill_colour = type_colour_fill(type_name);
        let outline_colour = Rgba([0xFF, 0xFF, 0xFF, (0.80 * 255.0) as u8]);

        draw_rect_filled(&mut img, outer, fill_colour);
        draw_rect_outline(&mut img, outer, outline_colour);
    }

    img
}

/// Derive a stable type-name string for colouring (strips `BuildingBlocks_`).
fn type_name_str(ty: &BbNodeType) -> &str {
    match ty {
        BbNodeType::DisplayWidget => "DisplayWidget",
        BbNodeType::WidgetCanvas => "WidgetCanvas",
        BbNodeType::WidgetIcon => "WidgetIcon",
        BbNodeType::WidgetCard => "WidgetCard",
        BbNodeType::WidgetTextField => "WidgetTextField",
        BbNodeType::ComponentGeneralButton => "ComponentGeneralButton",
        BbNodeType::ComponentGeneralButtonSecondary => "ComponentGeneralButtonSecondary",
        BbNodeType::WidgetImage => "WidgetImage",
        BbNodeType::WidgetText => "WidgetText",
        BbNodeType::WidgetCustomShape => "WidgetCustomShape",
        BbNodeType::WidgetBodyBackground => "WidgetBodyBackground",
        BbNodeType::Other(s) => {
            s.strip_prefix("BuildingBlocks_").unwrap_or(s.as_str())
        }
    }
}

/// Hash a type-name string to a stable RGBA fill colour at 30 % alpha.
fn type_colour_fill(type_name: &str) -> Rgba<u8> {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    type_name.hash(&mut hasher);
    let hash = hasher.finish();
    // Map the hash to a hue in [0, 360).
    let hue = (hash % 360) as f32;
    let (r, g, b) = hsv_to_rgb(hue, 0.70, 0.85);
    Rgba([
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
        (0.30 * 255.0) as u8,
    ])
}

/// Convert HSV (h in 0–360, s and v in 0–1) to linear RGB (0–1 per channel).
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let h1 = h / 60.0;
    let x = c * (1.0 - (h1 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h1 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r1 + m, g1 + m, b1 + m)
}

/// Alpha-blend a single pixel.  `overlay` alpha drives the blend; the output
/// alpha is always 255 (fully opaque canvas).
fn blend_pixel(base: &mut Rgba<u8>, overlay: Rgba<u8>) {
    let a = overlay[3] as f32 / 255.0;
    let ia = 1.0 - a;
    for i in 0..3 {
        base[i] = (base[i] as f32 * ia + overlay[i] as f32 * a) as u8;
    }
    base[3] = 255;
}

/// Draw a filled rectangle with alpha blending.  Clips to image bounds.
fn draw_rect_filled(img: &mut RgbaImage, rect: Rect, colour: Rgba<u8>) {
    let (iw, ih) = img.dimensions();
    let x0 = (rect.x.floor() as i32).max(0) as u32;
    let y0 = (rect.y.floor() as i32).max(0) as u32;
    let x1 = ((rect.x + rect.w).ceil() as i32).min(iw as i32) as u32;
    let y1 = ((rect.y + rect.h).ceil() as i32).min(ih as i32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = img.get_pixel_mut(x, y);
            blend_pixel(px, colour);
        }
    }
}

/// Draw a 1-pixel outline of a rectangle with alpha blending.  Clips to bounds.
fn draw_rect_outline(img: &mut RgbaImage, rect: Rect, colour: Rgba<u8>) {
    let (iw, ih) = img.dimensions();
    let x0 = rect.x.floor() as i32;
    let y0 = rect.y.floor() as i32;
    let x1 = (rect.x + rect.w).ceil() as i32 - 1;
    let y1 = (rect.y + rect.h).ceil() as i32 - 1;

    // Draw all four sides.
    for x in x0..=x1 {
        for &y in &[y0, y1] {
            if x >= 0 && x < iw as i32 && y >= 0 && y < ih as i32 {
                let px = img.get_pixel_mut(x as u32, y as u32);
                blend_pixel(px, colour);
            }
        }
    }
    for y in y0..=y1 {
        for &x in &[x0, x1] {
            if x >= 0 && x < iw as i32 && y >= 0 && y < ih as i32 {
                let px = img.get_pixel_mut(x as u32, y as u32);
                blend_pixel(px, colour);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/canvas/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"))
    }

    // ── A1.6 — fixture-based layout tests ────────────────────────────────────

    /// MC_S_Target_Master: root DisplayWidget fills the canvas at Percent(1) sizing.
    #[test]
    fn mc_s_target_master_root_fills_canvas() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1600, 900);

        // Root rect should cover ≥99 % of target width (Percent(1.0) → 100 %).
        let root_id = *scene.roots.first().expect("at least one root");
        let root_rect = result.rects[&root_id];
        let ratio_w = root_rect.w / 1600.0;
        let ratio_h = root_rect.h / 900.0;
        assert!(
            (ratio_w - 1.0).abs() < 0.02,
            "root width ratio {ratio_w} not within 2 % of 1.0"
        );
        assert!(
            (ratio_h - 1.0).abs() < 0.02,
            "root height ratio {ratio_h} not within 2 % of 1.0"
        );
    }

    /// MC_S_Self_Master: 6 WidgetCanvas siblings all share the same parent
    /// inner rect (i.e. their x/y origins all lie inside the root's inner
    /// rect, modulo anchor offsets).
    #[test]
    fn mc_s_self_master_siblings_share_parent_inner() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1600, 900);

        // Find the root node.
        let root_id = *scene.roots.first().expect("root");
        let root_rect = result.rects[&root_id];

        // All WidgetCanvas siblings should have positive area.
        let widget_canvas_nodes: Vec<BbNodeId> = scene
            .nodes
            .values()
            .filter(|n| matches!(n.ty, BbNodeType::WidgetCanvas))
            .map(|n| n.id)
            .collect();

        assert!(
            widget_canvas_nodes.len() >= 6,
            "expected ≥6 WidgetCanvas nodes, got {}",
            widget_canvas_nodes.len()
        );

        for id in &widget_canvas_nodes {
            let rect = result.rects[id];
            assert!(rect.w > 0.0, "WidgetCanvas {id} has zero width");
            assert!(rect.h > 0.0, "WidgetCanvas {id} has zero height");
        }

        // The root rect should have positive area too.
        assert!(root_rect.w > 0.0 && root_rect.h > 0.0, "root has zero area");
    }

    /// BB_ScreenRadar: every WidgetCard's centre lies inside the canvas rect.
    #[test]
    fn bb_screen_radar_widget_cards_inside_canvas() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1024, 1024);

        let canvas = result.canvas;
        // Expand the canvas slightly for floating-point tolerance.
        let expanded = Rect {
            x: canvas.x - 1.0,
            y: canvas.y - 1.0,
            w: canvas.w + 2.0,
            h: canvas.h + 2.0,
        };

        for (id, node) in &scene.nodes {
            if !matches!(node.ty, BbNodeType::WidgetCard) {
                continue;
            }
            let rect = result.rects[id];
            let (cx, cy) = rect.centre();
            assert!(
                expanded.contains_point(cx, cy),
                "WidgetCard {:?} centre ({cx:.1},{cy:.1}) lies outside canvas bounds",
                node.name,
            );
        }
    }

    /// EC_PowerManagement: 1 root node, rects has 1 entry, draw_order has
    /// length 1.
    #[test]
    fn ec_power_management_single_node() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1600, 900);

        assert_eq!(scene.roots.len(), 1, "expected 1 root");
        assert_eq!(result.rects.len(), 1, "expected 1 rect");
        assert_eq!(result.draw_order.len(), 1, "expected 1 draw_order entry");
    }

    /// Edge case: a `BbValue::Other` with an unknown behavior does not panic
    /// and produces a positive dimension.
    #[test]
    fn unknown_behavior_does_not_panic() {
        use crate::bb_scene::{BbNode, BbSizing, BbTrbl, BbValue, Vec2, Vec3};
        use crate::bb_scene::BbNodeType;

        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetCanvas,
            name: "test".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 0.5, behavior: "Auto".into() },
                height: BbValue::Other { value: 0.5, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { canvas_size: (1920.0, 1080.0), roots: vec![1], nodes, operations: vec![] };

        // Must not panic.
        let result = layout(&scene, 1600, 900);
        let rect = result.rects[&1];
        // Fallback is fill → width = parent_inner.w
        assert!(rect.w > 0.0, "fallback width must be positive");
        assert!(rect.h > 0.0, "fallback height must be positive");
    }

    /// A3-PIVOT.3: when the canvas declares a 4:3 aspect (e.g. 800×600) and the
    /// target is 16:9 (1600×900), a uniform-scale letterbox is applied.  A root
    /// node with `Fixed(800)` sizing must NOT produce a rect of width 1600
    /// (the old non-uniform-stretch result); instead it should be scaled to 1200
    /// and horizontally centred with 200 px letterbox on each side.
    #[test]
    fn uniform_scale_letterboxes_mismatched_aspect() {
        use crate::bb_scene::{BbNode, BbSizing, BbTrbl, BbValue, Vec2, Vec3};
        use crate::bb_scene::BbNodeType;

        // Canvas 800×600, Fixed(800)×Fixed(600) root.
        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetCard,
            name: "root_card".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Fixed(800.0),
                height: BbValue::Fixed(600.0),
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { canvas_size: (800.0, 600.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1600, 900);
        let rect = result.rects[&1];

        // Uniform scale = min(1600/800, 900/600) = min(2.0, 1.5) = 1.5.
        // Fixed(800) → 800 × 1.5 = 1200.  NOT 1600 (old non-uniform stretch).
        assert_ne!(
            (rect.w, rect.h),
            (1600.0, 900.0),
            "Fixed(800×600) node must not be stretched to 1600×900"
        );
        assert!(
            (rect.w - 1200.0).abs() < 1.0,
            "expected width ≈ 1200 (uniform scale 1.5), got {:.1}",
            rect.w
        );
        assert!(
            (rect.h - 900.0).abs() < 1.0,
            "expected height ≈ 900 (uniform scale 1.5), got {:.1}",
            rect.h
        );
        // Letterbox: x offset = (1600 − 1200) / 2 = 200.
        assert!(
            (rect.x - 200.0).abs() < 1.0,
            "expected x ≈ 200 (letterbox), got {:.1}",
            rect.x
        );
    }
}
