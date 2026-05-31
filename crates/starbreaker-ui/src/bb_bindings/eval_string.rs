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
        let probe = std::env::var("BB_PRIMARY_TAG_PROBE").as_deref() == Ok("1");
        if !seen.insert(ptr) {
            if probe {
                log::info!("primary-tag-probe: eval_string_ptr ptr={ptr} seen-cycle");
            }
            return None;
        }
        let Some(op) = self.ptr_to_op.get(&ptr) else {
            if probe {
                log::info!("primary-tag-probe: eval_string_ptr ptr={ptr} missing-op");
            }
            return None;
        };
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if probe {
            log::info!("primary-tag-probe: eval_string_ptr ptr={ptr} type={ty}");
        }
        match ty {
            "BuildingBlocks_BindingsStringComponentParameter" => {
                if let Some(value) = self.eval_string_component_parameter_override(op, ptr, defaults) {
                    return Some(value);
                }
                op.get("defaultValue")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            }
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
            "BuildingBlocks_BindingsTagFromIntegerSwitch" => {
                let mut seen_int = std::collections::HashSet::new();
                let input_field = op.get("input").and_then(|v| v.as_str());
                let Some(input) = self.eval_integer_ptr_from_field(input_field, defaults, &mut seen_int) else {
                    if probe {
                        log::info!(
                            "primary-tag-probe: tag-switch ptr={ptr} unresolved-input field={input_field:?}"
                        );
                    }
                    return None;
                };
                let values = op.get("values").and_then(|v| v.as_array())?;
                let mapped = values.iter().find_map(|pair| {
                    let first = pair.get("first").and_then(|v| v.as_i64())?;
                    if first == input {
                        pair.get("second").and_then(tag_reference_to_string)
                    } else {
                        None
                    }
                });
                let out = mapped.or_else(|| op.get("defaultValue").and_then(tag_reference_to_string));
                if std::env::var("BB_PRIMARY_TAG_PROBE").as_deref() == Ok("1") {
                    log::info!(
                        "primary-tag-probe: tag-switch ptr={ptr} input={input} out={out:?}"
                    );
                }
                out
            }
            "BuildingBlocks_BindingsTagFromBoolean" => {
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
                    && !value.is_empty()
                {
                    return Some(value);
                }

                let tag_value = if enabled {
                    op.get("isTrue")
                        .or_else(|| op.get("trueTag"))
                        .or_else(|| op.get("valueTrue"))
                        .or_else(|| op.get("valueA"))
                } else {
                    op.get("isFalse")
                        .or_else(|| op.get("falseTag"))
                        .or_else(|| op.get("valueFalse"))
                        .or_else(|| op.get("valueB"))
                };

                tag_value.and_then(|value| {
                    tag_reference_to_string(value)
                        .or_else(|| value.as_str().map(str::to_owned))
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

fn tag_reference_to_string(value: &serde_json::Value) -> Option<String> {
    value
        .get("_RecordName_")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("_RecordId_").and_then(|v| v.as_str()))
        .map(str::to_owned)
}
