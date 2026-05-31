use crate::bb_scene::BbNodeId;
use crate::defaults::DefaultValueRegistry;

use super::util::{
    apply_case_modifier,
    case_modifier_from_raw,
    number_to_compact_string,
    value_to_string,
};
use super::{BindingResolver, ResolvedText};

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

    /// Resolve non-text string bindings for a widget (e.g. ImagePath fields).
    pub fn resolve_string_binding(&self, node_id: BbNodeId) -> Option<&str> {
        self.widget_to_string.get(&node_id).map(|s| s.as_str())
    }

    /// Resolve a localized/string display value for a specific widget field
    /// (for example `ParamInput1` on label-caption components).
    pub fn resolve_field_text(
        &self,
        node_id: BbNodeId,
        field: &str,
        defaults: &DefaultValueRegistry,
    ) -> Option<String> {
        let probe = std::env::var("BB_PRIMARY_TAG_PROBE").as_deref() == Ok("1")
            && field == "PrimaryStateTag";
        let input_ptrs = self
            .widget_field_to_input_ptrs
            .get(&(node_id, field.to_string()));
        let Some(input_ptrs) = input_ptrs else {
            if probe {
                log::info!(
                    "primary-tag-probe: resolve_field_text node=ptr:{node_id} field={field} missing-mapping"
                );
            }
            return None;
        };
        if probe {
            log::info!(
                "primary-tag-probe: resolve_field_text node=ptr:{node_id} field={field} inputs={input_ptrs:?}"
            );
        }

        for &input_ptr in input_ptrs {
            let mut seen_str = std::collections::HashSet::new();
            if let Some(s) = self.eval_string_ptr(input_ptr, defaults, &mut seen_str)
                && !s.is_empty()
            {
                let cleaned = s.replace("//", "/");
                if probe {
                    log::info!(
                        "primary-tag-probe: resolve_field_text node=ptr:{node_id} string-result={cleaned:?}"
                    );
                }
                return Some(cleaned);
            }

            let mut seen = std::collections::HashSet::new();
            if let Some(s) = self.eval_localized_ptr(input_ptr, defaults, &mut seen)
                && !s.is_empty()
            {
                let cleaned = s.replace("//", "/");
                if probe {
                    log::info!(
                        "primary-tag-probe: resolve_field_text node=ptr:{node_id} localized-result={cleaned:?}"
                    );
                }
                return Some(cleaned);
            }

            let mut seen_num = std::collections::HashSet::new();
            if let Some(n) = self.eval_number_ptr(input_ptr, defaults, &mut seen_num) {
                if probe {
                    log::info!(
                        "primary-tag-probe: resolve_field_text node=ptr:{node_id} number-result={n}"
                    );
                }
                return Some(number_to_compact_string(n));
            }

            let mut seen_int = std::collections::HashSet::new();
            if let Some(i) = self.eval_integer_ptr(input_ptr, defaults, &mut seen_int) {
                if probe {
                    log::info!(
                        "primary-tag-probe: resolve_field_text node=ptr:{node_id} integer-result={i}"
                    );
                }
                return Some(i.to_string());
            }
        }

        if probe {
            log::info!(
                "primary-tag-probe: resolve_field_text node=ptr:{node_id} field={field} no-result"
            );
        }

        None
    }

    /// Resolve a numeric value for a specific widget field.
    pub fn resolve_field_number(
        &self,
        node_id: BbNodeId,
        field: &str,
        defaults: &DefaultValueRegistry,
    ) -> Option<f64> {
        let input_ptrs = self
            .widget_field_to_input_ptrs
            .get(&(node_id, field.to_string()))?;
        for &input_ptr in input_ptrs {
            let mut seen_num = std::collections::HashSet::new();
            if let Some(n) = self.eval_number_ptr(input_ptr, defaults, &mut seen_num) {
                return Some(n);
            }
            let mut seen_int = std::collections::HashSet::new();
            if let Some(i) = self.eval_integer_ptr(input_ptr, defaults, &mut seen_int) {
                return Some(i as f64);
            }
        }
        None
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
                            return ResolvedText {
                                text: apply_case_modifier(resolved, case_modifier_from_raw(node_raw)),
                                is_name_derived: false,
                            };
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
                if !resolved.is_empty() {
                    return ResolvedText {
                        text: apply_case_modifier(resolved, case_modifier_from_raw(node_raw)),
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
                    if matches!(
                        ty,
                        "BuildingBlocks_BindingsLocalizedFromBoolean"
                            | "BuildingBlocks_BindingsTagFromBoolean"
                            | "BuildingBlocks_BindingsBooleanComponentParameter"
                    ) {
                        if let Some(op) = self.ptr_to_op.get(&input_ptr) {
                            log::info!("A3-text-probe: node=ptr:{node_id} input_op={op}");
                        }
                    }
                }
                if let Some(s) = self.eval_localized_ptr(input_ptr, defaults, &mut seen) {
                    if !s.is_empty() {
                        return ResolvedText {
                            text: apply_case_modifier(&s, case_modifier_from_raw(node_raw)),
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

        // `locString` is an alternative loc-key field on WidgetText nodes.
        // It typically contains `@LOC_EMPTY` (→ empty sentinel) but may carry
        // a real key. Evaluate this after binding operations so field bindings
        // (e.g. LocalizedFromIntegerSwitch) can override authored fallback text.
        if let Some(loc_key) = node_raw
            .get("locString")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.starts_with('@'))
        {
            if let Some(resolved) = defaults.lookup_localization(loc_key) {
                if !resolved.is_empty() {
                    return ResolvedText {
                        text: apply_case_modifier(resolved, case_modifier_from_raw(node_raw)),
                        is_name_derived: false,
                    };
                }
            }
        }

        // Localized component parameter (e.g. annunciator chiclet labels).
        // Injected by bb_resolve::inject_param_overrides from paramInputValues.
        if let Some(loc_key) = self.widget_to_loc_key.get(&node_id) {
            if let Some(resolved) = defaults.lookup_localization(loc_key) {
                log::trace!("compose pass2: node={node_id} loc_key={loc_key:?} → {resolved:?}");
                return ResolvedText {
                    text: apply_case_modifier(resolved, case_modifier_from_raw(node_raw)),
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
}
