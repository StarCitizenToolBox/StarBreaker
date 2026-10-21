//! MFD active content-view selection for frame-canvas renders.
//!
//! The MFD content frame (`m_eng_mfdcontent`) embeds two mutually-exclusive
//! content-view slots gated by the engine's `useportraitview` boolean:
//! `canvas_LandscapeMFDView` (full width, `CanvasReferenceRecord ←
//! landscapecanvasguid`) and `canvas_PortraitMFDView` (a narrow `PercentOfY`
//! portrait box, `CanvasReferenceRecord ← portraitcanvasguid`). At runtime the
//! host fills both slots from the bound `SMFDView`'s `landscapeCanvas` /
//! `portraitCanvas` and instantiates exactly one based on the physical screen
//! aspect; the slots' authored `canvas:` placeholders are arbitrary editor
//! leftovers replaced by those bindings. A static export has neither the runtime
//! aspect state nor the namespace canvas GUIDs, so without help the frame would
//! render its placeholder content in whichever slot its static boolean filter
//! happens to pick.
//!
//! StarBreaker's MFD bindings always carry the view's `landscapeCanvas` as their
//! content (physical screens are landscape; the portrait casts are AR overlays
//! not bound to geometry), so the bound content belongs in the **landscape**
//! (full-width) slot. [`apply_bound_mfd_view`] identifies the two slots from the
//! engine's own binding wiring (`landscapecanvasguid` / `portraitcanvasguid`),
//! injects the bound content onto the landscape slot's Pass-2 follow URL, forces
//! that slot instantiated, and forces the portrait slot out. This is generic (no
//! per-asset names) and correct for every view, including ones whose landscape
//! canvas matches neither authored placeholder.
//!
//! Frames without that wiring fall back to
//! [`apply_bound_view_instantiation_by_canvas`], which forces the slot whose
//! authored `canvas:` reference matches the bound content and skips its
//! same-parent/same-layer peers.

use std::collections::HashSet;

use serde_json::Value;

use crate::bb_scene::{BbNode, BbNodeId, BbNodeType, BbScene};

/// A Pass-2 follow-list entry: `(widget node id, canvas URL, param inputs)`.
type CanvasUrl = (BbNodeId, String, Vec<Value>);

/// The GFx host's content-view inset, in stage pixels of the 1280×720
/// `BuildingBlocks_root.swf` stage: the runtime `BuildingBlocksView` hosts the
/// bound content view 44px in from the left, right, and bottom stage edges and
/// flush with the top. The inset is a measured framework constant: on the
/// Clipper power-screen capture the content maps at x-scale 0.93125
/// (= 1192/1280) centred and y-scale 676/720 top-anchored, with x- and
/// y-derived scales agreeing within 0.4%.
///
/// Provenance bound (plan P2.2a, 2026-06-12): a full AVM1 dump of
/// `BuildingBlocks_root.swf` (`examples/swf_avm1_dump.rs`, all 127
/// `__Packages.*` classes incl. `bhvr.views.{MainView,ScreenView,
/// BuildingBlocksView}`) contains NO 44/1192/676 pushes anywhere (the only
/// 44s are a keycode-style table in `gfx.core.UIComponent`), so the
/// placement is not authored ActionScript either — it is computed on the
/// C++ host side and remains a measured constant.
/// Fallback when the binding carries no readable host movie; the live value
/// comes from the host SWF header (`SwfAssetLibrary::stage_size`) via
/// [`apply_bound_mfd_view_with_host_stage`] (plan P5.4).
const HOST_STAGE_SIZE: (f32, f32) = (1280.0, 720.0);
const HOST_CONTENT_INSET: f32 = 44.0;

/// Width fraction of the frame the hosted content view occupies.
fn host_content_view_width_fraction(stage: (f32, f32)) -> f32 {
    (stage.0 - 2.0 * HOST_CONTENT_INSET) / stage.0
}

/// Height fraction of the frame the hosted content view occupies.
fn host_content_view_height_fraction(stage: (f32, f32)) -> f32 {
    (stage.1 - HOST_CONTENT_INSET) / stage.1
}

/// The landscape (full-width) and portrait (narrow) content-view slots of an MFD
/// content frame, identified by their `CanvasReferenceRecord` field bindings.
struct MfdViewSlots {
    landscape: BbNodeId,
    portrait: BbNodeId,
}

/// Place the binding's bound content into the MFD frame's correct content-view
/// slot at full width.
///
/// `bound_content_ref` is the binding's content canvas reference (record name or
/// URL — e.g. `"BuildingBlocks_Canvas.MC_S_Target_Master"`); it is the bound
/// view's `landscapeCanvas`. `canvas_urls` is the Pass-2 follow list and
/// `instantiated_false` the set of slots forced inactive.
///
/// When the frame exposes the `landscapecanvasguid` / `portraitcanvasguid`
/// wiring, the bound content's URL is injected onto the landscape slot (replacing
/// the frame's authored placeholder), the landscape slot is forced instantiated,
/// and the portrait slot forced out — mirroring the engine's `useportraitview =
/// false` outcome for a physical landscape screen. Otherwise this falls back to
/// [`apply_bound_view_instantiation_by_canvas`].
pub fn apply_bound_mfd_view(
    scene: &mut BbScene,
    bound_content_ref: &str,
    canvas_urls: &mut Vec<CanvasUrl>,
    instantiated_false: &mut HashSet<BbNodeId>,
) {
    apply_bound_mfd_view_with_host_stage(
        scene,
        bound_content_ref,
        canvas_urls,
        instantiated_false,
        None,
    );
}

/// [`apply_bound_mfd_view`] with the binding's host SWF stage size (from the
/// movie header) when the caller has one; `None` falls back to the
/// [`HOST_STAGE_SIZE`] constant.
pub fn apply_bound_mfd_view_with_host_stage(
    scene: &mut BbScene,
    bound_content_ref: &str,
    canvas_urls: &mut Vec<CanvasUrl>,
    instantiated_false: &mut HashSet<BbNodeId>,
    host_stage_size: Option<(f32, f32)>,
) {
    let Some(slots) = mfd_view_slots(scene) else {
        apply_bound_view_instantiation_by_canvas(scene, bound_content_ref, instantiated_false);
        return;
    };

    // Physical MFD screens are landscape → `useportraitview` is false → the
    // landscape slot is active and the portrait slot inactive.
    instantiated_false.remove(&slots.landscape);
    instantiated_false.insert(slots.portrait);

    // Render the bound (landscape) content via the landscape slot, replacing
    // the frame's authored placeholder canvas.
    set_canvas_url(canvas_urls, slots.landscape, bound_content_ref);

    // Size the slot to the GFx host's content-view stage sub-rect (x-centred,
    // top-anchored). The frame chrome (header/footer) outside the slot stays
    // full-bleed; only the bound content view is inset.
    let stage = host_stage_size
        .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0)
        .unwrap_or(HOST_STAGE_SIZE);
    if let Some(slot) = scene.nodes.get_mut(&slots.landscape) {
        slot.sizing.width =
            crate::bb_scene::BbValue::Percent(host_content_view_width_fraction(stage));
        slot.sizing.height =
            crate::bb_scene::BbValue::Percent(host_content_view_height_fraction(stage));
        slot.anchor = crate::bb_scene::Vec2 { x: 0.5, y: 0.0 };
        slot.pivot = crate::bb_scene::Vec2 { x: 0.5, y: 0.0 };
        slot.position = Default::default();
        slot.position_offset = Default::default();
    }
}

/// Set the Pass-2 follow URL for `node` to `url`, preserving any param inputs
/// already collected for that node. If the slot had no follow entry (its authored
/// placeholder was null), a fresh entry is appended so the bound content is still
/// followed.
fn set_canvas_url(canvas_urls: &mut Vec<CanvasUrl>, node: BbNodeId, url: &str) {
    if let Some(entry) = canvas_urls.iter_mut().find(|(id, _, _)| *id == node) {
        entry.1 = url.to_string();
    } else {
        canvas_urls.push((node, url.to_string(), Vec::new()));
    }
}

/// The landscape (full-width) content-view slot node, identified the same way
/// [`apply_bound_mfd_view`] does (the `landscapecanvasguid` `CanvasReferenceRecord`
/// binding). `None` when the frame lacks the landscape/portrait view wiring.
///
/// Exposed so the pipeline can apply the data-driven aspect-tag "Content Canvas
/// Scaling" width to this exact node (the engine's `bd1ebe5c` content canvas)
/// after resolution, replacing the measured host inset on the width axis.
pub(crate) fn landscape_slot_id(scene: &BbScene) -> Option<BbNodeId> {
    mfd_view_slots(scene).map(|slots| slots.landscape)
}

/// Identify the landscape/portrait MFD content slots from the frame's
/// `CanvasReferenceRecord` field bindings (`landscapecanvasguid` /
/// `portraitcanvasguid`). Returns `None` unless both are present.
fn mfd_view_slots(scene: &BbScene) -> Option<MfdViewSlots> {
    let mut landscape = None;
    let mut portrait = None;
    for op in &scene.operations {
        let Some((widget, binding)) = canvas_reference_binding(op, &scene.operations) else {
            continue;
        };
        match binding.as_str() {
            "landscapecanvasguid" => landscape = Some(widget),
            "portraitcanvasguid" => portrait = Some(widget),
            _ => {}
        }
    }
    Some(MfdViewSlots {
        landscape: landscape?,
        portrait: portrait?,
    })
}

/// If `op` is a `BindingsStringField` writing the `CanvasReferenceRecord` field,
/// resolve its source `BindingsStringVariable` and return `(widget node id,
/// source variable binding name)`.
fn canvas_reference_binding(op: &Value, operations: &[Value]) -> Option<(BbNodeId, String)> {
    if op.get("_Type_").and_then(Value::as_str)? != "BuildingBlocks_BindingsStringField" {
        return None;
    }
    if op.get("field").and_then(Value::as_str)? != "CanvasReferenceRecord" {
        return None;
    }
    let widget = points_to_id(op.get("widget").and_then(Value::as_str)?)?;
    let input_id = points_to_id(op.get("input").and_then(Value::as_str)?)?;
    let binding = operations.iter().find_map(|candidate| {
        if ptr_id(candidate.get("_Pointer_").and_then(Value::as_str)?)? != input_id {
            return None;
        }
        if candidate.get("_Type_").and_then(Value::as_str)?
            != "BuildingBlocks_BindingsStringVariable"
        {
            return None;
        }
        candidate
            .get("binding")
            .and_then(Value::as_str)
            .map(str::to_string)
    })?;
    Some((widget, binding))
}

/// Parse `"_PointsTo_:ptr:N"` → `Some(N)`.
fn points_to_id(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:ptr:").and_then(|n| n.parse().ok())
}

/// Parse `"ptr:N"` → `Some(N)`.
fn ptr_id(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

/// Fallback for MFD frames without `landscapecanvasguid` / `portraitcanvasguid`
/// wiring: adjust `instantiated_false` so only the content view whose authored
/// `canvas:` reference matches `bound_content_record_name` is instantiated among
/// its mutually-exclusive peers.
///
/// The matched slot is removed from `instantiated_false` (forced on) and its
/// peers — `WidgetCanvas` nodes sharing its parent **and** layer that reference a
/// different canvas — are inserted (forced off). No-op when no slot matches.
/// Peer detection is structural and relative to the matched slot's layer, so the
/// always-on chrome (header/footer) on a different layer is left alone.
pub fn apply_bound_view_instantiation_by_canvas(
    scene: &BbScene,
    bound_content_record_name: &str,
    instantiated_false: &mut HashSet<BbNodeId>,
) {
    let Some((matched_id, parent, layer)) = matched_view_slot(scene, bound_content_record_name)
    else {
        return;
    };

    instantiated_false.remove(&matched_id);
    for (id, node) in &scene.nodes {
        if *id != matched_id
            && node.ty == BbNodeType::WidgetCanvas
            && node.parent == parent
            && node.layer == layer
            && canvas_ref_record_name(node).is_some()
        {
            instantiated_false.insert(*id);
        }
    }
}

/// The `(id, parent, layer)` of the `WidgetCanvas` slot whose `canvas:` reference
/// matches `bound_content_record_name`, or `None`.
fn matched_view_slot(
    scene: &BbScene,
    bound_content_record_name: &str,
) -> Option<(BbNodeId, Option<BbNodeId>, i32)> {
    let bound = normalize_record_name(bound_content_record_name);
    if bound.is_empty() {
        return None;
    }
    scene.nodes.iter().find_map(|(id, node)| {
        if node.ty != BbNodeType::WidgetCanvas {
            return None;
        }
        (canvas_ref_record_name(node)? == bound).then_some((*id, node.parent, node.layer))
    })
}

/// The lower-cased, prefix-stripped record name a `WidgetCanvas` node references
/// via its `canvas:` URL, or `None` if it has no canvas reference.
fn canvas_ref_record_name(node: &BbNode) -> Option<String> {
    let url = node
        .raw
        .get("canvas")
        .and_then(|value| value.as_str())
        .filter(|s| !s.is_empty() && *s != "null")?;
    Some(normalize_record_name(&crate::record_name::extract_record_name(url)))
}

fn normalize_record_name(name: &str) -> String {
    name.strip_prefix("BuildingBlocks_Canvas.")
        .unwrap_or(name)
        .to_ascii_lowercase()
}

#[cfg(test)]
#[path = "mfd_view_tests.rs"]
mod tests;
