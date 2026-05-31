use std::collections::BTreeMap;

use super::clone_expand::expand_widget_clones;
use super::fields::*;
use super::types::*;

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
    let mut operations = record_value
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
    let mut scene_order_ids: Vec<BbNodeId> = Vec::new();
    let mut library_order_ids: Vec<BbNodeId> = Vec::new();
    let mut synthetic_base: u32 = 0x8000_0000;

    for raw_item in scene_arr {
        if let Some(id) = parse_node_into(raw_item, &mut nodes, &mut synthetic_base, "scene") {
            scene_order_ids.push(id);
        }
    }
    for raw_item in library_arr {
        if let Some(id) = parse_node_into(raw_item, &mut nodes, &mut synthetic_base, "library") {
            library_ids.insert(id);
            library_order_ids.push(id);
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
    let parent_child_pairs: Vec<(BbNodeId, BbNodeId)> = scene_order_ids
        .iter()
        .chain(library_order_ids.iter())
        .filter_map(|id| nodes.get(id).and_then(|n| n.parent.map(|p| (p, *id))))
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

    expand_widget_clones(&mut nodes, &mut operations, &mut synthetic_base);

    // ── Collect roots. ───────────────────────────────────────────────────────
    let roots: Vec<BbNodeId> = scene_order_ids
        .iter()
        .filter_map(|id| nodes.get(id).map(|n| (*id, n)))
        .filter(|(id, n)| n.parent.is_none() && !library_ids.contains(id))
        .map(|(id, _)| id)
        .collect();

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
    let authored_is_active = raw.get("isActive").and_then(|v| v.as_bool()).unwrap_or(true);
    let export_node = raw.get("exportNode").and_then(|v| v.as_bool()).unwrap_or(true);
    // `exportNode=false` marks helper/template widgets that should not be
    // emitted into the exported scene render.
    let is_active = authored_is_active && export_node;
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

    let text = if matches!(ty, BbNodeType::WidgetTextField | BbNodeType::WidgetText) {
        Some(parse_text(raw))
    } else {
        None
    };

    let icon = if matches!(
        ty,
        BbNodeType::WidgetIcon
            | BbNodeType::WidgetImage
            | BbNodeType::ComponentGeneralButton
            | BbNodeType::ComponentGeneralButtonSecondary
    ) {
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
// Pointer parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parse `"ptr:N"` → `Some(N)`.
fn parse_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

/// Parse `"_PointsTo_:ptr:N"` → `Some(N)`.
fn parse_points_to(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:").and_then(parse_ptr)
}

