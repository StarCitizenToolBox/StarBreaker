//! Bridge between the decomposed export pipeline and `starbreaker-ui`.
//!
//! Implements the [`CanvasFetcher`], [`SwfFetcher`], and [`StyleFetcher`]
//! traits over the live DataCore database and P4K archive, then exposes
//! [`render_ui_binding_png`] as the single call-site for `decomposed.rs`.

use std::str::FromStr;

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
        let bytes = starbreaker_datacore::export::to_json_compact(self.db, record)
            .map_err(|e| UiError::FetchFailed {
                guid: guid.to_string(),
                source: Box::new(e),
            })?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
            UiError::FetchFailed {
                guid: guid.to_string(),
                source: Box::new(e),
            }
        })?;
        Ok(value)
    }
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

/// Phase 9: Always returns the Drake amber fallback.
///
/// A proper manufacturer-record lookup (resolving the manufacturer entity
/// from the ship's `Components[VehicleItemParams].manufacturer` reference)
/// is deferred to Phase 10.
struct DrakeStyleFetcher;

impl StyleFetcher for DrakeStyleFetcher {
    fn fetch_manufacturer_style(&self, _manufacturer_id: &str) -> Result<ManufacturerStyle, UiError> {
        Ok(StyleLoader::for_manufacturer("drak").drake_amber_fallback())
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
) -> Result<Vec<u8>, String> {
    let view = UiBindingView {
        canvas_guid: binding.canvas_guid.as_deref(),
        content_canvas_guid: binding.content_canvas_guid.as_deref(),
        binding_kind: Some(&binding.binding_kind),
        manufacturer_id: None, // Phase 10: resolve from ship record
        helper_name: binding.helper_name.as_deref(),
        default_view_index: binding.dashboard_view_index,
        default_screen_slot: binding.dashboard_screen_slot,
    };
    let target_size = binding_target_size(&binding.binding_kind);
    let inputs = PipelineInputs {
        binding: &view,
        canvas_fetcher: &DatacoreCanvasFetcher { db },
        swf_fetcher: &P4kSwfFetcher { p4k },
        style_fetcher: &DrakeStyleFetcher,
        target_size,
        apply_postprocess: true,
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
