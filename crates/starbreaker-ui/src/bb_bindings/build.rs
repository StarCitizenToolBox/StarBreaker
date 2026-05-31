use std::collections::HashMap;

use crate::bb_scene::BbNodeId;

use super::util::{parse_points_to_or_ptr, parse_ptr};
use super::BindingResolver;

impl BindingResolver {
    /// Build a resolver from a flat slice of raw operation JSON values.
    ///
    /// Expects operations from a fully-merged scene (pointer IDs already
    /// offset-adjusted by [`crate::bb_resolve`]).
    pub fn from_operations(operations: &[serde_json::Value]) -> Self {
        // Pass 1: variable ops → ptr → binding path.
        let mut ptr_to_path: HashMap<BbNodeId, String> = HashMap::new();
        let mut ptr_to_op: HashMap<BbNodeId, serde_json::Value> = HashMap::new();
        for op in operations {
            if let Some(ptr) = op
                .get("_Pointer_")
                .and_then(|v| v.as_str())
                .and_then(parse_ptr)
            {
                ptr_to_op.insert(ptr, op.clone());
            }
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

        // Pass 1b: _SynthLocalizedParam_ ops → ptr → localization key.
        // These are injected by bb_resolve::inject_param_overrides when a
        // WidgetCanvas node carries paramInputValues with localized parameter
        // overrides (e.g. annunciator chiclet labels).
        let mut ptr_to_loc_key: HashMap<BbNodeId, String> = HashMap::new();
        for op in operations {
            let type_str = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if type_str == "_SynthLocalizedParam_" {
                let ptr = op
                    .get("_Pointer_")
                    .and_then(|v| v.as_str())
                    .and_then(parse_ptr);
                let loc_key = op
                    .get("resolvedLocKey")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                if let (Some(ptr), Some(key)) = (ptr, loc_key) {
                    if !key.is_empty() {
                        ptr_to_loc_key.insert(ptr, key);
                    }
                }
            }
        }

        log::debug!("bb_bindings::build: ptr_to_loc_key len={} ptr_to_path len={}", ptr_to_loc_key.len(), ptr_to_path.len());

        // Pass 2: field ops → widget ptr → input ptr.
        let mut widget_to_path: HashMap<BbNodeId, String> = HashMap::new();
        // Also collect localized-field ops → widget ptr → loc key.
        let mut widget_to_loc_key: HashMap<BbNodeId, String> = HashMap::new();
        let mut widget_to_string: HashMap<BbNodeId, String> = HashMap::new();
        let mut widget_to_input_prio_ptrs: HashMap<BbNodeId, Vec<(u8, BbNodeId)>> = HashMap::new();
        let mut widget_field_to_input_prio_ptrs: HashMap<(BbNodeId, String), Vec<(u8, BbNodeId)>> =
            HashMap::new();
        for op in operations {
            let type_str = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if type_str == "_SynthLocalizedWidget_" {
                let widget_ptr = parse_points_to_or_ptr(op.get("widget"));
                let loc_key = op
                    .get("resolvedLocKey")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                if let (Some(w), Some(key)) = (widget_ptr, loc_key) {
                    if !key.is_empty() {
                        widget_to_loc_key.insert(w, key);
                    }
                }
            }
            if type_str == "_SynthStringWidget_" {
                let widget_ptr = parse_points_to_or_ptr(op.get("widget"));
                let value = op
                    .get("resolvedString")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                if let (Some(w), Some(value)) = (widget_ptr, value) {
                    if !value.is_empty() {
                        widget_to_string.insert(w, value);
                    }
                }
            }
        }
        for op in operations {
            let type_str = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if type_str.contains("Field") {
                let widget_ptr = parse_points_to_or_ptr(op.get("widget"));
                let input_ptr = parse_points_to_or_ptr(op.get("input"));
                if let (Some(w), Some(inp)) = (widget_ptr, input_ptr) {
                    let field_name = op
                        .get("field")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .unwrap_or_default();
                    let prio = if type_str.contains("LocalizedField") {
                        3
                    } else if type_str.contains("StringField") {
                        2
                    } else {
                        1
                    };
                    widget_to_input_prio_ptrs.entry(w).or_default().push((prio, inp));
                    if !field_name.is_empty() {
                        widget_field_to_input_prio_ptrs
                            .entry((w, field_name))
                            .or_default()
                            .push((prio, inp));
                    }
                    if let Some(path) = ptr_to_path.get(&inp) {
                        widget_to_path.insert(w, path.clone());
                    }
                    if let Some(key) = ptr_to_loc_key.get(&inp) {
                        widget_to_loc_key.insert(w, key.clone());
                    }
                }
            }
        }
        let mut widget_to_input_ptrs: HashMap<BbNodeId, Vec<BbNodeId>> = HashMap::new();
        for (widget, mut pairs) in widget_to_input_prio_ptrs {
            pairs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let mut ordered: Vec<BbNodeId> = Vec::new();
            for (_, ptr) in pairs {
                if !ordered.contains(&ptr) {
                    ordered.push(ptr);
                }
            }
            widget_to_input_ptrs.insert(widget, ordered);
        }

        let mut widget_field_to_input_ptrs: HashMap<(BbNodeId, String), Vec<BbNodeId>> =
            HashMap::new();
        let mut field_name_to_input_ptrs: HashMap<String, Vec<BbNodeId>> = HashMap::new();
        for (key, mut pairs) in widget_field_to_input_prio_ptrs {
            let field_name = key.1.clone();
            pairs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let mut ordered: Vec<BbNodeId> = Vec::new();
            for (_, ptr) in pairs {
                if !ordered.contains(&ptr) {
                    ordered.push(ptr);
                }
            }
            for ptr in &ordered {
                let entry = field_name_to_input_ptrs
                    .entry(field_name.clone())
                    .or_default();
                if !entry.contains(ptr) {
                    entry.push(*ptr);
                }
            }
            widget_field_to_input_ptrs.insert(key, ordered);
        }

        if std::env::var("BB_PRIMARY_TAG_PROBE").as_deref() == Ok("1") {
            let mut count = 0usize;
            for ((widget, field), ptrs) in &widget_field_to_input_ptrs {
                if field == "PrimaryStateTag" || field == "IsActive" {
                    count += 1;
                    let typed_ptrs = ptrs
                        .iter()
                        .map(|ptr| {
                            let ty = ptr_to_op
                                .get(ptr)
                                .and_then(|op| op.get("_Type_"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("<none>");
                            format!("ptr:{ptr}:{ty}")
                        })
                        .collect::<Vec<_>>();
                    log::info!(
                        "primary-tag-probe: widget=ptr:{widget} field={field} inputs={typed_ptrs:?}"
                    );
                }
            }
            log::info!("primary-tag-probe: field mappings count={count}");
        }

        Self {
            widget_to_path,
            widget_to_loc_key,
            widget_to_input_ptrs,
            widget_field_to_input_ptrs,
            field_name_to_input_ptrs,
            ptr_to_op,
            ptr_to_path,
            widget_to_string,
        }
    }
}
