//! Helpers for inspecting component-parameter boolean expressions in state-filter graphs.

use std::collections::{HashMap, HashSet};

use super::{eval::parse_points_to_ptr, BbNodeId};

pub(super) fn contains_unresolved_component_parameter(
    input: &serde_json::Value,
    ptr_to_op: &HashMap<BbNodeId, &serde_json::Value>,
    param_overrides: &HashMap<String, bool>,
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
            contains_unresolved_component_parameter(op, ptr_to_op, param_overrides, visited)
        }
        serde_json::Value::Object(obj) => {
            let ty = obj.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if ty == "BuildingBlocks_BindingsBooleanComponentParameter" {
                let param_name = obj
                    .get("parameter")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_default();
                if param_name.is_empty() {
                    return false;
                }
                if param_overrides.contains_key(&param_name) {
                    return false;
                }

                // If the component parameter carries an authored boolean
                // default, it is fully resolvable without a runtime override.
                if obj.get("defaultValue").and_then(|v| v.as_bool()).is_some() {
                    return false;
                }

                return true;
            }

            for key in ["input", "inputL", "inputR", "inputTrue", "inputFalse"] {
                if obj.get(key).is_some_and(|inner| {
                    contains_unresolved_component_parameter(inner, ptr_to_op, param_overrides, visited)
                }) {
                    return true;
                }
            }

            obj.get("inputs")
                .and_then(|v| v.as_array())
                .is_some_and(|inputs| {
                    inputs.iter().any(|inp| {
                        contains_unresolved_component_parameter(
                            inp,
                            ptr_to_op,
                            param_overrides,
                            visited,
                        )
                    })
                })
        }
        _ => false,
    }
}
