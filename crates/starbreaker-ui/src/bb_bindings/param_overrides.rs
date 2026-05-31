use crate::bb_scene::BbNodeId;
use crate::defaults::DefaultValueRegistry;

use super::BindingResolver;

impl BindingResolver {
    fn parameter_input_ptrs(&self, parameter: &str) -> Option<&[BbNodeId]> {
        self.field_name_to_input_ptrs
            .get(parameter)
            .map(|ptrs| ptrs.as_slice())
    }

    pub(super) fn eval_localized_component_parameter_override(
        &self,
        op: &serde_json::Value,
        current_ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &std::collections::HashSet<BbNodeId>,
    ) -> Option<String> {
        let parameter = op.get("parameter").and_then(|v| v.as_str())?;
        let input_ptrs = self.parameter_input_ptrs(parameter)?;
        for &input_ptr in input_ptrs {
            if input_ptr == current_ptr {
                continue;
            }
            let mut seen_loc = seen.clone();
            if let Some(value) = self.eval_localized_ptr(input_ptr, defaults, &mut seen_loc)
                && !value.is_empty()
            {
                return Some(value);
            }
            let mut seen_str = seen.clone();
            if let Some(value) = self.eval_string_ptr(input_ptr, defaults, &mut seen_str)
                && !value.is_empty()
            {
                return Some(value);
            }
        }
        None
    }

    pub(super) fn eval_bool_component_parameter_override(
        &self,
        op: &serde_json::Value,
        current_ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<bool> {
        let parameter = op.get("parameter").and_then(|v| v.as_str())?;
        let input_ptrs = self.parameter_input_ptrs(parameter)?;
        for &input_ptr in input_ptrs {
            if input_ptr == current_ptr {
                continue;
            }
            if let Some(value) = self.eval_bool_ptr(input_ptr, defaults, seen) {
                return Some(value);
            }
        }
        None
    }

    pub(super) fn eval_integer_component_parameter_override(
        &self,
        op: &serde_json::Value,
        current_ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<i64> {
        let parameter = op.get("parameter").and_then(|v| v.as_str())?;
        let input_ptrs = self.parameter_input_ptrs(parameter)?;
        for &input_ptr in input_ptrs {
            if input_ptr == current_ptr {
                continue;
            }
            if let Some(value) = self.eval_integer_ptr(input_ptr, defaults, seen) {
                return Some(value);
            }
        }
        None
    }

    pub(super) fn eval_string_component_parameter_override(
        &self,
        op: &serde_json::Value,
        current_ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
    ) -> Option<String> {
        let parameter = op.get("parameter").and_then(|v| v.as_str())?;
        let input_ptrs = self.parameter_input_ptrs(parameter)?;
        for &input_ptr in input_ptrs {
            if input_ptr == current_ptr {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            if let Some(value) = self.eval_string_ptr(input_ptr, defaults, &mut seen)
                && !value.is_empty()
            {
                return Some(value);
            }
        }
        None
    }
}
