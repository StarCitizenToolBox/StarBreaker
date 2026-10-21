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
//! Three structural patterns name the cold-default state variable(s):
//!
//! 0. **Sole-root pattern**: the canvas's only top-level `WidgetCanvas` is the
//!    whole screen (its content arrives via a `CanvasReferenceRecord` style
//!    modifier, not a followed `url`). With no sibling state-canvas, a false
//!    `Instantiated` gate would blank everything, so the root is exempted from
//!    the false-set — the static export renders its switched-on state. The
//!    HUD-component masters `HC_HUD_Ship_*_Master` (g-force/velocity ball,
//!    countermeasures, bars/nums) bind the root
//!    `Instantiated = Or(screen, FlightController/AccelerationBallEnabled)`.
//!    See `sole_top_level_widget_canvas`.
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
mod integer;
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
    resolve_op_ref,
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
    instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
        record_value,
        param_inputs,
        inherited_bindings,
        None,
    )
}

/// Like [`instantiated_false_widgets_with_param_inputs_and_inherited_bindings`]
/// but also consults the default-value registry for boolean variable bindings
/// that have no authored/inherited static value (runtime host data like the
/// MFD content view's `backgroundenabled`).
pub fn instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
    record_value: &serde_json::Value,
    param_inputs: &[serde_json::Value],
    inherited_bindings: &HashMap<String, bool>,
    defaults: Option<&crate::defaults::DefaultValueRegistry>,
) -> HashSet<BbNodeId> {
    let mut static_vals = parse_static_variables(record_value);
    for (binding, value) in inherited_bindings {
        static_vals.entry(binding.clone()).or_insert(*value);
    }
    if let Some(defaults) = defaults
        && let Some(ops) = record_value.get("operations").and_then(|v| v.as_array())
    {
        for op in ops {
            if op.get("_Type_").and_then(|v| v.as_str())
                != Some("BuildingBlocks_BindingsBooleanVariable")
            {
                continue;
            }
            let Some(binding) = op.get("binding").and_then(|v| v.as_str()) else {
                continue;
            };
            if static_vals.contains_key(binding) {
                continue;
            }
            if let Some(crate::canvas::Value::Bool(value)) = defaults.lookup_path(binding) {
                static_vals.insert(binding.to_owned(), *value);
            }
        }
    }
    let param_overrides = parse_boolean_param_inputs(param_inputs);
    let ops = match record_value.get("operations").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return HashSet::new(),
    };

    apply_idle_defaults(ops, &mut static_vals);
    let static_nums = numeric_variable_defaults(ops, defaults);

    let ptr_vals = evaluate_bool_ops(ops, &static_vals, &static_nums, &param_overrides);
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
    // Scene nodes by pointer — lets the mutually-exclusive-toggle check confirm a
    // gated widget is a sub-canvas variant (`WidgetCanvas` + `canvas` URL).
    // `scene_nodes` is the raw array (some interchangeable slots carry no
    // `_Pointer_`, so the tiling-sibling scan reads the array, not the map).
    let scene_nodes: &[serde_json::Value] = record_value
        .get("scene")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let mut scene_by_ptr: HashMap<BbNodeId, &serde_json::Value> = HashMap::new();
    if let Some(scene) = record_value.get("scene").and_then(|v| v.as_array()) {
        for item in scene {
            if let Some(ptr) = item
                .get("_Pointer_")
                .and_then(|v| v.as_str())
                .and_then(parse_ptr_id)
            {
                scene_by_ptr.insert(ptr, item);
            }
        }
    }
    // Variables that belong to a multi-member state GROUP (≥2 distinct bindings
    // sharing a `.`-prefix). `apply_idle_defaults` resolves these by picking ONE
    // branch, so they are NOT standalone `X`/`NOT X` composite toggles — the
    // mutually-exclusive-instantiation rule must skip them (see that helper).
    let grouped_state_vars: HashSet<String> = {
        let mut by_prefix: HashMap<&str, HashSet<&str>> = HashMap::new();
        for op in ops {
            if op.get("_Type_").and_then(|v| v.as_str())
                == Some("BuildingBlocks_BindingsBooleanVariable")
                && let Some(binding) = op.get("binding").and_then(|v| v.as_str())
                && let Some((prefix, _)) = binding.rsplit_once('.')
            {
                by_prefix.entry(prefix).or_default().insert(binding);
            }
        }
        by_prefix
            .into_iter()
            .filter(|(_, members)| members.len() >= 2)
            .flat_map(|(_, members)| members.into_iter().map(str::to_owned))
            .collect()
    };
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
            &static_nums,
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
                &static_nums,
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
        // Mutually-exclusive instantiation toggle, UNSET at static rest: two
        // sibling canvas variants gated `X` and `NOT X` on the same boolean that
        // has no value at rest. With no value we cannot pick a mode, so KEEP BOTH
        // instantiated — the engine selects one at runtime, but the static export
        // composites both authored variants. Without this, the direct (`X`) side
        // is deactivated (eval → false default) while the inverted (`NOT X`) side
        // is kept by the unset-override above — an asymmetry. Motivating case: the
        // cockpit radar's `HostplaneVisuals_Large` (`StarMapData/CommonData/IsFullScreen`)
        // and `HostplaneVisuals_Small` (`NOT IsFullScreen`), `IsFullScreen` unset.
        if !val
            && field == "Instantiated"
            && is_unset_mutually_exclusive_instantiation_toggle(
                widget,
                input_ref,
                ops,
                &ptr_to_op,
                &static_vals,
                &scene_by_ptr,
                &grouped_state_vars,
            )
        {
            val = true;
        }
        // Sub-full TILING canvas slots render as one co-displayed panel group
        // (the LR-indicator master's left/right half-width columns). A gated
        // column whose at-rest `Instantiated` resolves false is kept alongside
        // its always-on tiling sibling so the static export doesn't blank half
        // the display. Full-size overlay modes are NOT tiling slots, so their
        // exclusivity is unchanged.
        if !val
            && field == "Instantiated"
            && is_tiling_sibling_canvas_slot(widget, &scene_by_ptr, scene_nodes)
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
    // Sole-root exemption (third cold-default pattern). A canvas whose ONLY
    // top-level `WidgetCanvas` is the screen itself has no sibling state-canvas
    // to fall back to: deactivating it on a false `Instantiated` gate blanks the
    // entire render. That contradicts the static-export contract — the export
    // renders each screen's switched-on state (the same principle that drives the
    // `default_state_is_off` fallback-canvas selection in
    // `child_payload.rs`). The HUD-component masters `HC_HUD_Ship_*_Master`
    // (g-force ball, velocity ball, countermeasures, the bars/nums/alerts) bind
    // the root `Instantiated = Or(screen, FlightController/AccelerationBallEnabled)`
    // — flight-capability flags, both false at rest — and deliver content via a
    // `CanvasReferenceRecord` style modifier rather than a followed `url`. The
    // discriminator vs. multi-state canvases is structural: medical
    // (`I_Med_MedicalBed_A` / `…EndOfBed_A`) has TWO+ top-level WidgetCanvases and
    // gates its state sub-canvases as CHILDREN, so it never qualifies and its
    // Attract/MainMenu/HealMe cold-default selection is untouched.
    if let Some(root) = sole_top_level_widget_canvas(record_value) {
        false_set.remove(&root);
    }
    false_set
}

/// Resolve an `Instantiated`/`IsActive` input ref to a SINGLE boolean-variable
/// gate: `Some((binding, negated))` when the ref is a `BooleanVariable` (or
/// `Invert` of one). `None` for compound gates (Or/And/...), which are not
/// mutually-exclusive toggles.
fn single_variable_gate(
    input_ref: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
) -> Option<(String, bool)> {
    let op = resolve_op_ref(input_ref, ptr_to_op)?;
    match op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("") {
        "BuildingBlocks_BindingsBooleanVariable" => op
            .get("binding")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| (s.to_owned(), false)),
        "BuildingBlocks_BindingsBooleanInvert" => {
            let inner = resolve_op_ref(op.get("input")?, ptr_to_op)?;
            (inner.get("_Type_").and_then(|v| v.as_str())
                == Some("BuildingBlocks_BindingsBooleanVariable"))
            .then(|| {
                inner
                    .get("binding")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| (s.to_owned(), true))
            })
            .flatten()
        }
        _ => None,
    }
}

/// True when `ptr` is a sub-canvas VARIANT scene node — a
/// `BuildingBlocks_WidgetCanvas` with a non-empty `canvas` URL (it instantiates
/// another canvas record). This is the structural signature of a host-plane
/// composite (e.g. the radar's `HostplaneVisuals_Small`/`_Large`), distinct from
/// an in-scene widget toggle (a text field / container gated mutually-exclusively,
/// as on the medical/target MFD), which must keep its normal exclusivity.
fn is_subcanvas_variant(
    ptr: BbNodeId,
    scene_by_ptr: &HashMap<BbNodeId, &serde_json::Value>,
) -> bool {
    scene_by_ptr.get(&ptr).is_some_and(|node| {
        node.get("_Type_").and_then(|v| v.as_str()) == Some("BuildingBlocks_WidgetCanvas")
            && node
                .get("canvas")
                .and_then(|v| v.as_str())
                .is_some_and(|c| !c.is_empty())
    })
}

/// Horizontal span `[x0, x1]` (fractions of the parent) of a sub-full-WIDTH
/// WidgetCanvas slot, from its `anchor.x` + `width` (`Percent` behaviour with
/// `value < 1.0`); `None` for a full-width / fixed / non-`Percent` slot (which
/// is not a horizontal tile column). `x0 = anchor.x * (1 - width)` places the
/// slot against its anchored edge (left-anchored 0 → `[0, w]`, right-anchored
/// 1 → `[1-w, 1]`).
fn subfull_width_span(node: &serde_json::Value) -> Option<(f64, f64)> {
    if node.get("_Type_").and_then(|v| v.as_str()) != Some("BuildingBlocks_WidgetCanvas") {
        return None;
    }
    let width = node.get("sizing").and_then(|s| s.get("width"))?;
    if width.get("behavior").and_then(|b| b.as_str()) != Some("Percent") {
        return None;
    }
    let w = width.get("value").and_then(|x| x.as_f64())?;
    if !(w < 1.0 - 1e-3) {
        return None;
    }
    let anchor_x = node
        .get("anchor")
        .and_then(|a| a.get("x"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let x0 = anchor_x * (1.0 - w);
    Some((x0, x0 + w))
}

/// True when `widget` is a sub-full-WIDTH WidgetCanvas column that shares its
/// parent with another sub-full-width WidgetCanvas occupying a DISJOINT
/// horizontal span — i.e. genuine side-by-side TILING columns that render
/// together. Motivating case: the LR-indicator master
/// (`HC_HUD_Ship_LRInd_Master`) — "one large tall image" split into apparent
/// indicators by cockpit geometry — tiles a left column (`anchor.x = 0`,
/// `width = 0.5`, gated `Instantiated = (powerstate==1) AND seatdashboard/isactive`)
/// and a right column (`anchor.x = 1`, `width = 0.5`, ungated). When the left
/// column's at-rest gate resolves false the static export would blank half the
/// display, so a gated tiling column is kept instantiated alongside its sibling.
///
/// Scoped to DISJOINT HORIZONTAL columns by counterexample so the mutual
/// exclusivity of stacked / overlaid sub-canvases is untouched:
/// - the medical bed (`ui_target_a`) stacks mutually-exclusive FULL-WIDTH state
///   screens (MainMenu/HealMe/…) in a sub-full-HEIGHT content band — not
///   sub-full-WIDTH, so excluded;
/// - MC_S_Self_Master's view modes are full-size centred overlays; and
/// - the radar's host-plane `X` / `NOT X` variants OVERLAP (no disjoint span).
///
/// The sibling scan reads the raw scene array (not `scene_by_ptr`) because an
/// interchangeable slot may carry no `_Pointer_`. No node-name / ship / screen
/// gating.
fn is_tiling_sibling_canvas_slot(
    widget: BbNodeId,
    scene_by_ptr: &HashMap<BbNodeId, &serde_json::Value>,
    scene: &[serde_json::Value],
) -> bool {
    let Some(node) = scene_by_ptr.get(&widget) else {
        return false;
    };
    let Some((x0, x1)) = subfull_width_span(node) else {
        return false;
    };
    let parent = node.get("parent");
    let name = node.get("name").and_then(|v| v.as_str());
    scene.iter().any(|other| {
        if other.get("name").and_then(|v| v.as_str()) == name || other.get("parent") != parent {
            return false;
        }
        match subfull_width_span(other) {
            // Disjoint horizontal spans (touching allowed) → genuine tiling.
            Some((ox0, ox1)) => x1 <= ox0 + 1e-3 || ox1 <= x0 + 1e-3,
            None => false,
        }
    })
}

/// True when `input_ref` gates `Instantiated` on a single boolean variable that
/// is UNSET at static rest (no static / idle-default / registry value), AND a
/// DIFFERENT widget gates its `Instantiated` on the COMPLEMENT of the same
/// variable — i.e. two mutually-exclusive SUB-CANVAS variants (`X` / `NOT X`)
/// whose selector has no value at rest. The caller keeps BOTH instantiated (the
/// engine picks one at runtime; the static export composites both authored
/// variants). Scoped structurally to sub-canvas variants (both sides are
/// `WidgetCanvas` with a `canvas` URL — see [`is_subcanvas_variant`]) so in-scene
/// widget toggles (medical/target MFD) keep their normal exclusivity. No
/// node-name / material gating.
fn is_unset_mutually_exclusive_instantiation_toggle(
    widget: BbNodeId,
    input_ref: &serde_json::Value,
    ops: &[serde_json::Value],
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    static_vals: &HashMap<String, bool>,
    scene_by_ptr: &HashMap<BbNodeId, &serde_json::Value>,
    grouped_state_vars: &HashSet<String>,
) -> bool {
    let Some((var, negated)) = single_variable_gate(input_ref, ptr_to_op) else {
        return false;
    };
    // A resolved value means the engine CAN pick a mode → honour normal gating.
    if static_vals.contains_key(&var) {
        return false;
    }
    // A member of a multi-variable state GROUP (`apply_idle_defaults` picks ONE
    // branch for these — e.g. the medical bed's `Bed/state.BaseScreens.{Attract,
    // MainMenu,Heal,…}`) is NOT a standalone toggle: its siblings being unset is
    // the cold-default mechanism, not a both-modes composite. Only a standalone
    // `X` / `NOT X` toggle (the radar's `IsFullScreen`, no `.`-grouped siblings)
    // composes both.
    if grouped_state_vars.contains(&var) {
        return false;
    }
    if !is_subcanvas_variant(widget, scene_by_ptr) {
        return false;
    }
    ops.iter().any(|op| {
        op.get("_Type_").and_then(|v| v.as_str()) == Some("BuildingBlocks_BindingsBooleanField")
            && op.get("field").and_then(|v| v.as_str()) == Some("Instantiated")
            && parse_points_to_ptr_value(op.get("widget"))
                .is_some_and(|w| w != widget && is_subcanvas_variant(w, scene_by_ptr))
            && op
                .get("input")
                .and_then(|inp| single_variable_gate(inp, ptr_to_op))
                .is_some_and(|(ovar, oneg)| ovar == var && oneg != negated)
    })
}

/// Nodes whose `IsActive` field binding evaluates to a GENUINE `Some(true)` (a
/// resolved/pinned state value) — the renderer ACTIVATES these even when the node
/// is authored `isActive=false`, because the engine treats the IsActive binding
/// as the runtime truth.
///
/// Motivating case: the cockpit-radar background `image_Background`
/// (`DRAK_GroundVehicle_Dashboard_background_2`) is authored `isActive=false` with
/// `IsActive ← NOT(IsVolumetric)`; at the flat radar (IsVolumetric pinned false)
/// it resolves `Some(true)` and must render — but the deactivation-only filter
/// above never activates an authored-false node.
///
/// SAFETY: requires a GENUINE `Some(true)` — the raw `eval_bool_ref` result, NOT
/// the `contains_unset_non_state_variable` override that drives unknown bindings
/// true. Medical's live-`IsActive`-gated nodes are unset at rest (`eval` = `None`,
/// not `Some(true)`), so they are never spuriously activated (the workflow §10
/// "generic IsActive pass breaks medical" hazard). Scoped to the `IsActive` field.
pub fn forced_active_widgets_with_defaults(
    record_value: &serde_json::Value,
    param_inputs: &[serde_json::Value],
    inherited_bindings: &HashMap<String, bool>,
    defaults: Option<&crate::defaults::DefaultValueRegistry>,
) -> HashSet<BbNodeId> {
    let mut static_vals = parse_static_variables(record_value);
    for (binding, value) in inherited_bindings {
        static_vals.entry(binding.clone()).or_insert(*value);
    }
    if let Some(defaults) = defaults
        && let Some(ops) = record_value.get("operations").and_then(|v| v.as_array())
    {
        for op in ops {
            if op.get("_Type_").and_then(|v| v.as_str())
                != Some("BuildingBlocks_BindingsBooleanVariable")
            {
                continue;
            }
            let Some(binding) = op.get("binding").and_then(|v| v.as_str()) else {
                continue;
            };
            if static_vals.contains_key(binding) {
                continue;
            }
            if let Some(crate::canvas::Value::Bool(value)) = defaults.lookup_path(binding) {
                static_vals.insert(binding.to_owned(), *value);
            }
        }
    }
    let param_overrides = parse_boolean_param_inputs(param_inputs);
    let ops = match record_value.get("operations").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return HashSet::new(),
    };
    apply_idle_defaults(ops, &mut static_vals);
    let static_nums = numeric_variable_defaults(ops, defaults);
    let ptr_vals = evaluate_bool_ops(ops, &static_vals, &static_nums, &param_overrides);
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
    // Map each scene node ptr → its widget _Type_, so activation can be scoped
    // to `WidgetImage` nodes only (see below).
    let mut ptr_to_type: HashMap<BbNodeId, &str> = HashMap::new();
    if let Some(scene) = record_value.get("scene").and_then(|v| v.as_array()) {
        for item in scene {
            if let (Some(ptr), Some(ty)) = (
                item.get("_Pointer_").and_then(|v| v.as_str()).and_then(parse_ptr_id),
                item.get("_Type_").and_then(|v| v.as_str()),
            ) {
                ptr_to_type.insert(ptr, ty);
            }
        }
    }
    let mut active_set: HashSet<BbNodeId> = HashSet::new();
    for op in ops {
        if op.get("_Type_").and_then(|v| v.as_str()) != Some("BuildingBlocks_BindingsBooleanField") {
            continue;
        }
        if op.get("field").and_then(|v| v.as_str()) != Some("IsActive") {
            continue;
        }
        let Some(widget) = parse_points_to_ptr_value(op.get("widget")) else {
            continue;
        };
        let Some(input_ref) = op.get("input") else {
            continue;
        };
        // SCOPE: the radar background `WidgetImage` (unconditional — its IsActive is
        // `NOT(IsVolumetric)`), PLUS a cockpit flight-mode `DisplayWidget` indicator
        // whose IsActive is grounded in the ship's FLIGHT-CONTROLLER state (the
        // LR-indicator CPLD/ESP/LOCK bind `flightcontroller.isdecoupled`/`isesp`/
        // `isrotationlocked`, genuinely lit/unlit at the at-rest power-on state).
        // The medical bed (`ui_target_a`) has authored-false `DisplayWidget`s gated
        // on `Bed/state.*` that ALSO resolve genuine-true at rest but must stay
        // inactive (the gold baseline) — excluded because they are NOT
        // flight-controller-grounded (the workflow §10 hazard the WidgetImage scope
        // guarded; broadening to ALL DisplayWidgets regresses ui_target_a).
        let allow = match ptr_to_type.get(&widget).copied() {
            Some("BuildingBlocks_WidgetImage") => true,
            Some("BuildingBlocks_DisplayWidget") => {
                let mut seen = HashSet::new();
                isactive_grounded_in_flight_controller(input_ref, &ptr_to_op, &mut seen)
            }
            _ => false,
        };
        if !allow {
            continue;
        }
        let mut visiting = HashSet::new();
        let eval = eval_bool_ref(
            input_ref,
            &ptr_vals,
            &ptr_to_op,
            &static_vals,
            &static_nums,
            &param_overrides,
            &mut visiting,
        );
        if eval == Some(true) {
            active_set.insert(widget);
        }
    }
    active_set
}

/// True when an `IsActive` input op-graph is grounded — through the boolean
/// combinators (`Invert`/`EvaluateAnd`/`EvaluateOr` and the integer/number
/// comparisons) — in a `flightcontroller.*` boolean state variable. This is the
/// discriminator that lets `forced_active_widgets_with_defaults` activate a
/// genuine-true authored-false `DisplayWidget` ONLY when it is a cockpit
/// flight-mode indicator (the LR-indicator CPLD/ESP/LOCK), while leaving a
/// medical-bed `DisplayWidget` (grounded in `Bed/state.*`) inactive. Follows
/// `_PointsTo_` pointer refs via `ptr_to_op`; `seen` guards against cycles.
fn isactive_grounded_in_flight_controller(
    input: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    seen: &mut HashSet<BbNodeId>,
) -> bool {
    match input {
        serde_json::Value::String(s) => {
            let Some(ptr) = parse_points_to_ptr(s) else {
                return false;
            };
            if !seen.insert(ptr) {
                return false;
            }
            let Some(op) = ptr_to_op.get(&ptr) else {
                return false;
            };
            isactive_grounded_in_flight_controller(op, ptr_to_op, seen)
        }
        serde_json::Value::Object(obj) => {
            if obj
                .get("binding")
                .and_then(|v| v.as_str())
                .is_some_and(|b| b.to_ascii_lowercase().starts_with("flightcontroller"))
            {
                return true;
            }
            // Recurse every nested operand ref (combinators carry `input`,
            // `inputs[]`, and `inputL`/`inputR` for the comparison ops).
            for value in obj.values() {
                match value {
                    serde_json::Value::String(_) | serde_json::Value::Object(_) => {
                        if isactive_grounded_in_flight_controller(value, ptr_to_op, seen) {
                            return true;
                        }
                    }
                    serde_json::Value::Array(arr) => {
                        if arr
                            .iter()
                            .any(|item| isactive_grounded_in_flight_controller(item, ptr_to_op, seen))
                        {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        }
        _ => false,
    }
}

/// Return the node id of the canvas's *sole* top-level `WidgetCanvas` — the one
/// scene node (with no `parent` pointer) of type `WidgetCanvas` — or `None` when
/// there are zero or more than one. A single top-level WidgetCanvas is the whole
/// screen's content container with no sibling state alternative; the caller
/// exempts it from `Instantiated`-gate deactivation so the static export renders
/// its switched-on state instead of a blank. Multiple top-level WidgetCanvases
/// indicate a mutual-exclusion state set (medical) where deactivation is correct.
fn sole_top_level_widget_canvas(record_value: &serde_json::Value) -> Option<BbNodeId> {
    let scene = record_value.get("scene").and_then(|v| v.as_array())?;
    let mut sole_ptr: Option<BbNodeId> = None;
    let mut count = 0usize;
    for node in scene {
        if node.get("_Type_").and_then(|v| v.as_str()) != Some("BuildingBlocks_WidgetCanvas") {
            continue;
        }
        // A node is top-level when it carries no `parent` pointer.
        let has_parent = node
            .get("parent")
            .and_then(|v| v.as_str())
            .and_then(parse_points_to_ptr)
            .is_some();
        if has_parent {
            continue;
        }
        count += 1;
        if count > 1 {
            return None;
        }
        sole_ptr = node
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(parse_ptr_id);
    }
    sole_ptr
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
    // No registry here, so runtime numeric variables stay unresolved (the
    // heuristic path) — this entry point only reports boolean-variable values.
    let static_nums: HashMap<String, f64> = HashMap::new();
    let ptr_vals = evaluate_bool_ops(ops, &static_vals, &static_nums, &param_overrides);

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

/// Resolve the at-rest cold defaults for the COMPONENT-LOCAL runtime numeric
/// variables (`IntegerVariable` / `NumberVariable`, bare binding = `path []` +
/// `inheritsNamespace`) referenced by the canvas operations, keyed by binding
/// name. Mirrors the boolean `static_vals` defaults consultation above: it lets
/// `BooleanFromInteger` / `BooleanFromNumber` gates resolve statically when the
/// engine pushes a known at-rest value. The motivating case is the cockpit
/// countermeasure firing overlay, gated `IsActive = Or(CurrentBurstSize > 1,
/// BurstSizeHoldRatio > 0)` — both are 0 when not firing (registered as
/// well-known cold defaults), so the gate resolves `false` and the stray "0"
/// hides.
///
/// SCOPE: bare bindings only (no `/`). A bare binding is a component-relative
/// runtime variable resolved against the instance namespace; resolving its
/// at-rest cold default is local and safe. ABSOLUTE engine-state paths (slash-
/// prefixed — `/seatdashboard/powerstate`, `/AnnunciatorProvider/.../Severity`,
/// …) are deliberately EXCLUDED: many frozen-screen gates reference them and
/// their established at-rest visibility is encoded by the unset→override /
/// heuristic path (the gold baselines were calibrated against it). Newly
/// resolving them in the state filter would flip those gates (it regressed the
/// `clipper_self_master` gold baseline). Today only the firing-state vars are
/// bare numeric registry keys, so this is byte-identical for every frozen
/// screen. A variable with no registry entry is omitted (stays on the
/// heuristic).
fn numeric_variable_defaults(
    ops: &[serde_json::Value],
    defaults: Option<&crate::defaults::DefaultValueRegistry>,
) -> HashMap<String, f64> {
    let mut out: HashMap<String, f64> = HashMap::new();
    let Some(defaults) = defaults else {
        return out;
    };
    for op in ops {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "BuildingBlocks_BindingsIntegerVariable"
            && ty != "BuildingBlocks_BindingsNumberVariable"
        {
            continue;
        }
        let Some(binding) = op.get("binding").and_then(|v| v.as_str()) else {
            continue;
        };
        // Component-local bare bindings only — see SCOPE in the doc comment.
        if binding.is_empty() || binding.contains('/') || out.contains_key(binding) {
            continue;
        }
        match defaults.lookup_path(binding) {
            Some(crate::canvas::Value::Int(v)) => {
                out.insert(binding.to_owned(), *v as f64);
            }
            Some(crate::canvas::Value::Float(v)) => {
                out.insert(binding.to_owned(), *v);
            }
            _ => {}
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

