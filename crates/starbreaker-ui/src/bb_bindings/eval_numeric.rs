//! Numeric binding evaluation: integer pointers (`IntegerVariable`,
//! `IntegerArithmatic`, `IntegerFromBoolean`, component-parameter defaults)
//! and number pointers (`NumberVariable`, `NumberFromInteger`,
//! `NumberArithmatic`). Split from `eval.rs` (line-cap); boolean and
//! localized evaluation stay there.

use crate::bb_scene::BbNodeId;
use crate::canvas::Value;
use crate::defaults::DefaultValueRegistry;
use super::util::parse_points_to_or_ptr_str;
use super::BindingResolver;

impl BindingResolver {
    pub(super) fn eval_integer_ptr(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<i64> {
        // Break reference cycles: an `IntegerComponentParameter` override chain can
        // loop back on itself across several hops (e.g. 373→372→369→384→373). The
        // per-call `parameter == current_ptr` check only catches direct self-refs,
        // so without this the resolver recurses until the stack overflows (observed
        // exporting the Drake Clipper's power MFD). `seen` tracks the *active*
        // recursion path: we insert on entry and remove on exit, so a true cycle
        // (ptr already on the path) is cut while a pointer legitimately shared by
        // sibling branches (a DAG — e.g. a localization-combine chain) still
        // resolves on each visit.
        if !seen.insert(ptr) {
            return None;
        }
        let result = self.eval_integer_ptr_resolved(ptr, defaults, seen);
        seen.remove(&ptr);
        result
    }

    /// Resolve an integer pointer assuming `ptr` is already recorded on the active
    /// recursion path (see [`Self::eval_integer_ptr`], which owns cycle tracking).
    fn eval_integer_ptr_resolved(
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
                    // Unbound engine variable: authored staticVariables default.
                    return self
                        .static_variable_values
                        .get(&path.to_ascii_lowercase())
                        .and_then(|v| v.as_i64());
                };
                match val {
                    Value::Int(i) => Some(*i as i64),
                    Value::Float(f) => Some(*f as i64),
                    Value::Bool(b) => Some(if *b { 1 } else { 0 }),
                    Value::Str(s) | Value::Guid(s) => s.parse::<i64>().ok(),
                }
            }
            "BuildingBlocks_BindingsIntegerFromNumber" => {
                // Number → integer bridge (the OUTPUT card's
                // `LocalizedFromInteger(IntegerFromNumber(availablepower))`).
                let mut seen_num = std::collections::HashSet::new();
                op.get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, &mut seen_num))
                    .map(|value| value.round() as i64)
            }
            "BuildingBlocks_BindingsIntegerFromBoolean" => {
                // A boolean variable's at-rest value is its type default
                // (`false`), so an unresolved input selects the `isFalse`
                // branch (the medical header's screen-state encoders sum
                // these to 0 to drive the hide-back-button tag).
                let mut seen_bool = std::collections::HashSet::new();
                let enabled = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_bool_ptr(p, defaults, &mut seen_bool))
                    .unwrap_or(false);
                let branch_input = op
                    .get(if enabled { "inputTrue" } else { "inputFalse" })
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str);
                if let Some(branch_ptr) = branch_input {
                    return self.eval_integer_ptr(branch_ptr, defaults, seen);
                }
                op.get(if enabled { "isTrue" } else { "isFalse" })
                    .and_then(|v| v.as_i64())
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

    /// Whether a widget field's at-rest VALUE flows from a live engine
    /// `Bindings*Variable` (an external telemetry binding such as
    /// `flightcontroller/linearvelocity/ratio/z`) rather than an unwired
    /// component parameter or a divide-by-zero artifact.
    ///
    /// Path-sensitive: boolean-gated `NumberFromBoolean` / `IntegerFromBoolean`
    /// nodes follow only the branch their at-rest selector takes, and a `Div`
    /// by (near-)zero is treated as data-absent (`false`). So the power-pip's
    /// `1/MaxPipList` and the widget-standard icons' unwired `ParamInput` sizes
    /// stay placeholders while the velocity ball's `|velocity|/2` is recognised
    /// as a genuine engine-driven value. Used by
    /// `resolve_geometry_fields_into_scene` to decide whether a resolved size of
    /// `0` is a real at-rest collapse (apply) or a half-resolved editor
    /// placeholder (keep the authored size).
    pub(super) fn field_value_source_is_engine_variable(
        &self,
        node_id: BbNodeId,
        field: &str,
        defaults: &DefaultValueRegistry,
    ) -> bool {
        let Some(input_ptrs) = self
            .widget_field_to_input_ptrs
            .get(&(node_id, field.to_string()))
        else {
            return false;
        };
        input_ptrs.iter().any(|&ptr| {
            let mut seen = std::collections::HashSet::new();
            self.ptr_source_is_engine_variable(ptr, defaults, &mut seen)
        })
    }

    fn ptr_source_is_engine_variable(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> bool {
        if !seen.insert(ptr) {
            return false;
        }
        let result = self.ptr_source_is_engine_variable_inner(ptr, defaults, seen);
        seen.remove(&ptr);
        result
    }

    fn ptr_source_is_engine_variable_inner(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> bool {
        let Some(op) = self.ptr_to_op.get(&ptr) else {
            return false;
        };
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        let follow = |key: &str, seen: &mut std::collections::HashSet<BbNodeId>| -> bool {
            op.get(key)
                .and_then(|v| v.as_str())
                .and_then(parse_points_to_or_ptr_str)
                .is_some_and(|p| self.ptr_source_is_engine_variable(p, defaults, seen))
        };
        match ty {
            "BuildingBlocks_BindingsNumberVariable"
            | "BuildingBlocks_BindingsIntegerVariable" => {
                // A live engine telemetry binding (non-empty path); component
                // parameters carry no path and fall through to `false`.
                self.ptr_to_path.get(&ptr).is_some_and(|p| !p.is_empty())
            }
            "BuildingBlocks_BindingsNumberFromInteger"
            | "BuildingBlocks_BindingsNumberRound"
            | "BuildingBlocks_BindingsNumberClamp"
            | "BuildingBlocks_BindingsNumberFromIntegerSwitch" => follow("input", seen),
            "BuildingBlocks_BindingsNumberFromBoolean"
            | "BuildingBlocks_BindingsIntegerFromBoolean" => {
                let mut seen_bool = std::collections::HashSet::new();
                let enabled = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_bool_ptr(p, defaults, &mut seen_bool))
                    .unwrap_or(false);
                follow(if enabled { "inputTrue" } else { "inputFalse" }, seen)
            }
            "BuildingBlocks_BindingsNumberArithmatic"
            | "BuildingBlocks_BindingsIntegerArithmatic" => {
                if op.get("type").and_then(|v| v.as_str()) == Some("Div") {
                    // A divide-by-zero divisor is data-absent (the pip's
                    // `1/MaxPipList`), not a genuine engine value.
                    let amount = op.get("amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
                    let mut seen_div = std::collections::HashSet::new();
                    let divisor = op
                        .get("inputB")
                        .and_then(|v| v.as_str())
                        .and_then(parse_points_to_or_ptr_str)
                        .and_then(|p| self.eval_number_ptr(p, defaults, &mut seen_div))
                        .unwrap_or(amount);
                    if divisor.abs() <= f64::EPSILON {
                        return false;
                    }
                }
                follow("input", seen)
                    || follow("inputB", seen)
                    || follow("inputL", seen)
                    || follow("inputR", seen)
            }
            _ => false,
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
            "_SynthNumberParam_" => op.get("resolvedNumber").and_then(|v| v.as_f64()),
            "BuildingBlocks_BindingsNumberComponentParameter" => {
                if let Some(value) =
                    self.eval_number_component_parameter_override(op, ptr, defaults, seen)
                {
                    return Some(value);
                }
                op.get("defaultValue").and_then(|v| v.as_f64())
            }
            "BuildingBlocks_BindingsNumberVariable" => {
                let path = self.ptr_to_path.get(&ptr)?;
                let Some(val) = defaults.lookup_path(path) else {
                    // Unbound engine variable: authored staticVariables default.
                    return self
                        .static_variable_values
                        .get(&path.to_ascii_lowercase())
                        .and_then(|v| v.as_f64());
                };
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
                // An absent operand is the authored `amount` constant (the pip
                // slot height's `Div(amount=1, inputB=MaxPipList)` = 1/max); an
                // unresolved wired operand is the number type default 0.
                let a = match input {
                    Some(p) => self.eval_number_ptr(p, defaults, seen).unwrap_or(0.0),
                    None => amount,
                };
                let b = input_b
                    .and_then(|p| self.eval_number_ptr(p, defaults, seen))
                    .unwrap_or(amount);
                Some(match kind {
                    "Add" => if has_explicit_rhs { a + b } else { a + amount },
                    "Sub" => a - b,
                    "Mul" => a * b,
                    "Div" => if b.abs() > f64::EPSILON { a / b } else { 0.0 },
                    "Min" => a.min(b),
                    "Max" => a.max(b),
                    _ => a,
                })
            }
            "BuildingBlocks_BindingsNumberFromBoolean" => {
                // The boolean input's at-rest default is `false`.
                let mut seen_bool = std::collections::HashSet::new();
                let enabled = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_bool_ptr(p, defaults, &mut seen_bool))
                    .unwrap_or(false);
                let branch_input = op
                    .get(if enabled { "inputTrue" } else { "inputFalse" })
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str);
                if let Some(branch_ptr) = branch_input {
                    return self.eval_number_ptr(branch_ptr, defaults, seen);
                }
                op.get(if enabled { "isTrue" } else { "isFalse" })
                    .and_then(|v| v.as_f64())
            }
            "BuildingBlocks_BindingsNumberClamp" => {
                // `minValue`/`maxValue` literals with optional wired
                // `inputMinValue`/`inputMaxValue` overrides (the heat gauge's
                // fill ratio clamps to 0.015..1).
                let value = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, seen))?;
                let min = op
                    .get("inputMinValue")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, seen))
                    .or_else(|| op.get("minValue").and_then(|v| v.as_f64()))
                    .unwrap_or(f64::NEG_INFINITY);
                let max = op
                    .get("inputMaxValue")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, seen))
                    .or_else(|| op.get("maxValue").and_then(|v| v.as_f64()))
                    .unwrap_or(f64::INFINITY);
                Some(value.clamp(min, max))
            }
            "BuildingBlocks_BindingsNumberRound" => {
                // `amount` is the number of DECIMAL PLACES (the pip slot height
                // rounds `1/max` to 3 places).
                let value = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_number_ptr(p, defaults, seen))?;
                let places = op.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
                let factor = 10f64.powi(places.clamp(0, 12) as i32);
                Some((value * factor).round() / factor)
            }
            "BuildingBlocks_BindingsNumberFromIntegerSwitch" => {
                let selector = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_integer_ptr(p, defaults, seen));
                let matched = selector.and_then(|sel| {
                    op.get("values")?.as_array()?.iter().find_map(|pair| {
                        (pair.get("first")?.as_i64()? == sel)
                            .then(|| pair.get("second").and_then(|v| v.as_f64()))?
                    })
                });
                matched.or_else(|| op.get("defaultValue").and_then(|v| v.as_f64()))
            }
            _ => self.eval_integer_ptr(ptr, defaults, seen).map(|v| v as f64),
        }
    }
}
