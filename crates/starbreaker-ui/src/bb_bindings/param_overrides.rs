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
        // A parameter WIRED to an engine variable that is unbound in a static
        // render takes the variable's at-rest type default (`false`), not the
        // authored editor `defaultValue` (the power screen's notification
        // overlays are authored `true` for editor preview but bound from
        // unbound `BooleanVariable`s). Unwired parameters (no host ParamInput
        // ops at all) keep editor semantics: authored default applies.
        if input_ptrs
            .iter()
            .any(|&p| p != current_ptr && self.input_is_unbound_variable(p, defaults))
        {
            return Some(false);
        }
        None
    }

    /// True when `ptr` is a `Bindings*Variable` op whose binding path has no
    /// default-registry entry — wired to an engine variable that is unbound in
    /// a static render.
    fn input_is_unbound_variable(&self, ptr: BbNodeId, defaults: &DefaultValueRegistry) -> bool {
        let Some(op) = self.ptr_to_op.get(&ptr) else {
            return false;
        };
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if !(ty.starts_with("BuildingBlocks_Bindings") && ty.ends_with("Variable")) {
            return false;
        }
        match self.ptr_to_path.get(&ptr) {
            Some(path) => defaults.lookup_path(path).is_none(),
            None => true,
        }
    }

    pub(super) fn eval_number_component_parameter_override(
        &self,
        op: &serde_json::Value,
        current_ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<f64> {
        let parameter = op.get("parameter").and_then(|v| v.as_str())?;
        let input_ptrs = self.parameter_input_ptrs(parameter)?;
        for &input_ptr in input_ptrs {
            if input_ptr == current_ptr {
                continue;
            }
            if let Some(value) = self.eval_number_ptr(input_ptr, defaults, seen) {
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
