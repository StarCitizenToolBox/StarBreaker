//! Manufacturer style lookup adapter used by the UI pipeline bridge.

use log::warn;
use starbreaker_datacore::Database;
use starbreaker_ui::style::{ManufacturerStyle, StyleLoader};
use starbreaker_ui::{UiError, pipeline::StyleFetcher};

/// Resolves the manufacturer style by looking up a `BuildingBlocks_Style`
/// record in DataCore.
///
/// Discovery strategy (no hardcoded ship/manufacturer names):
/// 1. Enumerate all `BuildingBlocks_Style` records.
/// 2. Match the record name against the manufacturer id (case-insensitive
///    substring/suffix/prefix on the dotted-stem). Allows authored names like
///    `BuildingBlocks_Style.DRAK_Default` or `Style_drak` to resolve for
///    `manufacturer_id = "drak"`.
/// 3. Parse the matched record via
///    [`StyleLoader::parse_buildingblocks_style_record`].
/// 4. If no record matches, fall back to the Drake amber defaults *with a
///    warning*. This is the only allowed fallback path.
pub(super) struct ManufacturerStyleFetcher<'a> {
    pub(super) db: &'a Database<'a>,
}

fn manufacturer_style_match_score(stem_lower: &str, manufacturer_lower: &str) -> Option<i32> {
    if stem_lower == format!("s_{manufacturer_lower}_hud") {
        return Some(0);
    }
    if stem_lower == format!("s_{manufacturer_lower}") {
        return Some(1);
    }
    if stem_lower.starts_with(&format!("s_{manufacturer_lower}_")) {
        return Some(2);
    }
    if stem_lower == manufacturer_lower {
        return Some(3);
    }
    if stem_lower.contains(manufacturer_lower) {
        return Some(4);
    }
    None
}

impl<'a> StyleFetcher for ManufacturerStyleFetcher<'a> {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError> {
        let loader = StyleLoader::for_manufacturer(manufacturer_id);
        let needle = manufacturer_id.to_ascii_lowercase();

        let mut candidates: Vec<_> = self
            .db
            .records_by_type_name("BuildingBlocks_Style")
            .filter_map(|record| {
                let full = self.db.resolve_string2(record.name_offset).to_string();
                let stem = full.rsplit('.').next().unwrap_or(&full).to_ascii_lowercase();
                let bucket = manufacturer_style_match_score(&stem, &needle)?;
                Some((bucket, stem, full, record))
            })
            .collect();

        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

        if let Some((_, _, full_name, record)) = candidates.first() {
            match starbreaker_datacore::export::to_json_compact(self.db, record) {
                Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                    Ok(value) => match loader.parse_buildingblocks_style_record(&value) {
                        Ok(style) => return Ok(style),
                        Err(e) => warn!(
                            "ui: failed to parse BuildingBlocks_Style record '{}' for manufacturer '{}': {}; using Drake amber fallback",
                            full_name, manufacturer_id, e
                        ),
                    },
                    Err(e) => warn!(
                        "ui: failed to deserialize BuildingBlocks_Style record '{}' for manufacturer '{}': {}; using Drake amber fallback",
                        full_name, manufacturer_id, e
                    ),
                },
                Err(e) => warn!(
                    "ui: failed to export BuildingBlocks_Style record '{}' for manufacturer '{}': {}; using Drake amber fallback",
                    full_name, manufacturer_id, e
                ),
            }
        } else {
            warn!(
                "ui: no BuildingBlocks_Style record matches manufacturer '{}'; using Drake amber fallback",
                manufacturer_id
            );
        }

        Ok(loader.drake_amber_fallback())
    }
}
