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
    pipeline::{CanvasFetcher, PipelineInputs, SwfFetcher, UiBindingView},
};

use crate::types::UiBinding;

mod style_fetcher;
use style_fetcher::ManufacturerStyleFetcher;

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
        "BuildingBlocks_Timeline",
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
    fn push_with_data_prefix(candidates: &mut Vec<String>, candidate: String) {
        if !candidates.iter().any(|existing| existing.eq_ignore_ascii_case(&candidate)) {
            candidates.push(candidate.clone());
        }
        let lower = candidate.to_ascii_lowercase();
        if !lower.starts_with("data\\") {
            let prefixed = format!("Data\\{candidate}");
            if !candidates
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&prefixed))
            {
                candidates.push(prefixed);
            }
        }
    }

    let native = path.replace('/', "\\");
    let normalised = starbreaker_ui::bb_assets::UiAssetResolver::normalise_path(path)
        .replace('/', "\\");
    let mut candidates = Vec::new();
    for seed in [native, normalised] {
        push_with_data_prefix(&mut candidates, seed.clone());
        if seed.to_ascii_lowercase().ends_with(".tif") {
            if let Some(stem) = seed.strip_suffix(".tif") {
                push_with_data_prefix(&mut candidates, format!("{stem}.dds"));
            } else if let Some(stem) = seed.strip_suffix(".TIF") {
                push_with_data_prefix(&mut candidates, format!("{stem}.dds"));
            }
        }
    }
    candidates
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
    let animation_sample_percent = if binding.binding_kind == "mfd" {
        Some(0.0)
    } else {
        Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT)
    };
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
        animation_sample_percent,
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
    let animation_sample_percent = if binding.binding_kind == "mfd" {
        Some(0.0)
    } else {
        Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT)
    };
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
        animation_sample_percent,
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
mod tests;
