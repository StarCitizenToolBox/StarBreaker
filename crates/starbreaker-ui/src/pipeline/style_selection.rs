//! Style source selection for pipeline IR rendering.

use std::collections::BTreeMap;

use crate::error::UiError;
use crate::style::{ManufacturerStyle, StyleLoader};
use crate::ui_ir::UiIrDocument;

use super::{CanvasFetcher, PipelineInputs, StyleFetcher, extract_record_name, load_style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StyleSelectionManifest {
    pub selected_source: Option<String>,
    pub fallback_counters: BTreeMap<String, u32>,
}

pub(super) fn load_style_for_ir(
    ir: &UiIrDocument,
    inputs: &PipelineInputs<'_>,
) -> Result<ManufacturerStyle, UiError> {
    if let Some(style_name) = ir
        .selected_style_source
        .as_deref()
        .and_then(|source| source.strip_prefix("canvas:"))
    {
        let style_record = inputs.canvas_fetcher.fetch_canvas_by_name(style_name)?;
        let loader = StyleLoader::for_manufacturer(inputs.binding.manufacturer_id.unwrap_or("drak"));
        return loader.parse_buildingblocks_style_record(&style_record);
    }

    if let Some(manufacturer_id) = ir
        .selected_style_source
        .as_deref()
        .and_then(|source| source.strip_prefix("manufacturer:"))
    {
        return Ok(load_style(Some(manufacturer_id), inputs.style_fetcher));
    }

    Ok(load_style(inputs.binding.manufacturer_id, inputs.style_fetcher))
}

pub(super) fn build_style_selection_manifest(
    raw_root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    canvas_fetcher: &dyn CanvasFetcher,
    style_fetcher: &dyn StyleFetcher,
) -> StyleSelectionManifest {
    let effective_manufacturer = manufacturer_id.unwrap_or("drak");
    let mut fallback_counters = BTreeMap::new();

    if let Some(style_name) = raw_root_json
        .get("_RecordValue_")
        .and_then(|rv| rv.get("style"))
        .and_then(|v| v.as_str())
        .map(extract_record_name)
    {
        match canvas_fetcher.fetch_canvas_by_name(&style_name) {
            Ok(_) => {
                return StyleSelectionManifest {
                    selected_source: Some(format!("canvas:{style_name}")),
                    fallback_counters,
                };
            }
            Err(e) => {
                log::warn!(
                    "pipeline: failed to fetch canvas-level style record '{}': {}",
                    style_name,
                    e
                );
                fallback_counters.insert("canvas_style_fetch_failed".to_string(), 1);
            }
        }
    }

    match style_fetcher.fetch_manufacturer_style(effective_manufacturer) {
        Ok(_) => StyleSelectionManifest {
            selected_source: Some(format!("manufacturer:{effective_manufacturer}")),
            fallback_counters,
        },
        Err(e) => {
            log::warn!(
                "pipeline: manufacturer style fetch failed for '{}': {}; falling back to drak",
                effective_manufacturer,
                e
            );
            fallback_counters.insert("manufacturer_style_fallback_drak".to_string(), 1);
            StyleSelectionManifest {
                selected_source: Some("manufacturer:drak".to_string()),
                fallback_counters,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MissingCanvasStyleFetcher;

    impl crate::pipeline::CanvasFetcher for MissingCanvasStyleFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
            Err(UiError::FetchFailed {
                guid: guid.to_string(),
                source: "missing canvas".to_string().into(),
            })
        }
    }

    struct MissingManufacturerStyleFetcher;

    impl crate::pipeline::StyleFetcher for MissingManufacturerStyleFetcher {
        fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError> {
            Err(UiError::RenderError(format!("missing manufacturer style: {manufacturer_id}")))
        }
    }

    #[test]
    fn style_selection_manifest_counts_manufacturer_fallback() {
        let root = serde_json::json!({
            "_RecordValue_": {
                "style": "file://./foo_style.json"
            }
        });

        let manifest = build_style_selection_manifest(
            &root,
            Some("aegs"),
            &MissingCanvasStyleFetcher,
            &MissingManufacturerStyleFetcher,
        );

        assert_eq!(manifest.selected_source.as_deref(), Some("manufacturer:drak"));
        assert_eq!(manifest.fallback_counters.get("canvas_style_fetch_failed"), Some(&1));
        assert_eq!(manifest.fallback_counters.get("manufacturer_style_fallback_drak"), Some(&1));
    }
}
