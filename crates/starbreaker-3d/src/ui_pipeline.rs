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
        for type_name in datacore_ui_lookup_type_names() {
            let matches: Vec<_> = self
                .db
                .records_by_type_name(type_name)
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
                        "ui_pipeline: found {} {} records named '{}'; using first",
                        matches.len(),
                        type_name,
                        record_name
                    );
                }
                return export_canvas_record(self.db, record, record_name);
            }
        }

        Err(UiError::FetchFailed {
            guid: record_name.to_string(),
            source: format!(
                "no UI-support record found by name: {record_name}"
            )
            .into(),
        })
    }
}

fn datacore_ui_lookup_type_names() -> &'static [&'static str] {
    &[
        // DataCore stores the full name as "<Type>.<Stem>" in name_offset
        // (e.g. "BuildingBlocks_Canvas.M_Eng_MFDContent").  These are all
        // record families the UI resolver fetches through file-URL basenames.
        "BuildingBlocks_Canvas",
        "BuildingBlocks_Style",
        "BuildingBlocks_FontStyle",
        "TagDatabase",
    ]
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
        let candidates = p4k_swf_candidates(p4k_path);
        let entry = self
            .p4k
            .entries()
            .iter()
            .find(|entry| candidates.iter().any(|candidate| entry.name.eq_ignore_ascii_case(candidate)))
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

fn p4k_swf_candidates(path: &str) -> Vec<String> {
    let native = path.replace('/', "\\");
    let mut candidates = vec![native.clone()];
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with("data/") && !lower.starts_with("data\\") {
        candidates.push(format!("Data\\{native}"));
    }
    candidates
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
    let canvas_fetcher = DatacoreCanvasFetcher { db };
    let view = UiBindingView {
        canvas_guid: binding.canvas_guid.as_deref(),
        content_canvas_guid: binding.content_canvas_guid.as_deref(),
        binding_kind: Some(&binding.binding_kind),
        manufacturer_id: root_manufacturer_id,
        helper_name: binding.helper_name.as_deref(),
        default_view_index: binding.dashboard_view_index,
        default_screen_slot: binding.dashboard_screen_slot,
    };
    let effective_guid = binding
        .content_canvas_guid
        .as_deref()
        .filter(|g| !g.is_empty())
        .or_else(|| binding.canvas_guid.as_deref().filter(|g| !g.is_empty()));
    let authored_canvas_size = effective_guid
        .and_then(|guid| canvas_fetcher.fetch_canvas_json(guid).ok())
        .and_then(|json| authored_canvas_size(&json));
    let target_size = binding_target_size(&binding.binding_kind, authored_canvas_size);
    let localization_map = crate::pipeline::load_localization_map(p4k);
    let ini_loc_fetcher = starbreaker_ui::bb_loc_p4k::load_global_ini(|path| p4k.read_file(path).ok());
    let inputs = PipelineInputs {
        binding: &view,
        canvas_fetcher: &canvas_fetcher,
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
        animation_sample_percent: Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT),
        localization_map: Some(localization_map),
        loc_fetcher: Some(&ini_loc_fetcher),
    };
    let _ = texture_mip; // size is fixed per binding_kind; mip is applied at texture level
    starbreaker_ui::pipeline::render_for_binding(&inputs).map_err(|e| e.to_string())
}

/// Compile `binding` to canonical UI IR JSON using the same live DataCore + P4K
/// inputs as [`render_ui_binding_png`].
pub fn compile_ui_binding_ir_json(
    binding: &UiBinding,
    db: &Database<'_>,
    p4k: &MappedP4k,
    texture_mip: u32,
    root_manufacturer_id: Option<&str>,
) -> Result<String, String> {
    let canvas_fetcher = DatacoreCanvasFetcher { db };
    let view = UiBindingView {
        canvas_guid: binding.canvas_guid.as_deref(),
        content_canvas_guid: binding.content_canvas_guid.as_deref(),
        binding_kind: Some(&binding.binding_kind),
        manufacturer_id: root_manufacturer_id,
        helper_name: binding.helper_name.as_deref(),
        default_view_index: binding.dashboard_view_index,
        default_screen_slot: binding.dashboard_screen_slot,
    };
    let effective_guid = binding
        .content_canvas_guid
        .as_deref()
        .filter(|g| !g.is_empty())
        .or_else(|| binding.canvas_guid.as_deref().filter(|g| !g.is_empty()));
    let authored_canvas_size = effective_guid
        .and_then(|guid| canvas_fetcher.fetch_canvas_json(guid).ok())
        .and_then(|json| authored_canvas_size(&json));
    let target_size = binding_target_size(&binding.binding_kind, authored_canvas_size);
    let localization_map = crate::pipeline::load_localization_map(p4k);
    let ini_loc_fetcher = starbreaker_ui::bb_loc_p4k::load_global_ini(|path| p4k.read_file(path).ok());
    let inputs = PipelineInputs {
        binding: &view,
        canvas_fetcher: &canvas_fetcher,
        swf_fetcher: &P4kSwfFetcher { p4k },
        style_fetcher: &ManufacturerStyleFetcher { db },
        asset_fetcher: &P4kAssetFetcher { p4k },
        target_size,
        apply_postprocess: false,
        animation_sample_percent: Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT),
        localization_map: Some(localization_map),
        loc_fetcher: Some(&ini_loc_fetcher),
    };
    let _ = texture_mip;
    let ir = starbreaker_ui::pipeline::compile_ir_for_binding(&inputs).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&ir).map_err(|e| e.to_string())
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Map `binding_kind` to a canvas raster size.
fn binding_target_size(binding_kind: &str, authored_canvas_size: Option<(u32, u32)>) -> (u32, u32) {
    match binding_kind {
        "mfd" => (1600, 900),
        "radar" => (1024, 1024),
        _ => authored_canvas_size.unwrap_or((2048, 1024)),
    }
}

fn authored_canvas_size(canvas_json: &serde_json::Value) -> Option<(u32, u32)> {
    let record = canvas_json.get("_RecordValue_")?;
    let size = record.get("size")?;
    let width = size.get("x")?.as_f64()?;
    let height = size.get("y")?.as_f64()?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let width = width.round() as u32;
    let height = height.round() as u32;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

/// Parse a GUID string, tolerating surrounding braces and optional hyphens.
fn parse_guid(value: &str) -> Option<starbreaker_datacore::starbreaker_common::CigGuid> {
    use starbreaker_datacore::starbreaker_common::CigGuid;
    let trimmed = value.trim().trim_matches('{').trim_matches('}');
    CigGuid::from_str(trimmed).ok()
}

#[cfg(test)]
mod tests {
    use super::{authored_canvas_size, binding_target_size, datacore_ui_lookup_type_names, p4k_swf_candidates};
    use serde_json::json;

    #[test]
    fn authored_canvas_size_reads_record_value_size() {
        let canvas = json!({
            "_RecordValue_": {
                "size": { "x": 1920.0, "y": 1080.0, "z": 0.0 }
            }
        });
        assert_eq!(authored_canvas_size(&canvas), Some((1920, 1080)));
    }

    #[test]
    fn authored_canvas_size_ignores_invalid_size() {
        let canvas = json!({
            "_RecordValue_": {
                "size": { "x": 0.0, "y": 1080.0, "z": 0.0 }
            }
        });
        assert_eq!(authored_canvas_size(&canvas), None);
    }

    #[test]
    fn non_mfd_prefers_authored_canvas_size() {
        assert_eq!(binding_target_size("physical", Some((1920, 1080))), (1920, 1080));
        assert_eq!(binding_target_size("physical", None), (2048, 1024));
    }

    #[test]
    fn mfd_and_radar_keep_fixed_sizes() {
        assert_eq!(binding_target_size("mfd", Some((1920, 1080))), (1600, 900));
        assert_eq!(binding_target_size("radar", Some((1920, 1080))), (1024, 1024));
    }

    #[test]
    fn live_ui_lookup_includes_tag_database_records() {
        assert!(datacore_ui_lookup_type_names().contains(&"TagDatabase"));
    }

    #[test]
    fn swf_candidates_normalize_forward_slashes_to_p4k_paths() {
        assert_eq!(
            p4k_swf_candidates("Data/UI/fonts/Shared/fonts_en.gfx"),
            vec!["Data\\UI\\fonts\\Shared\\fonts_en.gfx".to_string()]
        );
    }

    #[test]
    fn swf_candidates_add_data_prefix_for_relative_paths() {
        assert_eq!(
            p4k_swf_candidates("UI/BuildingBlocks/assets/SWF/Canvas.swf"),
            vec![
                "UI\\BuildingBlocks\\assets\\SWF\\Canvas.swf".to_string(),
                "Data\\UI\\BuildingBlocks\\assets\\SWF\\Canvas.swf".to_string(),
            ]
        );
    }
}
