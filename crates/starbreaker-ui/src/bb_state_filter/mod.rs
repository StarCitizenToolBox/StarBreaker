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

mod eval;
mod idle_defaults;
#[cfg(test)]
mod tests_a;
#[cfg(test)]
mod tests_b;

mod component_params;

use self::component_params::contains_unresolved_component_parameter;
use self::eval::{
    contains_non_boolean_runtime_binding,
    contains_namespace_placeholder_variable,
    contains_unset_non_state_variable,
    eval_bool_ref,
    evaluate_bool_ops,
    parse_points_to_ptr,
    parse_points_to_ptr_value,
    parse_ptr_id,
};
use self::idle_defaults::apply_idle_defaults;
#[cfg(test)]
use self::idle_defaults::{scene_widget_boolean_param_count, scene_widget_map};

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
        // Non-state runtime/sensor bindings (for example `/~/MapNamespace~/...`)
        // should not hide static export content when no explicit static default
        // is authored for them.
        if !val {
            let has_unresolved_component_param = contains_unresolved_component_parameter(
                input_ref,
                &ptr_to_op,
                &param_overrides,
                &mut HashSet::new(),
            );
            if field != "Instantiated" && has_unresolved_component_param {
                val = true;
            }
        }
        if !val {
            let has_unset_non_state = contains_unset_non_state_variable(
                input_ref,
                &ptr_to_op,
                &static_vals,
                &mut HashSet::new(),
            );
            if has_unset_non_state {
                if field == "Instantiated" && eval.is_some() {
                    if contains_namespace_placeholder_variable(
                        input_ref,
                        &ptr_to_op,
                        &mut HashSet::new(),
                    ) {
                        val = true;
                    }
                } else if field == "Instantiated" && eval.is_none() {
                    let has_placeholder = contains_namespace_placeholder_variable(
                        input_ref,
                        &ptr_to_op,
                        &mut HashSet::new(),
                    );
                    let has_non_boolean_runtime = contains_non_boolean_runtime_binding(
                        input_ref,
                        &ptr_to_op,
                        &mut HashSet::new(),
                    );
                    if has_placeholder || has_non_boolean_runtime {
                        val = true;
                    }
                } else {
                    val = true;
                }
            }
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

