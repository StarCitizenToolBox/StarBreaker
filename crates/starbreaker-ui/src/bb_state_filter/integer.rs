//! At-rest resolution of `BooleanFromInteger` comparison ops.
//!
//! BuildingBlocks gates content and event overlays on integer-state comparisons
//! of the form `inputL <type> {inputR | value}` (operator types: `Equal`,
//! `NotEqual`, `Greater`, `GreaterOrEqual`, `Less`, `LessOrEqual`). A static
//! export has no runtime integer state, but one integer source IS statically
//! known: an `IntegerComponentParameter` carries an authored `defaultValue`.
//!
//! [`eval_bool_from_integer`] resolves both operands and, when both are known,
//! computes the real comparison. Otherwise it falls back to the at-rest
//! heuristic — no *specific* integer state is active, so an `Equal value` check
//! is `false` (event overlays stay hidden) and `NotEqual value` is `true`
//! (content gated by "not the off-state" stays shown); ordered comparisons
//! depend on the actual integer and stay unresolved (`None`). Runtime
//! `IntegerVariable` / `NumberVariable` bindings (the frame's `powerstate` /
//! `criticalWarningState` gates) have no authored `defaultValue`, but a derived
//! AT-REST cold default in the registry — e.g. the countermeasure firing state
//! `CurrentBurstSize` / `BurstSizeHoldRatio`, both 0 when not firing — is
//! threaded in via `static_nums` (keyed by bare binding name), so those ordered
//! comparisons resolve. A binding with no `static_nums` entry still takes the
//! heuristic path, preserving the established at-rest frame behaviour.

use std::collections::HashMap;

use super::eval::parse_points_to_ptr;
use super::BbNodeId;

/// Resolve a numeric operand (integer OR float) to its statically-known value.
///
/// Sources: an authored `Integer`/`Number` `ComponentParameter` `defaultValue`,
/// or a runtime `Integer`/`Number` `Variable` binding whose at-rest cold default
/// is supplied in `static_nums` (keyed by the bare binding name). Any other
/// source returns `None`, so the caller keeps the conservative at-rest
/// heuristic. `operand` may be a `_PointsTo_:` pointer string (dereferenced via
/// `ptr_to_op`) or an inline operand object.
pub(super) fn resolve_static_number(
    operand: Option<&serde_json::Value>,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    static_nums: &HashMap<String, f64>,
) -> Option<f64> {
    let obj = match operand? {
        serde_json::Value::String(s) => *ptr_to_op.get(&parse_points_to_ptr(s)?)?,
        v @ serde_json::Value::Object(_) => v,
        _ => return None,
    };
    match obj.get("_Type_").and_then(|v| v.as_str()).unwrap_or("") {
        "BuildingBlocks_BindingsIntegerComponentParameter"
        | "BuildingBlocks_BindingsNumberComponentParameter" => {
            obj.get("defaultValue").and_then(|v| v.as_f64())
        }
        "BuildingBlocks_BindingsIntegerVariable" | "BuildingBlocks_BindingsNumberVariable" => {
            let binding = obj.get("binding").and_then(|v| v.as_str())?;
            static_nums.get(binding).copied()
        }
        _ => None,
    }
}

/// Resolve an integer operand to its statically-known value, or `None`. Wraps
/// [`resolve_static_number`] and rounds to the nearest integer (variable cold
/// defaults are whole numbers; the round only guards float representation).
pub(super) fn resolve_static_integer(
    operand: Option<&serde_json::Value>,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    static_nums: &HashMap<String, f64>,
) -> Option<i64> {
    resolve_static_number(operand, ptr_to_op, static_nums).map(|n| n.round() as i64)
}

/// Evaluate a `BooleanFromInteger` comparison op at rest.
///
/// The right operand is the wired `inputR` when present (it overrides the inline
/// `value` literal, which the engine uses only for an unwired right operand).
/// When both operands resolve statically the real comparison is returned;
/// otherwise the at-rest heuristic applies (see the module docs).
pub(super) fn eval_bool_from_integer(
    op: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    static_nums: &HashMap<String, f64>,
) -> Option<bool> {
    let cmp = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let lhs = resolve_static_integer(op.get("inputL"), ptr_to_op, static_nums);
    // A wired `inputR` operand takes precedence over the inline `value` literal;
    // `value` is the right-hand constant only when `inputR` is absent.
    let rhs = match op.get("inputR") {
        Some(input_r) if !input_r.is_null() => {
            resolve_static_integer(Some(input_r), ptr_to_op, static_nums)
        }
        _ => op.get("value").and_then(|v| v.as_i64()),
    };
    if let (Some(l), Some(r)) = (lhs, rhs) {
        return match cmp {
            "Equal" => Some(l == r),
            "NotEqual" => Some(l != r),
            "Greater" => Some(l > r),
            "GreaterOrEqual" => Some(l >= r),
            "Less" => Some(l < r),
            "LessOrEqual" => Some(l <= r),
            _ => None,
        };
    }
    // No specific integer state is active at rest: an equality check fails and
    // an inequality check holds; ordered comparisons stay unresolved.
    match cmp {
        "Equal" => Some(false),
        "NotEqual" => Some(true),
        _ => None,
    }
}

/// Evaluate a `BooleanFromNumber` comparison op at rest.
///
/// Float analogue of [`eval_bool_from_integer`]: the threshold is the wired
/// `inputB` operand when present, else the inline `number` literal. Resolves
/// `input` (the value operand) and the threshold via [`resolve_static_number`];
/// when both are known the real comparison is returned, else the same at-rest
/// heuristic (equality fails, inequality holds, ordered comparisons unresolved).
pub(super) fn eval_bool_from_number(
    op: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    static_nums: &HashMap<String, f64>,
) -> Option<bool> {
    let cmp = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let lhs = resolve_static_number(op.get("input"), ptr_to_op, static_nums);
    let rhs = match op.get("inputB") {
        Some(input_b) if !input_b.is_null() => {
            resolve_static_number(Some(input_b), ptr_to_op, static_nums)
        }
        _ => op.get("number").and_then(|v| v.as_f64()),
    };
    if let (Some(l), Some(r)) = (lhs, rhs) {
        return match cmp {
            "Equal" => Some(l == r),
            "NotEqual" => Some(l != r),
            "Greater" => Some(l > r),
            "GreaterOrEqual" => Some(l >= r),
            "Less" => Some(l < r),
            "LessOrEqual" => Some(l <= r),
            _ => None,
        };
    }
    match cmp {
        "Equal" => Some(false),
        "NotEqual" => Some(true),
        _ => None,
    }
}
