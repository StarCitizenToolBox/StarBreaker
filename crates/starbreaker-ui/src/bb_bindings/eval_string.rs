use crate::bb_scene::BbNodeId;
use crate::defaults::DefaultValueRegistry;

use super::util::{parse_points_to_or_ptr_str, value_to_string};
use super::BindingResolver;

impl BindingResolver {
    pub(super) fn eval_string_ptr(
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
            "BuildingBlocks_BindingsStringComponentParameter" => op
                .get("defaultValue")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            "_SynthStringParam_" => op
                .get("resolvedString")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            "BuildingBlocks_BindingsStringVariable" => {
                let path = self.ptr_to_path.get(&ptr)?;
                let val = defaults.lookup_path(path)?;
                Some(value_to_string(val))
            }
            "BuildingBlocks_BindingsStringFromIntegerSwitch" => {
                let mut seen_int = std::collections::HashSet::new();
                let input = self
                    .eval_integer_ptr_from_field(
                        op.get("input").and_then(|v| v.as_str()),
                        defaults,
                        &mut seen_int,
                    )?;
                let values = op.get("values").and_then(|v| v.as_array())?;
                let mapped = values.iter().find_map(|pair| {
                    let first = pair.get("first").and_then(|v| v.as_i64())?;
                    if first == input {
                        pair.get("second")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                    } else {
                        None
                    }
                });
                mapped.or_else(|| {
                    op.get("defaultValue")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                })
            }
            "BuildingBlocks_BindingsStringFromBoolean" => {
                let mut seen_bool = std::collections::HashSet::new();
                let enabled = self
                    .eval_bool_ptr_from_field(op.get("input"), defaults, &mut seen_bool)
                    .unwrap_or(false);

                let mut branch_seen = std::collections::HashSet::new();
                let branch_ptr = if enabled {
                    op.get("inputTrue")
                } else {
                    op.get("inputFalse")
                };
                if let Some(value) = branch_ptr
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to_or_ptr_str)
                    .and_then(|p| self.eval_string_ptr(p, defaults, &mut branch_seen))
                {
                    return Some(value);
                }

                if enabled {
                    op.get("isTrue")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                } else {
                    op.get("isFalse")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                }
            }
            _ => None,
        }
    }
}
