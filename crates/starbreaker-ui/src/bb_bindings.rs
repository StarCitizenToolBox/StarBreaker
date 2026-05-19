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

}

/// Outcome of [`BindingResolver::resolve_text_detailed`].
pub struct ResolvedText {
    pub text: String,
    /// True when the text comes from the widget-name fallback (no literal,
    /// no live-binding default). Callers can use this to force centring /
    /// upper-casing for static "switched-on default" rendering.
    pub is_name_derived: bool,
}

impl BindingResolver {
    /// Convenience wrapper returning just the string.
    pub fn resolve_text(
        &self,
        node_id: BbNodeId,
        node_raw: &serde_json::Value,
        defaults: &DefaultValueRegistry,
    ) -> String {
        self.resolve_text_detailed(node_id, node_raw, defaults).text
    }

    /// Resolve display text with provenance for downstream formatting.
    pub fn resolve_text_detailed(
        &self,
        node_id: BbNodeId,
        node_raw: &serde_json::Value,
        defaults: &DefaultValueRegistry,
    ) -> ResolvedText {
        if let Some(lit) = node_raw.get("text").and_then(|v| v.as_str()) {
            let lit = lit.trim();
            if !lit.is_empty() {
                // A literal text field starting with '@' is itself a
                // localization key — look it up before using as-is.
                if lit.starts_with('@') {
                    if let Some(resolved) = defaults.lookup_localization(lit) {
                        return ResolvedText { text: resolved.to_owned(), is_name_derived: false };
                    }
                }
                return ResolvedText { text: lit.to_owned(), is_name_derived: false };
            }
        }

        // `labelProperties.label` carries a localization key such as
        // `@hud_NoTarget`.  Resolve it before falling back to the binding path.
        if let Some(loc_key) = node_raw
            .get("labelProperties")
            .and_then(|lp| lp.get("label"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            if let Some(resolved) = defaults.lookup_localization(loc_key) {
                return ResolvedText { text: resolved.to_owned(), is_name_derived: false };
            }
        }

        if let Some(path) = self.widget_to_path.get(&node_id) {
            if let Some(val) = defaults.lookup_path(path) {
                let s = value_to_string(val);
                return ResolvedText {
                    text: s,
                    is_name_derived: false,
                };
            }
        }

        // Name-derived labels are widget implementation names, not game content.
        // They must not appear in static renders (they would show internal
        // identifiers like "GIMBAL", "GROUP", "GUNS" instead of real data).
        ResolvedText { text: String::new(), is_name_derived: false }
    }
}

/// Strip common widget prefixes and split camelCase / snake_case into spaced words.
///
/// Reserved for future localisation lookup.  Not called in production renders
/// because widget names are implementation identifiers, not display text.
///
/// Examples:
/// - `text_NoTarget` → `No Target`
/// - `text_BodyValueFaction` → `Body Value Faction`
/// - `MachineTypeNameText` → `Machine Type Name`
/// - `txt_PresetName` → `Preset Name`
/// - `lbl_HeaderValue` → `Header Value`
#[allow(dead_code)]
fn derive_label_from_name(name: &str) -> String {
    let trimmed = name.trim();
    // Strip leading prefix segments that are pure widget-role tags.
    let stripped = strip_widget_prefix(trimmed);
    // Strip trailing "Text"/"Label" suffix.
    let stripped = strip_widget_suffix(stripped);
    if stripped.is_empty() {
        return String::new();
    }

    // Split on '_' and camelCase boundaries.
    let mut out = String::new();
    let mut prev_lower = false;
    let mut prev_alpha = false;
    for ch in stripped.chars() {
        if ch == '_' || ch == '-' {
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
            prev_lower = false;
            prev_alpha = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower {
            out.push(' ');
        } else if ch.is_ascii_digit() && prev_alpha {
            out.push(' ');
        }
        out.push(ch);
        prev_lower = ch.is_lowercase();
        prev_alpha = ch.is_alphabetic();
    }
    out.trim().to_owned()
}

#[allow(dead_code)]
fn strip_widget_prefix(s: &str) -> &str {
    const PREFIXES: &[&str] = &["text_", "txt_", "lbl_", "label_", "Text_", "Label_"];
    for p in PREFIXES {
        if let Some(rest) = s.strip_prefix(p) {
            return rest;
        }
    }
    s
}

#[allow(dead_code)]
fn strip_widget_suffix(s: &str) -> &str {
    const SUFFIXES: &[&str] = &["Text", "Label"];
    for sfx in SUFFIXES {
        if let Some(rest) = s.strip_suffix(sfx) {
            if !rest.is_empty() {
                return rest;
            }
        }
    }
    s
}

#[allow(dead_code)]
fn apply_case_modifier(s: &str, modifier: &str) -> String {
    match modifier {
        "Upper" | "AllCaps" => s.to_uppercase(),
        "Lower" => s.to_lowercase(),
        _ => s.to_owned(),
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
