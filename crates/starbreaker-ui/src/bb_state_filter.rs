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
//! Two structural patterns name the cold-default state variable(s):
//!
//! 1. **Invert-of-Or framing-widget pattern**: a framing widget (Header /
//!    Footer / always-on sibling) gates its `Instantiated` on
//!    `Invert(EvaluateOr(state1, state2, …))` — visible only when no active
//!    state in that hidden-set is selected.  When Or operands also appear as
//!    directly-gated sibling canvases, the first Or input remains the selected
//!    idle overlay, but the framing widget itself is kept visible so authored
//!    chrome can coexist with that overlay.  A plain `Invert(SingleVariable)`
//!    is a single-flag hide gate, NOT this pattern, and never triggers an
//!    idle-default.
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
    instantiated_false_widgets_with_param_inputs(record_value, &[])
}

/// Like [`instantiated_false_widgets`] but applies boolean component parameter
/// overrides from parent `paramInputValues`.
pub fn instantiated_false_widgets_with_param_inputs(
    record_value: &serde_json::Value,
    param_inputs: &[serde_json::Value],
) -> HashSet<BbNodeId> {
    instantiated_false_widgets_with_param_inputs_and_inherited_bindings(
        record_value,
        param_inputs,
        &HashMap::new(),
    )
}

/// Like [`instantiated_false_widgets_with_param_inputs`] but also accepts
/// inherited boolean binding values from parent canvases.
pub fn instantiated_false_widgets_with_param_inputs_and_inherited_bindings(
    record_value: &serde_json::Value,
    param_inputs: &[serde_json::Value],
    inherited_bindings: &HashMap<String, bool>,
) -> HashSet<BbNodeId> {
    let mut static_vals = parse_static_variables(record_value);
    for (binding, value) in inherited_bindings {
        static_vals.entry(binding.clone()).or_insert(*value);
    }
    let param_overrides = parse_boolean_param_inputs(param_inputs);
    let ops = match record_value.get("operations").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return HashSet::new(),
    };

    apply_idle_defaults(ops, &mut static_vals);

    let ptr_vals = evaluate_bool_ops(ops, &static_vals, &param_overrides);
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
    let state_probe = std::env::var("BB_STATE_PROBE").as_deref() == Ok("1");
    let mut ptr_to_name: HashMap<BbNodeId, String> = HashMap::new();
    if state_probe {
        if let Some(scene) = record_value.get("scene").and_then(|v| v.as_array()) {
            for item in scene {
                if let Some(ptr) = item
                    .get("_Pointer_")
                    .and_then(|v| v.as_str())
                    .and_then(parse_ptr_id)
                {
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    ptr_to_name.insert(ptr, name);
                }
            }
        }
    }

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
        let Some(widget) = parse_points_to_ptr_value(op.get("widget")) else {
            continue;
        };
        let Some(input_ref) = op.get("input") else {
            continue;
        };
        let mut visiting = HashSet::new();
        let eval = eval_bool_ref(
            input_ref,
            &ptr_vals,
            &ptr_to_op,
            &static_vals,
            &param_overrides,
            &mut visiting,
        );
        // Unknown expressions default differently by field:
        // - Instantiated: conservative false to avoid merging unknown state canvases.
        // - IsActive/Visible/Enabled: conservative true to avoid hiding runtime-gated UI cards.
        let mut val = eval.unwrap_or(field != "Instantiated");
        if !val
            && field != "Instantiated"
            && contains_unset_non_state_variable(input_ref, &ptr_to_op, &static_vals, &mut HashSet::new())
        {
            val = true;
        }
        if state_probe {
            let widget_name = ptr_to_name.get(&widget).cloned().unwrap_or_default();
            let input_ty = input_ref
                .as_object()
                .and_then(|o| o.get("_Type_"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    input_ref
                        .as_str()
                        .and_then(parse_points_to_ptr)
                        .and_then(|p| ptr_to_op.get(&p))
                        .and_then(|op| op.get("_Type_"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("<unknown>");
            let input_op_dump = input_ref
                .as_str()
                .and_then(parse_points_to_ptr)
                .and_then(|p| ptr_to_op.get(&p))
                .map(|op| op.to_string())
                .or_else(|| input_ref.as_object().map(|o| serde_json::Value::Object(o.clone()).to_string()))
                .unwrap_or_default();
            log::info!(
                "bb_state_probe: widget=ptr:{widget} name={widget_name:?} field={field} input_ty={input_ty} eval={eval:?} final={val} op={input_op_dump}"
            );
        }
        if !val {
            false_set.insert(widget);
        }
    }
    false_set
}

/// Resolve all boolean variable bindings available in this canvas under the
/// same static-default semantics used by state filtering.
pub fn resolved_boolean_variable_bindings_with_param_inputs_and_inherited(
    record_value: &serde_json::Value,
    param_inputs: &[serde_json::Value],
    inherited_bindings: &HashMap<String, bool>,
) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    let mut static_vals = parse_static_variables(record_value);
    for (binding, value) in inherited_bindings {
        static_vals.entry(binding.clone()).or_insert(*value);
    }

    let param_overrides = parse_boolean_param_inputs(param_inputs);
    let Some(ops) = record_value.get("operations").and_then(|v| v.as_array()) else {
        return out;
    };

    apply_idle_defaults(ops, &mut static_vals);
    let ptr_vals = evaluate_bool_ops(ops, &static_vals, &param_overrides);

    for op in ops {
        if op.get("_Type_").and_then(|v| v.as_str())
            != Some("BuildingBlocks_BindingsBooleanVariable")
        {
            continue;
        }
        let Some(ptr) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(parse_ptr_id)
        else {
            continue;
        };
        let Some(binding) = op.get("binding").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(value) = ptr_vals.get(&ptr).copied() {
            out.entry(binding.to_owned()).or_insert(value);
        }
    }

    out
}

fn parse_boolean_param_inputs(param_inputs: &[serde_json::Value]) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    for entry in param_inputs {
        let ty = entry.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if !ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputBoolean") {
            continue;
        }
        let Some(param) = entry.get("parameter").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(value) = entry.get("value").and_then(|v| v.as_bool()) else {
            continue;
        };
        if !param.is_empty() {
            out.insert(param.to_ascii_lowercase(), value);
        }
    }
    out
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
/// The idle-gate variable set is the one whose **inverted** form gates the
/// `Instantiated` (or `IsActive`) field of some widget — typically a Header
/// or Footer that is hidden during selected startup overlays.
///
/// **Structural requirement**: the Invert's inner operand must be a
/// `BindingsBooleanEvaluateOr` over multiple variables (the active-state
/// hidden-set).  The first input of the Or names the cold-default. When the
/// same Or operands also directly gate sibling canvases, the framing widget's
/// own false result is ignored so chrome remains visible with the selected
/// overlay. A plain `Invert(SingleVariable)` is a *different* pattern
/// (single-flag hide gate) and never triggers an idle-default.
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
fn scene_widget_map(record_value: &serde_json::Value) -> HashMap<BbNodeId, &serde_json::Value> {
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
fn scene_widget_boolean_param_count(
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

/// Evaluate the boolean operation graph, returning a map from pointer ID to
/// the statically-resolved bool value.
///
/// Only operations with a `_Pointer_` field contribute to the map.  Operations
/// whose inputs are not yet resolved are retried in subsequent iterations until
/// no further progress is made (fixpoint).
fn evaluate_bool_ops(
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

fn parse_points_to_ptr_value(v: Option<&serde_json::Value>) -> Option<BbNodeId> {
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

fn resolve_op_ref<'a>(
    input: &'a serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &'a serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    match input {
        serde_json::Value::String(s) => parse_points_to_ptr(s).and_then(|p| ptr_to_op.get(&p).copied()),
        serde_json::Value::Object(_) => Some(input),
        _ => None,
    }
}

fn resolve_op_ref_with_visited<'a>(
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

fn eval_bool_ref(
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

fn parse_ptr_id(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

fn parse_points_to_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:ptr:").and_then(|n| n.parse().ok())
}

fn contains_unset_non_state_variable(
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
            match ty {
                "BuildingBlocks_BindingsBooleanVariable" => {
                    let binding = obj.get("binding").and_then(|v| v.as_str()).unwrap_or("");
                    !binding.is_empty() && !static_vals.contains_key(binding) && !is_state_binding(binding)
                }
                "BuildingBlocks_BindingsBooleanInvert" => obj
                    .get("input")
                    .is_some_and(|inner| contains_unset_non_state_variable(inner, ptr_to_op, static_vals, visited)),
                "BuildingBlocks_BindingsBooleanEvaluateOr" | "BuildingBlocks_BindingsBooleanEvaluateAnd" => {
                    obj.get("inputs")
                        .and_then(|v| v.as_array())
                        .is_some_and(|inputs| {
                            inputs.iter().any(|inp| {
                                contains_unset_non_state_variable(inp, ptr_to_op, static_vals, visited)
                            })
                        })
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn is_state_binding(binding: &str) -> bool {
    let lower = binding.to_ascii_lowercase();
    lower.starts_with("state.") || lower.contains("/state.") || lower.contains(".state.")
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

    fn scene_widget(ptr: u32, params: Vec<serde_json::Value>) -> serde_json::Value {
        json!({
            "_Pointer_": format!("ptr:{ptr}"),
            "_Type_": "BuildingBlocks_WidgetCanvas",
            "paramInputValues": params,
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

    /// Boolean parameter slots should be detected anywhere under the candidate
    /// widget subtree, not just on the candidate node itself.
    #[test]
    fn scene_widget_boolean_param_count_tracks_descendants() {
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "scene": [
                scene_widget(9, vec![]),
                json!({
                    "_Pointer_": "ptr:10",
                    "_Type_": "BuildingBlocks_WidgetCanvas",
                    "parent": "_PointsTo_:ptr:9",
                    "paramInputValues": [
                        json!({
                            "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
                            "parameter": "ParamInput0",
                            "value": false
                        })
                    ]
                }),
            ]
        });
        let items = scene_widget_map(&rv);
        assert_eq!(scene_widget_boolean_param_count(&items, 9), 1);
    }

    #[test]
    fn scene_widget_without_boolean_param_counts_zero() {
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "scene": [scene_widget(9, vec![])]
        });
        let items = scene_widget_map(&rv);
        assert_eq!(scene_widget_boolean_param_count(&items, 9), 0);
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

    #[test]
    fn inline_widget_and_input_refs_are_evaluated() {
        let rv = make_record_value(
            vec![],
            vec![json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "field": "Instantiated",
                "widget": {"_Type_":"BuildingBlocks_WidgetCanvas","_Pointer_":"ptr:3"},
                "input": {"_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"state.Attract"}
            })],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&3),
            "inline object refs should evaluate like pointer refs"
        );
    }

    #[test]
    fn bed_mainmenu_default_applies_with_inline_boolean_variable_inputs() {
        let rv = make_record_value(
            vec![],
            vec![
                json!({
                    "_Type_":"BuildingBlocks_BindingsBooleanField",
                    "field":"Instantiated",
                    "widget":{"_Type_":"BuildingBlocks_WidgetCanvas","_Pointer_":"ptr:5"},
                    "input":{"_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"Bed/state.BaseScreens.Attract"}
                }),
                json!({
                    "_Type_":"BuildingBlocks_BindingsBooleanField",
                    "field":"Instantiated",
                    "widget":{"_Type_":"BuildingBlocks_WidgetCanvas","_Pointer_":"ptr:6"},
                    "input":{"_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"Bed/state.BaseScreens.MainMenu"}
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Attract should be hidden for bed cold-default");
        assert!(!result.contains(&6), "MainMenu should be shown for bed cold-default");
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

    /// Bed base-screen canvases should prefer MainMenu as cold-default when no
    /// explicit static override exists.
    #[test]
    fn bed_base_screens_prefers_mainmenu() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "Bed/state.BaseScreens.Attract"),
                variable_op(4, "Bed/state.BaseScreens.MainMenu"),
                variable_op(7, "Bed/state.BaseScreens.Heal"),
                boolean_field_op(5, 3), // AttractCanvas
                boolean_field_op(6, 4), // MainMenuCanvas
                boolean_field_op(8, 7), // HealCanvas
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "AttractCanvas (ptr:5) hidden for Bed default");
        assert!(
            !result.contains(&6),
            "MainMenuCanvas (ptr:6) shown as Bed cold-default"
        );
        assert!(result.contains(&8), "HealCanvas (ptr:8) hidden");
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


    /// If an `Invert(Or(...))` gate names directly-gated sibling canvases,
    /// the first Or operand remains the idle overlay and the framing canvas
    /// follows its evaluated `Instantiated` value (hidden when false).
    #[test]
    fn idle_gate_or_filters_framing_canvas_when_false() {
        // Mirrors the wall medbay shape:
        // ptr:3 = Attract, ptr:4 = LogIn, ptr:19 = Or(3, 4), ptr:6 = NOT(19)
        // ptr:5 (Header) bound to ptr:6 → hidden when Attract is cold-default
        // ptr:11 (LogInCanvas) bound to ptr:4 → hidden
        // ptr:8 (AttractCanvas) bound to ptr:3 → shown as first Or operand
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
                // Direct-field order in the real wall canvas is LogIn, then Attract.
                boolean_field_op(11, 4),
                boolean_field_op(8, 3),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Header (ptr:5) hidden when Invert(Or(...)) evaluates false");
        assert!(result.contains(&11), "LogInCanvas (ptr:11) hidden (LogIn=false)");
        assert!(!result.contains(&8), "AttractCanvas (ptr:8) shown (first Or operand)");
    }

    /// `Invert(Or(...))` still falls back to the first Or operand when the Or
    /// operands do not have directly-gated sibling canvases.
    #[test]
    fn idle_gate_or_without_direct_siblings_uses_first_operand() {
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
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Header (ptr:5) hidden under first-operand idle default");
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

    #[test]
    fn boolean_component_parameter_param_input_override_controls_visibility() {
        let rv = make_record_value(
            vec![],
            vec![serde_json::json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:5",
                "field": "IsActive",
                "input": {
                    "_Pointer_": "ptr:9",
                    "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                    "parameter": "ParamInput0",
                    "defaultValue": false
                }
            })],
        );

        let no_override = instantiated_false_widgets_with_param_inputs(&rv, &[]);
        assert!(
            no_override.contains(&5),
            "without paramInput override, defaultValue=false should hide ptr:5"
        );

        let with_override = instantiated_false_widgets_with_param_inputs(
            &rv,
            &[serde_json::json!({
                "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
                "parameter": "ParamInput0",
                "value": true
            })],
        );
        assert!(
            !with_override.contains(&5),
            "paramInput override true should show ptr:5"
        );
    }

    #[test]
    fn non_state_variables_without_static_values_do_not_hide_widgets() {
        let rv = make_record_value(
            vec![],
            vec![
                json!({
                    "_Pointer_": "ptr:12",
                    "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                    "binding": "CloneLocationInfo/UserOwnsLocation"
                }),
                json!({
                    "_Pointer_": "ptr:13",
                    "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                    "binding": "Bed/MedBed/MedBedStatus/CanRespawnHere"
                }),
                json!({
                    "_Pointer_": "ptr:8",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateAnd",
                    "inputs": ["_PointsTo_:ptr:12", "_PointsTo_:ptr:13"]
                }),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:7",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:8"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&7),
            "non-state sensor variables without static defaults should not hide ptr:7"
        );
    }
}
