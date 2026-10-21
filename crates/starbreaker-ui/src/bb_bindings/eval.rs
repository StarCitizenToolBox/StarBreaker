use crate::bb_scene::BbNodeId;
use crate::canvas::Value;
use crate::defaults::DefaultValueRegistry;
use super::util::{parse_points_to_or_ptr_str, value_to_string};
use super::BindingResolver;

impl BindingResolver {
    pub(super) fn eval_localized_ptr(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<String> {
        if !seen.insert(ptr) {
            return None;
        }
        let op = self.ptr_to_op.get(&ptr)?;
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "BindingsOperations_LocalizationCombine" => {
                let value_key = op.get("value").and_then(|v| v.as_str()).unwrap_or("");
                let base = defaults.lookup_localization(value_key).unwrap_or(value_key);
                let left_ptr = op
                    .get("inputL")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str);
                let right_ptr = op
                    .get("inputR")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str);
                // The integer fallback is a separate evaluation domain, so it runs
                // with a fresh `seen`: the failed `eval_localized_ptr` attempt above
                // leaves the input ptr on the localized path, which would otherwise
                // make the integer cycle-guard reject this legitimate re-resolution
                // (the input is shared, not cyclic).
                let left = left_ptr
                    .and_then(|p| self.eval_localized_ptr(p, defaults, seen))
                    .or_else(|| left_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, &mut std::collections::HashSet::new())).map(|v| v.to_string()))
                    .unwrap_or_default();
                let right = right_ptr
                    .and_then(|p| self.eval_localized_ptr(p, defaults, seen))
                    .or_else(|| right_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, &mut std::collections::HashSet::new())).map(|v| v.to_string()))
                    .unwrap_or_default();
                // `withSpace` joins the parts with single spaces (the OUTPUT
                // card's "2 / 16": the total combine is `"/" + 16` withSpace).
                let sep = if op.get("withSpace").and_then(|v| v.as_bool()).unwrap_or(false) {
                    " "
                } else {
                    ""
                };
                let mut out = base.to_string();
                if out.contains("%d") {
                    out = out.replacen("%d", if !right.is_empty() { &right } else { &left }, 1);
                } else if out.contains("%s") {
                    out = out.replacen("%s", if !right.is_empty() { &right } else { &left }, 1);
                } else if left.is_empty() && right.is_empty() {
                    // keep base
                } else if left.is_empty() {
                    out = format!("{out}{sep}{right}");
                } else if right.is_empty() {
                    out = format!("{left}{sep}{out}");
                } else {
                    out = format!("{left}{sep}{out}{sep}{right}");
                }
                Some(out)
            }
            "BuildingBlocks_BindingsLocalizedFromInteger" => self
                .eval_integer_ptr_from_field(op.get("input").and_then(|v| v.as_str()), defaults, seen)
                .map(|v| v.to_string()),
            "BuildingBlocks_BindingsLocalizedFromNumber" => {
                let mut seen_num = std::collections::HashSet::new();
                let value = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, &mut seen_num))?;
                let n_places = op.get("nPlaces").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let trailing = op.get("trailingZeros").and_then(|v| v.as_bool()).unwrap_or(true);
                Some(format_number_places(value, n_places, trailing))
            }
            "BuildingBlocks_BindingsLocalizedSIUnitFromNumber" => {
                // Formatted number + the localized SI unit symbol. Engine knobs:
                //   `forcedSIPrefix` — "Unit" pins the BASE unit (no magnitude
                //     prefix); a NAMED prefix ("Kilo"/"Mega"/"Giga") FORCES that
                //     prefix regardless of magnitude (the MFD-radar range:
                //     `forcedSIPrefix=Kilo` renders 700 m as "0.7km"); any other /
                //     auto mode (e.g. "INVALID") scales to K/M/G by magnitude (the
                //     emissions header's "3.5K", 3500).
                //   `unitSuffix` — the quantity enum (Speed/Distance/...). The unit
                //     symbol is its localization entry `text_ui_SIUnit_<suffix>`
                //     (Speed → "m/s", Distance → "m" so Kilo → "km"); "None" appends
                //     nothing (the g-force readout, whose "G" comes from a later
                //     LocalizationCombine).
                let mut seen_num = std::collections::HashSet::new();
                let value = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, &mut seen_num))?;
                let n_places = op.get("nPlaces").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let forced_prefix = op.get("forcedSIPrefix").and_then(|v| v.as_str()).unwrap_or("");
                let (scaled, prefix) = if forced_prefix.eq_ignore_ascii_case("Unit") {
                    (value, "")
                } else if forced_prefix.eq_ignore_ascii_case("Kilo") {
                    (value / 1_000.0, "k")
                } else if forced_prefix.eq_ignore_ascii_case("Mega") {
                    (value / 1_000_000.0, "M")
                } else if forced_prefix.eq_ignore_ascii_case("Giga") {
                    (value / 1_000_000_000.0, "G")
                } else {
                    // Auto-scale by magnitude (unrecognised / auto prefix).
                    let magnitude = value.abs();
                    if magnitude >= 1_000_000_000.0 {
                        (value / 1_000_000_000.0, "G")
                    } else if magnitude >= 1_000_000.0 {
                        (value / 1_000_000.0, "M")
                    } else if magnitude >= 1_000.0 {
                        (value / 1_000.0, "K")
                    } else {
                        (value, "")
                    }
                };
                let number = format!("{}{prefix}", format_number_places(scaled, n_places, true));
                let unit = match op.get("unitSuffix").and_then(|v| v.as_str()) {
                    Some(suffix) if !suffix.eq_ignore_ascii_case("None") => defaults
                        .lookup_localization(&format!("text_ui_SIUnit_{suffix}"))
                        .unwrap_or_default(),
                    _ => "",
                };
                Some(format!("{number}{unit}"))
            }
            "BuildingBlocks_BindingsLocalizationFromIntegerSwitch" => {
                let input = self.eval_integer_ptr_from_field(op.get("input").and_then(|v| v.as_str()), defaults, seen)?;
                let values = op.get("values").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let key = values
                    .iter()
                    .find_map(|pair| {
                        let first = pair.get("first").and_then(|v| v.as_i64())?;
                        if first == input {
                            pair.get("second").and_then(|v| v.as_str())
                        } else {
                            None
                        }
                    })
                    .or_else(|| op.get("defaultValue").and_then(|v| v.as_str()))
                    .unwrap_or("");
                if key.is_empty() {
                    return None;
                }
                Some(defaults.lookup_localization(key).unwrap_or(key).to_string())
            }
            "BuildingBlocks_BindingsLocalizedVariable" => {
                let path = self.ptr_to_path.get(&ptr)?;
                let val = defaults.lookup_path(path)?;
                Some(value_to_string(val))
            }
            "BuildingBlocks_BindingsLocalizedComponentParameter" => {
                if let Some(value) = self.eval_localized_component_parameter_override(op, ptr, defaults, seen) {
                    return Some(value);
                }
                let key = op.get("defaultValue").and_then(|v| v.as_str()).unwrap_or("");
                if key.is_empty() {
                    return None;
                }
                Some(defaults.lookup_localization(key).unwrap_or(key).to_string())
            }
            "_SynthLocalizedParam_" => {
                let key = op.get("resolvedLocKey").and_then(|v| v.as_str()).unwrap_or("");
                if key.is_empty() {
                    return None;
                }
                Some(defaults.lookup_localization(key).unwrap_or(key).to_string())
            }
            "BuildingBlocks_BindingsLocalizedFromBoolean" => {
                let mut seen_bool = std::collections::HashSet::new();
                let enabled = self
                    .eval_bool_ptr_from_field(op.get("input"), defaults, &mut seen_bool)
                    .unwrap_or(false);
                let mut branch_seen = std::collections::HashSet::new();
                let ptr_branch = if enabled { op.get("inputTrue") } else { op.get("inputFalse") };
                if std::env::var("BB_A3_TEXT_PROBE").as_deref() == Ok("1") {
                    if let Some(ptr) = ptr_branch
                        .and_then(|v| v.as_str())
                        .and_then(parse_points_to_or_ptr_str)
                    {
                        if let Some(branch_op) = self.ptr_to_op.get(&ptr) {
                            let branch_ty = branch_op
                                .get("_Type_")
                                .and_then(|v| v.as_str())
                                .unwrap_or("<none>");
                            log::info!(
                                "A3-text-probe: LocalizedFromBoolean branch_ptr=ptr:{ptr} type={branch_ty} op={branch_op}"
                            );
                        }
                    }
                    log::info!(
                        "A3-text-probe: LocalizedFromBoolean enabled={} ptr_branch={:?} isTrue={:?} isFalse={:?}",
                        enabled,
                        ptr_branch.and_then(|v| v.as_str()),
                        op.get("isTrue").and_then(|v| v.as_str()),
                        op.get("isFalse").and_then(|v| v.as_str()),
                    );
                }
                if let Some(ptr_key) = self.eval_localized_ptr_from_field(ptr_branch, defaults, &mut branch_seen)
                {
                    if std::env::var("BB_A3_TEXT_PROBE").as_deref() == Ok("1") {
                        log::info!("A3-text-probe: LocalizedFromBoolean branch resolved={ptr_key:?}");
                    }
                    return Some(ptr_key);
                }
                let key = if enabled { op.get("isTrue") } else { op.get("isFalse") }
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if key.is_empty() {
                    return None;
                }
                Some(defaults.lookup_localization(key).unwrap_or(key).to_string())
            }
            "BuildingBlocks_BindingsTagFromBoolean" => {
                let mut seen_bool = std::collections::HashSet::new();
                let enabled = self
                    .eval_bool_ptr_from_field(op.get("input"), defaults, &mut seen_bool)
                    .unwrap_or(false);
                let true_tag = op
                    .get("trueTag")
                    .or_else(|| op.get("valueTrue"))
                    .or_else(|| op.get("valueA"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let false_tag = op
                    .get("falseTag")
                    .or_else(|| op.get("valueFalse"))
                    .or_else(|| op.get("valueB"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tag = if enabled { true_tag } else { false_tag };
                if tag.is_empty() {
                    None
                } else if tag.starts_with('@') {
                    Some(defaults.lookup_localization(tag).unwrap_or(tag).to_string())
                } else {
                    Some(tag.to_string())
                }
            }
            _ => None,
        }
    }

    pub(super) fn eval_integer_ptr_from_field(
        &self,
        field: Option<&str>,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<i64> {
        let ptr = field.and_then(parse_points_to_or_ptr_str)?;
        self.eval_integer_ptr(ptr, defaults, seen)
    }

    fn eval_localized_ptr_from_field(
        &self,
        field: Option<&serde_json::Value>,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<String> {
        let ptr = field.and_then(|v| v.as_str()).and_then(parse_points_to_or_ptr_str)?;
        self.eval_localized_ptr(ptr, defaults, seen)
    }

    pub(super) fn eval_bool_ptr_from_field(
        &self,
        field: Option<&serde_json::Value>,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<bool> {
        let ptr = field.and_then(|v| v.as_str()).and_then(parse_points_to_or_ptr_str)?;
        self.eval_bool_ptr(ptr, defaults, seen)
    }

    pub(super) fn eval_bool_ptr(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<bool> {
        if !seen.insert(ptr) {
            return None;
        }
        let op = self.ptr_to_op.get(&ptr)?;
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "_SynthBooleanParam_" => op.get("resolvedBool").and_then(|v| v.as_bool()),
            "BuildingBlocks_BindingsBooleanComponentParameter" => {
                if let Some(value) = self.eval_bool_component_parameter_override(op, ptr, defaults, seen) {
                    return Some(value);
                }
                // An unwired param takes a registry at-rest value matching its
                // NAME before the editor defaultValue: `iscast` authors TRUE
                // (the editor pose) but the engine wires it FALSE for screen
                // render targets — the registry carries that engine value.
                if let Some(value) = op
                    .get("name")
                    .and_then(|v| v.as_str())
                    .filter(|name| !name.is_empty())
                    .and_then(|name| defaults.lookup_path(name))
                {
                    return match value {
                        Value::Bool(b) => Some(*b),
                        Value::Int(i) => Some(*i != 0),
                        Value::Float(f) => Some(*f != 0.0),
                        _ => None,
                    };
                }
                op.get("defaultValue").and_then(|v| v.as_bool()).or(Some(false))
            }
            "BuildingBlocks_BindingsBooleanVariable" => {
                let Some(path) = self.ptr_to_path.get(&ptr) else {
                    return None;
                };
                let Some(val) = defaults.lookup_path(path) else {
                    // Unbound engine variable: the canvas's authored
                    // staticVariables value is its at-rest default.
                    return self
                        .static_variable_values
                        .get(&path.to_ascii_lowercase())
                        .and_then(|v| v.as_bool());
                };
                match val {
                    Value::Bool(b) => Some(*b),
                    Value::Int(i) => Some(*i != 0),
                    Value::Float(f) => Some(*f != 0.0),
                    Value::Str(s) | Value::Guid(s) => match s.to_ascii_lowercase().as_str() {
                        "1" | "true" | "yes" => Some(true),
                        "0" | "false" | "no" => Some(false),
                        _ => None,
                    },
                }
            }
            "BuildingBlocks_BindingsBooleanInvert" => {
                let inp = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)?;
                self.eval_bool_ptr(inp, defaults, seen).map(|v| !v)
            }
            "BuildingBlocks_BindingsBooleanFromInteger" => {
                // `inputL <type> {inputR | value}`. When both operands resolve
                // statically (an `IntegerComponentParameter` default), compute the
                // real comparison; otherwise fall back to the at-rest heuristic (no
                // specific integer state is active, so `Equal` is false / `NotEqual`
                // is true; ordered comparisons stay unresolved). Mirrors the
                // `bb_state_filter::integer` rule used for visibility gating.
                let cmp = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let l_ptr = op
                    .get("inputL")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str);
                let mut seen_l = std::collections::HashSet::new();
                let lhs = l_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, &mut seen_l));
                let l_unbound = l_ptr.is_some_and(|p| self.is_unbound_integer_param(p, defaults));
                let (rhs, r_unbound) = match op.get("inputR") {
                    Some(r) if !r.is_null() => {
                        let r_ptr = r.as_str().and_then(parse_points_to_or_ptr_str);
                        let mut seen_r = std::collections::HashSet::new();
                        let rv = r_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, &mut seen_r));
                        let ru = r_ptr.is_some_and(|p| self.is_unbound_integer_param(p, defaults));
                        (rv, ru)
                    }
                    // An authored literal `value` is a real constant, not an unbound param.
                    _ => (op.get("value").and_then(|v| v.as_i64()), false),
                };
                // Two *unbound* component parameters (both at their `defaultValue`
                // sentinel, e.g. an idle MFD header's `bindingid == selectedmfd`,
                // both -1) are not a real runtime state: comparing the sentinels
                // would spuriously fire a selected/active gate. Fall to the at-rest
                // heuristic unless at least one side is bound or an authored literal.
                if let (Some(l), Some(r)) = (lhs, rhs)
                    && !(l_unbound && r_unbound)
                {
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
            "BuildingBlocks_BindingsBooleanFromNumber" => {
                // `input <type> {inputB | number}` — the authored `number`
                // literal is the comparison constant (the pip sizing chains'
                // `count > 15` fallback thresholds).
                let cmp = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let mut seen_num = std::collections::HashSet::new();
                let lhs = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, &mut seen_num))?;
                let rhs = match op.get("inputB") {
                    Some(r) if !r.is_null() => {
                        let mut seen_r = std::collections::HashSet::new();
                        r.as_str()
                            .and_then(parse_points_to_or_ptr_str)
                            .and_then(|p| self.eval_number_ptr(p, defaults, &mut seen_r))?
                    }
                    _ => op.get("number").and_then(|v| v.as_f64())?,
                };
                match cmp {
                    "Equal" => Some(lhs == rhs),
                    "NotEqual" => Some(lhs != rhs),
                    "Greater" => Some(lhs > rhs),
                    "GreaterOrEqual" => Some(lhs >= rhs),
                    "Less" => Some(lhs < rhs),
                    "LessOrEqual" => Some(lhs <= rhs),
                    _ => None,
                }
            }
            "BindingsOperation_BooleanFromStringIsEmpty" => {
                let inp = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)?;
                let mut seen_str = std::collections::HashSet::new();
                self.eval_string_ptr(inp, defaults, &mut seen_str)
                    .map(|s| s.trim().is_empty())
            }
            "BuildingBlocks_BindingsBooleanEvaluateOr" => {
                let inputs = op.get("inputs").and_then(|v| v.as_array())?;
                let mut out = false;
                for input in inputs {
                    let ptr = input.as_str().and_then(parse_points_to_or_ptr_str)?;
                    out |= self.eval_bool_ptr(ptr, defaults, seen).unwrap_or(false);
                }
                Some(out)
            }
            "BuildingBlocks_BindingsBooleanEvaluateAnd" => {
                let inputs = op.get("inputs").and_then(|v| v.as_array())?;
                let mut out = true;
                for input in inputs {
                    let ptr = input.as_str().and_then(parse_points_to_or_ptr_str)?;
                    out &= self.eval_bool_ptr(ptr, defaults, seen).unwrap_or(false);
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// True when `ptr` is an `IntegerComponentParameter` with no parent-supplied
    /// override — i.e. it resolves only to its design-time `defaultValue` sentinel
    /// and carries no real runtime value.
    fn is_unbound_integer_param(&self, ptr: BbNodeId, defaults: &DefaultValueRegistry) -> bool {
        let Some(op) = self.ptr_to_op.get(&ptr) else {
            return false;
        };
        if op.get("_Type_").and_then(|v| v.as_str())
            != Some("BuildingBlocks_BindingsIntegerComponentParameter")
        {
            return false;
        }
        let mut seen = std::collections::HashSet::new();
        self.eval_integer_component_parameter_override(op, ptr, defaults, &mut seen)
            .is_none()
    }

}

/// Format a number at `n_places` decimals; `trailing_zeros: false` trims the
/// fractional part's trailing zeros (and a bare trailing point).
fn format_number_places(value: f64, n_places: usize, trailing_zeros: bool) -> String {
    let formatted = format!("{value:.n_places$}");
    if trailing_zeros || n_places == 0 {
        return formatted;
    }
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}
