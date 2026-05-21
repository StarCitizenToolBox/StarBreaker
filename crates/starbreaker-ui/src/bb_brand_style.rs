//! Brand style resolution for BuildingBlocks canvases.
//!
//! Handles per-canvas brand override (IC_* family) and ship-manufacturer-based
//! brand selection (MC_* family) with generic fallback support.

use crate::pipeline::extract_record_name;

/// Canvas family classification by record-name prefix.
///
/// Different families follow different brand-resolution rules:
/// - `MC_*` → ship manufacturer brand (e.g. `s_drak` for Drake ships)
/// - `IC_*` → per-canvas brand override (e.g. `s_bioc` for medical screens)
/// - `M_*` → MFD-root composites (ship manufacturer brand)
/// - Fluff modular → passive ambient (template + override, no brand styles)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasFamily {
    /// Master Canvas (cockpit MFD, brand-styled per ship).
    Mfd,
    /// Fluff Modular (passive ambient; template + override).
    FluffModular,
    /// Interactive Canvas (terminal w/ user input; shared PU asset,
    /// brand-styled per *canvas*, not per ship).
    InteractiveCanvas,
    /// MFD-root composite canvases (top-level for cockpit MFDs; embeds
    /// `MC_*` per-manufacturer sub-canvases).
    MfdRoot,
    /// Any other canvas type.
    Other,
}

/// Classify a canvas by its record name prefix.
///
/// Case-insensitive prefix matching on the `_RecordName_` field.
pub fn classify_canvas_family(record_name: &str) -> CanvasFamily {
    let lower = record_name.to_ascii_lowercase();
    
    // MC_* but NOT M_* alone (M_ would match MC_ too, so check MC_ first)
    if lower.starts_with("mc_") {
        return CanvasFamily::Mfd;
    }
    
    // M_* (MFD root) after MC_ check
    if lower.starts_with("m_") {
        return CanvasFamily::MfdRoot;
    }
    
    // Fluff modular families
    if lower.starts_with("fms_") || lower.starts_with("fmp_") 
        || lower.starts_with("fmt_") || lower.starts_with("f_") {
        return CanvasFamily::FluffModular;
    }
    
    // Interactive Canvas
    if lower.starts_with("ic_") {
        return CanvasFamily::InteractiveCanvas;
    }
    
    CanvasFamily::Other
}

/// Borrowed view into a selected brand-style entry.
///
/// Provides access to the entries array without copying.
#[derive(Debug)]
pub struct BrandStyle<'a> {
    /// Basename of brandIdentifier file://… path, e.g. "s_drak".
    pub identifier: String,
    /// Reference to the entries array for this brand.
    pub entries: &'a [serde_json::Value],
    /// Reference to the raw brand-styles entry JSON.
    pub raw: &'a serde_json::Value,
}

/// Resolve the active brand-style entry for a canvas record.
///
/// Algorithm (R1 spec):
/// 1. Read `record_value.brandStyles` (array). If absent or empty → None.
/// 2. **Per-canvas brand override (IC_* rule):** if the canvas has exactly ONE
///    brandStyles entry AND `record_value._RecordName_` starts case-insensitively
///    with `IC_`, return that entry regardless of ship manufacturer.
///    (This is finding #6 — `IC_Med_MedicalCommon_A_*` is BioCorp on every ship.)
/// 3. Otherwise, look for the brandStyles entry whose `brandIdentifier` file-path
///    basename starts with `s_<manufacturer>` (case-insensitively). Manufacturer
///    is the lowercase `ship_manufacturer_id` parameter. This matches both exact
///    manufacturer brand files (e.g. `s_drak.json`) and suffixed variants
///    (e.g. `s_drak_hud.json`).
/// 4. If no match and ship_manufacturer_id is Some, fall back to the brandStyles
///    entry whose basename starts with `gen_` or `s_default_`
///    (manufacturer-agnostic fallback).
/// 5. If still nothing, return None.
pub fn resolve_brand_style<'a>(
    record_or_value: &'a serde_json::Value,
    ship_manufacturer_id: Option<&str>,
    preferred_identifier: Option<&str>,
) -> Option<BrandStyle<'a>> {
    let record_value = record_or_value
        .get("_RecordValue_")
        .unwrap_or(record_or_value);

    let brand_styles = record_value
        .get("brandStyles")
        .and_then(|v| v.as_array())?;
    
    if brand_styles.is_empty() {
        return None;
    }
    
    // Step 2: Per-canvas brand override for IC_* canvases with exactly one brand entry
    let record_name = record_or_value
        .get("_RecordName_")
        .and_then(|v| v.as_str())
        .or_else(|| {
            record_value
                .get("_RecordName_")
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    
    let family = classify_canvas_family(record_name);
    if family == CanvasFamily::InteractiveCanvas && brand_styles.len() == 1 {
        return build_brand_style(&brand_styles[0]);
    }
    
    // Step 3: Ship-manufacturer-based brand selection
    if let Some(mfr) = ship_manufacturer_id {
        let prefix = format!("s_{}", mfr.to_ascii_lowercase());
        for entry in brand_styles {
            if let Some(basename) = brand_identifier_basename(entry) {
                let lower_base = basename.to_ascii_lowercase();
                // Match if basename starts with "s_<manufacturer>" (e.g. s_drak, s_drak_hud, etc.)
                if lower_base.starts_with(&prefix) {
                    return build_brand_style(entry);
                }
            }
        }
        
        // Step 4: Generic fallback for known manufacturers
        for entry in brand_styles {
            if let Some(basename) = brand_identifier_basename(entry) {
                let lower_base = basename.to_ascii_lowercase();
                if lower_base.starts_with("gen_") || lower_base.starts_with("s_default_") {
                    return build_brand_style(entry);
                }
            }
        }
    }

    if let Some(preferred) = preferred_identifier {
        let preferred = preferred.to_ascii_lowercase();
        for entry in brand_styles {
            if let Some(basename) = brand_identifier_basename(entry) {
                if basename.to_ascii_lowercase() == preferred {
                    return build_brand_style(entry);
                }
            }
        }
    }
    
    None
}

/// Helper to construct a `BrandStyle` from a brand-styles entry.
fn build_brand_style(entry: &serde_json::Value) -> Option<BrandStyle<'_>> {
    let identifier = brand_identifier_basename(entry)?;
    let entries_arr = entry.get("entries").and_then(|v| v.as_array())?;
    
    Some(BrandStyle {
        identifier,
        entries: entries_arr.as_slice(),
        raw: entry,
    })
}

/// Extract and lower-case the basename of a `brandIdentifier` field.
///
/// The `brandIdentifier` is a `_PointsTo_` ref with `value: "file://path/to/s_drak.json"`.
/// Returns the basename without the directory path or `.json` extension.
pub fn brand_identifier_basename(brand_styles_entry: &serde_json::Value) -> Option<String> {
    let brand_id = brand_styles_entry
        .get("brandIdentifier")
        .and_then(|v| v.as_str())?;
    
    Some(extract_record_name(brand_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_classify_canvas_family() {
        assert_eq!(classify_canvas_family("MC_S_Target_Master"), CanvasFamily::Mfd);
        assert_eq!(classify_canvas_family("mc_s_self"), CanvasFamily::Mfd);
        assert_eq!(classify_canvas_family("M_MFD_Screen"), CanvasFamily::MfdRoot);
        assert_eq!(classify_canvas_family("m_physical_screen"), CanvasFamily::MfdRoot);
        assert_eq!(classify_canvas_family("IC_Med_MedicalCommon_A_MainMenu"), CanvasFamily::InteractiveCanvas);
        assert_eq!(classify_canvas_family("ic_door_standard"), CanvasFamily::InteractiveCanvas);
        assert_eq!(classify_canvas_family("FMS_Test"), CanvasFamily::FluffModular);
        assert_eq!(classify_canvas_family("FMP_Test"), CanvasFamily::FluffModular);
        assert_eq!(classify_canvas_family("FMT_Test"), CanvasFamily::FluffModular);
        assert_eq!(classify_canvas_family("F_Test"), CanvasFamily::FluffModular);
        assert_eq!(classify_canvas_family("gen_mc_s_target"), CanvasFamily::Other);
        assert_eq!(classify_canvas_family("Some_Random_Canvas"), CanvasFamily::Other);
    }

    #[test]
    fn test_resolve_brand_style_ship_manufacturer() {
        // MC_S_Self_Master with two brand entries (s_drak + s_rsi)
        let record = json!({
            "_RecordName_": "MC_S_Self_Master",
            "brandStyles": [
                {
                    "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_drak.json",
                    "entries": [
                        {"modifiers": [{"field": "FillColor", "value": "#FF6600"}]}
                    ]
                },
                {
                    "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_rsi.json",
                    "entries": [
                        {"modifiers": [{"field": "FillColor", "value": "#0066FF"}]}
                    ]
                }
            ]
        });

        let record_value = record.get("_RecordValue_").unwrap_or(&record);

        // Drake manufacturer picks drak
        let result = resolve_brand_style(record_value, Some("drak"), None);
        assert!(result.is_some());
        let brand = result.unwrap();
        assert_eq!(brand.identifier, "s_drak");
        assert_eq!(brand.entries.len(), 1);

        // RSI manufacturer picks rsi
        let result = resolve_brand_style(record_value, Some("rsi"), None);
        assert!(result.is_some());
        let brand = result.unwrap();
        assert_eq!(brand.identifier, "s_rsi");

        // Unknown manufacturer with no generic fallback → None
        let result = resolve_brand_style(record_value, Some("unknown"), None);
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_brand_style_ic_override() {
        // IC_Med_MedicalCommon_A_MainMenu with one brandStyles entry (s_bioc)
        let record = json!({
            "_RecordName_": "IC_Med_MedicalCommon_A_MainMenu",
            "brandStyles": [
                {
                    "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_bioc.json",
                    "entries": [
                        {"modifiers": [{"field": "ImagePath", "value": "i_med_bioc_MenuOption_A.dds"}]}
                    ]
                }
            ]
        });

        let record_value = record.get("_RecordValue_").unwrap_or(&record);

        // Drake ship still gets BioCorp brand (per-canvas override)
        let result = resolve_brand_style(record_value, Some("drak"), None);
        assert!(result.is_some());
        let brand = result.unwrap();
        assert_eq!(brand.identifier, "s_bioc");

        // Aegis ship also gets BioCorp brand
        let result = resolve_brand_style(record_value, Some("aegs"), None);
        assert!(result.is_some());
        let brand = result.unwrap();
        assert_eq!(brand.identifier, "s_bioc");
    }

    #[test]
    fn test_resolve_brand_style_ic_override_wrapped_record() {
        let record = json!({
            "_RecordName_": "IC_Med_MedicalCommon_A_Footer",
            "_RecordValue_": {
                "brandStyles": [
                    {
                        "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_bioc.json",
                        "entries": [
                            {"modifiers": [{"field": "ImagePath", "value": "i_med_bioc_bottom-bar.dds"}]}
                        ]
                    },
                    {
                        "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_rsi.json",
                        "entries": []
                    }
                ]
            }
        });

        let result = resolve_brand_style(&record, Some("drak"), Some("s_bioc"));
        assert!(result.is_some());
        let brand = result.unwrap();
        assert_eq!(brand.identifier, "s_bioc");
    }

    #[test]
    fn test_resolve_brand_style_generic_fallback() {
        // gen_mc_s_target with s_drak + gen_s entries
        let record = json!({
            "_RecordName_": "gen_mc_s_target",
            "brandStyles": [
                {
                    "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_drak.json",
                    "entries": []
                },
                {
                    "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/gen_s.json",
                    "entries": [
                        {"modifiers": [{"field": "FillColor", "value": "#FFFFFF"}]}
                    ]
                }
            ]
        });

        let record_value = record.get("_RecordValue_").unwrap_or(&record);

        // Unknown manufacturer falls back to gen_s
        let result = resolve_brand_style(record_value, Some("unknown"), None);
        assert!(result.is_some());
        let brand = result.unwrap();
        assert_eq!(brand.identifier, "gen_s");
        assert_eq!(brand.entries.len(), 1);
    }

    #[test]
    fn test_brand_identifier_basename() {
        let entry = json!({
            "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_drak.json"
        });
        assert_eq!(brand_identifier_basename(&entry), Some("s_drak".to_string()));

        let entry = json!({
            "brandIdentifier": "file://path/to/gen_s.json"
        });
        assert_eq!(brand_identifier_basename(&entry), Some("gen_s".to_string()));

        let entry = json!({
            "brandIdentifier": "s_rsi.json"
        });
        assert_eq!(brand_identifier_basename(&entry), Some("s_rsi".to_string()));
    }
}
