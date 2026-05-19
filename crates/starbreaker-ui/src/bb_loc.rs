//! Localization string resolution for BuildingBlocks canvases.
//!
//! Handles `@key` prefix localization lookups with printf-style parameter
//! substitution (the `,P=variable` suffix notation used by DataCore).
//!
//! # Example
//! ```no_run
//! # use starbreaker_ui::bb_loc::*;
//! # struct MockFetcher;
//! # impl LocFetcher for MockFetcher {
//! #     fn fetch_loc(&self, key: &str) -> Option<String> {
//! #         if key == "Med_T_Tier" {
//! #             Some("T%d".to_string())
//! #         } else { None }
//! #     }
//! # }
//! # let fetcher = MockFetcher;
//! # let param_input_values = vec![serde_json::json!({"name": "T", "defaultValue": 3})];
//! let result = resolve_loc_string("@Med_T_Tier,P=T", &param_input_values, &fetcher);
//! assert_eq!(result, "T3");
//! ```

/// Localization fetcher trait.
///
/// Implementers should look up `key` in a localization database (e.g.
/// `Data\Localization\english\global.ini` from the P4K archive) and return the
/// English-language value, or `None` if the key is absent.
pub trait LocFetcher {
    fn fetch_loc(&self, key: &str) -> Option<String>;
}

/// Resolve a BuildingBlocks localization string.
///
/// # Algorithm
/// 1. If `raw_key` does NOT start with `@`, return it as-is (literal text).
/// 2. Strip the leading `@` and parse any `,P=variable` suffixes.  These
///    suffixes specify printf-style parameter names whose values are drawn from
///    `param_input_values`.
/// 3. Look up the bare key (without `,P` suffixes) via `fetcher`.  On `None`,
///    return `raw_key` unmodified (missing keys stay visible in PNG output for
///    debugging).
/// 4. Substitute each `%ls`, `%s`, `%d`, `%u` placeholder in the resolved
///    localization string with the corresponding parameter value (matched by
///    name from the `,P` list, or by ordinal position if no name match).
/// 5. Return the final substituted string.
///
/// # Example `,P` syntax
/// - `Med_T_Tier,P=T` → look up `Med_T_Tier`, substitute `%d` with the value
///   of the `paramInputValues` entry whose `name` field is `"T"`.
/// - `Multi,P=X,P=Y` → multiple parameters (though this is rare in real data).
///
/// # `param_input_values` shape
/// Each element should be a JSON object like:
/// ```json
/// {
///   "name": "T",
///   "defaultValue": 3
/// }
/// ```
/// The `defaultValue` is used for substitution; `name` is the matching key.
pub fn resolve_loc_string(
    raw_key: &str,
    param_input_values: &[serde_json::Value],
    fetcher: &dyn LocFetcher,
) -> String {
    // Step 1: passthrough if not a loc key.
    if !raw_key.starts_with('@') {
        return raw_key.to_string();
    }

    // Step 2: parse `,P=` suffix list.
    let stripped = &raw_key[1..]; // Remove leading `@`.
    let (bare_key, param_names) = parse_param_suffix(stripped);

    // Step 3: fetch localization value.
    let template = match fetcher.fetch_loc(bare_key) {
        Some(s) => s,
        None => {
            log::debug!("bb_loc: missing localization key: {}", bare_key);
            return raw_key.to_string();
        }
    };

    // Step 4: substitute parameters.
    if param_names.is_empty() {
        // No `,P` suffix → return template as-is (no substitution needed).
        return template;
    }

    // Build a lookup map from param names to their default values.
    let mut param_map: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for piv in param_input_values {
        if let Some(name) = piv.get("name").and_then(|v| v.as_str()) {
            if let Some(default_val) = piv.get("defaultValue") {
                let val_str = value_to_string(default_val);
                param_map.insert(name, val_str);
            }
        }
    }

    // Perform printf-style substitution.  Replace each `%ls`, `%s`, `%d`, `%u`
    // placeholder with the corresponding parameter value from the `,P` list.
    substitute_printf_params(&template, &param_names, &param_map, param_input_values)
}

/// Parse the `,P=variable` suffix into a (bare_key, variable_names) tuple.
///
/// # Examples
/// - `"Med_T_Tier,P=T"` → `("Med_T_Tier", vec!["T"])`
/// - `"Key,P=X,P=Y"` → `("Key", vec!["X", "Y"])`
/// - `"NoSuffix"` → `("NoSuffix", vec![])`
fn parse_param_suffix(key_with_suffix: &str) -> (&str, Vec<&str>) {
    if !key_with_suffix.contains(",P=") {
        return (key_with_suffix, vec![]);
    }

    let parts: Vec<&str> = key_with_suffix.split(",P=").collect();
    let bare_key = parts[0];
    let param_names = parts[1..].to_vec();
    (bare_key, param_names)
}

/// Convert a JSON value to a string suitable for printf substitution.
fn value_to_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Number(n) => {
            // Prefer integer format if it's an i64 with no fractional part.
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                n.to_string()
            }
        }
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => val.to_string(),
    }
}

/// Substitute printf-style placeholders in `template` with values from
/// `param_map` / `param_input_values`.
///
/// Supports `%ls`, `%s`, `%d`, `%u`.  Each placeholder is replaced by the
/// corresponding parameter value in order from `param_names`.
fn substitute_printf_params(
    template: &str,
    param_names: &[&str],
    param_map: &std::collections::HashMap<&str, String>,
    param_input_values: &[serde_json::Value],
) -> String {
    let mut result = template.to_string();
    let mut param_idx = 0;

    // Regex-like approach: scan for %ls, %s, %d, %u and replace in order.
    // We'll do multiple passes since we need to preserve order.
    let patterns = ["%ls", "%s", "%d", "%u"];

    for pattern in &patterns {
        while let Some(_pos) = result.find(pattern) {
            if param_idx >= param_names.len() {
                // No more parameters to substitute; leave pattern as-is.
                log::debug!(
                    "bb_loc: not enough params for pattern {} in template '{}'",
                    pattern,
                    template
                );
                break;
            }

            let param_name = param_names[param_idx];
            let replacement = param_map
                .get(param_name)
                .cloned()
                .or_else(|| {
                    // Fallback: use ordinal index from param_input_values.
                    param_input_values.get(param_idx).and_then(|v| {
                        v.get("defaultValue")
                            .map(|dv| value_to_string(dv))
                    })
                })
                .unwrap_or_else(|| {
                    log::debug!(
                        "bb_loc: missing param value for '{}' in template '{}'",
                        param_name,
                        template
                    );
                    format!("?{}", param_name)
                });

            result = result.replacen(pattern, &replacement, 1);
            param_idx += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct MockLocFetcher {
        entries: std::collections::HashMap<String, String>,
    }

    impl LocFetcher for MockLocFetcher {
        fn fetch_loc(&self, key: &str) -> Option<String> {
            self.entries.get(key).cloned()
        }
    }

    #[test]
    fn test_passthrough_no_at_prefix() {
        let fetcher = MockLocFetcher {
            entries: std::collections::HashMap::new(),
        };
        let result = resolve_loc_string("MEDICAL ASSISTANT", &[], &fetcher);
        assert_eq!(result, "MEDICAL ASSISTANT");
    }

    #[test]
    fn test_missing_key_fallback() {
        let fetcher = MockLocFetcher {
            entries: std::collections::HashMap::new(),
        };
        let result = resolve_loc_string("@Unknown_Key", &[], &fetcher);
        assert_eq!(result, "@Unknown_Key");
    }

    #[test]
    fn test_simple_lookup() {
        let mut entries = std::collections::HashMap::new();
        entries.insert("med_Header_Title".to_string(), "Medical Assistant".to_string());
        let fetcher = MockLocFetcher { entries };
        let result = resolve_loc_string("@med_Header_Title", &[], &fetcher);
        assert_eq!(result, "Medical Assistant");
    }

    #[test]
    fn test_param_substitution_numeric() {
        let mut entries = std::collections::HashMap::new();
        entries.insert("Med_T_Tier".to_string(), "T%d".to_string());
        let fetcher = MockLocFetcher { entries };

        let params = vec![json!({
            "name": "T",
            "defaultValue": 3
        })];

        let result = resolve_loc_string("@Med_T_Tier,P=T", &params, &fetcher);
        assert_eq!(result, "T3");
    }

    #[test]
    fn test_param_substitution_string() {
        let mut entries = std::collections::HashMap::new();
        entries.insert("Greeting".to_string(), "Hello %s".to_string());
        let fetcher = MockLocFetcher { entries };

        let params = vec![json!({
            "name": "Name",
            "defaultValue": "World"
        })];

        let result = resolve_loc_string("@Greeting,P=Name", &params, &fetcher);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_param_substitution_multiple() {
        let mut entries = std::collections::HashMap::new();
        entries.insert("Multi".to_string(), "%s has %d credits".to_string());
        let fetcher = MockLocFetcher { entries };

        let params = vec![
            json!({
                "name": "Player",
                "defaultValue": "Alice"
            }),
            json!({
                "name": "Credits",
                "defaultValue": 1500
            }),
        ];

        let result = resolve_loc_string("@Multi,P=Player,P=Credits", &params, &fetcher);
        assert_eq!(result, "Alice has 1500 credits");
    }

    #[test]
    fn test_param_ordinal_fallback() {
        let mut entries = std::collections::HashMap::new();
        entries.insert("Test".to_string(), "Value: %d".to_string());
        let fetcher = MockLocFetcher { entries };

        // param without matching name → falls back to ordinal index
        let params = vec![json!({
            "name": "UnrelatedName",
            "defaultValue": 42
        })];

        let result = resolve_loc_string("@Test,P=T", &params, &fetcher);
        // Should still substitute using ordinal fallback
        assert_eq!(result, "Value: 42");
    }

    #[test]
    fn test_no_params_needed() {
        let mut entries = std::collections::HashMap::new();
        entries.insert("Static".to_string(), "No substitution".to_string());
        let fetcher = MockLocFetcher { entries };

        let result = resolve_loc_string("@Static", &[], &fetcher);
        assert_eq!(result, "No substitution");
    }

    #[test]
    fn test_parse_param_suffix() {
        let (key, params) = parse_param_suffix("Med_T_Tier,P=T");
        assert_eq!(key, "Med_T_Tier");
        assert_eq!(params, vec!["T"]);

        let (key, params) = parse_param_suffix("Multi,P=X,P=Y");
        assert_eq!(key, "Multi");
        assert_eq!(params, vec!["X", "Y"]);

        let (key, params) = parse_param_suffix("NoSuffix");
        assert_eq!(key, "NoSuffix");
        assert_eq!(params.len(), 0);
    }
}
