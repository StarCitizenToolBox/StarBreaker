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
//! # Capability vs active-mode variables
//!
//! BB canvases use two distinct families of boolean variables that share a
//! common namespace prefix but have different semantics:
//!
//! - **Active-mode** variables — bare binding path
//!   (e.g. `"Standing/state.BaseScreens.Admin"`).  Referenced from
//!   `BuildingBlocks_BindingsBooleanVariable` operations and gate which
//!   sub-canvas is currently visible.  Exactly one is true at a time.
//! - **Capability/sensor** variables — same path with an `"_SV"` suffix
//!   (e.g. `"Standing/state.BaseScreens.Admin_SV"`).  Authored into
//!   `staticVariables[]` to declare "this surface permits the named mode".
//!   They do NOT activate the matching active-mode variable; they only
//!   enable optional UI affordances (e.g. an Admin button in MainMenu)
//!   inside sub-canvases.  Treated as opaque names that no `BooleanVariable`
//!   operation references.
//!
//! # Idle / cold-default state
//!
//! When a canvas declares no `true` active-mode variable in
//! `staticVariables[]` (the common case — most canvases leave state
//! selection to the C++ runtime), the static export must still pick one
//! sub-canvas as the visible "switched-on but not interacted-with" state.
//!
//! Two structural patterns name the cold-default state variable.  Both
//! resolve to the same variable on canvases that use both (e.g. the wall
//! medbay canvas which has Header bound via Invert(Or) AND child canvases
//! bound via direct variables):
//!
//! 1. **Invert-of-Or framing-widget pattern**: a framing widget (Header /
//!    Footer / always-on sibling) gates its `Instantiated` on
//!    `Invert(EvaluateOr(state1, state2, …))` — visible only when no active
//!    state is selected.  The *first* input of the Or is the cold-default
//!    (Or convention: list states in idle-priority order).  A plain
//!    `Invert(SingleVariable)` is a single-flag hide gate, NOT this
//!    pattern, and never triggers an idle-default.
//!
//! 2. **Direct-variable scene-order pattern**: a sibling `WidgetCanvas`
//!    has `Instantiated = SingleVariable` (direct, no Or).  Scanning
//!    `operations[]` in order (which matches scene-child order), the
//!    *first* such state-group variable referenced is the cold-default.
//!    This handles canvases like `I_Med_MedicalBed_A` where every state
//!    sub-canvas has a direct variable and no framing widget uses the
//!    Invert(Or) pattern.
//!
//! In both patterns the candidate must belong to a *mutual-exclusion
//! group* — a set of `BindingsBooleanVariable` bindings sharing the same
//! dotted prefix (e.g. `Bed/state.BaseScreens.*`).  The cold-default is
//! applied only when no other group member has an explicit static-true
//! override.  Capability flags (`_SV` suffix) are excluded from group
//! membership.

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
    let mut static_vals = parse_static_variables(record_value);
    let ops = match record_value.get("operations").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return HashSet::new(),
    };

    apply_idle_defaults(ops, &mut static_vals);

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

/// Parse `staticVariables[]` into a map from variable name → bool.
///
/// The variable name is preserved verbatim.  In particular, the `"_SV"`
/// suffix (capability flag) is NOT stripped, so capability flags occupy
/// distinct keys from their matching active-mode bool and never alias.
/// Operations reference active-mode variables by their bare name; capability
/// flags are effectively unused by the state-selection evaluator.
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
        if !name.is_empty() {
            map.insert(name.to_owned(), val);
        }
    }
    map
}

/// Walk `operations[]` to discover the idle-gate state variable for each
/// mutual-exclusion group, and insert a default `true` for that variable
/// into `static_vals` when no other group member has an explicit override.
///
/// A *mutual-exclusion group* is the set of `BindingsBooleanVariable` ops
/// whose `binding` shares the same dotted-prefix (everything before the
/// last `.`).  In the medbay canvases this is the
/// `Bed/state.BaseScreens` / `Standing/state.BaseScreens` prefix; in MFD
/// canvases it is `eView_*` etc.
///
/// The idle-gate variable is the one whose **inverted** form gates the
/// `Instantiated` (or `IsActive`) field of some widget — typically a Header
/// or Footer that is hidden during the idle/attract state.
///
/// **Structural requirement**: the Invert's inner operand must be a
/// `BindingsBooleanEvaluateOr` over multiple variables (the active-state
/// disjunction).  The *first* input of the Or names the cold-default.
/// A plain `Invert(SingleVariable)` is a *different* pattern (single-flag
/// hide gate) and never triggers an idle-default.
fn apply_idle_defaults(
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
        ptr: BbNodeId,
        ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
        visited: &mut HashSet<BbNodeId>,
    ) -> Vec<String> {
        if !visited.insert(ptr) {
            return Vec::new();
        }
        let Some(op) = ptr_to_op.get(&ptr) else {
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
                        if let Some(p) = inp.as_str().and_then(parse_points_to_ptr) {
                            out.extend(resolve_vars(p, ptr_to_op, visited));
                        }
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
        if ty != "BuildingBlocks_BindingsBooleanVariable" {
            continue;
        }
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
    }
    // Retain only prefixes with ≥2 distinct member variables.
    group_index.retain(|_, v| v.len() >= 2);

    // `seen_prefix` ensures we record only the FIRST candidate per group.
    let mut seen_prefix: HashSet<String> = HashSet::new();
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();

    for op in ops {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "BuildingBlocks_BindingsBooleanField" {
            continue;
        }
        let field = op.get("field").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(field, "Instantiated" | "IsActive") {
            continue;
        }
        let Some(input_ptr) = op
            .get("input")
            .and_then(|v| v.as_str())
            .and_then(parse_points_to_ptr)
        else {
            continue;
        };
        let Some(input_op) = ptr_to_op.get(&input_ptr) else {
            continue;
        };
        let input_ty = input_op
            .get("_Type_")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Pattern detection: figure out the candidate variable name.
        let cold_default: Option<String> = match input_ty {
            // Pattern 1: Invert(EvaluateOr(Var1, Var2, ...)) → Var1 is default.
            // A plain Invert(SingleVariable) is a single-flag hide gate, not
            // an idle-default candidate; skipped here.
            "BuildingBlocks_BindingsBooleanInvert" => {
                let inner_ptr = input_op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_ptr);
                let inner_ty = inner_ptr
                    .and_then(|p| ptr_to_op.get(&p))
                    .and_then(|o| o.get("_Type_"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if inner_ty == "BuildingBlocks_BindingsBooleanEvaluateOr" {
                    let mut visited = HashSet::new();
                    let vars = resolve_vars(inner_ptr.unwrap(), &ptr_to_op, &mut visited);
                    vars.into_iter().next()
                } else {
                    None
                }
            }
            // Pattern 2: direct SingleVariable → that variable, if it belongs
            // to a recognized state-group.
            "BuildingBlocks_BindingsBooleanVariable" => input_op
                .get("binding")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty() && !s.ends_with("_SV"))
                .map(|s| s.to_owned()),
            _ => None,
        };

        let Some(name) = cold_default else { continue };
        let Some((prefix, _)) = name.rsplit_once('.') else {
            continue;
        };
        // Must be a recognized multi-member state group.
        let Some(group_members) = group_index.get(prefix) else {
            continue;
        };
        // Only the first candidate per group wins (operations order ≈ scene
        // order; the framing-widget Invert pattern is processed in the same
        // pass and naturally precedes state-child direct-variable patterns
        // when both are present).
        if !seen_prefix.insert(prefix.to_owned()) {
            continue;
        }
        candidates.push((name, group_members.clone()));
    }

    // For each candidate, apply only if no group member has an explicit
    // static override.  Group membership: variables that share the same
    // dotted prefix as `cold_default`.
    for (cold_default, group_members) in candidates {
        let prefix = match cold_default.rsplit_once('.') {
            Some((p, _)) => p,
            None => continue, // ungrouped variable; skip
        };
        // Group includes both the resolved `group_members` (same Or chain)
        // and any other static-var name sharing the prefix.
        let mut group: HashSet<String> = group_members.into_iter().collect();
        for key in static_vals.keys() {
            // Exclude `_SV` capability flags from group-override consideration.
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
        if !any_explicit_override {
            static_vals.entry(cold_default).or_insert(true);
        }
    }
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

    /// A WidgetCanvas whose Instantiated is bound to a variable with a direct
    /// `staticVariables` entry of `true` (no `_SV` suffix) must NOT appear in
    /// the false set.
    #[test]
    fn static_true_variable_widget_is_not_filtered() {
        let rv = make_record_value(
            vec![static_var("state.Admin", true)],
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

    /// A `_SV`-suffixed capability flag in `staticVariables` must NOT activate
    /// the same-named active-mode bool.  This is the wall-medbay case:
    /// `state.Admin_SV=true` is a capability flag, NOT an Admin-state activator.
    ///
    /// Scaffold: include an Invert(EvaluateOr) framing-widget pattern that
    /// names Attract as the cold-default (matches real wall medbay shape) so
    /// the direct-variable scene-order rule does NOT promote Admin.
    #[test]
    fn sv_capability_flag_does_not_activate_active_mode() {
        let rv = make_record_value(
            vec![static_var("state.Admin_SV", true)],
            vec![
                // Cold-default chain: Invert(Or(Attract, LogIn)) → Attract is
                // first Or operand → cold-default = Attract.
                variable_op(20, "state.Attract"),
                variable_op(30, "state.LogIn"),
                json!({
                    "_Pointer_": "ptr:23",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:20", "_PointsTo_:ptr:30"]
                }),
                json!({
                    "_Pointer_": "ptr:22",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:23"
                }),
                boolean_field_op(21, 22), // Header hidden when Or(Attract, LogIn) is true
                // Admin sub-canvas gated by direct variable; should remain
                // false (Attract is the cold-default for the group).
                variable_op(10, "state.Admin"),
                boolean_field_op(5, 10),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&5),
            "Admin canvas (ptr:5) must be filtered — _SV capability flag must NOT activate the matching active-mode bool"
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

    /// Plain `Invert(SingleVariable)` is NOT an idle-default pattern.  This
    /// is a single-flag hide gate: the framing widget is hidden only when
    /// the flag is explicitly true.  At cold-default the flag is false so
    /// the framing widget remains visible.
    ///
    /// This protects Header/Footer widgets bound via
    /// `Instantiated = NOT(SomeFlag)` from being incorrectly hidden when
    /// `SomeFlag` is the only thing being inverted.
    #[test]
    fn invert_single_variable_does_not_trigger_idle_default() {
        // ptr:3 = SomeFlag (no static value)
        // ptr:6 = NOT(ptr:3)
        // ptr:5 (Header WidgetCanvas) bound to ptr:6 → visible while flag is false
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.SomeFlag"),
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
            "Header (ptr:5) must NOT be filtered — Invert(SingleVariable) is not an idle-default pattern"
        );
    }

    /// Direct-variable scene-order rule: when sibling WidgetCanvases each
    /// bind `Instantiated` to a SingleVariable (no Invert/Or framing), and
    /// those variables share a dotted prefix forming a state-group, the
    /// FIRST such field op in operations order names the cold-default.
    ///
    /// This is the overhead-medbay (`I_Med_MedicalBed_A`) shape: Attract,
    /// MainMenu, Heal canvases each gated directly by their state variable
    /// with no framing-widget Invert(Or).
    #[test]
    fn direct_variable_scene_order_picks_first_as_cold_default() {
        // ptr:3 = Attract, ptr:4 = MainMenu, ptr:7 = Heal
        // ptr:5 (AttractCanvas) bound to ptr:3
        // ptr:6 (MainMenuCanvas) bound to ptr:4
        // ptr:8 (HealCanvas) bound to ptr:7
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.BaseScreens.Attract"),
                variable_op(4, "state.BaseScreens.MainMenu"),
                variable_op(7, "state.BaseScreens.Heal"),
                boolean_field_op(5, 3),
                boolean_field_op(6, 4),
                boolean_field_op(8, 7),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "AttractCanvas (ptr:5) shown — first direct-variable in group is cold-default");
        assert!(result.contains(&6), "MainMenuCanvas (ptr:6) hidden — not the cold-default");
        assert!(result.contains(&8), "HealCanvas (ptr:8) hidden — not the cold-default");
    }

    /// Direct-variable rule requires a group of ≥2 same-prefix variables.
    /// A single-member group (just `state.X` with no siblings) must NOT be
    /// elected — it's likely a single hide/show flag, not a state-machine.
    #[test]
    fn direct_variable_singleton_group_not_promoted() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.LonelyFlag"),
                boolean_field_op(5, 3),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&5),
            "Singleton-group canvas (ptr:5) must remain filtered — no other group members to make it a state-machine"
        );
    }


    /// `Or(Attract, LogIn)`, the FIRST operand (Attract) is the cold-default;
    /// LogIn defaults to false.  This matches the wall medbay shape.
    #[test]
    fn idle_gate_or_first_operand_is_cold_default() {
        // ptr:3 = Attract, ptr:4 = LogIn, ptr:19 = Or(3, 4), ptr:6 = NOT(19)
        // ptr:5 (Header) bound to ptr:6
        // ptr:11 (LogInCanvas) bound to ptr:4 → must remain hidden
        // ptr:8 (AttractCanvas) bound to ptr:3 → must be shown
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(4, "state.LogIn"),
                json!({
                    "_Pointer_": "ptr:19",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:3", "_PointsTo_:ptr:4"]
                }),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:19"
                }),
                boolean_field_op(5, 6),
                boolean_field_op(11, 4),
                boolean_field_op(8, 3),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Header (ptr:5) hidden under idle (Attract=true)");
        assert!(result.contains(&11), "LogInCanvas (ptr:11) hidden (LogIn=false)");
        assert!(!result.contains(&8), "AttractCanvas (ptr:8) shown (Attract=true cold default)");
    }

    /// Explicit static-true override of any group member SUPPRESSES the
    /// idle-default rule: the idle-gate variable stays false and the
    /// framing widget is shown.
    #[test]
    fn explicit_group_override_suppresses_idle_default() {
        // ptr:3 = Attract, ptr:7 = Admin, ptr:6 = NOT(3)
        // staticVariables[]: state.Admin=true (explicit override → suppresses
        // Attract idle-default)
        let rv = make_record_value(
            vec![static_var("state.Admin", true)],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(7, "state.Admin"),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
                boolean_field_op(8, 7),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "Header (ptr:5) shown — Admin override suppresses Attract idle-default");
        assert!(!result.contains(&8), "AdminCanvas (ptr:8) shown via explicit static-true");
    }

    /// Old test 6 kept as a separate scenario: when the inverted variable is
    /// NOT a member of an idle-gate group (no shared dotted prefix at all),
    /// idle-default does not kick in and `NOT(false) → true`.
    #[test]
    fn ungrouped_invert_does_not_trigger_idle_default() {
        // Variable binding is a single segment with no dotted prefix.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "Attract"), // no `.` → no group
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "ungrouped variable: NOT(false)=true, Header shown");
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
