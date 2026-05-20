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
/// 2. Localized component parameter resolved via `_SynthLocalizedParam_` ops
///    (injected by `bb_resolve::inject_param_overrides`) + `BuildingBlocks_BindingsLocalizedField`
///    → looked up in [`DefaultValueRegistry`] via `lookup_localization`.
/// 3. Binding path resolved via `operations[]` → looked up in
///    [`DefaultValueRegistry`] → returned as string.
/// 4. Binding path found but no default → `"[<path>]"`.
/// 5. No binding → `""`.
pub struct BindingResolver {
    /// widget_node_id → binding path (e.g. "/vehicle/targetname")
    widget_to_path: HashMap<BbNodeId, String>,
    /// widget_node_id → localization key (e.g. "@hud_Pwr") resolved from
    /// `BuildingBlocks_BindingsLocalizedField` + `_SynthLocalizedParam_` ops.
    widget_to_loc_key: HashMap<BbNodeId, String>,
    /// widget_node_id → ordered candidate input operation pointers from *Field
    /// ops (highest-priority first: LocalizedField > StringField > others).
    widget_to_input_ptrs: HashMap<BbNodeId, Vec<BbNodeId>>,
    /// op pointer → raw operation object.
    ptr_to_op: HashMap<BbNodeId, serde_json::Value>,
    /// op pointer (Variable ops) → binding path.
    ptr_to_path: HashMap<BbNodeId, String>,
}

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
        let mut widget_to_input_prio_ptrs: HashMap<BbNodeId, Vec<(u8, BbNodeId)>> = HashMap::new();
        for op in operations {
            let type_str = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if type_str == "_SynthLocalizedWidget_" {
                let widget_ptr = op
                    .get("widget")
                    .and_then(|v| v.as_str())
                    .and_then(parse_points_to);
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
        }
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
                    let prio = if type_str.contains("LocalizedField") {
                        3
                    } else if type_str.contains("StringField") {
                        2
                    } else {
                        1
                    };
                    widget_to_input_prio_ptrs.entry(w).or_default().push((prio, inp));
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

        Self { widget_to_path, widget_to_loc_key, widget_to_input_ptrs, ptr_to_op, ptr_to_path }
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

        // `locString` is an alternative loc-key field on WidgetText nodes.
        // It typically contains `@LOC_EMPTY` (→ empty sentinel) but may carry
        // a real key, so we check it after `text` and skip the empty sentinel.
        if let Some(loc_key) = node_raw
            .get("locString")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.starts_with('@'))
        {
            if let Some(resolved) = defaults.lookup_localization(loc_key) {
                if !resolved.is_empty() {
                    return ResolvedText { text: resolved.to_owned(), is_name_derived: false };
                }
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
                if !resolved.is_empty() {
                    let case_modifier = node_raw
                        .get("labelProperties")
                        .and_then(|lp| lp.get("caseModifier"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    return ResolvedText {
                        text: apply_case_modifier(resolved, case_modifier),
                        is_name_derived: false,
                    };
                }
            }
            // Log every miss so B11 static-label sweep can collect new keys.
            log::debug!("bb_bindings: unresolved labelProperties.label key={loc_key:?}");
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

        // Resolve localized-operation graphs (LocalizationCombine,
        // LocalizedFromInteger, LocalizationFromIntegerSwitch, etc.).
        if let Some(input_ptrs) = self.widget_to_input_ptrs.get(&node_id) {
            for &input_ptr in input_ptrs {
                let mut seen = std::collections::HashSet::new();
                if std::env::var("BB_A3_TEXT_PROBE").as_deref() == Ok("1") {
                    let ty = self
                        .ptr_to_op
                        .get(&input_ptr)
                        .and_then(|op| op.get("_Type_"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("<none>");
                    let mut seen_num = std::collections::HashSet::new();
                    let num = self.eval_integer_ptr(input_ptr, defaults, &mut seen_num);
                    log::info!("A3-text-probe: node=ptr:{node_id} input=ptr:{input_ptr} type={ty}");
                    log::info!("A3-text-probe: node=ptr:{node_id} input_num={num:?}");
                }
                if let Some(s) = self.eval_localized_ptr(input_ptr, defaults, &mut seen) {
                    if !s.is_empty() {
                        let case_modifier = node_raw
                            .get("labelProperties")
                            .and_then(|lp| lp.get("caseModifier"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        return ResolvedText {
                            text: apply_case_modifier(&s, case_modifier),
                            is_name_derived: false,
                        };
                    }
                }
                let mut seen_num = std::collections::HashSet::new();
                if let Some(n) = self.eval_number_ptr(input_ptr, defaults, &mut seen_num) {
                    let text = if (n.fract()).abs() < 0.0001 {
                        (n as i64).to_string()
                    } else {
                        format!("{n:.2}")
                    };
                    if !text.is_empty() {
                        return ResolvedText { text, is_name_derived: false };
                    }
                }
            }
        }

        // Localized component parameter (e.g. annunciator chiclet labels).
        // Injected by bb_resolve::inject_param_overrides from paramInputValues.
        if let Some(loc_key) = self.widget_to_loc_key.get(&node_id) {
            if let Some(resolved) = defaults.lookup_localization(loc_key) {
                log::trace!("compose pass2: node={node_id} loc_key={loc_key:?} → {resolved:?}");
                let case_modifier = node_raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("caseModifier"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                return ResolvedText {
                    text: apply_case_modifier(resolved, case_modifier),
                    is_name_derived: false,
                };
            }
            // loc_key not found in registry → render it raw so the label is visible.
            let bare = loc_key.strip_prefix('@').unwrap_or(loc_key);
            if !bare.is_empty() {
                log::trace!("compose pass2: node={node_id} loc_key={loc_key:?} → bare fallback");
                return ResolvedText { text: bare.to_ascii_uppercase(), is_name_derived: false };
            }
        }

        // Name-derived labels are widget implementation names, not game content.
        // They must not appear in static renders (they would show internal
        // identifiers like "GIMBAL", "GROUP", "GUNS" instead of real data).
        ResolvedText { text: String::new(), is_name_derived: false }
    }

    fn eval_localized_ptr(
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
            "BindingsOperations_LocalizationCombine" => {
                let value_key = op.get("value").and_then(|v| v.as_str()).unwrap_or("");
                let base = defaults.lookup_localization(value_key).unwrap_or(value_key);
                let left_ptr = op.get("inputL").and_then(|v| v.as_str()).and_then(parse_points_to);
                let right_ptr = op.get("inputR").and_then(|v| v.as_str()).and_then(parse_points_to);
                let left = left_ptr
                    .and_then(|p| self.eval_localized_ptr(p, defaults, seen))
                    .or_else(|| left_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, seen)).map(|v| v.to_string()))
                    .unwrap_or_default();
                let right = right_ptr
                    .and_then(|p| self.eval_localized_ptr(p, defaults, seen))
                    .or_else(|| right_ptr.and_then(|p| self.eval_integer_ptr(p, defaults, seen)).map(|v| v.to_string()))
                    .unwrap_or_default();
                let mut out = base.to_string();
                if out.contains("%d") {
                    out = out.replacen("%d", if !right.is_empty() { &right } else { &left }, 1);
                } else if out.contains("%s") {
                    out = out.replacen("%s", if !right.is_empty() { &right } else { &left }, 1);
                } else if left.is_empty() && right.is_empty() {
                    // keep base
                } else if left.is_empty() {
                    out = format!("{out}{right}");
                } else if right.is_empty() {
                    out = format!("{left}{out}");
                } else {
                    out = format!("{left}{out}{right}");
                }
                Some(out)
            }
            "BuildingBlocks_BindingsLocalizedFromInteger" => self
                .eval_integer_ptr_from_field(op.get("input").and_then(|v| v.as_str()), defaults, seen)
                .map(|v| v.to_string()),
            "BuildingBlocks_BindingsLocalizationFromIntegerSwitch" => {
                let input = self.eval_integer_ptr_from_field(op.get("input").and_then(|v| v.as_str()), defaults, seen)?;
                let values = op.get("values").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let key = values
                    .iter()
                    .find_map(|pair| {
                        let first = pair.get("first").and_then(|v| v.as_i64())?;
                        if first == input {
                            pair.get("second").and_then(|v| v.as_str())
                        } else {
                            None
                        }
                    })
                    .or_else(|| op.get("defaultValue").and_then(|v| v.as_str()))
                    .unwrap_or("");
                if key.is_empty() {
                    return None;
                }
                Some(defaults.lookup_localization(key).unwrap_or(key).to_string())
            }
            "BuildingBlocks_BindingsLocalizedVariable" => {
                let path = self.ptr_to_path.get(&ptr)?;
                let val = defaults.lookup_path(path)?;
                Some(value_to_string(val))
            }
            "BuildingBlocks_BindingsLocalizedComponentParameter" => {
                let key = op.get("defaultValue").and_then(|v| v.as_str()).unwrap_or("");
                if key.is_empty() {
                    return None;
                }
                Some(defaults.lookup_localization(key).unwrap_or(key).to_string())
            }
            _ => None,
        }
    }

    fn eval_integer_ptr_from_field(
        &self,
        field: Option<&str>,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<i64> {
        let ptr = field.and_then(parse_points_to)?;
        self.eval_integer_ptr(ptr, defaults, seen)
    }

    fn eval_integer_ptr(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<i64> {
        let op = self.ptr_to_op.get(&ptr)?;
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "BuildingBlocks_BindingsIntegerVariable" => {
                let path = self.ptr_to_path.get(&ptr)?;
                let val = defaults.lookup_path(path)?;
                match val {
                    Value::Int(i) => Some(*i as i64),
                    Value::Float(f) => Some(*f as i64),
                    Value::Bool(b) => Some(if *b { 1 } else { 0 }),
                    Value::Str(s) | Value::Guid(s) => s.parse::<i64>().ok(),
                }
            }
            "BuildingBlocks_BindingsIntegerArithmatic" => {
                let kind = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let amount = op.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
                let l = self
                    .eval_integer_ptr_from_field(op.get("inputL").and_then(|v| v.as_str()), defaults, seen)
                    .or_else(|| self.eval_integer_ptr_from_field(op.get("input").and_then(|v| v.as_str()), defaults, seen))
                    .unwrap_or(0);
                let r = self
                    .eval_integer_ptr_from_field(op.get("inputR").and_then(|v| v.as_str()), defaults, seen)
                    .or_else(|| self.eval_integer_ptr_from_field(op.get("inputB").and_then(|v| v.as_str()), defaults, seen))
                    .unwrap_or(amount);
                Some(match kind {
                    "Add" => l + amount,
                    "Min" => l.min(r),
                    "Max" => l.max(r),
                    "Sub" => l - r,
                    _ => l,
                })
            }
            _ => None,
        }
    }

    fn eval_number_ptr(
        &self,
        ptr: BbNodeId,
        defaults: &DefaultValueRegistry,
        seen: &mut std::collections::HashSet<BbNodeId>,
    ) -> Option<f64> {
        let op = self.ptr_to_op.get(&ptr)?;
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "BuildingBlocks_BindingsNumberVariable" => {
                let path = self.ptr_to_path.get(&ptr)?;
                let val = defaults.lookup_path(path)?;
                match val {
                    Value::Int(i) => Some(*i as f64),
                    Value::Float(f) => Some(*f as f64),
                    Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                    Value::Str(s) | Value::Guid(s) => s.parse::<f64>().ok(),
                }
            }
            "BuildingBlocks_BindingsNumberFromInteger" => {
                let inp = op.get("input").and_then(|v| v.as_str()).and_then(parse_points_to)?;
                self.eval_integer_ptr(inp, defaults, seen).map(|v| v as f64)
            }
            "BuildingBlocks_BindingsNumberArithmatic" => {
                let kind = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let amount = op.get("amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
                let input = op.get("input").and_then(|v| v.as_str()).and_then(parse_points_to);
                let input_b = op.get("inputB").and_then(|v| v.as_str()).and_then(parse_points_to);
                let a = input.and_then(|p| self.eval_number_ptr(p, defaults, seen)).unwrap_or(0.0);
                let b = input_b
                    .and_then(|p| self.eval_number_ptr(p, defaults, seen))
                    .unwrap_or(amount);
                Some(match kind {
                    "Add" => a + amount,
                    "Sub" => a - b,
                    "Mul" => a * b,
                    "Div" => {
                        if b.abs() > f64::EPSILON { a / b } else { 0.0 }
                    }
                    _ => a,
                })
            }
            _ => self.eval_integer_ptr(ptr, defaults, seen).map(|v| v as f64),
        }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn resolver() -> BindingResolver {
        BindingResolver {
            widget_to_path: Default::default(),
            widget_to_loc_key: Default::default(),
            widget_to_input_ptrs: Default::default(),
            ptr_to_op: Default::default(),
            ptr_to_path: Default::default(),
        }
    }

    #[test]
    fn text_at_key_resolves_via_loc_map() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("foo".to_string(), "POWER MANAGEMENT".to_string())].into());

        let raw = json!({"text": "@foo"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "POWER MANAGEMENT");
    }

    #[test]
    fn text_literal_returned_as_is() {
        let resolver = resolver();
        let defaults = DefaultValueRegistry::default();

        let raw = json!({"text": "Hello World"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "Hello World");
    }

    #[test]
    fn loc_string_field_resolved() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("mykey".to_string(), "My Label".to_string())].into());

        // `locString` field carries the loc key; `text` is absent.
        let raw = json!({"locString": "@mykey"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "My Label");
    }

    #[test]
    fn label_properties_case_modifier_upper_applied() {
        let resolver = resolver();
        let mut defaults = DefaultValueRegistry::default();
        defaults.merge_localization([("info_kiosks_logoscreen_001".to_string(), "Touch to start".to_string())].into());

        let raw = json!({
            "labelProperties": {
                "label": "@Info_Kiosks_LogoScreen_001",
                "caseModifier": "Upper"
            }
        });
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "TOUCH TO START");
    }

    #[test]
    fn loc_empty_sentinel_skipped() {
        let resolver = resolver();
        let defaults = DefaultValueRegistry::default();

        // @LOC_EMPTY resolves to "" (suppressed sentinel) — must not emit that.
        let raw = json!({"locString": "@LOC_EMPTY"});
        let result = resolver.resolve_text_detailed(0, &raw, &defaults);
        assert_eq!(result.text, "");
    }
}
