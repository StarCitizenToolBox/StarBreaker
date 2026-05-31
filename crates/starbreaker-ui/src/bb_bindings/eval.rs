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
                let left = left_ptr
                    .and_then(|p| self.eval_localized_ptr(p, defaults, seen))
                    .or_else(|| left_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, seen)).map(|v| v.to_string()))
                    .unwrap_or_default();
                let right = right_ptr
                    .and_then(|p| self.eval_localized_ptr(p, defaults, seen))
                    .or_else(|| right_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, seen)).map(|v| v.to_string()))
                    .unwrap_or_default();
                let mut out = base.to_string();
                if out.contains("%d") {
                    out = out.replacen("%d", if !right.is_empty() { &right } else { &left }, 1);
                } else if out.contains("%s") {
                    out = out.replacen("%s", if !right.is_empty() { &right } else { &left }, 1);
                } else if left.is_empty() && right.is_empty() {
                    // keep base
                } else if left.is_empty() {
                    out = format!("{out}{right}");
                } else if right.is_empty() {
                    out = format!("{left}{out}");
                } else {
                    out = format!("{left}{out}{right}");
                }
                Some(out)
            }
            "BuildingBlocks_BindingsLocalizedFromInteger" => self
                .eval_integer_ptr_from_field(op.get("input").and_then(|v| v.as_str()), defaults, seen)
                .map(|v| v.to_string()),
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
                op.get("defaultValue").and_then(|v| v.as_bool()).or(Some(false))
            }
            "BuildingBlocks_BindingsBooleanVariable" => {
                let Some(path) = self.ptr_to_path.get(&ptr) else {
                    return None;
                };
                let Some(val) = defaults.lookup_path(path) else {
                    return None;
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

    pub(super) fn eval_integer_ptr(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<i64> {
        let op = self.ptr_to_op.get(&ptr)?;
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "_SynthIntegerParam_" => op.get("resolvedInt").and_then(|v| v.as_i64()),
            "BuildingBlocks_BindingsIntegerComponentParameter" => {
                if let Some(value) = self.eval_integer_component_parameter_override(op, ptr, defaults, seen) {
                    return Some(value);
                }
                op.get("defaultValue").and_then(|v| v.as_i64())
            }
            "BuildingBlocks_BindingsIntegerVariable" => {
                let Some(path) = self.ptr_to_path.get(&ptr) else {
                    return None;
                };
                let Some(val) = defaults.lookup_path(path) else {
                    return None;
                };
                match val {
                    Value::Int(i) => Some(*i as i64),
                    Value::Float(f) => Some(*f as i64),
                    Value::Bool(b) => Some(if *b { 1 } else { 0 }),
                    Value::Str(s) | Value::Guid(s) => s.parse::<i64>().ok(),
                }
            }
            "BuildingBlocks_BindingsIntegerArithmatic" => {
                let kind = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let amount = op.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
                let has_explicit_rhs = op.get("inputR").and_then(|v| v.as_str()).and_then(parse_points_to_or_ptr_str).is_some()
                    || op.get("inputB").and_then(|v| v.as_str()).and_then(parse_points_to_or_ptr_str).is_some();
                let l = self
                    .eval_integer_ptr_from_field(op.get("inputL").and_then(|v| v.as_str()), defaults, seen)
                    .or_else(|| self.eval_integer_ptr_from_field(op.get("input").and_then(|v| v.as_str()), defaults, seen))
                    .unwrap_or(0);
                let r = self
                    .eval_integer_ptr_from_field(op.get("inputR").and_then(|v| v.as_str()), defaults, seen)
                    .or_else(|| self.eval_integer_ptr_from_field(op.get("inputB").and_then(|v| v.as_str()), defaults, seen))
                    .unwrap_or(amount);
                Some(match kind {
                    "Add" => {
                        if has_explicit_rhs {
                            l + r
                        } else {
                            l + amount
                        }
                    }
                    "Min" => l.min(r),
                    "Max" => l.max(r),
                    "Sub" => l - r,
                    _ => l,
                })
            }
            _ => None,
        }
    }

    pub(super) fn eval_number_ptr(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<f64> {
        let op = self.ptr_to_op.get(&ptr)?;
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "BuildingBlocks_BindingsNumberVariable" => {
                let path = self.ptr_to_path.get(&ptr)?;
                let val = defaults.lookup_path(path)?;
                match val {
                    Value::Int(i) => Some(*i as f64),
                    Value::Float(f) => Some(*f as f64),
                    Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                    Value::Str(s) | Value::Guid(s) => s.parse::<f64>().ok(),
                }
            }
            "BuildingBlocks_BindingsNumberFromInteger" => {
                let inp = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)?;
                self.eval_integer_ptr(inp, defaults, seen).map(|v| v as f64)
            }
            "BuildingBlocks_BindingsNumberArithmatic" => {
                let kind = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let amount = op.get("amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
                let input = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str);
                let input_b = op
                    .get("inputB")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str);
                let has_explicit_rhs = input_b.is_some();
                let a = input.and_then(|p| self.eval_number_ptr(p, defaults, seen)).unwrap_or(0.0);
                let b = input_b
                    .and_then(|p| self.eval_number_ptr(p, defaults, seen))
                    .unwrap_or(amount);
                Some(match kind {
                    "Add" => if has_explicit_rhs { a + b } else { a + amount },
                    "Sub" => a - b,
                    "Mul" => a * b,
                    "Div" => if b.abs() > f64::EPSILON { a / b } else { 0.0 },
                    _ => a,
                })
            }
            _ => self.eval_integer_ptr(ptr, defaults, seen).map(|v| v as f64),
        }
    }
}
