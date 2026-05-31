//! Widget-clone expansion for parsed BuildingBlocks scenes.
//!
//! Duplicates the referenced library subtree for `BuildingBlocks_WidgetClone`
//! nodes and remaps any widget-bound operations onto the synthetic clone IDs so
//! cloned visuals keep their authored binding graphs.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::types::{BbNode, BbNodeId, BbNodeType};

pub(super) fn expand_widget_clones(
    nodes: &mut BTreeMap<BbNodeId, BbNode>,
    operations: &mut Vec<serde_json::Value>,
    synthetic_base: &mut u32,
) {
    let clone_ids: Vec<BbNodeId> = nodes
        .iter()
        .filter_map(|(id, node)| {
            matches!(
                node.ty,
                BbNodeType::Other(ref ty)
                    if ty.eq_ignore_ascii_case("BuildingBlocks_WidgetClone")
            )
            .then_some(*id)
        })
        .collect();

    for clone_id in clone_ids {
        let Some(clone_node) = nodes.get(&clone_id).cloned() else {
            continue;
        };
        let Some(target_id) = clone_node
            .raw
            .get("target")
            .and_then(|v| v.as_str())
            .and_then(parse_points_to)
        else {
            continue;
        };
        if !nodes.contains_key(&target_id) {
            continue;
        }

        let mut id_map = BTreeMap::new();
        id_map.insert(target_id, clone_id);

        let mut stack = vec![target_id];
        let mut order = Vec::new();
        while let Some(src_id) = stack.pop() {
            if id_map.get(&src_id).is_none() {
                id_map.insert(src_id, next_synthetic_id(synthetic_base));
            }
            order.push(src_id);
            if let Some(src_node) = nodes.get(&src_id) {
                for &child_id in src_node.children.iter().rev() {
                    stack.push(child_id);
                }
            }
        }

        for src_id in order {
            let Some(src_node) = nodes.get(&src_id).cloned() else {
                continue;
            };
            let Some(&new_id) = id_map.get(&src_id) else {
                continue;
            };

            let mut new_node = src_node;
            new_node.id = new_id;
            new_node.children = new_node
                .children
                .iter()
                .filter_map(|child| id_map.get(child).copied())
                .collect();

            if src_id == target_id {
                new_node.parent = clone_node.parent;
                new_node.position = clone_node.position.clone();
                new_node.position_offset = clone_node.position_offset.clone();
                new_node.sizing = clone_node.sizing.clone();
                new_node.pivot = clone_node.pivot.clone();
                new_node.anchor = clone_node.anchor.clone();
                new_node.margin = clone_node.margin.clone();
                new_node.padding = clone_node.padding.clone();
                new_node.layer = clone_node.layer;
                new_node.alpha = clone_node.alpha;
                new_node.is_active = clone_node.is_active && new_node.is_active;
            } else {
                let src_parent = new_node.parent;
                new_node.parent = src_parent.and_then(|parent| id_map.get(&parent).copied());
            }

            nodes.insert(new_id, new_node);
        }

        clone_widget_clone_operations(&id_map, operations, synthetic_base);
    }
}

fn clone_widget_clone_operations(
    node_id_map: &BTreeMap<BbNodeId, BbNodeId>,
    operations: &mut Vec<serde_json::Value>,
    synthetic_base: &mut u32,
) {
    let source_node_ids: BTreeSet<BbNodeId> = node_id_map.keys().copied().collect();
    let mut op_ptr_to_index = HashMap::new();
    for (index, op) in operations.iter().enumerate() {
        if let Some(ptr) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(parse_ptr)
        {
            op_ptr_to_index.insert(ptr, index);
        }
    }

    let mut root_indexes = BTreeSet::new();
    let mut dependent_ptrs = BTreeSet::new();
    for (index, op) in operations.iter().enumerate() {
        let Some(widget_ptr) = op
            .get("widget")
            .and_then(|value| value.as_str())
            .and_then(parse_points_to)
        else {
            continue;
        };
        if !source_node_ids.contains(&widget_ptr) {
            continue;
        }
        root_indexes.insert(index);
        collect_referenced_operation_ptrs(op, operations, &op_ptr_to_index, &mut dependent_ptrs);
    }

    if root_indexes.is_empty() {
        return;
    }

    let mut op_ptr_map = BTreeMap::new();
    for ptr in &dependent_ptrs {
        op_ptr_map.insert(*ptr, next_synthetic_id(synthetic_base));
    }

    let mut cloned_ops = Vec::new();
    for (index, op) in operations.iter().enumerate() {
        let op_ptr = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(parse_ptr);
        let should_clone = root_indexes.contains(&index)
            || op_ptr.is_some_and(|ptr| dependent_ptrs.contains(&ptr));
        if !should_clone {
            continue;
        }

        let mut cloned = op.clone();
        remap_operation_ptrs(&mut cloned, node_id_map, &op_ptr_map);
        if let Some(old_ptr) = op_ptr {
            if let Some(new_ptr) = op_ptr_map.get(&old_ptr) {
                cloned["_Pointer_"] = serde_json::Value::String(format!("ptr:{new_ptr}"));
            }
        }
        cloned_ops.push(cloned);
    }

    operations.extend(cloned_ops);
}

fn collect_referenced_operation_ptrs(
    value: &serde_json::Value,
    operations: &[serde_json::Value],
    op_ptr_to_index: &HashMap<BbNodeId, usize>,
    collected: &mut BTreeSet<BbNodeId>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for nested in map.values() {
                collect_referenced_operation_ptrs(nested, operations, op_ptr_to_index, collected);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_referenced_operation_ptrs(item, operations, op_ptr_to_index, collected);
            }
        }
        serde_json::Value::String(text) => {
            let ptr = parse_points_to(text).or_else(|| parse_ptr(text));
            let Some(ptr) = ptr else {
                return;
            };
            let Some(&index) = op_ptr_to_index.get(&ptr) else {
                return;
            };
            if collected.insert(ptr) {
                collect_referenced_operation_ptrs(
                    &operations[index],
                    operations,
                    op_ptr_to_index,
                    collected,
                );
            }
        }
        _ => {}
    }
}

fn remap_operation_ptrs(
    value: &mut serde_json::Value,
    node_id_map: &BTreeMap<BbNodeId, BbNodeId>,
    op_ptr_map: &BTreeMap<BbNodeId, BbNodeId>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for nested in map.values_mut() {
                remap_operation_ptrs(nested, node_id_map, op_ptr_map);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remap_operation_ptrs(item, node_id_map, op_ptr_map);
            }
        }
        serde_json::Value::String(text) => {
            if let Some(ptr) = parse_points_to(text) {
                if let Some(new_ptr) = node_id_map.get(&ptr).or_else(|| op_ptr_map.get(&ptr)) {
                    *text = format!("_PointsTo_:ptr:{new_ptr}");
                }
            } else if let Some(ptr) = parse_ptr(text) {
                if let Some(new_ptr) = node_id_map.get(&ptr).or_else(|| op_ptr_map.get(&ptr)) {
                    *text = format!("ptr:{new_ptr}");
                }
            }
        }
        _ => {}
    }
}

fn next_synthetic_id(synthetic_base: &mut u32) -> BbNodeId {
    let id = *synthetic_base;
    *synthetic_base += 1;
    id
}

fn parse_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

fn parse_points_to(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:").and_then(parse_ptr)
}