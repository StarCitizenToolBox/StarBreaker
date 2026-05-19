//! Static instantiation filter for BuildingBlocks canvases.
//!
//! When a BB canvas hosts multiple `BuildingBlocks_WidgetCanvas` children that
//! each represent one UI *state* (e.g. Attract, LogIn, MainMenu, Admin), each
//! child's visibility field is bound to a runtime boolean variable via an
//! `operations[]` entry.  At runtime exactly one child is active at a time.
//!
//! Visibility-controlling fields observed in real canvases:
//! - `Instantiated` — widget is created only when true.
//! - `IsActive` — widget is enabled/visible only when true.
//! - `Visible` / `Enabled` — common widget visibility shorthands.
//!
//! During a static export there is no runtime; instead the canvas record may
//! carry a `staticVariables[]` array that declares which variables are `true`
//! by default.  All other variables default to `false`.
//!
//! This module evaluates the boolean expression graph for every
//! `BuildingBlocks_BindingsBooleanField` operation that targets one of the
//! visibility fields and returns the set of widget pointer IDs whose
//! visibility evaluates to `false`, so the caller can skip following those
//! canvas URLs in Pass 2 of the resolver.
//!
//! # Naming convention
//!
//! `staticVariables[i].name` is the binding path with an `"_SV"` suffix
//! (e.g. `"Standing/state.BaseScreens.Admin_SV"` for the `Admin` variable
//! whose binding path is `"Standing/state.BaseScreens.Admin"`).
//! Stripping the `"_SV"` suffix recovers the binding path used in the
//! corresponding `BuildingBlocks_BindingsBooleanVariable` operation.

use std::collections::{HashMap, HashSet};

use crate::bb_scene::BbNodeId;

// ──────────────────────────────────────────────────────────────────────────────
// Public entry point
// ──────────────────────────────────────────────────────────────────────────────

/// Return the set of WidgetCanvas node pointer IDs whose `Instantiated` field
/// binding evaluates to `false` under static defaults.
///
/// Canvas nodes in the returned set should be skipped by Pass 2 of the
/// resolver — their URL should not be followed.  Any canvas node whose
/// `Instantiated` has no binding, or whose binding evaluates to `true`, is
/// **not** in the set and must be followed normally.
///
/// `record_value` is the `_RecordValue_` object of the root canvas record.
pub fn instantiated_false_widgets(record_value: &serde_json::Value) -> HashSet<BbNodeId> {
    let static_vals = parse_static_variables(record_value);
    let ops = match record_value.get("operations").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return HashSet::new(),
    };

    let ptr_vals = evaluate_bool_ops(ops, &static_vals);

    let mut false_set: HashSet<BbNodeId> = HashSet::new();
    for op in ops {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "BuildingBlocks_BindingsBooleanField" {
            continue;
        }
        let field = op.get("field").and_then(|v| v.as_str()).unwrap_or("");
        // Generalized visibility fields. `Instantiated` is the canonical state
        // switcher on `WidgetCanvas`.  `IsActive` is the equivalent for many
        // composed canvases (e.g. `I_Med_MedicalEndOfBed_A`).  `Visible` and
        // `Enabled` cover ad-hoc widget hiding (e.g. "View Patient" only when
        // ActorIsInBed is true).
        if !matches!(field, "Instantiated" | "IsActive" | "Visible" | "Enabled") {
            continue;
        }
        let Some(widget) = op
            .get("widget")
            .and_then(|v| v.as_str())
            .and_then(parse_points_to_ptr)
        else {
            continue;
        };
        let val = op
            .get("input")
            .and_then(|v| v.as_str())
            .and_then(parse_points_to_ptr)
            .and_then(|inp| ptr_vals.get(&inp).copied())
            .unwrap_or(true); // no binding or unknown expression → default true (show)
        if !val {
            false_set.insert(widget);
        }
    }
    false_set
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Parse `staticVariables[]` into a map from binding path → bool.
///
/// The `name` field uses the `"<binding>_SV"` convention; stripping the
/// `"_SV"` suffix recovers the binding path.
fn parse_static_variables(record_value: &serde_json::Value) -> HashMap<String, bool> {
    let mut map: HashMap<String, bool> = HashMap::new();
    let Some(arr) = record_value
        .get("staticVariables")
        .and_then(|v| v.as_array())
    else {
        return map;
    };
    for sv in arr {
        let name = sv.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let val = sv.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
        let binding = name.strip_suffix("_SV").unwrap_or(name);
        if !binding.is_empty() {
            map.insert(binding.to_owned(), val);
        }
    }
    map
}

/// Evaluate the boolean operation graph, returning a map from pointer ID to
/// the statically-resolved bool value.
///
/// Only operations with a `_Pointer_` field contribute to the map.  Operations
/// whose inputs are not yet resolved are retried in subsequent iterations until
/// no further progress is made (fixpoint).
fn evaluate_bool_ops(
    ops: &[serde_json::Value],
    static_vals: &HashMap<String, bool>,
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

fn parse_ptr_id(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

fn parse_points_to_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:ptr:").and_then(|n| n.parse().ok())
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn boolean_field_op(widget_ptr: u32, input_ptr: u32) -> serde_json::Value {
        json!({
            "_Type_": "BuildingBlocks_BindingsBooleanField",
            "widget": format!("_PointsTo_:ptr:{widget_ptr}"),
            "field": "Instantiated",
            "input": format!("_PointsTo_:ptr:{input_ptr}")
        })
    }

    fn variable_op(ptr: u32, binding: &str) -> serde_json::Value {
        json!({
            "_Pointer_": format!("ptr:{ptr}"),
            "_Type_": "BuildingBlocks_BindingsBooleanVariable",
            "binding": binding
        })
    }

    fn static_var(name: &str, val: bool) -> serde_json::Value {
        json!({ "name": name, "value": val })
    }

    fn make_record_value(
        static_vars: Vec<serde_json::Value>,
        ops: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        json!({
            "_Type_": "BuildingBlocks_Canvas",
            "staticVariables": static_vars,
            "operations": ops
        })
    }

    // ── test 1 ──────────────────────────────────────────────────────────────

    /// Canvas with no operations produces an empty false set.
    #[test]
    fn no_operations_returns_empty_set() {
        let rv = make_record_value(vec![], vec![]);
        let result = instantiated_false_widgets(&rv);
        assert!(result.is_empty(), "expected empty set, got {result:?}");
    }

    // ── test 2 ──────────────────────────────────────────────────────────────

    /// A WidgetCanvas whose Instantiated is bound to a variable with a
    /// `staticVariables` entry of `true` must NOT appear in the false set.
    #[test]
    fn static_true_variable_widget_is_not_filtered() {
        let rv = make_record_value(
            vec![static_var("state.Admin_SV", true)],
            vec![
                variable_op(10, "state.Admin"),
                boolean_field_op(5, 10), // ptr:5 (WidgetCanvas) bound to ptr:10 (Admin=true)
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&5),
            "Admin=true canvas (ptr:5) must not be filtered"
        );
    }

    // ── test 3 ──────────────────────────────────────────────────────────────

    /// A WidgetCanvas whose Instantiated is bound to a variable with no static
    /// override (defaults to `false`) must appear in the false set.
    #[test]
    fn no_static_var_defaults_to_false_and_is_filtered() {
        let rv = make_record_value(
            vec![], // no static variables → all default false
            vec![
                variable_op(10, "state.Attract"),
                boolean_field_op(3, 10), // ptr:3 bound to Attract=false
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&3),
            "Attract=false canvas (ptr:3) must be in false set"
        );
    }

    // ── test 4 ──────────────────────────────────────────────────────────────

    /// Compound expression: `Instantiated = OR(false, false)` must produce
    /// `false` → canvas is in the false set.
    #[test]
    fn compound_or_false_false_is_filtered() {
        // ptr:20 = ConfirmMore (no static → false)
        // ptr:21 = ConfirmNone (no static → false)
        // ptr:23 = OR(ptr:20, ptr:21) → false
        // ptr:7 (WidgetCanvas) bound to ptr:23 → filtered
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(20, "state.ConfirmMore"),
                variable_op(21, "state.ConfirmNone"),
                json!({
                    "_Pointer_": "ptr:23",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:20", "_PointsTo_:ptr:21"]
                }),
                boolean_field_op(7, 23),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&7),
            "OR(false, false) canvas (ptr:7) must be in false set"
        );
    }

    // ── test 5 ──────────────────────────────────────────────────────────────

    /// A WidgetCanvas with no Instantiated binding at all is never filtered.
    #[test]
    fn canvas_without_instantiated_binding_is_not_filtered() {
        // Only a non-Instantiated field op exists for ptr:9.
        let rv = make_record_value(
            vec![],
            vec![json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:9",
                "field": "IsActive",
                "input": "_PointsTo_:ptr:10"
            })],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&9),
            "canvas with no Instantiated binding must not be filtered"
        );
    }

    // ── test 6 ──────────────────────────────────────────────────────────────

    /// Invert: `Instantiated = NOT(false)` → `true` → canvas is shown.
    #[test]
    fn invert_of_false_is_true_and_not_filtered() {
        // ptr:3 = Attract (no static → false)
        // ptr:6 = NOT(ptr:3) → true
        // ptr:5 (Header WidgetCanvas) bound to ptr:6 → shown
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Attract"),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&5),
            "NOT(false) canvas (ptr:5) must not be filtered"
        );
    }

    #[test]
    fn is_active_false_widget_is_filtered() {
        // `IsActive` is treated as a visibility field just like `Instantiated`.
        // Variable `state.Foo` has no static default → false → widget ptr:5 hidden.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Foo"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:5",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "IsActive=false widget (ptr:5) must be filtered");
    }

    #[test]
    fn visible_false_widget_is_filtered() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.ActorIsInBed"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:7",
                    "field": "Visible",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&7), "Visible=false widget (ptr:7) must be filtered");
    }

    #[test]
    fn unknown_field_is_not_filtered() {
        // Bindings to non-visibility fields (e.g. `Text`, `Color`) must not
        // cause widget filtering.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.SomeFlag"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:9",
                    "field": "Text",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&9),
            "Bindings to non-visibility fields must not filter the widget"
        );
    }
}
