//! Bridge between the decomposed export pipeline and `starbreaker-ui`.
//!
//! Implements the [`CanvasFetcher`], [`SwfFetcher`], [`StyleFetcher`], and
//! atlas asset-fetcher traits over the live DataCore database and P4K archive,
//! then exposes [`render_ui_binding_png`] as the single call-site for
//! `decomposed.rs`.

use std::str::FromStr;

use log::warn;
use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;
use starbreaker_ui::{
    UiError,
    pipeline::{CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView},
};
use starbreaker_ui::style::{ManufacturerStyle, StyleLoader};

use crate::types::UiBinding;

// ──────────────────────────────────────────────────────────────────────────────
// Fetcher implementations
// ──────────────────────────────────────────────────────────────────────────────

struct DatacoreCanvasFetcher<'a> {
    db: &'a Database<'a>,
}

impl<'a> CanvasFetcher for DatacoreCanvasFetcher<'a> {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        let cig_guid = parse_guid(guid).ok_or_else(|| UiError::FetchFailed {
            guid: guid.to_string(),
            source: "invalid GUID format".into(),
        })?;
        let record = self.db.record_by_id(&cig_guid).ok_or_else(|| UiError::FetchFailed {
            guid: guid.to_string(),
            source: format!("record not found in DataCore for GUID {guid}").into(),
        })?;
        export_canvas_record(self.db, record, guid)
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        // DataCore stores the full name as "<Type>.<Stem>" in name_offset
        // (e.g. "BuildingBlocks_Canvas.M_Eng_MFDContent").  Match against both
        // the full name and just the stem so that callers using file-URL basenames
        // (e.g. "m_eng_mfdcontent") still resolve correctly.
        let matches: Vec<_> = self
            .db
            .records_by_type_name("BuildingBlocks_Canvas")
            .filter(|record| {
                let full_name = self.db.resolve_string2(record.name_offset);
                let stem = full_name.rsplit('.').next().unwrap_or(full_name);
                stem.eq_ignore_ascii_case(record_name)
                    || full_name.eq_ignore_ascii_case(record_name)
            })
            .collect();

        if let Some(record) = matches.first().copied() {
            if matches.len() > 1 {
                warn!(
                    "ui_pipeline: found {} BuildingBlocks_Canvas records named '{}'; using first",
                    matches.len(),
                    record_name
                );
            }
            return export_canvas_record(self.db, record, record_name);
        }

        // Fallback: BuildingBlocks_Style records share the same StyleEntry shape
        // as brandStyles entries, so a canvas-level `style` file-URL can be
        // resolved through this same lookup.  This keeps brand resolution
        // structural (canvas → linked Style record) without a separate fetcher.
        let style_matches: Vec<_> = self
            .db
            .records_by_type_name("BuildingBlocks_Style")
            .filter(|record| {
                let full_name = self.db.resolve_string2(record.name_offset);
                let stem = full_name.rsplit('.').next().unwrap_or(full_name);
                stem.eq_ignore_ascii_case(record_name)
                    || full_name.eq_ignore_ascii_case(record_name)
            })
            .collect();

        if let Some(record) = style_matches.first().copied() {
            if style_matches.len() > 1 {
                warn!(
                    "ui_pipeline: found {} BuildingBlocks_Style records named '{}'; using first",
                    style_matches.len(),
                    record_name
                );
            }
            return export_canvas_record(self.db, record, record_name);
        }

        Err(UiError::FetchFailed {
            guid: record_name.to_string(),
            source: format!(
                "no BuildingBlocks_Canvas or BuildingBlocks_Style record found by name: {record_name}"
            )
            .into(),
        })
    }
}

fn export_canvas_record(
    db: &Database<'_>,
    record: &starbreaker_datacore::types::Record,
    lookup_key: &str,
) -> Result<serde_json::Value, UiError> {
    let bytes = starbreaker_datacore::export::to_json_compact(db, record).map_err(|e| {
        UiError::FetchFailed {
            guid: lookup_key.to_string(),
            source: Box::new(e),
        }
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
        UiError::FetchFailed {
            guid: lookup_key.to_string(),
            source: Box::new(e),
        }
    })?;
    Ok(value)
}

struct P4kSwfFetcher<'a> {
    p4k: &'a MappedP4k,
}

impl<'a> SwfFetcher for P4kSwfFetcher<'a> {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError> {
        let entry = self
            .p4k
            .entries()
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(p4k_path))
            .ok_or_else(|| UiError::FetchFailed {
                guid: p4k_path.to_string(),
                source: format!("SWF not found in P4K: {p4k_path}").into(),
            })?;
        self.p4k.read(entry).map_err(|e| UiError::FetchFailed {
            guid: p4k_path.to_string(),
            source: Box::new(e),
        })
    }
}

struct P4kAssetFetcher<'a> {
    p4k: &'a MappedP4k,
}

impl<'a> starbreaker_ui::bb_atlas::AssetFetcher for P4kAssetFetcher<'a> {
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        read_p4k_asset(self.p4k, p4k_path)
    }
}

fn read_p4k_asset(p4k: &MappedP4k, p4k_path: &str) -> Option<Vec<u8>> {
    for candidate in p4k_asset_candidates(p4k_path) {
        if let Ok(bytes) = p4k.read_file(&candidate) {
            return Some(bytes);
        }
    }
    None
}

fn p4k_asset_candidates(path: &str) -> Vec<String> {
    let native = path.replace('/', "\\");
    let mut candidates = vec![native.clone()];
    if !path.starts_with("data/") {
        candidates.push(format!("Data\\{native}"));
    }
    candidates
}

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
///    warning*.  This is the only allowed fallback path.
struct ManufacturerStyleFetcher<'a> {
    db: &'a Database<'a>,
}

impl<'a> StyleFetcher for ManufacturerStyleFetcher<'a> {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError> {
        let loader = StyleLoader::for_manufacturer(manufacturer_id);
        let needle = manufacturer_id.to_ascii_lowercase();

        let candidates: Vec<_> = self
            .db
            .records_by_type_name("BuildingBlocks_Style")
            .filter_map(|record| {
                let full = self.db.resolve_string2(record.name_offset).to_string();
                let stem = full.rsplit('.').next().unwrap_or(&full).to_ascii_lowercase();
                if stem.contains(&needle) {
                    Some((full, record))
                } else {
                    None
                }
            })
            .collect();

        if let Some((full_name, record)) = candidates.first() {
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

// ──────────────────────────────────────────────────────────────────────────────
// Public entry point
// ──────────────────────────────────────────────────────────────────────────────

/// Render `binding` to a PNG byte vector using live DataCore + P4K access.
///
/// Returns the PNG bytes on success, or a descriptive error string on failure.
/// Callers should log the error and set `generated_image_path = None` rather
/// than propagating.
pub fn render_ui_binding_png(
    binding: &UiBinding,
    db: &Database<'_>,
    p4k: &MappedP4k,
    texture_mip: u32,
    root_manufacturer_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    let view = UiBindingView {
        canvas_guid: binding.canvas_guid.as_deref(),
        content_canvas_guid: binding.content_canvas_guid.as_deref(),
        binding_kind: Some(&binding.binding_kind),
        manufacturer_id: root_manufacturer_id,
        helper_name: binding.helper_name.as_deref(),
        default_view_index: binding.dashboard_view_index,
        default_screen_slot: binding.dashboard_screen_slot,
    };
    let target_size = binding_target_size(&binding.binding_kind);
    let localization_map = crate::pipeline::load_localization_map(p4k);
    let ini_loc_fetcher = starbreaker_ui::bb_loc_p4k::load_global_ini(|path| p4k.read_file(path).ok());
    let inputs = PipelineInputs {
        binding: &view,
        canvas_fetcher: &DatacoreCanvasFetcher { db },
        swf_fetcher: &P4kSwfFetcher { p4k },
        style_fetcher: &ManufacturerStyleFetcher { db },
        asset_fetcher: &P4kAssetFetcher { p4k },
        target_size,
        // Phase 11: postprocess is disabled while compose.rs is the magenta-grid
        // placeholder.  The tint/scanline/vignette passes assume *lit* pixels
        // come from a real canvas render; running them over the placeholder
        // would mask the "not yet rendered" signal.  Re-enable in Phase 13
        // once the paint engine produces real content.
        apply_postprocess: false,
        localization_map: Some(localization_map),
        loc_fetcher: Some(&ini_loc_fetcher),
    };
    let _ = texture_mip; // size is fixed per binding_kind; mip is applied at texture level
    starbreaker_ui::pipeline::render_for_binding(&inputs).map_err(|e| e.to_string())
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Map `binding_kind` to a canvas raster size.
fn binding_target_size(binding_kind: &str) -> (u32, u32) {
    match binding_kind {
        "mfd" => (1600, 900),
        "radar" => (1024, 1024),
        _ => (2048, 1024),
    }
}

/// Parse a GUID string, tolerating surrounding braces and optional hyphens.
fn parse_guid(value: &str) -> Option<starbreaker_datacore::starbreaker_common::CigGuid> {
    use starbreaker_datacore::starbreaker_common::CigGuid;
    let trimmed = value.trim().trim_matches('{').trim_matches('}');
    CigGuid::from_str(trimmed).ok()
}
