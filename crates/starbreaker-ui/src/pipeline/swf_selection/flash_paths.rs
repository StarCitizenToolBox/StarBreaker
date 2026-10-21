//! Flash-canvas SWF path derivation helpers.
//!
//! All ship-subdir enumeration is driven by P4K directory listing via the
//! `SwfFetcher::list_swf_dirs` method; no ship or asset names are
//! hard-coded.  Brand-level screen-set dirs (e.g. `SupportScreen16-9`) are
//! still constructed from the known P4K directory grammar without probing.

use super::super::extract_record_name;
use super::super::SwfFetcher;

pub fn flash_swf_candidates(
    record_name: &str,
    manufacturer_id: &str,
    fetcher: &dyn SwfFetcher,
) -> Vec<String> {
    let name = record_name
        .strip_prefix("BuildingBlocks_Canvas.")
        .unwrap_or(record_name);

    let brand: String = manufacturer_id
        .chars()
        .take(3)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if brand.is_empty() {
        return vec![];
    }

    if name.to_ascii_lowercase().contains("annunciator") {
        return annunciator_swf_candidates(name, &brand, fetcher);
    }

    let is_generic_mc = name.starts_with("MC_S_") || name.starts_with("GEN_MC_S_");

    let without_prefix = name
        .strip_prefix("MC_S_")
        .or_else(|| name.strip_prefix("GEN_MC_S_"))
        .unwrap_or(name);
    let stem = without_prefix
        .strip_suffix("_Master")
        .or_else(|| without_prefix.strip_suffix("_master"))
        .unwrap_or(without_prefix);

    if stem.is_empty() {
        return vec![];
    }

    let mut candidates = support_screen_candidates_for_brand(&brand, stem, fetcher);
    if is_generic_mc && brand != "RSI" {
        candidates.extend(support_screen_candidates_for_brand("RSI", stem, fetcher));
    }

    candidates
}

pub(super) fn flash_swf_candidates_from_canvas_refs(
    raw_root_json: &serde_json::Value,
    manufacturer_id: &str,
    fetcher: &dyn SwfFetcher,
) -> Vec<String> {
    let Some(record_value) = raw_root_json.get("_RecordValue_") else {
        return Vec::new();
    };

    let mut refs = Vec::new();
    refs.extend(canvas_reference_paths_from_entries(
        record_value
            .get("defaultStyles")
            .and_then(|styles| styles.get("entries"))
            .and_then(|entries| entries.as_array()),
    ));

    let preferred_brand_prefix = format!("s_{}", manufacturer_id.to_ascii_lowercase());
    if let Some(brand_styles) = record_value.get("brandStyles").and_then(|styles| styles.as_array()) {
        for brand_style in brand_styles {
            let brand_identifier = brand_style
                .get("brandIdentifier")
                .and_then(|value| value.as_str())
                .map(extract_record_name)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if brand_identifier.starts_with(&preferred_brand_prefix)
                || brand_identifier.contains(manufacturer_id)
            {
                refs.extend(canvas_reference_paths_from_entries(
                    brand_style.get("entries").and_then(|entries| entries.as_array()),
                ));
            }
        }
    }

    let mut candidates = Vec::new();
    for path in refs {
        let reference_name = extract_record_name(&path).to_ascii_lowercase();
        if let Some(stem) = flash_stem_from_canvas_reference_name(&reference_name) {
            candidates.extend(flash_swf_candidates_from_stem(&stem, manufacturer_id, fetcher));
        }
    }
    candidates
}

fn canvas_reference_paths_from_entries(entries: Option<&Vec<serde_json::Value>>) -> Vec<String> {
    let Some(entries) = entries else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for entry in entries {
        let Some(modifiers) = entry.get("modifiers").and_then(|mods| mods.as_array()) else {
            continue;
        };
        for modifier in modifiers {
            let is_canvas_ref = modifier
                .get("field")
                .and_then(|field| field.get("_Type_"))
                .and_then(|value| value.as_str())
                == Some("BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord");
            if !is_canvas_ref {
                continue;
            }
            if let Some(path) = modifier
                .get("field")
                .and_then(|field| field.get("value"))
                .and_then(|value| value.as_str())
            {
                refs.push(path.to_string());
            }
        }
    }
    refs
}

fn flash_stem_from_canvas_reference_name(reference_name: &str) -> Option<String> {
    let stem = if let Some((_, tail)) = reference_name.rsplit_once("_mc_s_") {
        tail
    } else if let Some((_, tail)) = reference_name.rsplit_once("_s_") {
        tail
    } else {
        return None;
    }
    .trim_matches('_')
    .trim();
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

fn flash_swf_candidates_from_stem(stem: &str, manufacturer_id: &str, fetcher: &dyn SwfFetcher) -> Vec<String> {
    let brand: String = manufacturer_id
        .chars()
        .take(3)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if brand.is_empty() || stem.is_empty() {
        return Vec::new();
    }
    support_screen_candidates_for_brand(&brand, stem, fetcher)
}

/// Enumerate ship subdirs for `brand` from the P4K tree.
///
/// Returns bare subdir names (e.g. `"DRAK_Buccaneer"`) sorted
/// lexicographically.  Only dirs whose name starts with the brand prefix
/// are treated as ship dirs; others (e.g. `SupportScreen16-9`) are
/// screen-set dirs handled by the static grammar in
/// `support_screen_candidates_for_brand`.
fn p4k_ship_subdirs(brand: &str, fetcher: &dyn SwfFetcher) -> Vec<String> {
    let brand_root = format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\");
    let brand_upper = brand.to_ascii_uppercase();
    let mut dirs: Vec<String> = fetcher
        .list_swf_dirs(&brand_root)
        .into_iter()
        .filter(|d| d.to_ascii_uppercase().starts_with(&brand_upper))
        .collect();
    dirs.sort();
    dirs
}

fn support_screen_candidates_for_brand(brand: &str, stem: &str, fetcher: &dyn SwfFetcher) -> Vec<String> {
    if brand.is_empty() || stem.is_empty() {
        return Vec::new();
    }
    let direct_dirs = [
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\SupportScreen16-9\"),
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\SupportScreen1-1\"),
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\SupportScreenBespoke2\"),
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\Support_Bespoke_2\"),
    ];
    let mut candidates: Vec<String> = direct_dirs
        .into_iter()
        .flat_map(|base| [format!("{base}{stem}Status.swf"), format!("{base}{stem}.swf")])
        .collect();

    let ship_support_dirs = [
        "Support_Bespoke_2",
        "Support_Bespoke_1",
        "SupportScreen16-9",
        "SupportScreen1-1",
        "SupportScreenBespoke2",
    ];
    for ship in p4k_ship_subdirs(brand, fetcher) {
        for dir in &ship_support_dirs {
            let base = format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\{ship}\{dir}\");
            candidates.push(format!("{base}{stem}Status.swf"));
            candidates.push(format!("{base}{stem}.swf"));
        }
    }

    candidates
}

fn annunciator_swf_candidates(canvas_name: &str, brand: &str, fetcher: &dyn SwfFetcher) -> Vec<String> {
    let name_lower = canvas_name.to_ascii_lowercase();

    let halve = if name_lower.contains("_left") {
        1u8
    } else if name_lower.contains("_right") {
        2u8
    } else {
        1u8
    };

    let halve_file = format!("AnnunciatorHalve{halve}.swf");
    let swf_root = r"Data\UI\ShipInterface\assets\SWF\";

    let mut candidates = Vec::new();
    candidates.push(format!(r"{swf_root}{brand}\AnnunciatorScreen\{halve_file}"));

    for ship in p4k_ship_subdirs(brand, fetcher) {
        candidates.push(format!(
            r"{swf_root}{brand}\{ship}\AnnunciatorScreen\{halve_file}"
        ));
    }

    candidates
}
