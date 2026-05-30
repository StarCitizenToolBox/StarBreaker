use std::collections::{HashMap, HashSet};

use crate::bb_scene::BbNodeId;

pub(super) fn evaluate_bool_ops(
    ops: &[serde_json::Value],
    static_vals: &HashMap<String, bool>,
    _param_overrides: &HashMap<String, bool>,
) -> HashMap<BbNodeId, bool> {
    let mut ptr_val: HashMap<BbNodeId, bool> = HashMap::new();

    loop {
        let mut changed = false;
        for op in ops {
            let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            let Some(ptr) = op
                .get("_Pointer_")
                .and_then(|v| v.as_str())
                .and_then(parse_ptr_id)
            else {
                continue;
            };
            if ptr_val.contains_key(&ptr) {
                continue;
            }

            let val: Option<bool> = (|| -> Option<bool> {
                match ty {
                    "BuildingBlocks_BindingsBooleanVariable" => {
                        let binding = op.get("binding").and_then(|v| v.as_str()).unwrap_or("");
                        Some(*static_vals.get(binding).unwrap_or(&false))
                    }
                    "BuildingBlocks_BindingsBooleanInvert" => {
                        let inp = op
                            .get("input")
                            .and_then(|v| v.as_str())
                            .and_then(parse_points_to_ptr)?;
                        ptr_val.get(&inp).copied().map(|v| !v)
                    }
                    "BuildingBlocks_BindingsBooleanEvaluateOr" => {
                        let inputs = op.get("inputs").and_then(|v| v.as_array())?;
                        let mut result = false;
                        let mut all_known = true;
                        for inp_v in inputs {
                            let inp = inp_v.as_str().and_then(parse_points_to_ptr)?;
                            match ptr_val.get(&inp).copied() {
                                Some(v) => result |= v,
                                None => {
                                    all_known = false;
                                    break;
                                }
                            }
                        }
                        if all_known { Some(result) } else { None }
                    }
                    "BuildingBlocks_BindingsBooleanEvaluateAnd" => {
                        let inputs = op.get("inputs").and_then(|v| v.as_array())?;
                        let mut result = true;
                        let mut all_known = true;
                        for inp_v in inputs {
                            let inp = inp_v.as_str().and_then(parse_points_to_ptr)?;
                            match ptr_val.get(&inp).copied() {
                                Some(v) => result &= v,
                                None => {
                                    all_known = false;
                                    break;
                                }
                            }
                        }
                        if all_known { Some(result) } else { None }
                    }
                    _ => None,
                }
            })();

            if let Some(v) = val {
                ptr_val.insert(ptr, v);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    ptr_val
}

pub(super) fn parse_points_to_ptr_value(v: Option<&serde_json::Value>) -> Option<BbNodeId> {
    match v {
        Some(serde_json::Value::String(s)) => parse_points_to_ptr(s),
        Some(serde_json::Value::Object(_)) => {
            v.and_then(|obj| obj.get("_Pointer_"))
                .and_then(|p| p.as_str())
                .and_then(parse_ptr_id)
        }
        _ => None,
    }
}

pub(super) fn resolve_op_ref<'a>(
    input: &'a serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &'a serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    match input {
        serde_json::Value::String(s) => parse_points_to_ptr(s).and_then(|p| ptr_to_op.get(&p).copied()),
        serde_json::Value::Object(_) => Some(input),
        _ => None,
    }
}

pub(super) fn resolve_op_ref_with_visited<'a>(
    input: &'a serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &'a serde_json::Value>,
    visited: &mut HashSet<BbNodeId>,
) -> Option<&'a serde_json::Value> {
    match input {
        serde_json::Value::String(s) => {
            let ptr = parse_points_to_ptr(s)?;
            if !visited.insert(ptr) {
                return None;
            }
            ptr_to_op.get(&ptr).copied()
        }
        serde_json::Value::Object(_) => Some(input),
        _ => None,
    }
}

pub(super) fn eval_bool_ref(
    input: &serde_json::Value,
    ptr_vals: &HashMap<BbNodeId, bool>,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    static_vals: &HashMap<String, bool>,
    param_overrides: &HashMap<String, bool>,
    visiting: &mut HashSet<BbNodeId>,
) -> Option<bool> {
    match input {
        serde_json::Value::String(s) => {
            let ptr = parse_points_to_ptr(s)?;
            if let Some(v) = ptr_vals.get(&ptr).copied() {
                return Some(v);
            }
            if !visiting.insert(ptr) {
                return None;
            }
            let op = ptr_to_op.get(&ptr)?;
            eval_bool_ref(op, ptr_vals, ptr_to_op, static_vals, param_overrides, visiting)
        }
        serde_json::Value::Object(obj) => {
            let ty = obj.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            match ty {
                "_SynthBooleanParam_" => obj.get("resolvedBool").and_then(|v| v.as_bool()),
                "BuildingBlocks_BindingsBooleanVariable" => {
                    let binding = obj.get("binding").and_then(|v| v.as_str()).unwrap_or("");
                    Some(*static_vals.get(binding).unwrap_or(&false))
                }
                "BuildingBlocks_BindingsBooleanComponentParameter" => {
                    let param_name = obj
                        .get("parameter")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_ascii_lowercase())
                        .unwrap_or_default();
                    param_overrides
                        .get(&param_name)
                        .copied()
                        .or_else(|| obj.get("defaultValue").and_then(|v| v.as_bool()))
                        .or(Some(true))
                }
                "BuildingBlocks_BindingsBooleanInvert" => {
                    let inner = obj.get("input")?;
                    eval_bool_ref(inner, ptr_vals, ptr_to_op, static_vals, param_overrides, visiting)
                        .map(|v| !v)
                }
                "BuildingBlocks_BindingsBooleanEvaluateOr" => {
                    let inputs = obj.get("inputs").and_then(|v| v.as_array())?;
                    let mut any = false;
                    for inp in inputs {
                        let v = eval_bool_ref(
                            inp,
                            ptr_vals,
                            ptr_to_op,
                            static_vals,
                            param_overrides,
                            visiting,
                        )?;
                        any |= v;
                    }
                    Some(any)
                }
                "BuildingBlocks_BindingsBooleanEvaluateAnd" => {
                    let inputs = obj.get("inputs").and_then(|v| v.as_array())?;
                    let mut all = true;
                    for inp in inputs {
                        let v = eval_bool_ref(
                            inp,
                            ptr_vals,
                            ptr_to_op,
                            static_vals,
                            param_overrides,
                            visiting,
                        )?;
                        all &= v;
                    }
                    Some(all)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

pub(super) fn parse_ptr_id(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

pub(super) fn parse_points_to_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:ptr:").and_then(|n| n.parse().ok())
}

pub(super) fn contains_unset_non_state_variable(
    input: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    static_vals: &HashMap<String, bool>,
    visited: &mut HashSet<BbNodeId>,
) -> bool {
    match input {
        serde_json::Value::String(s) => {
            let Some(ptr) = parse_points_to_ptr(s) else {
                return false;
            };
            if !visited.insert(ptr) {
                return false;
            }
            let Some(op) = ptr_to_op.get(&ptr) else {
                return false;
            };
            contains_unset_non_state_variable(op, ptr_to_op, static_vals, visited)
        }
        serde_json::Value::Object(obj) => {
            let ty = obj.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(binding) = obj.get("binding").and_then(|v| v.as_str()) {
                if !binding.is_empty() && !is_state_binding(binding) {
                    if ty == "BuildingBlocks_BindingsBooleanVariable" {
                        if !static_vals.contains_key(binding) {
                            return true;
                        }
                    } else {
                        // Non-boolean runtime binding families (for example
                        // IntegerVariable used by BooleanFromInteger gates)
                        // do not have authored static defaults in staticVariables.
                        return true;
                    }
                }
            }

            for key in ["input", "inputL", "inputR", "inputTrue", "inputFalse"] {
                if obj
                    .get(key)
                    .is_some_and(|inner| contains_unset_non_state_variable(inner, ptr_to_op, static_vals, visited))
                {
                    return true;
                }
            }

            obj.get("inputs")
                .and_then(|v| v.as_array())
                .is_some_and(|inputs| {
                    inputs
                        .iter()
                        .any(|inp| contains_unset_non_state_variable(inp, ptr_to_op, static_vals, visited))
                })
        }
        _ => false,
    }
}

pub(super) fn contains_namespace_placeholder_variable(
    input: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    visited: &mut HashSet<BbNodeId>,
) -> bool {
    match input {
        serde_json::Value::String(s) => {
            let Some(ptr) = parse_points_to_ptr(s) else {
                return false;
            };
            if !visited.insert(ptr) {
                return false;
            }
            let Some(op) = ptr_to_op.get(&ptr) else {
                return false;
            };
            contains_namespace_placeholder_variable(op, ptr_to_op, visited)
        }
        serde_json::Value::Object(obj) => {
            if let Some(binding) = obj.get("binding").and_then(|v| v.as_str())
                && binding.contains("/~/")
            {
                return true;
            }

            for key in ["input", "inputL", "inputR", "inputTrue", "inputFalse"] {
                if obj
                    .get(key)
                    .is_some_and(|inner| contains_namespace_placeholder_variable(inner, ptr_to_op, visited))
                {
                    return true;
                }
            }

            obj.get("inputs")
                .and_then(|v| v.as_array())
                .is_some_and(|inputs| {
                    inputs
                        .iter()
                        .any(|inp| contains_namespace_placeholder_variable(inp, ptr_to_op, visited))
                })
        }
        _ => false,
    }
}

pub(super) fn contains_non_boolean_runtime_binding(
    input: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    visited: &mut HashSet<BbNodeId>,
) -> bool {
    match input {
        serde_json::Value::String(s) => {
            let Some(ptr) = parse_points_to_ptr(s) else {
                return false;
            };
            if !visited.insert(ptr) {
                return false;
            }
            let Some(op) = ptr_to_op.get(&ptr) else {
                return false;
            };
            contains_non_boolean_runtime_binding(op, ptr_to_op, visited)
        }
        serde_json::Value::Object(obj) => {
            let ty = obj.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if obj.get("binding").and_then(|v| v.as_str()).is_some()
                && !ty.eq_ignore_ascii_case("BuildingBlocks_BindingsBooleanVariable")
            {
                return true;
            }

            for key in ["input", "inputL", "inputR", "inputTrue", "inputFalse"] {
                if obj
                    .get(key)
                    .is_some_and(|inner| contains_non_boolean_runtime_binding(inner, ptr_to_op, visited))
                {
                    return true;
                }
            }

            obj.get("inputs")
                .and_then(|v| v.as_array())
                .is_some_and(|inputs| {
                    inputs
                        .iter()
                        .any(|inp| contains_non_boolean_runtime_binding(inp, ptr_to_op, visited))
                })
        }
        _ => false,
    }
}


fn is_state_binding(binding: &str) -> bool {
    let lower = binding.to_ascii_lowercase();
    lower.starts_with("state.") || lower.contains("/state.") || lower.contains(".state.")
}

