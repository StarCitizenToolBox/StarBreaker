//! Binding resolver — maps text widget nodes to runtime binding paths.
//!
//! Builds a widget-pointer to binding-path lookup from merged BuildingBlocks
//! `operations[]` JSON and resolves display strings from [`DefaultValueRegistry`].

use std::collections::HashMap;

use crate::bb_scene::BbNodeId;
use crate::canvas::Value;
use crate::defaults::DefaultValueRegistry;

/// Resolves text content for `WidgetTextField` and `WidgetText` nodes from the
/// canvas `operations[]` array.
///
/// Resolution chain:
/// 1. Literal `text` field on the node's raw JSON → returned as-is.
/// 2. Binding path resolved via `operations[]` → looked up in
///    [`DefaultValueRegistry`] → returned as string.
/// 3. Binding path found but no default → `"[<path>]"`.
/// 4. No binding → `""`.
pub struct BindingResolver {
    /// widget_node_id → binding path (e.g. "/vehicle/targetname")
    widget_to_path: HashMap<BbNodeId, String>,
}

impl BindingResolver {
    /// Build a resolver from a flat slice of raw operation JSON values.
    ///
    /// Expects operations from a fully-merged scene (pointer IDs already
    /// offset-adjusted by [`crate::bb_resolve`]).
    pub fn from_operations(operations: &[serde_json::Value]) -> Self {
        // Pass 1: variable ops → ptr → binding path.
        let mut ptr_to_path: HashMap<BbNodeId, String> = HashMap::new();
        for op in operations {
            let type_str = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if type_str.contains("Variable") {
                let ptr = op
                    .get("_Pointer_")
                    .and_then(|v| v.as_str())
                    .and_then(parse_ptr);
                let binding = op
                    .get("binding")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                if let (Some(ptr), Some(path)) = (ptr, binding) {
                    if !path.is_empty() {
                        ptr_to_path.insert(ptr, path);
                    }
                }
            }
        }

        // Pass 2: field ops → widget ptr → input ptr.
        let mut widget_to_path: HashMap<BbNodeId, String> = HashMap::new();
        for op in operations {
            let type_str = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if type_str.contains("Field") {
                let widget_ptr = op
                    .get("widget")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to);
                let input_ptr = op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to);
                if let (Some(w), Some(inp)) = (widget_ptr, input_ptr) {
                    if let Some(path) = ptr_to_path.get(&inp) {
                        widget_to_path.insert(w, path.clone());
                    }
                }
            }
        }

        Self { widget_to_path }
    }

    /// Resolve display text for a node.
    ///
    /// Returns the literal text if present, the default-registry value if a
    /// binding maps to a known path, a bracketed path for unknown bindings, or
    /// an empty string if no binding exists at all.
    pub fn resolve_text(
        &self,
        node_id: BbNodeId,
        node_raw: &serde_json::Value,
        defaults: &DefaultValueRegistry,
    ) -> String {
        if let Some(lit) = node_raw.get("text").and_then(|v| v.as_str()) {
            let lit = lit.trim();
            if !lit.is_empty() {
                return lit.to_owned();
            }
        }

        if let Some(path) = self.widget_to_path.get(&node_id) {
            if let Some(val) = defaults.lookup_path(path) {
                return value_to_string(val);
            }
            return format!("[{path}]");
        }

        String::new()
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::Str(s) | Value::Guid(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e9 {
                format!("{}", *f as i64)
            } else {
                format!("{f:.2}")
            }
        }
        Value::Bool(b) => {
            if *b {
                "ON".to_owned()
            } else {
                "OFF".to_owned()
            }
        }
    }
}

fn parse_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("ptr:").and_then(|n| n.parse().ok())
}

fn parse_points_to(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:").and_then(parse_ptr)
}
