//! P4K-backed localization fetcher for BuildingBlocks string resolution.
//!
//! Loads `Data\Localization\english\global.ini` from the P4K archive and
//! implements [`LocFetcher`] so that brand-applied `@KEY` strings can be
//! resolved to their display text during scene-graph construction.
//!
//! # INI format
//! The file is UTF-8 (with or without a leading BOM).  Each non-empty,
//! non-comment line has the form `key=value`.  The key is taken from the text
//! before the first `=`; the value is everything after it (including any
//! additional `=` signs).  Lines beginning with `;` or `#` are comments.
//! Keys are compared case-insensitively — both the stored key and the lookup
//! key are lowercased before comparison.
//!
//! # Availability note
//! `global.ini` is present in Star Citizen 4.x P4K archives at
//! `Data\Localization\english\global.ini`.  When absent, the returned map is
//! empty and the pipeline continues correctly: unresolved `@KEY` strings are
//! passed through as-is and resolved at render time by [`DefaultValueRegistry`].
//!
//! [`DefaultValueRegistry`]: crate::defaults::DefaultValueRegistry

use std::collections::HashMap;

use crate::bb_loc::LocFetcher;

/// Canonical P4K path for the English localization file.
///
/// The P4K archive stores paths with Windows-style backslash separators.
const GLOBAL_INI_PATH: &str = "Data\\Localization\\english\\global.ini";

/// Localization fetcher backed by a parsed `global.ini` map.
pub struct IniLocFetcher {
    entries: HashMap<String, String>,
}

impl LocFetcher for IniLocFetcher {
    fn fetch_loc(&self, key: &str) -> Option<String> {
        self.entries.get(&key.to_ascii_lowercase()).cloned()
    }
}

/// Load `global.ini` from the archive using `fetch`.
///
/// `fetch` receives the P4K path for `global.ini` and should return its raw
/// bytes, or `None` if the file is absent.  When absent (typical for SC 4.x),
/// an empty `IniLocFetcher` is returned without error.
pub fn load_global_ini(fetch: impl Fn(&str) -> Option<Vec<u8>>) -> IniLocFetcher {
    let bytes = match fetch(GLOBAL_INI_PATH) {
        Some(b) => b,
        None => {
            log::debug!("bb_loc_p4k: global.ini not found in archive; localization disabled");
            return IniLocFetcher { entries: HashMap::new() };
        }
    };
    IniLocFetcher {
        entries: parse_ini_bytes(&bytes),
    }
}

/// Parse INI bytes into a lowercased-key map.
///
/// Strips an optional UTF-8 BOM, skips blank lines and comment lines
/// (`;` or `#` prefix), and splits each remaining line on the first `=`.
///
/// # Plural-form suffix
/// Star Citizen `global.ini` uses `,P` (e.g. `engineering_ui_Item_Output,P=Output`)
/// for plural/parameter-form entries.  Both the raw key (`foo,p`) and the bare
/// base key (`foo`) are inserted, with the base form taking priority so that
/// simple `@foo` lookups always resolve.
pub fn parse_ini_bytes(bytes: &[u8]) -> HashMap<String, String> {
    // Strip UTF-8 BOM if present.
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);

    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("bb_loc_p4k: global.ini is not valid UTF-8: {e}");
            return HashMap::new();
        }
    };

    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_ascii_lowercase();
            let value = line[eq_pos + 1..].trim().to_string();
            if key.is_empty() {
                continue;
            }
            // Also insert under the bare key (without `,p` suffix) so that
            // lookups of the form `@engineering_ui_item_output` work even when
            // the file only has `engineering_ui_item_output,p=Output`.
            // `or_insert` preserves a previously inserted value (e.g. a
            // non-suffixed singular form comes first).
            if let Some(base) = key.strip_suffix(",p") {
                map.entry(base.to_string()).or_insert_with(|| value.clone());
            }
            map.insert(key, value);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bb_loc::resolve_loc_string;

    #[test]
    fn parse_simple_ini() {
        let ini = b"foo=bar\nbaz=qux\n";
        let map = parse_ini_bytes(ini);
        assert_eq!(map.get("foo"), Some(&"bar".to_string()));
        assert_eq!(map.get("baz"), Some(&"qux".to_string()));
    }

    #[test]
    fn bom_stripped() {
        // UTF-8 BOM followed by a key=value line.
        let ini = b"\xef\xbb\xbfhud_notarget=No Target\nother=val\n";
        let map = parse_ini_bytes(ini);
        assert_eq!(map.get("hud_notarget"), Some(&"No Target".to_string()));
        assert_eq!(map.get("other"), Some(&"val".to_string()));
    }

    #[test]
    fn value_with_equals() {
        // Only the first `=` splits key from value; the rest stays in the value.
        let ini = b"equation=a=b+c\n";
        let map = parse_ini_bytes(ini);
        assert_eq!(map.get("equation"), Some(&"a=b+c".to_string()));
    }

    #[test]
    fn comments_and_blanks_skipped() {
        let ini = b"; This is a comment\n# Also a comment\n\nkey=value\n";
        let map = parse_ini_bytes(ini);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn missing_key_returns_none() {
        let fetcher = load_global_ini(|_| None);
        assert_eq!(fetcher.fetch_loc("nonexistent"), None);
    }

    #[test]
    fn keys_are_case_insensitive() {
        let ini = b"HUD_NoTarget=No Target\n";
        let map = parse_ini_bytes(ini);
        // Keys are stored lowercased.
        assert_eq!(map.get("hud_notarget"), Some(&"No Target".to_string()));
    }

    #[test]
    fn full_round_trip_via_resolve_loc_string() {
        let ini = b"hud_notarget=No Target\n";
        let fetcher = IniLocFetcher {
            entries: parse_ini_bytes(ini),
        };
        // resolve_loc_string strips `@` then lowercases the key via fetch_loc.
        let result = resolve_loc_string("@hud_NoTarget", &[], &fetcher);
        assert_eq!(result, "No Target");
    }

    #[test]
    fn plural_suffix_base_key_resolved() {
        // Star Citizen global.ini uses `,P` for plural-form entries.
        // Looking up the bare key (no suffix) should return the value.
        let ini = b"engineering_ui_Item_Output,P=Output\n";
        let map = parse_ini_bytes(ini);
        // Bare key inserted automatically.
        assert_eq!(map.get("engineering_ui_item_output"), Some(&"Output".to_string()));
        // Original suffixed key also present.
        assert_eq!(map.get("engineering_ui_item_output,p"), Some(&"Output".to_string()));
    }

    #[test]
    fn plural_suffix_singular_wins_over_plural() {
        // If a singular form appears before the `,P` form, it should win.
        let ini = b"mykey=Singular\nmykey,P=Plural\n";
        let map = parse_ini_bytes(ini);
        assert_eq!(map.get("mykey"), Some(&"Singular".to_string()));
    }
}
