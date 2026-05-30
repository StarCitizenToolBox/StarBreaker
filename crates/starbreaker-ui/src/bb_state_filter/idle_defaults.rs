use std::collections::{HashMap, HashSet};
use crate::bb_scene::BbNodeId;
use super::eval::{
    parse_points_to_ptr_value,
    parse_ptr_id,
    resolve_op_ref,
    resolve_op_ref_with_visited,
};
#[cfg(test)]
use super::eval::parse_points_to_ptr;
pub(super) fn apply_idle_defaults(
    ops: &[serde_json::Value],
    static_vals: &mut HashMap<String, bool>,
) {
    // ptr → op
    let mut ptr_to_op: HashMap<BbNodeId, &serde_json::Value> = HashMap::new();
    for op in ops {
        if let Some(p) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(parse_ptr_id)
        {
            ptr_to_op.insert(p, op);
        }
    }

    // Resolve a pointer to the variable bindings it ultimately reads.
    // Returns the ordered list of variable binding names (in Or-operand
    // order, with the first being the cold-default candidate).
    fn resolve_vars(
        input: &serde_json::Value,
        ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
        visited: &mut HashSet<BbNodeId>,
    ) -> Vec<String> {
        let Some(op) = resolve_op_ref_with_visited(input, ptr_to_op, visited) else {
            return Vec::new();
        };
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "BuildingBlocks_BindingsBooleanVariable" => op
                .get("binding")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| vec![s.to_owned()])
                .unwrap_or_default(),
            "BuildingBlocks_BindingsBooleanEvaluateOr" => {
                let mut out = Vec::new();
                if let Some(inputs) = op.get("inputs").and_then(|v| v.as_array()) {
                    for inp in inputs {
                        out.extend(resolve_vars(inp, ptr_to_op, visited));
                    }
                }
                out
            }
            // EvaluateAnd is not an idle-gate (would require ALL false to
            // unhide Header), so skip; ditto for unknown ops.
            _ => Vec::new(),
        }
    }

    // Collect cold-default candidates from both structural patterns:
    //   1. Invert-of-Or framing-widget pattern (Header hidden when ANY state)
    //   2. Direct-variable scene-order pattern (sibling canvas gated by a
    //      single state variable; first such in operations order is default)
    //
    // Group membership for a state-group is detected by inspecting all
    // `BindingsBooleanVariable` ops in `operations[]` and grouping by the
    // dotted prefix of their `binding`.  A group must contain ≥2 variables
    // to qualify (a single variable is not a mutual-exclusion group).
    let mut group_index: HashMap<String, Vec<String>> = HashMap::new();
    for op in ops {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if ty == "BuildingBlocks_BindingsBooleanVariable" {
            let Some(name) = op.get("binding").and_then(|v| v.as_str()) else {
                continue;
            };
            if name.ends_with("_SV") {
                continue;
            }
            let Some((prefix, _)) = name.rsplit_once('.') else {
                continue;
            };
            let bucket = group_index.entry(prefix.to_owned()).or_default();
            if !bucket.iter().any(|s| s == name) {
                bucket.push(name.to_owned());
            }
            continue;
        }
        if ty == "BuildingBlocks_BindingsBooleanField" {
            let Some(input) = op.get("input") else {
                continue;
            };
            let mut visited = HashSet::new();
            for name in resolve_vars(input, &ptr_to_op, &mut visited) {
                if name.ends_with("_SV") {
                    continue;
                }
                let Some((prefix, _)) = name.rsplit_once('.') else {
                    continue;
                };
                let bucket = group_index.entry(prefix.to_owned()).or_default();
                if !bucket.iter().any(|s| s == &name) {
                    bucket.push(name);
                }
            }
        }
    }
    // Retain only prefixes with ≥2 distinct member variables.
    group_index.retain(|_, v| v.len() >= 2);

    #[derive(Clone)]
    struct Candidate {
        cold_defaults: Vec<String>,
    }

    let mut candidates: Vec<Candidate> = Vec::new();

    for op in ops {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "BuildingBlocks_BindingsBooleanField" {
            continue;
        }
        let field = op.get("field").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(field, "Instantiated" | "IsActive") {
            continue;
        }
        let Some(input_op) = op
            .get("input")
            .and_then(|v| resolve_op_ref(v, &ptr_to_op))
        else {
            continue;
        };
        let input_ty = input_op
            .get("_Type_")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Pattern detection: figure out the candidate variable name(s).
        let cold_defaults: Vec<String> = match input_ty {
            // Pattern 1: Invert(EvaluateOr(Var1, Var2, ...)) → Var1 is default.
            //
            // A plain Invert(SingleVariable) is a single-flag hide gate, not
            // an idle-default candidate; skipped here.
            "BuildingBlocks_BindingsBooleanInvert" => {
                let inner_ref = input_op.get("input");
                let inner_ty = inner_ref
                    .and_then(|v| resolve_op_ref(v, &ptr_to_op))
                    .and_then(|o| o.get("_Type_"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if inner_ty == "BuildingBlocks_BindingsBooleanEvaluateOr" {
                    let mut visited = HashSet::new();
                    let vars = inner_ref
                        .map(|v| resolve_vars(v, &ptr_to_op, &mut visited))
                        .unwrap_or_default();
                    vars.into_iter().take(1).collect()
                } else {
                    Vec::new()
                }
            }
            // Pattern 2: direct SingleVariable → that variable, if it belongs
            // to a recognized state-group.
            "BuildingBlocks_BindingsBooleanVariable" => input_op
                .get("binding")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty() && !s.ends_with("_SV"))
                .map(|s| vec![s.to_owned()])
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        if cold_defaults.is_empty() {
            continue;
        }
        let Some(prefix) = common_state_prefix(&cold_defaults) else {
            continue;
        };
        // Must be a recognized multi-member state group.
        let Some(_group_members) = group_index.get(prefix) else {
            continue;
        };
        candidates.push(Candidate {
            cold_defaults,
        });
    }

    let mut grouped: HashMap<String, Vec<Candidate>> = HashMap::new();
    for candidate in candidates {
        if let Some(prefix) = common_state_prefix(&candidate.cold_defaults) {
            grouped.entry(prefix.to_owned()).or_default().push(candidate);
        }
    }

    for (prefix, group_members) in group_index {
        let mut group: HashSet<String> = group_members.into_iter().collect();
        for key in static_vals.keys() {
            if key.ends_with("_SV") {
                continue;
            }
            if let Some((p, _)) = key.rsplit_once('.') {
                if p == prefix {
                    group.insert(key.clone());
                }
            }
        }
        let any_explicit_override = group.iter().any(|m| static_vals.get(m).copied() == Some(true));
        if any_explicit_override {
            continue;
        }

        let Some(candidates) = grouped.remove(&prefix) else {
            continue;
        };

        let bed_mainmenu = if prefix == "Bed/state.BaseScreens" {
            candidates.iter().find(|candidate| {
                candidate
                    .cold_defaults
                    .iter()
                    .any(|binding| binding.ends_with(".MainMenu"))
            })
        } else {
            None
        };
        let selected = bed_mainmenu.or_else(|| candidates.first());

        let Some(selected) = selected else {
            continue;
        };

        for cold_default in &selected.cold_defaults {
            static_vals.entry(cold_default.clone()).or_insert(true);
        }
    }
}

#[cfg(test)]
pub(super) fn scene_widget_map(record_value: &serde_json::Value) -> HashMap<BbNodeId, &serde_json::Value> {
    let mut map = HashMap::new();
    let Some(scene) = record_value.get("scene").and_then(|v| v.as_array()) else {
        return map;
    };
    for item in scene {
        let Some(ptr) = item
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(parse_ptr_id)
        else {
            continue;
        };
        map.insert(ptr, item);
    }
    map
}

#[cfg(test)]
pub(super) fn scene_widget_boolean_param_count(
    scene_items: &HashMap<BbNodeId, &serde_json::Value>,
    widget_ptr: BbNodeId,
) -> usize {
    let mut stack = vec![widget_ptr];
    let mut count = 0usize;
    while let Some(current_ptr) = stack.pop() {
        let Some(item) = scene_items.get(&current_ptr) else {
            continue;
        };
        count += item
            .get("paramInputValues")
            .and_then(|v| v.as_array())
            .map(|params| {
                params
                    .iter()
                    .filter(|param| {
                        param
                            .get("_Type_")
                            .and_then(|v| v.as_str())
                            .is_some_and(|ty| ty.contains("Boolean"))
                    })
                    .count()
            })
            .unwrap_or(0);
        for (child_ptr, child_item) in scene_items {
            let Some(parent_ptr) = child_item
                .get("parent")
                .and_then(|v| v.as_str())
                .and_then(parse_points_to_ptr)
            else {
                continue;
            };
            if parent_ptr == current_ptr {
                stack.push(*child_ptr);
            }
        }
    }
    count
}

#[allow(dead_code)]
fn is_framing_hidden_set_field(
    input_ref: &serde_json::Value,
    widget_ptr: BbNodeId,
    ops: &[serde_json::Value],
) -> bool {
    let mut ptr_to_op: HashMap<BbNodeId, &serde_json::Value> = HashMap::new();
    for op in ops {
        if let Some(p) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(parse_ptr_id)
        {
            ptr_to_op.insert(p, op);
        }
    }
    let mut visited = HashSet::new();
    let Some(input_op) = resolve_op_ref_with_visited(input_ref, &ptr_to_op, &mut visited) else {
        return false;
    };
    if input_op.get("_Type_").and_then(|v| v.as_str())
        != Some("BuildingBlocks_BindingsBooleanInvert")
    {
        return false;
    }
    let Some(inner_ref) = input_op.get("input") else {
        return false;
    };
    let mut inner_visited = HashSet::new();
    let Some(inner_op) = resolve_op_ref_with_visited(inner_ref, &ptr_to_op, &mut inner_visited) else {
        return false;
    };
    if inner_op.get("_Type_").and_then(|v| v.as_str())
        != Some("BuildingBlocks_BindingsBooleanEvaluateOr")
    {
        return false;
    }

    let mut vars = HashSet::new();
    if let Some(inputs) = inner_op.get("inputs").and_then(|v| v.as_array()) {
        for inp in inputs {
            let mut var_visited = HashSet::new();
            if let Some(var_op) = resolve_op_ref_with_visited(inp, &ptr_to_op, &mut var_visited) {
                if var_op.get("_Type_").and_then(|v| v.as_str())
                    == Some("BuildingBlocks_BindingsBooleanVariable")
                {
                    if let Some(binding) = var_op.get("binding").and_then(|v| v.as_str()) {
                        vars.insert(binding.to_owned());
                    }
                }
            }
        }
    }
    if vars.is_empty() {
        return false;
    }

    for op in ops {
        if op.get("_Type_").and_then(|v| v.as_str())
            != Some("BuildingBlocks_BindingsBooleanField")
        {
            continue;
        }
        let Some(other_widget) = parse_points_to_ptr_value(op.get("widget")) else {
            continue;
        };
        if other_widget == widget_ptr {
            continue;
        }
        let Some(other_input) = op.get("input") else {
            continue;
        };
        let mut other_visited = HashSet::new();
        let Some(other_op) = resolve_op_ref_with_visited(other_input, &ptr_to_op, &mut other_visited) else {
            continue;
        };
        if other_op.get("_Type_").and_then(|v| v.as_str())
            != Some("BuildingBlocks_BindingsBooleanVariable")
        {
            continue;
        }
        if other_op
            .get("binding")
            .and_then(|v| v.as_str())
            .is_some_and(|binding| vars.contains(binding))
        {
            return true;
        }
    }
    false
}

fn common_state_prefix(names: &[String]) -> Option<&str> {
    let mut iter = names.iter();
    let first = iter.next()?;
    let (prefix, _) = first.rsplit_once('.')?;
    if iter.all(|name| name.rsplit_once('.').is_some_and(|(p, _)| p == prefix)) {
        Some(prefix)
    } else {
        None
    }
}

