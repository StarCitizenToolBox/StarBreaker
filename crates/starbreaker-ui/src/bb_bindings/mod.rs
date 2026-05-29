//! Binding resolver — maps text widget nodes to runtime binding paths.

use std::collections::HashMap;

use crate::bb_scene::BbNodeId;

mod build;
mod eval;
mod eval_string;
mod resolve_text;
#[cfg(test)]
mod tests;
mod util;

/// Resolves text content for `WidgetTextField` and `WidgetText` nodes from operations.
pub struct BindingResolver {
    pub(super) widget_to_path: HashMap<BbNodeId, String>,
    pub(super) widget_to_loc_key: HashMap<BbNodeId, String>,
    pub(super) widget_to_input_ptrs: HashMap<BbNodeId, Vec<BbNodeId>>,
    pub(super) widget_field_to_input_ptrs: HashMap<(BbNodeId, String), Vec<BbNodeId>>,
    pub(super) ptr_to_op: HashMap<BbNodeId, serde_json::Value>,
    pub(super) ptr_to_path: HashMap<BbNodeId, String>,
    pub(super) widget_to_string: HashMap<BbNodeId, String>,
}

/// Outcome of [`BindingResolver::resolve_text_detailed`].
pub struct ResolvedText {
    pub text: String,
    pub is_name_derived: bool,
}
