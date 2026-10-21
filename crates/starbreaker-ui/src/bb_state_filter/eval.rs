use std::collections::{HashMap, HashSet};

use crate::bb_scene::BbNodeId;

pub(super) fn evaluate_bool_ops(
    ops: &[serde_json::Value],
    static_vals: &HashMap<String, bool>,
    static_nums: &HashMap<String, f64>,
    _param_overrides: &HashMap<String, bool>,
) -> HashMap<BbNodeId, bool> {
    let mut ptr_val: HashMap<BbNodeId, bool> = HashMap::new();

    // Static op index for resolving integer operands (`inputL`/`inputR`
    // pointers) when evaluating `BooleanFromInteger`. Built once — it depends
    // only on the op structure, not on the fixpoint's resolved booleans.
    let ptr_to_op: HashMap<BbNodeId, &serde_json::Value> = ops
        .iter()
        .filter_map(|op| {
            let p = op.get("_Pointer_").and_then(|v| v.as_str()).and_then(parse_ptr_id)?;
            Some((p, op))
        })
        .collect();

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
                        // Short-circuit on a determining operand (any known `true`
                        // → `true`); consistent with `eval_bool_ref`.
                        let inputs = op.get("inputs").and_then(|v| v.as_array())?;
                        let mut any_unknown = false;
                        for inp_v in inputs {
                            let inp = inp_v.as_str().and_then(parse_points_to_ptr)?;
                            match ptr_val.get(&inp).copied() {
                                Some(true) => return Some(true),
                                Some(false) => {}
                                None => any_unknown = true,
                            }
                        }
                        if any_unknown { None } else { Some(false) }
                    }
                    "BuildingBlocks_BindingsBooleanEvaluateAnd" => {
                        // Short-circuit on a determining operand (any known `false`
                        // → `false`); consistent with `eval_bool_ref`.
                        let inputs = op.get("inputs").and_then(|v| v.as_array())?;
                        let mut any_unknown = false;
                        for inp_v in inputs {
                            let inp = inp_v.as_str().and_then(parse_points_to_ptr)?;
                            match ptr_val.get(&inp).copied() {
                                Some(false) => return Some(false),
                                Some(true) => {}
                                None => any_unknown = true,
                            }
                        }
                        if any_unknown { None } else { Some(true) }
                    }
                    // Integer-state op-types resolved to their at-rest values,
                    // mirroring `eval_bool_ref` (see there for the rationale).
                    "BuildingBlocks_BindingsBooleanFromIntegerSwitch" => {
                        op.get("defaultValue").and_then(|v| v.as_bool()).or(Some(false))
                    }
                    "BuildingBlocks_BindingsBooleanFromInteger" => {
                        super::integer::eval_bool_from_integer(op, &ptr_to_op, static_nums)
                    }
                    "BuildingBlocks_BindingsBooleanFromNumber" => {
                        super::integer::eval_bool_from_number(op, &ptr_to_op, static_nums)
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
    static_nums: &HashMap<String, f64>,
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
            eval_bool_ref(op, ptr_vals, ptr_to_op, static_vals, static_nums, param_overrides, visiting)
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
                        // The parameter's `name` is its source-variable
                        // identity: an authored staticVariable of that name
                        // carries the canvas's static default and wins over
                        // the editor `defaultValue` (the power screen's
                        // notification overlays are authored
                        // `engineeringoverride`/`PresetNotification = false`
                        // but default `true` for editor preview). Names are
                        // matched case-insensitively: gen-level parameter
                        // names are lower-cased while master staticVariables
                        // keep authored casing.
                        .or_else(|| {
                            let name = obj.get("name").and_then(|v| v.as_str())?;
                            static_vals
                                .iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                                .map(|(_, v)| *v)
                        })
                        .or_else(|| obj.get("defaultValue").and_then(|v| v.as_bool()))
                        .or(Some(true))
                }
                "BuildingBlocks_BindingsBooleanInvert" => {
                    let inner = obj.get("input")?;
                    eval_bool_ref(inner, ptr_vals, ptr_to_op, static_vals, static_nums, param_overrides, visiting)
                        .map(|v| !v)
                }
                "BuildingBlocks_BindingsBooleanEvaluateOr" => {
                    // Short-circuit on a determining resolved operand: any `true`
                    // → `true`. Only undetermined (an unresolved operand and no
                    // resolved `true`) yields `None`, instead of bailing the whole
                    // Or to `None` the moment one operand is unresolved.
                    let inputs = obj.get("inputs").and_then(|v| v.as_array())?;
                    let mut any_unresolved = false;
                    for inp in inputs {
                        match eval_bool_ref(inp, ptr_vals, ptr_to_op, static_vals, static_nums, param_overrides, visiting) {
                            Some(true) => return Some(true),
                            Some(false) => {}
                            None => any_unresolved = true,
                        }
                    }
                    if any_unresolved { None } else { Some(false) }
                }
                "BuildingBlocks_BindingsBooleanEvaluateAnd" => {
                    // Short-circuit on a determining resolved operand: any `false`
                    // → `false` (so an at-rest event flag of `false` hides the
                    // overlay even when a sibling operand is unresolved).
                    let inputs = obj.get("inputs").and_then(|v| v.as_array())?;
                    let mut any_unresolved = false;
                    for inp in inputs {
                        match eval_bool_ref(inp, ptr_vals, ptr_to_op, static_vals, static_nums, param_overrides, visiting) {
                            Some(false) => return Some(false),
                            Some(true) => {}
                            None => any_unresolved = true,
                        }
                    }
                    if any_unresolved { None } else { Some(true) }
                }
                "BuildingBlocks_BindingsBooleanFromIntegerSwitch" => {
                    // No integer-state evaluator: at rest the integer input is its
                    // cold default, which is not in `exceptions`, so the switch
                    // yields its authored `defaultValue`. This resolves event
                    // overlays (incoming-call, low-power, warnings) to their
                    // at-rest hidden state instead of a conservative `None`.
                    obj.get("defaultValue").and_then(|v| v.as_bool()).or(Some(false))
                }
                "BuildingBlocks_BindingsBooleanFromInteger" => {
                    // A comparison `inputL <type> {inputR | value}`. When the
                    // operands resolve statically (an `IntegerComponentParameter`
                    // default, or a runtime `IntegerVariable` whose at-rest cold
                    // default is in `static_nums`) the real comparison is
                    // computed; otherwise the at-rest heuristic applies. This
                    // keeps the engine pattern working — content gated by
                    // `Invert(Equal off_state)` stays shown, event overlays gated
                    // by `Equal event_state` stay hidden — and the frame's
                    // unpinned runtime `IntegerVariable` gates (e.g.
                    // `Invert(powerstate == 0)`) stay on the heuristic.
                    super::integer::eval_bool_from_integer(input, ptr_to_op, static_nums)
                }
                "BuildingBlocks_BindingsBooleanFromNumber" => {
                    // Float analogue: `input <type> {inputB | number}`. Resolves a
                    // runtime `NumberVariable` from its `static_nums` cold default
                    // (e.g. the countermeasure `BurstSizeHoldRatio` = 0 at rest)
                    // so the firing-state overlay gate evaluates `false`.
                    super::integer::eval_bool_from_number(input, ptr_to_op, static_nums)
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
    static_nums: &HashMap<String, f64>,
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
            contains_unset_non_state_variable(op, ptr_to_op, static_vals, static_nums, visited)
        }
        serde_json::Value::Object(obj) => {
            let ty = obj.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(binding) = obj.get("binding").and_then(|v| v.as_str()) {
                if !binding.is_empty() && !is_state_binding(binding) {
                    if ty == "BuildingBlocks_BindingsBooleanVariable" {
                        if !static_vals.contains_key(binding) {
                            return true;
                        }
                    } else if !static_nums.contains_key(binding) {
                        // Non-boolean runtime binding families (for example
                        // IntegerVariable used by BooleanFromInteger gates) are
                        // "unset" only when no at-rest cold default resolves them.
                        // A binding present in `static_nums` (e.g. the firing
                        // state CurrentBurstSize/BurstSizeHoldRatio = 0) IS
                        // resolved, so the gate's computed value must stand.
                        return true;
                    }
                }
            }

            for key in ["input", "inputL", "inputR", "inputTrue", "inputFalse"] {
                if obj
                    .get(key)
                    .is_some_and(|inner| contains_unset_non_state_variable(inner, ptr_to_op, static_vals, static_nums, visited))
                {
                    return true;
                }
            }

            obj.get("inputs")
                .and_then(|v| v.as_array())
                .is_some_and(|inputs| {
                    inputs
                        .iter()
                        .any(|inp| contains_unset_non_state_variable(inp, ptr_to_op, static_vals, static_nums, visited))
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

