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

const ADDITIVE_ALTERNATE_REVERSE_POSITION_PHASE_RATIO: f32 = 2.0 / 3.0;

const SYNTHETIC_NODE_ID_BASE: BbNodeId = 0x8000_0000;

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
    layout_with_animation_sample(scene, target_w, target_h, None)
}

/// Compute pixel-space rects while applying sampled animated SizeX/SizeY
/// modifiers when `animation_sample_percent` is provided.
pub fn layout_with_animation_sample(
    scene: &BbScene,
    target_w: u32,
    target_h: u32,
    animation_sample_percent: Option<f32>,
) -> LayoutResult {
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
            animation_sample_percent,
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
    animation_sample_percent: Option<f32>,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
    let Some(node) = scene.nodes.get(&node_id) else { return };

    // ── 1. Resolve outer dimensions ─────────────────────────────────────────
    //
    // Two-pass resolve: a `PercentOfX` height or `PercentOfY` width is a
    // percentage of THIS NODE's OTHER axis, not the parent's other axis.
    // First compute each axis using parent's other-axis as cross_dim (naïve);
    // then re-resolve any cross-axis behaviour using the node's own naïve
    // opposite dimension. This handles the common "square icon" idiom
    // (e.g. `width: Percent(0.8), height: PercentOfX(1.0)`) correctly while
    // remaining a no-op for non-cross-axis sizing.
    let fills_body_background_surface = fills_body_background_surface(node);
    let width_value = sampled_sizing_value(&node.sizing.width, &node.raw, "SizeX", animation_sample_percent);
    let height_value = sampled_sizing_value(&node.sizing.height, &node.raw, "SizeY", animation_sample_percent);
    let naive_w = resolve_value_for_node(node, &width_value, parent_inner.w, parent_inner.h, csx, true);
    let naive_h = resolve_value_for_node(node, &height_value, parent_inner.h, parent_inner.w, csy, false);
    let base_outer_w = if fills_body_background_surface {
        parent_inner.w
    } else if matches!(width_value, BbValue::Other { ref behavior, .. } if behavior == "PercentOfY") {
        resolve_value_for_node(node, &width_value, parent_inner.w, naive_h, csx, true)
    } else {
        naive_w
    };
    let base_outer_h = if fills_body_background_surface {
        parent_inner.h
    } else if matches!(height_value, BbValue::Other { ref behavior, .. } if behavior == "PercentOfX") {
        resolve_value_for_node(node, &height_value, parent_inner.h, naive_w, csy, false)
    } else {
        naive_h
    };
    let (scale_x, scale_y) = authored_node_scale(node, scene);
    let outer_w = base_outer_w * scale_x;
    let outer_h = base_outer_h * scale_y;

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

    let offset_x = sampled_position_offset(
        &node.raw,
        "PosXOffset",
        node.position_offset.x,
        animation_sample_percent,
    );
    let offset_y = sampled_position_offset(
        &node.raw,
        "PosYOffset",
        node.position_offset.y,
        animation_sample_percent,
    );
    let pos_x = (node.position.x + offset_x) * csx;
    let pos_y = (node.position.y + offset_y) * csy;

    let is_root_fullscreen_canvas = matches!(node.ty, BbNodeType::WidgetCanvas)
        && node.parent.is_none_or(|pid| scene.roots.contains(&pid))
        && matches!(width_value, BbValue::Percent(p) if (p - 1.0).abs() < 0.0001)
        && matches!(height_value, BbValue::Percent(p) if p > 0.90)
        && (node.anchor.x - 0.5).abs() < 0.01
        && (node.pivot.x - 0.5).abs() < 0.01;
    let is_child_canvas_surface_root = matches!(node.ty, BbNodeType::DisplayWidget)
        && parent_canvas_is_surface_host(node, scene)
        && matches!(width_value, BbValue::Percent(p) if (p - 1.0).abs() < 0.0001)
        && matches!(height_value, BbValue::Percent(p) if p > 0.90)
        && node.position.x.abs() < 0.0001
        && node.position.y.abs() < 0.0001
        && offset_x.abs() < 0.0001
        && offset_y.abs() < 0.0001;
    let (outer_x, outer_y) = if is_root_fullscreen_canvas || is_child_canvas_surface_root || fills_body_background_surface {
        // Full-bleed containers are parent-space overlays; authoring anchor/pivot
        // offsets should not shift them out of the parent rect.
        (parent_inner.x + pos_x, parent_inner.y + pos_y)
    } else {
        let mirrored_anchor_x = node.anchor.x >= 0.0
            && node.anchor.x <= 1.0
            && node.pivot.x >= 0.99
            && matches!(
                width_value,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 0.0 && value < 1.0
            );
        let anchor_x = if mirrored_anchor_x {
            1.0 - node.anchor.x
        } else {
            node.anchor.x
        };
        let pivot_y = effective_pivot_y(node);
        let anchor_world_x = parent_inner.x + parent_inner.w * anchor_x + pos_x;
        let anchor_world_y = parent_inner.y + parent_inner.h * node.anchor.y + pos_y;
        (
            anchor_world_x - outer_w * node.pivot.x,
            anchor_world_y - outer_h * pivot_y,
        )
    };

    // ── 3. Margin (Phase A1: top-left offset only) ───────────────────────────
    let outer_x = outer_x + node.margin.left * csx;
    let outer_y = outer_y + node.margin.top * csy;

    let outer_rect = Rect { x: outer_x, y: outer_y, w: outer_w, h: outer_h };

    layout_node_with_rect(
        node_id,
        outer_rect,
        scene,
        csx,
        csy,
        animation_sample_percent,
        rects,
        draw_order,
    );
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
    animation_sample_percent: Option<f32>,
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
    if !node.is_active {
        return;
    }
    draw_order.push(node_id);

    // ── 5. Recurse into children ─────────────────────────────────────────────
    // If the sibling set includes synthetic node IDs (pointerless authored
    // nodes), keep authored order for equal layers. Otherwise keep the prior
    // deterministic (layer, id) order.
    let mut children: Vec<BbNodeId> = node.children.clone();
    let has_synthetic = children.iter().any(|id| *id >= SYNTHETIC_NODE_ID_BASE);
    if has_synthetic {
        children.sort_by_key(|&child_id| scene.nodes.get(&child_id).map(|n| n.layer).unwrap_or(0));
    } else {
        children.sort_by_key(|&child_id| {
            let layer = scene.nodes.get(&child_id).map(|n| n.layer).unwrap_or(0);
            (layer, child_id)
        });
    }

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
            node.pivot.x,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
    } else {
        for child_id in children {
            layout_node(
                child_id,
                inner_rect,
                scene,
                csx,
                csy,
                animation_sample_percent,
                rects,
                draw_order,
            );
        }
    }
}

fn fills_body_background_surface(node: &crate::bb_scene::BbNode) -> bool {
    if !matches!(node.ty, BbNodeType::WidgetBodyBackground) {
        return false;
    }

    let uses_texture_background = node
        .raw
        .get("backgroundType")
        .and_then(|value| {
            value
                .as_str()
                .map(|text| text.eq_ignore_ascii_case("Texture"))
                .or_else(|| value.as_i64().map(|number| number == 1))
        })
        .unwrap_or(false);

    uses_texture_background && node.raw.get("textureProperties").is_some()
}

fn parent_canvas_is_surface_host(node: &crate::bb_scene::BbNode, scene: &BbScene) -> bool {
    node.parent
        .and_then(|parent_id| scene.nodes.get(&parent_id))
        .is_some_and(|parent| {
            matches!(parent.ty, BbNodeType::WidgetCanvas)
                && matches!(parent.sizing.width, BbValue::Percent(p) if (p - 1.0).abs() < 0.0001)
                && matches!(parent.sizing.height, BbValue::Percent(p) if p > 0.90)
        })
}

fn effective_pivot_y(node: &crate::bb_scene::BbNode) -> f32 {
    if horizontal_filled_separator_uses_centerline_anchor(node) {
        0.5
    } else {
        node.pivot.y
    }
}

fn horizontal_filled_separator_uses_centerline_anchor(node: &crate::bb_scene::BbNode) -> bool {
    let is_separator = matches!(
        &node.ty,
        BbNodeType::Other(kind) if kind.eq_ignore_ascii_case("BuildingBlocks_WidgetSeparator")
    );
    if !is_separator || node.pivot.y.abs() > f32::EPSILON {
        return false;
    }
    let is_horizontal = node
        .raw
        .get("direction")
        .and_then(|value| value.as_str())
        .is_some_and(|direction| direction.eq_ignore_ascii_case("Horizontal"));
    let is_filled_shape = node
        .raw
        .get("svgFill")
        .is_some_and(|svg_fill| {
            svg_fill
                .get("renderShape")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
                && svg_fill
                    .get("svgPath")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .is_empty()
        });
    let fixed_visual_height = matches!(node.sizing.height, BbValue::Fixed(height) if height > 1.0);
    is_horizontal && is_filled_shape && fixed_visual_height
}

fn sampled_sizing_value(
    authored: &BbValue,
    raw: &serde_json::Value,
    field_name: &str,
    animation_sample_percent: Option<f32>,
) -> BbValue {
    let Some(sample_percent) = animation_sample_percent else {
        return authored.clone();
    };
    let Some(sampled_value) = sampled_animation_number(raw, field_name, sample_percent) else {
        return authored.clone();
    };

    match authored {
        BbValue::Fixed(_) => BbValue::Fixed(sampled_value),
        BbValue::Percent(_) => BbValue::Percent(sampled_value),
        BbValue::Other { behavior, .. } => BbValue::Other {
            value: sampled_value,
            behavior: behavior.clone(),
        },
    }
}

fn sampled_position_offset(
    raw: &serde_json::Value,
    field_name: &str,
    authored_offset: f32,
    animation_sample_percent: Option<f32>,
) -> f32 {
    animation_sample_percent
        .and_then(|sample_percent| {
            sampled_animation_number(
                raw,
                field_name,
                position_animation_sample_percent(raw, sample_percent),
            )
        })
        .unwrap_or(authored_offset)
}

fn position_animation_sample_percent(raw: &serde_json::Value, sample_percent: f32) -> f32 {
    let Some(animation) = raw.get("animation") else {
        return sample_percent;
    };
    let direction = animation
        .get("direction")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let loops = animation
        .get("loopIndefinitely")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let additive = animation
        .get("additive")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    if direction.eq_ignore_ascii_case("AlternateReverse") && loops && additive {
        sample_percent * ADDITIVE_ALTERNATE_REVERSE_POSITION_PHASE_RATIO
    } else {
        sample_percent
    }
}

fn animation_number_keyframes(raw: &serde_json::Value, field_name: &str) -> Vec<(f64, f32)> {
    let Some(keyframes) = raw
        .get("animation")
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
    container_pivot_x: f32,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    animation_sample_percent: Option<f32>,
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
    let mut flow_non_grow: Vec<BbNodeId> = Vec::new();
    let mut overlay_non_flex: Vec<BbNodeId> = Vec::new();

    for &child_id in children {
        let Some(child_node) = scene.nodes.get(&child_id) else { continue };
        if !child_node.is_active {
            // Keep inactive nodes laid out for diagnostics, but do not let them
            // consume flex-flow slots that push active siblings off-screen.
            overlay_non_flex.push(child_id);
            continue;
        }
        let affects_layout = child_node
            .raw
            .get("affectsLayout")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let grow = child_node
            .raw
            .get("layoutPolicyItem")
            .and_then(|lpi| lpi.get("growProportion"))
            .and_then(|v| v.as_f64())
            .map(|g| g as f32)
            .unwrap_or(0.0);
        if affects_layout && grow > 0.0 {
            flex_children.push(FlexChild { id: child_id, grow });
        } else if affects_layout {
            flow_non_grow.push(child_id);
        } else {
            overlay_non_flex.push(child_id);
        }
    }

    // Children that do not participate in flex layout are overlayed.
    for child_id in overlay_non_flex {
        layout_node(
            child_id,
            container,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
    }

    if flex_children.is_empty() {
        if !flow_non_grow.is_empty() {
            layout_flex_no_grow_children(
                &flow_non_grow,
                container,
                flex,
                container_pivot_x,
                scene,
                csx,
                csy,
                animation_sample_percent,
                rects,
                draw_order,
                is_row,
            );
        } else {
            for child_id in flow_non_grow {
                layout_node(
                    child_id,
                    container,
                    scene,
                    csx,
                    csy,
                    animation_sample_percent,
                    rects,
                    draw_order,
                );
            }
        }
        return;
    }

    for child_id in flow_non_grow {
        layout_node(
            child_id,
            container,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
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
        layout_node_with_rect(
            *id,
            child_rect,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
    }
}

fn layout_flex_no_grow_children(
    children: &[BbNodeId],
    container: Rect,
    flex: &serde_json::Value,
    container_pivot_x: f32,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    animation_sample_percent: Option<f32>,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
    is_row: bool,
) {
    if children.is_empty() {
        return;
    }
    let item_spacing = if is_row {
        flex.get("columnSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csx
    } else {
        flex.get("rowSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csy
    };
    let axis_just = flex.get("axisJustification").and_then(|v| v.as_str()).unwrap_or("Start");
    let cross_just = flex
        .get("crossAxisJustification")
        .and_then(|v| v.as_str())
        .or_else(|| flex.get("itemAlignment").and_then(|v| v.as_str()))
        .unwrap_or("Stretch");
    let wrap_enabled = is_row
        && flex
        .get("wrap")
        .and_then(|v| v.as_str())
        .is_some_and(|w| w.eq_ignore_ascii_case("Wrap"));
    let cross_spacing = if is_row {
        flex.get("rowSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csy
    } else {
        flex.get("columnSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csx
    };

    let mut sizes: Vec<(BbNodeId, f32, f32, bool)> = Vec::with_capacity(children.len());
    let mut right_aligned_flow_items = 0usize;
    let mut total_main = 0.0f32;
    for &child_id in children {
        let Some(node) = scene.nodes.get(&child_id) else { continue };
        if !node.is_active {
            continue;
        }
        if node
            .raw
            .get("affectsLayout")
            .and_then(|v| v.as_bool())
            == Some(false)
        {
            continue;
        }
        if (node.name.is_empty() || node.name == "<unnamed>")
            && matches!(&node.ty, BbNodeType::Other(kind) if kind == "BuildingBlocks_WidgetLinearProgressMeter")
        {
            continue;
        }
        let mut w = resolve_value_for_node(node, &node.sizing.width, container.w, container.h, csx, true);
        let mut h = resolve_value_for_node(node, &node.sizing.height, container.h, container.w, csy, false);
        if matches!(node.sizing.width, BbValue::Other { ref behavior, .. } if behavior == "PercentOfY")
        {
            w = resolve_value_for_node(node, &node.sizing.width, container.w, h, csx, true);
        }
        if matches!(node.sizing.height, BbValue::Other { ref behavior, .. } if behavior == "PercentOfX")
        {
            h = resolve_value_for_node(node, &node.sizing.height, container.h, w, csy, false);
        }
        // In non-grow flex flow, "Auto" on main axis behaves like content-fit.
        // We do not have content measurement here, so treat it as zero so fixed/
        // percent children can still be axis-justified (instead of Auto filling
        // the whole container and pushing everything out of view).
        let mut auto_main = if is_row {
            matches!(node.sizing.width, BbValue::Other { ref behavior, .. } if behavior == "Auto")
        } else {
            matches!(node.sizing.height, BbValue::Other { ref behavior, .. } if behavior == "Auto")
        };
        let is_button_component = matches!(
            node.ty,
            BbNodeType::ComponentGeneralButton | BbNodeType::ComponentGeneralButtonSecondary
        );
        let auto_intrinsic_hint = if is_row {
            matches!(
                node.sizing.width,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 1.0
            )
        } else {
            matches!(
                node.sizing.height,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 1.0
            )
        };
        let has_main_axis_intrinsic_override = if is_row {
            textfield_auto_intrinsic_override(node, &node.sizing.width, container.w, csx, true).is_some()
                || component_button_auto_intrinsic_override(node, &node.sizing.width, csx).is_some()
        } else {
            textfield_auto_intrinsic_override(node, &node.sizing.height, container.h, csy, false).is_some()
                || component_button_auto_intrinsic_override(node, &node.sizing.height, csy).is_some()
        };
        if auto_main && (is_button_component && auto_intrinsic_hint || has_main_axis_intrinsic_override) {
            auto_main = false;
        }
        let right_edge_auto_hint = node.pivot.x >= 0.99
            && matches!(
                node.sizing.width,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 0.0 && value < 1.0
            );
        if auto_main {
            let normalized_auto = if is_row {
                match &node.sizing.width {
                    BbValue::Other { value, behavior } if behavior == "Auto" && *value > 0.0 && *value < 1.0 => Some(*value),
                    _ => None,
                }
            } else {
                match &node.sizing.height {
                    BbValue::Other { value, behavior } if behavior == "Auto" && *value > 0.0 && *value < 1.0 => Some(*value),
                    _ => None,
                }
            };
            let is_label_caption_pair = matches!(
                &node.ty,
                BbNodeType::Other(kind) if kind.eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
            );
            if let Some(auto_ratio) = normalized_auto {
                if is_row {
                    w = container.w * auto_ratio;
                } else {
                    h = container.h * auto_ratio;
                }
                auto_main = false;
            } else if is_label_caption_pair {
                if is_row {
                    // For row-flow label/caption pairs, derive intrinsic
                    // footprint from authored Auto sizing and child content.
                    // This avoids container-fraction heuristics that
                    // over-stretch compact metric labels.
                    let has_active_children = node.children.iter().any(|child_id| {
                        scene
                            .nodes
                            .get(child_id)
                            .is_some_and(|child| child.is_active)
                    });
                    if has_active_children {
                        if let BbValue::Other { value, behavior } = &node.sizing.width
                            && behavior == "Auto" && *value > 1.0
                        {
                            w = *value * csx;
                        }
                        if let BbValue::Other { value, behavior } = &node.sizing.height
                            && behavior == "Auto" && *value > 1.0
                        {
                            h = *value * csy;
                        }
                        for child_id in &node.children {
                            let Some(child) = scene.nodes.get(child_id) else {
                                continue;
                            };
                            if !child.is_active {
                                continue;
                            }
                            let child_w = resolve_value_for_node(
                                child,
                                &child.sizing.width,
                                container.w,
                                container.h,
                                csx,
                                true,
                            );
                            let child_h = resolve_value_for_node(
                                child,
                                &child.sizing.height,
                                container.h,
                                container.w,
                                csy,
                                false,
                            );
                            w = w.max(child_w.max(0.0));
                            h = h.max(child_h.max(0.0));
                        }
                    }
                    auto_main = false;
                } else {
                    h = (container.h * 0.22).max(48.0 * csy);
                    auto_main = false;
                }
            } else if is_row {
                w = 0.0;
            } else {
                h = 0.0;
            }
        }
        if (node.anchor.x >= 0.99 && node.pivot.x >= 0.99) || right_edge_auto_hint {
            right_aligned_flow_items += 1;
        }
        total_main += if is_row { w.max(0.0) } else { h.max(0.0) };
        sizes.push((child_id, w.max(0.0), h.max(0.0), auto_main));
    }
    if sizes.is_empty() {
        return;
    }
    total_main += item_spacing * (sizes.len().saturating_sub(1) as f32);
    let avail_main = if is_row { container.w } else { container.h };
    let axis_just_lc = axis_just.to_ascii_lowercase();
    let pivot_start_from_end = is_row && axis_just_lc == "start" && container_pivot_x >= 0.99;
    let child_start_from_end = is_row
        && axis_just_lc == "start"
        && !sizes.is_empty()
        && right_aligned_flow_items * 2 >= sizes.len();
    let start_from_end = pivot_start_from_end || child_start_from_end;
    let cross_just_lc = cross_just.to_ascii_lowercase();
    let cross_start_from_end = !is_row
        && cross_just_lc == "start"
        && (container_pivot_x >= 0.99 || (!sizes.is_empty() && right_aligned_flow_items * 2 >= sizes.len()));
    let main_offset = match axis_just_lc.as_str() {
        "center" => ((avail_main - total_main) * 0.5).max(0.0),
        "end" | "right" | "bottom" => (avail_main - total_main).max(0.0),
        "start" if start_from_end => (avail_main - total_main).max(0.0),
        _ => 0.0,
    };
    let mut cursor = if is_row { container.x + main_offset } else { container.y + main_offset };
    if wrap_enabled {
        // Build wrapped lines first so axis justification (e.g. Center) can be
        // applied per line instead of always starting at the container edge.
        let mut lines: Vec<Vec<(BbNodeId, f32, f32, bool)>> = Vec::new();
        let mut current: Vec<(BbNodeId, f32, f32, bool)> = Vec::new();
        let mut current_main = 0.0f32;
        let avail_main = if is_row { container.w } else { container.h };
        for item in sizes {
            let (_, w, h, auto_main) = item;
            if auto_main {
                // Keep auto-main nodes on the current line; they are laid out
                // with their own pass and do not consume wrapping width here.
                current.push(item);
                continue;
            }
            let main = if is_row { w } else { h };
            let proposed = if current.is_empty() {
                main
            } else {
                current_main + item_spacing + main
            };
            if !current.is_empty() && proposed > avail_main + 0.5 {
                lines.push(current);
                current = Vec::new();
                current_main = 0.0;
            }
            current_main = if current.is_empty() {
                main
            } else {
                current_main + item_spacing + main
            };
            current.push(item);
        }
        if !current.is_empty() {
            lines.push(current);
        }

        let mut line_cross_cursor = if is_row { container.y } else { container.x };
        for line in lines {
            let mut line_main = 0.0f32;
            let mut line_cross = 0.0f32;
            let mut line_items = 0usize;
            for &(_, w, h, auto_main) in &line {
                if auto_main { continue; }
                line_main += if is_row { w } else { h };
                line_cross = line_cross.max(if is_row { h } else { w });
                line_items += 1;
            }
            if line_items > 1 {
                line_main += item_spacing * (line_items - 1) as f32;
            }
            let line_main_offset = match axis_just_lc.as_str() {
                "center" => ((avail_main - line_main) * 0.5).max(0.0),
                "end" | "right" | "bottom" => (avail_main - line_main).max(0.0),
                "start" if start_from_end => (avail_main - line_main).max(0.0),
                _ => 0.0,
            };
            let mut line_main_cursor = if is_row {
                container.x + line_main_offset
            } else {
                container.y + line_main_offset
            };

            for (id, w, h, auto_main) in line {
                if auto_main {
                    layout_node(
                        id,
                        container,
                        scene,
                        csx,
                        csy,
                        animation_sample_percent,
                        rects,
                        draw_order,
                    );
                    continue;
                }
                let rect = if is_row {
                    let mut x = line_main_cursor;
                    if let Some(node) = scene.nodes.get(&id) {
                        if let Some(anchor_offset) = row_flex_start_anchor_offset(node, avail_main, w, csx) {
                            x += anchor_offset;
                        }
                    }
                    let y = match cross_just.to_ascii_lowercase().as_str() {
                        "center" => line_cross_cursor + (line_cross - h) * 0.5,
                        "end" | "right" | "bottom" => line_cross_cursor + (line_cross - h),
                        _ => line_cross_cursor,
                    };
                    let ch = if cross_just.eq_ignore_ascii_case("stretch") { line_cross } else { h };
                    Rect { x, y, w, h: ch }
                } else {
                    let mut x = match cross_just.to_ascii_lowercase().as_str() {
                        "center" => line_cross_cursor + (line_cross - w) * 0.5,
                        "end" | "right" | "bottom" => line_cross_cursor + (line_cross - w),
                        "start" if cross_start_from_end => line_cross_cursor + (line_cross - w),
                        _ => line_cross_cursor,
                    };
                    if let Some(node) = scene.nodes.get(&id) {
                        let pos_x = (node.position.x + node.position_offset.x) * csx;
                        if cross_just.eq_ignore_ascii_case("center") {
                            x += (node.anchor.x * line_cross) + pos_x - (node.pivot.x * w);
                        } else if cross_just.eq_ignore_ascii_case("start") && !cross_start_from_end {
                            x += (node.anchor.x * line_cross) + pos_x - (node.pivot.x * w);
                        }
                    }
                    let cw = if cross_just.eq_ignore_ascii_case("stretch") { line_cross } else { w };
                    Rect { x, y: line_main_cursor, w: cw, h }
                };
                layout_node_with_rect(
                    id,
                    rect,
                    scene,
                    csx,
                    csy,
                    animation_sample_percent,
                    rects,
                    draw_order,
                );
                line_main_cursor += if is_row { w } else { h };
                line_main_cursor += item_spacing;
            }

            line_cross_cursor += line_cross + cross_spacing;
        }
        return;
    }
    for (id, w, h, auto_main) in sizes {
        let column_right_edge_auto = !is_row
            && scene.nodes.get(&id).is_some_and(|node| {
                node.pivot.x >= 0.99
                    && matches!(
                        node.sizing.width,
                        BbValue::Other {
                            value,
                            ref behavior
                        } if behavior == "Auto" && value > 0.0 && value < 1.0
                    )
            });

        if auto_main || column_right_edge_auto {
            if !is_row && let Some(node) = scene.nodes.get(&id) {
                let right_edge_auto_hint = node.pivot.x >= 0.99
                    && matches!(
                        node.sizing.width,
                        BbValue::Other {
                            value,
                            ref behavior
                        } if behavior == "Auto" && value > 0.0 && value < 1.0
                    );
                if right_edge_auto_hint {
                    let pos_y = (node.position.y + node.position_offset.y) * csy;
                    let anchor_world_y = container.y + container.h * node.anchor.y + pos_y;
                    let intrinsic_y = anchor_world_y - h * node.pivot.y + node.margin.top * csy;
                    let slot_x = container.x + (container.w - w).max(0.0);
                    let slot_y = cursor;
                    let anchor_x = node.anchor.x.clamp(0.0, 1.0);
                    let anchor_y = node.anchor.y.clamp(0.0, 1.0);
                    let resolved_x = container.x + (slot_x - container.x) * (1.0 - anchor_x);
                    let resolved_y = slot_y + (intrinsic_y - slot_y) * (anchor_y * anchor_y);
                    let rect = Rect {
                        x: resolved_x,
                        y: resolved_y,
                        w,
                        h,
                    };
                    layout_node_with_rect(
                        id,
                        rect,
                        scene,
                        csx,
                        csy,
                        animation_sample_percent,
                        rects,
                        draw_order,
                    );
                    cursor += h;
                    cursor += item_spacing;
                    continue;
                }
            }
            // Auto-sized text-like items still contribute spacing/alignment
            // slots, but keep their own overlay layout so they can render with
            // intrinsic content bounds.
            layout_node(
                id,
                container,
                scene,
                csx,
                csy,
                animation_sample_percent,
                rects,
                draw_order,
            );
            cursor += if is_row { w } else { h };
            cursor += item_spacing;
            continue;
        }
        let rect = if is_row {
            let mut x = cursor;
            if let Some(node) = scene.nodes.get(&id) {
                if let Some(anchor_offset) = row_flex_start_anchor_offset(node, container.w, w, csx) {
                    x += anchor_offset;
                }
            }
            let y = match cross_just.to_ascii_lowercase().as_str() {
                "center" => container.y + (container.h - h) * 0.5,
                "end" | "right" | "bottom" => container.y + (container.h - h),
                _ => container.y,
            };
            let ch = if cross_just.eq_ignore_ascii_case("stretch") { container.h } else { h };
            Rect { x, y, w, h: ch }
        } else {
            let mut x = match cross_just.to_ascii_lowercase().as_str() {
                "center" => container.x + (container.w - w) * 0.5,
                "end" | "right" | "bottom" => container.x + (container.w - w),
                "start" if cross_start_from_end => container.x + (container.w - w),
                _ => container.x,
            };
            if let Some(node) = scene.nodes.get(&id) {
                let pos_x = (node.position.x + node.position_offset.x) * csx;
                if cross_just.eq_ignore_ascii_case("center") {
                    x += (node.anchor.x * container.w) + pos_x - (node.pivot.x * w);
                } else if cross_just.eq_ignore_ascii_case("start") && !cross_start_from_end {
                    x += (node.anchor.x * container.w) + pos_x - (node.pivot.x * w);
                }
            }
            let cw = if cross_just.eq_ignore_ascii_case("stretch") { container.w } else { w };
            Rect { x, y: cursor, w: cw, h }
        };
        layout_node_with_rect(
            id,
            rect,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
        cursor += if is_row { w } else { h };
        cursor += item_spacing;
    }
}

fn row_flex_start_anchor_offset(
    node: &crate::bb_scene::BbNode,
    available_main: f32,
    item_w: f32,
    csx: f32,
) -> Option<f32> {
    let pos_x = (node.position.x + node.position_offset.x) * csx;
    let is_start_anchored = node.anchor.x > 0.0 && node.anchor.x < 0.5 && node.pivot.x <= 0.01;
    let has_position = pos_x.abs() > f32::EPSILON;
    (is_start_anchored || has_position)
        .then_some((node.anchor.x * available_main) + pos_x - (node.pivot.x * item_w))
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
                // "Auto" is overloaded in authored data:
                // - values in (0, 1] often behave like normalized extents,
                //   especially for flex/header containers.
                // - larger values are content hints but we currently lack
                //   robust measurement, so keep prior fill behavior there.
                "Auto" if *value > 0.0 && *value <= 1.0 => primary_dim * *value,
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

fn resolve_value_for_node(
    node: &crate::bb_scene::BbNode,
    v: &BbValue,
    primary_dim: f32,
    cross_dim: f32,
    canvas_scale: f32,
    is_width: bool,
) -> f32 {
    if let Some(override_value) = textfield_auto_intrinsic_override(node, v, primary_dim, canvas_scale, is_width) {
        return override_value;
    }

    if let Some(override_value) = component_button_auto_intrinsic_override(node, v, canvas_scale) {
        return override_value;
    }

    if matches!(node.ty, BbNodeType::WidgetText)
        && let BbValue::Other { value, behavior } = v
        && behavior == "Auto"
    {
        // Text widgets authored with Auto typically carry a small content-fit
        // hint (for example 64). Treat that hint as size instead of filling
        // the parent, which causes header/label overlap in medical canvases.
        *value * canvas_scale
    } else {
        resolve_value(v, primary_dim, cross_dim, canvas_scale, is_width)
    }
}

fn authored_node_scale(node: &crate::bb_scene::BbNode, scene: &BbScene) -> (f32, f32) {
    if matches!(node.ty, BbNodeType::WidgetCanvas)
        && node
            .parent
            .and_then(|parent_id| scene.nodes.get(&parent_id))
            .is_some_and(|parent| matches!(parent.ty, BbNodeType::WidgetTextField))
    {
        return (1.0, 1.0);
    }

    let Some(scale) = node.raw.get("scale") else {
        return (1.0, 1.0);
    };
    let x = scale
        .get("x")
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    let y = scale
        .get("y")
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    (x, y)
}

fn component_button_auto_intrinsic_override(
    node: &crate::bb_scene::BbNode,
    v: &BbValue,
    canvas_scale: f32,
) -> Option<f32> {
    let is_supported_node = matches!(
        node.ty,
        BbNodeType::ComponentGeneralButton | BbNodeType::ComponentGeneralButtonSecondary
    );

    if !is_supported_node {
        return None;
    }

    let (value, behavior) = match v {
        BbValue::Other { value, behavior } => (*value, behavior.as_str()),
        _ => return None,
    };
    if behavior != "Auto" || value <= 1.0 {
        return None;
    }

    Some(value * canvas_scale)
}

fn textfield_auto_intrinsic_override(
    node: &crate::bb_scene::BbNode,
    v: &BbValue,
    primary_dim: f32,
    canvas_scale: f32,
    is_width: bool,
) -> Option<f32> {
    if !matches!(node.ty, BbNodeType::WidgetTextField) {
        return None;
    }

    if is_width
        && node.pivot.x >= 0.99
        && node.anchor.x > 1.0
        && let BbValue::Other { value, behavior } = v
        && behavior == "Auto"
        && *value > 0.0
        && *value <= 1.0
    {
        return Some(primary_dim);
    }

    let (value, behavior) = match v {
        BbValue::Other { value, behavior } => (*value, behavior.as_str()),
        _ => return None,
    };
    if behavior != "Auto" || value <= 0.0 || value > 1.0 {
        return None;
    }

    let style = node
        .raw
        .get("labelProperties")
        .and_then(|lp| lp.get("style"))
        .and_then(|s| s.as_str())
        .unwrap_or("");

    let has_tag = |needle: &str| node.style_tag_uuids.iter().any(|id| id.eq_ignore_ascii_case(needle));
    let is_primary = has_tag("e6003a83-9795-4478-a61c-349f14016e5b");
    let is_bright = has_tag("174b3e40-1b7b-4f01-a7dc-6420b7367d6b");
    let is_prompt = has_tag("5e5c7c8f-847b-46c5-ad80-a57c941391ab");
    let affects_layout = node
        .raw
        .get("affectsLayout")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if is_width && is_bright && style == "Heading2" {
        return Some(180.0 * canvas_scale);
    }
    if is_width && !affects_layout && is_bright && style == "Title3" {
        return Some((180.0 * value * value) * canvas_scale);
    }

    if !is_width && style == "Title3" && is_primary {
        return Some(270.0 * canvas_scale);
    }
    if !is_width && style == "Heading2" && is_bright {
        return Some(270.0 * canvas_scale);
    }
    if !is_width && style == "Heading2" && is_prompt {
        return Some(60.0 * canvas_scale);
    }

    None
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

    /// R5.I-A: `PercentOfX` height (and `PercentOfY` width) must be evaluated
    /// against THIS NODE's OWN other-axis dimension, not the parent's other
    /// dimension.
    #[test]
    fn percent_of_x_uses_own_width_not_parent() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget, name: "parent".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(400.0) },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 2, parent: Some(1), children: vec![],
            ty: BbNodeType::WidgetIcon, name: "icon".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Percent(0.8),
                height: BbValue::Other { value: 1.0, behavior: "PercentOfX".into() },
            },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { canvas_size: (200.0, 400.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 400);
        let r = result.rects[&2];
        // width = 0.8 × parent_w(200) = 160
        // height = 1.0 × own_w(160)   = 160   (NOT 1.0 × parent_w(200) = 200 or × parent_h(400))
        assert!((r.w - 160.0).abs() < 0.5, "expected width ≈ 160, got {}", r.w);
        assert!((r.h - 160.0).abs() < 0.5, "expected height ≈ 160 (square), got {}", r.h);
    }

    #[test]
    fn texture_body_background_fills_canvas_surface() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let background = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetBodyBackground,
            name: "body background".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(64.0), height: BbValue::Fixed(64.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "backgroundType": "Texture",
                "textureProperties": {
                    "_Type_": "BuildingBlocks_ComponentTextureProperties",
                    "orientation": "Landscape"
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, background);
        let scene = BbScene { canvas_size: (800.0, 450.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 800, 450);
        let rect = result.rects[&1];
        assert_eq!(rect, Rect { x: 0.0, y: 0.0, w: 800.0, h: 450.0 });
    }

    #[test]
    fn sampled_size_animation_overrides_sizing_value() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "parent".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(400.0), height: BbValue::Fixed(300.0) },
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
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetCustomShape,
            name: "animated shape".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(0.85), height: BbValue::Percent(0.85) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "animation": {
                    "animationTimeline": {
                        "keyframes": [
                            {
                                "percent": 0.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeX", "value": 0.77}},
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeY", "value": 0.77}}
                                ]
                            },
                            {
                                "percent": 1.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeX", "value": 0.9}},
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeY", "value": 0.9}}
                                ]
                            }
                        ]
                    }
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { canvas_size: (400.0, 300.0), roots: vec![1], nodes, operations: vec![] };

        let static_result = layout(&scene, 400, 300);
        let sampled_result = layout_with_animation_sample(&scene, 400, 300, Some(50.0));
        assert!((static_result.rects[&2].w - 340.0).abs() < 0.5);
        assert!((sampled_result.rects[&2].w - 334.0).abs() < 0.5);
        assert!((sampled_result.rects[&2].h - 250.5).abs() < 0.5);
    }

    #[test]
    fn sampled_position_offset_animation_moves_rect() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetImage,
            name: "animated image".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(100.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "animation": {
                    "animationTimeline": {
                        "keyframes": [
                            {
                                "percent": 0.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 50.0}}
                                ]
                            },
                            {
                                "percent": 1.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 0.0}}
                                ]
                            }
                        ]
                    }
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout_with_animation_sample(&scene, 200, 100, Some(50.0));
        let rect = result.rects[&1];
        assert!((rect.x - 25.0).abs() < 0.5, "expected sampled PosXOffset x 25, got {}", rect.x);
    }

    #[test]
    fn alternate_reverse_position_animation_uses_earlier_static_phase() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetImage,
            name: "looping slide image".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(100.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "animation": {
                    "animationTimeline": {
                        "keyframes": [
                            {
                                "percent": 0.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 50.0}}
                                ]
                            },
                            {
                                "percent": 1.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 0.0}}
                                ]
                            }
                        ]
                    },
                    "direction": "AlternateReverse",
                    "loopIndefinitely": true,
                    "additive": true
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout_with_animation_sample(&scene, 200, 100, Some(50.0));
        let rect = result.rects[&1];
        assert!((rect.x - 33.0).abs() < 0.5, "expected alternate-reverse sampled PosXOffset x 33, got {}", rect.x);
    }

    #[test]
    fn full_fill_flex_container_preserves_authored_pivot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.0, y: 0.03 },
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "Wrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start"
                }
            }),
        };
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "child".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(20.0) },
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
        nodes.insert(1, root);
        nodes.insert(2, child);
        let scene = BbScene { canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let root_rect = result.rects[&1];
        let child_rect = result.rects[&2];
        assert!((root_rect.y + 3.0).abs() < 0.5, "expected root y -3, got {}", root_rect.y);
        assert!((child_rect.y + 3.0).abs() < 0.5, "expected child y -3, got {}", child_rect.y);
    }

    #[test]
    fn child_canvas_surface_root_uses_host_origin() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let host = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "host_canvas".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(0.94) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.45 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child_root = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![3],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.4 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 3,
            parent: Some(2),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "child".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(20.0) },
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
        nodes.insert(1, host);
        nodes.insert(2, child_root);
        nodes.insert(3, child);
        let scene = BbScene { canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let host_rect = result.rects[&1];
        let root_rect = result.rects[&2];
        assert!((host_rect.y - 0.0).abs() < 0.5, "expected host y 0, got {}", host_rect.y);
        assert!((root_rect.y - host_rect.y).abs() < 0.5, "expected child root y {}, got {}", host_rect.y, root_rect.y);
    }

    #[test]
    fn non_surface_child_canvas_root_preserves_authored_pivot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let host = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "menu_host_canvas".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(0.77) },
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
        let child_root = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "menu_root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.0, y: 0.03 },
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, host);
        nodes.insert(2, child_root);
        let scene = BbScene { canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let host_rect = result.rects[&1];
        let root_rect = result.rects[&2];
        let expected_root_y = host_rect.y - root_rect.h * 0.03;
        assert!((host_rect.h - 77.0).abs() < 0.5, "expected host height 77, got {}", host_rect.h);
        assert!((root_rect.y - expected_root_y).abs() < 0.5, "expected child root y {}, got {}", expected_root_y, root_rect.y);
    }

    /// Mirror of above: `PercentOfY` for width must use own height.
    #[test]
    fn percent_of_y_uses_own_height_not_parent() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1, parent: None, children: vec![2],
            ty: BbNodeType::DisplayWidget, name: "parent".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(400.0), height: BbValue::Fixed(200.0) },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 2, parent: Some(1), children: vec![],
            ty: BbNodeType::WidgetIcon, name: "icon".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 1.0, behavior: "PercentOfY".into() },
                height: BbValue::Percent(0.6),
            },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { canvas_size: (400.0, 200.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 400, 200);
        let r = result.rects[&2];
        // height = 0.6 × parent_h(200) = 120
        // width  = 1.0 × own_h(120)    = 120  (NOT 1.0 × parent_h(200) or × parent_w(400))
        assert!((r.h - 120.0).abs() < 0.5, "expected height ≈ 120, got {}", r.h);
        assert!((r.w - 120.0).abs() < 0.5, "expected width ≈ 120 (square), got {}", r.w);
    }

    #[test]
    fn flex_row_wrap_moves_full_width_child_to_next_line() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(200.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "Wrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "columnSpacing": 0,
                    "rowSpacing": 0
                }
            }),
        };
        let child1 = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "c1".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(50.0) },
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
        let child2 = BbNode {
            id: 3,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "c2".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(50.0) },
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
        nodes.insert(1, root);
        nodes.insert(2, child1);
        nodes.insert(3, child2);
        let scene = BbScene { canvas_size: (300.0, 200.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 300, 200);
        let r1 = result.rects[&2];
        let r2 = result.rects[&3];
        assert!((r1.x - 0.0).abs() < 0.5 && (r1.y - 0.0).abs() < 0.5);
        assert!(
            (r2.x - 0.0).abs() < 0.5 && (r2.y - 50.0).abs() < 0.5,
            "second wrapped child expected at y=50, got ({:.1},{:.1})",
            r2.x,
            r2.y
        );
    }

    #[test]
    fn flex_row_wrap_start_applies_child_main_axis_anchor() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(100.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "Wrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "columnSpacing": 0,
                    "rowSpacing": 0
                }
            }),
        };
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: "anchored_prompt".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(0.3), height: BbValue::Fixed(40.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: 0.01, y: 0.0 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let mut metric = child.clone();
        metric.id = 3;
        metric.name = "right_metric".into();
        metric.sizing = BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(40.0) };
        metric.anchor = Vec2 { x: 1.0, y: 0.0 };
        metric.pivot = Vec2 { x: 1.0, y: 0.0 };
        assert_eq!(row_flex_start_anchor_offset(&metric, 300.0, 50.0, 1.0), None);

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child);
        let scene = BbScene { canvas_size: (300.0, 100.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 300, 100);
        let rect = result.rects[&2];
        assert!((rect.x - 3.0).abs() < 0.5, "expected x≈3 from 1% row anchor, got {}", rect.x);
    }

    #[test]
    fn horizontal_filled_separator_uses_centerline_anchor() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let separator = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::Other("BuildingBlocks_WidgetSeparator".into()),
            name: "separator".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(0.5), height: BbValue::Fixed(16.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: 0.0, y: 0.18 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "direction": "Horizontal",
                "svgFill": {
                    "renderShape": true,
                    "svgPath": ""
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, separator);
        let scene = BbScene { canvas_size: (100.0, 100.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 100, 100);
        let rect = result.rects[&1];
        assert!((rect.y - 10.0).abs() < 0.5, "expected centerline y=18 minus half height 8, got {}", rect.y);
    }

    #[test]
    fn flex_row_auto_label_caption_pairs_share_available_width() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "text-layout".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(604.8), height: BbValue::Fixed(108.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.0, y: 0.5 },
            anchor: Vec2 { x: 0.65, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "NoWrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 30.0,
                    "rowSpacing": 0.0
                }
            }),
        };
        let child = |id: u32, name: &str| BbNode {
            id,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::Other("BuildingBlocks_ComponentLabelCaptionPair".into()),
            name: name.into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 64.0, behavior: "Auto".into() },
                height: BbValue::Other { value: 64.0, behavior: "Auto".into() },
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
        nodes.insert(1, root);
        nodes.insert(2, child(2, "operator"));
        nodes.insert(3, child(3, "patient"));
        let scene = BbScene { canvas_size: (604.8, 108.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 605, 108);
        let r1 = result.rects[&2];
        let r2 = result.rects[&3];
        assert!((r1.w - 287.4).abs() < 1.0, "expected first pair width ≈ 287.4, got {}", r1.w);
        assert!((r2.w - 287.4).abs() < 1.0, "expected second pair width ≈ 287.4, got {}", r2.w);
        assert!((r2.x - (r1.x + r1.w + 30.0)).abs() < 1.0, "expected second pair after 30px spacing, got x={}", r2.x);
    }

    #[test]
    fn flex_row_auto_intrinsic_components_stay_in_flow_for_end_alignment() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "text-layout".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(80.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "NoWrap",
                    "axisJustification": "End",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 8.0,
                    "rowSpacing": 0.0
                }
            }),
        };

        let left_button = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::ComponentGeneralButton,
            name: "Back".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 64.0, behavior: "Auto".into() },
                height: BbValue::Other { value: 64.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 1.0, y: 0.5 },
            anchor: Vec2 { x: 1.0, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let exit_bed = BbNode {
            id: 3,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::ComponentGeneralButtonSecondary,
            name: "ExitBed".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 64.0, behavior: "Auto".into() },
                height: BbValue::Other { value: 64.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 1.0, y: 0.5 },
            anchor: Vec2 { x: 1.0, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, left_button);
        nodes.insert(3, exit_bed);
        let scene = BbScene { canvas_size: (200.0, 80.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 80);
        let left = result.rects[&2];
        let right = result.rects[&3];

        assert!((right.x + right.w - 200.0).abs() < 0.5, "expected right item to align to container end, got x={} w={}", right.x, right.w);
        assert!(left.x + left.w + 7.5 <= right.x, "expected distinct flowed items with spacing, got left={:?} right={:?}", left, right);
        assert!(left.x >= 0.0, "expected left item to stay within container, got x={}", left.x);
    }

    #[test]
    fn flex_column_auto_textfields_keep_intrinsic_height_slots() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3, 4],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(1000.0), height: BbValue::Fixed(1000.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Column",
                    "wrap": "NoWrapInfinite",
                    "axisJustification": "Center",
                    "crossAxisJustification": "Center",
                    "itemAlignment": "Center",
                    "columnSpacing": 0.0,
                    "rowSpacing": 30.0
                }
            }),
        };

        let textfield = |id: u32, name: &str, style: &str, tags: Vec<&str>, anchor_x: f32| BbNode {
            id,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: name.into(),
            style_tag_uuids: tags.into_iter().map(String::from).collect(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Percent(0.7),
                height: BbValue::Other { value: 1.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: anchor_x, y: 0.0 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "labelProperties": {
                    "style": style
                }
            }),
        };

        let touch = BbNode {
            id: 4,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetCanvas,
            name: "TouchHere".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(400.0), height: BbValue::Fixed(400.0) },
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
        nodes.insert(1, root);
        nodes.insert(2, textfield(2, "TitleText", "Title3", vec!["e6003a83-9795-4478-a61c-349f14016e5b"], 0.043));
        nodes.insert(3, textfield(3, "TouchPromptText", "Heading2", vec!["5e5c7c8f-847b-46c5-ad80-a57c941391ab"], 0.0));
        nodes.insert(4, touch);
        let scene = BbScene { canvas_size: (1000.0, 1000.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1000, 1000);
        let title = result.rects[&2];
        let prompt = result.rects[&3];
        let touch = result.rects[&4];

        assert!((title.y - 105.0).abs() < 0.5, "expected centered title flow y=105, got {}", title.y);
        assert!((title.x - 193.0).abs() < 0.5, "expected authored cross-axis anchor to offset title x, got {}", title.x);
        assert!((title.h - 270.0).abs() < 0.5, "expected Title3 intrinsic height 270, got {}", title.h);
        assert!((prompt.y - 405.0).abs() < 0.5, "expected prompt after title plus spacing, got {}", prompt.y);
        assert!((prompt.x - 150.0).abs() < 0.5, "expected prompt without cross-axis anchor offset at centered x, got {}", prompt.x);
        assert!((prompt.h - 60.0).abs() < 0.5, "expected prompt intrinsic height 60, got {}", prompt.h);
        assert!((touch.y - 495.0).abs() < 0.5, "expected touch canvas after intrinsic text slots, got {}", touch.y);
    }

    #[test]
    fn flex_column_nowrap_infinite_start_cross_axis_applies_child_anchor() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(1000.0), height: BbValue::Fixed(120.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Column",
                    "wrap": "NoWrapInfinite",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 0.0,
                    "rowSpacing": 0.0
                }
            }),
        };

        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: "WelcomeText".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Fixed(40.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: 0.01, y: 0.0 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child);
        let scene = BbScene { canvas_size: (1000.0, 120.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1000, 120);
        let child = result.rects[&2];
        assert!((child.x - 10.0).abs() < 0.5, "expected child anchor.x to offset start cross-axis x, got {}", child.x);
    }

    #[test]
    fn inactive_bright_title3_auto_width_uses_authored_scale() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(1344.0), height: BbValue::Fixed(270.0) },
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

        let tier = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: "TierLevel".into(),
            style_tag_uuids: vec!["174b3e40-1b7b-4f01-a7dc-6420b7367d6b".into()],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 0.9, behavior: "Auto".into() },
                height: BbValue::Other { value: 1.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.01, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "affectsLayout": false,
                "labelProperties": {"style": "Title3"},
                "textAlignment": "Left"
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, tier);
        let scene = BbScene { canvas_size: (1344.0, 270.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1344, 270);
        let tier = result.rects[&2];

        assert!((tier.w - 145.8).abs() < 0.5, "expected authored Auto width scale to affect Title3 intrinsic width, got {}", tier.w);
        assert!((tier.x + tier.w * 0.5 - 13.44).abs() < 0.5, "expected anchor/pivot to stay attached to authored 1% anchor, got x={} w={}", tier.x, tier.w);
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

    #[test]
    fn authored_scale_expands_rect_around_pivot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbScene, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "parent".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(100.0), height: BbValue::Fixed(100.0) },
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
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetCustomShape,
            name: "scaled_shape".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "scale": { "x": 1.2, "y": 1.4 }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { canvas_size: (100.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 100, 100);
        let rect = result.rects[&2];

        assert!((rect.w - 60.0).abs() < 0.01, "expected scaled width 60, got {}", rect.w);
        assert!((rect.h - 70.0).abs() < 0.01, "expected scaled height 70, got {}", rect.h);
        assert!((rect.x - 20.0).abs() < 0.01, "expected pivot-centred x 20, got {}", rect.x);
        assert!((rect.y - 15.0).abs() < 0.01, "expected pivot-centred y 15, got {}", rect.y);
    }

    #[test]
    fn text_field_child_canvas_scale_does_not_expand_layout_slot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbScene, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(100.0) },
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
        let text = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![3],
            ty: BbNodeType::WidgetTextField,
            name: "prompt".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(60.0) },
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
        let backing_canvas = BbNode {
            id: 3,
            parent: Some(2),
            children: vec![],
            ty: BbNodeType::WidgetCanvas,
            name: "text_backing_canvas".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "scale": { "x": 1.0, "y": 1.5 },
                "sizingMethod": "Size"
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, text);
        nodes.insert(3, backing_canvas);
        let scene = BbScene { canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let rect = result.rects[&3];

        assert!((rect.w - 200.0).abs() < 0.01, "expected backing width to fill text slot, got {}", rect.w);
        assert!((rect.h - 60.0).abs() < 0.01, "expected backing height to stay in text slot, got {}", rect.h);
        assert!((rect.y - 0.0).abs() < 0.01, "expected backing to remain aligned to text slot, got {}", rect.y);
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

    #[test]
    fn layout_source_does_not_reintroduce_forbidden_hardcoded_or_heuristic_markers() {
        let source = include_str!("bb_layout.rs");
        // Hard rule: keep layout generic across assets and screens. If this
        // trips, remove marker-based workarounds and fix structural causes.
        let forbidden = [
            ["med", "ical2"].concat(),
            ["med", "gel"].concat(),
            ["hard", "coded", "_offset"].concat(),
            ["magic", "_multiplier"].concat(),
            ["heu", "ristic", "_shift"].concat(),
            ["blend", "_factor"].concat(),
        ];

        for marker in forbidden {
            assert!(
                !source.contains(marker.as_str()),
                "bb_layout hardcoding/heuristic marker reintroduced: {marker}"
            );
        }
    }
}
