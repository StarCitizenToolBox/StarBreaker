use std::collections::HashSet;

use crate::bb_scene::BbNodeId;
use crate::defaults::DefaultValueRegistry;

use super::BindingResolver;

impl BindingResolver {
    /// Resolve a boolean binding path using the current operation graph.
    ///
    /// Returns `None` when the path is not present or cannot be evaluated.
    pub fn resolve_bool_binding_path(
        &self,
        binding_path: &str,
        defaults: &DefaultValueRegistry,
    ) -> Option<bool> {
        let mut candidate_ptrs: Vec<BbNodeId> = self
            .ptr_to_path
            .iter()
            .filter_map(|(ptr, path)| path.eq_ignore_ascii_case(binding_path).then_some(*ptr))
            .collect();
        candidate_ptrs.sort_unstable();
        for ptr in candidate_ptrs {
            let mut seen = HashSet::new();
            if let Some(value) = self.eval_bool_ptr(ptr, defaults, &mut seen) {
                return Some(value);
            }
        }
        None
    }
}
