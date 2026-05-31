//! High-level rendering pipeline entry points.

use std::collections::BTreeMap;

use crate::bb_brand_apply::{apply_brand_modifiers, apply_scene_style_entries};
use crate::bb_brand_style::resolve_brand_style;
use crate::canvas::CanvasWidgetTreeResolver;
use crate::compose::{ComposeContext, encode_png};
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::hybrid_compose::render_ui_ir_with_swf_overlay;
use crate::ir_compose::render_ui_ir_document;
use crate::style::{ManufacturerStyle, StyleLoader};
use crate::ui_ir::{UiIrDocument, UiRendererHint};

mod asset_manifest;
mod style_selection;
mod swf_selection;
#[cfg(test)]
mod tests;

use asset_manifest::build_asset_reference_manifest;
use style_selection::{build_style_selection_manifest, load_style_for_ir};
use swf_selection::{build_swf_selection_manifest, load_first_swf};

pub use crate::bb_atlas::AssetFetcher;
pub use swf_selection::flash_swf_candidates;

/// Static UI captures use midpoint sampling for authored animations.
pub const DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT: f32 = 50.0;

/// Extract a DataCore record name from a BuildingBlocks file URL or bare name.
pub fn extract_record_name(file_url_or_name: &str) -> String {
    let without_scheme = file_url_or_name
        .strip_prefix("file://")
        .unwrap_or(file_url_or_name);
    let basename = without_scheme.rsplit('/').next().unwrap_or(without_scheme);
    if basename
        .get(basename.len().saturating_sub(5)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".json"))
    {
        basename[..basename.len() - 5].to_string()
    } else {
        basename.to_string()
    }
}

/// Fetch a BuildingBlocks canvas record as JSON.
pub trait CanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError>;

    fn fetch_canvas_by_path(&self, path_or_name: &str) -> Result<serde_json::Value, UiError> {
        let name = extract_record_name(path_or_name);
        self.fetch_canvas_by_name(&name)
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        Err(UiError::FetchFailed {
            guid: record_name.into(),
            source: "fetch_canvas_by_name not implemented".into(),
        })
    }
}

/// Fetch raw SWF bytes by P4K path.
pub trait SwfFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError>;
}

/// Resolve a manufacturer style by short id.
pub trait StyleFetcher {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError>;
}

/// Borrowed snapshot of UiBinding fields needed by the pipeline.
pub struct UiBindingView<'a> {
    pub canvas_guid: Option<&'a str>,
    pub content_canvas_guid: Option<&'a str>,
    pub binding_kind: Option<&'a str>,
    pub manufacturer_id: Option<&'a str>,
    pub helper_name: Option<&'a str>,
    pub default_view_index: Option<u32>,
    pub default_screen_slot: Option<u32>,
}

/// All inputs required by pipeline entrypoints.
pub struct PipelineInputs<'a> {
    pub binding: &'a UiBindingView<'a>,
    pub canvas_fetcher: &'a dyn CanvasFetcher,
    pub swf_fetcher: &'a dyn SwfFetcher,
    pub style_fetcher: &'a dyn StyleFetcher,
    pub asset_fetcher: &'a dyn crate::bb_atlas::AssetFetcher,
    pub target_size: (u32, u32),
    pub apply_postprocess: bool,
    pub animation_sample_percent: Option<f32>,
    pub localization_map: Option<std::collections::HashMap<String, String>>,
    pub loc_fetcher: Option<&'a dyn crate::bb_loc::LocFetcher>,
}

/// Diagnostics captured while rendering a UI image.
#[derive(Debug, Clone)]
pub struct UiRenderDiagnostics {
    pub resolved_canvas_ids: Vec<String>,
    pub resolved_canvas_names: Vec<String>,
    pub selected_style_source: String,
    pub selected_swf_source: String,
    pub render_backend: String,
    pub fallback_counters: BTreeMap<String, u32>,
    pub unresolved_references: Vec<String>,
    pub confidence: u8,
}

/// Render output plus diagnostics for provenance metadata.
#[derive(Debug, Clone)]
pub struct UiRenderOutput {
    pub png: Vec<u8>,
    pub diagnostics: UiRenderDiagnostics,
}

/// Compile canonical UI IR for a binding.
pub fn compile_ir_for_binding(inputs: &PipelineInputs<'_>) -> Result<UiIrDocument, UiError> {
    let b = inputs.binding;

    let effective_guid = b
        .content_canvas_guid
        .filter(|g| !g.is_empty())
        .or_else(|| b.canvas_guid.filter(|g| !g.is_empty()))
        .ok_or_else(|| {
            UiError::RenderError(format!(
                "no canvas GUID available for helper {:?} (kind {:?})",
                b.helper_name, b.binding_kind,
            ))
        })?;

    let raw_root_json = inputs.canvas_fetcher.fetch_canvas_json(effective_guid)?;
    let resolver = CanvasWidgetTreeResolver::new();
    let resolved = resolver.resolve(effective_guid, |guid| {
        inputs.canvas_fetcher.fetch_canvas_json(guid)
    })?;
    let canvas_name = raw_root_json
        .get("_RecordName_")
        .and_then(|v| v.as_str());

    let style_manifest = build_style_selection_manifest(
        &raw_root_json,
        b.manufacturer_id,
        inputs.canvas_fetcher,
        inputs.style_fetcher,
    );
    let effective_manufacturer_id = b
        .manufacturer_id
        .or_else(|| {
            style_manifest
                .selected_source
                .as_deref()
                .and_then(|source| source.strip_prefix("manufacturer:"))
        })
        // Keep brand/style resolution deterministic when binding-level
        // manufacturer metadata is absent.
        .or(Some("drak"));

    let mut scene = crate::bb_resolve::resolve_canvas_graph_with_loc(
        &raw_root_json,
        effective_manufacturer_id,
        &|p| {
            inputs
                .canvas_fetcher
                .fetch_canvas_by_path(p)
                .map_err(|e| e.to_string())
        },
        inputs.loc_fetcher,
    )
    .map_err(UiError::RenderError)?;

    project_canvas_style_entries(
        &mut scene,
        &raw_root_json,
        effective_manufacturer_id,
        inputs.loc_fetcher,
    );

    let swf_manifest = build_swf_selection_manifest(
        &raw_root_json,
        &resolved,
        effective_manufacturer_id.unwrap_or("drak"),
        inputs.swf_fetcher,
    );
    let selected_swf_source = swf_manifest
        .valid_candidates
        .first()
        .map(|candidate| candidate.path.clone());

    let mut effective_target_size = inputs.target_size;
    if let Some(swf_source) = selected_swf_source.as_deref()
        && let Ok(swf_bytes) = inputs.swf_fetcher.fetch_swf_bytes(swf_source)
        && let Ok(swf_library) = crate::swf_assets::SwfAssetLibrary::new(swf_bytes)
    {
        let (sw, sh) = swf_library.stage_size();
        let content_aspect = swf_library
            .stage_visual_bounds(0)
            .and_then(|(x0, y0, x1, y1)| {
                let w = (x1 - x0).abs();
                let h = (y1 - y0).abs();
                if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
                    Some(h / w)
                } else {
                    None
                }
            });
        if sw.is_finite() && sh.is_finite() && sw > 0.0 && sh > 0.0 {
            let aspect = content_aspect.unwrap_or(sh / sw);
            if aspect > 0.0 && aspect.is_finite() {
                let width = inputs.target_size.0.max(1);
                let swf_height = ((width as f32) * aspect).round().max(1.0) as u32;
                let height = swf_height.max(1);
                // Avoid pathological SWF headers from collapsing layout.
                if width <= 8192 && height <= 8192 {
                    effective_target_size = (width, height);
                }
            }
        }
    }

    let defaults = DefaultValueRegistry::with_pipeline_defaults(inputs.localization_map.clone());
    let asset_manifest = build_asset_reference_manifest(&scene, inputs.asset_fetcher);

    let mut ir = crate::ui_ir::compile_ui_ir_from_scene_with_animation_sample(
        &scene,
        Some(inputs.canvas_fetcher),
        effective_guid,
        canvas_name,
        effective_target_size,
        &defaults,
        style_manifest.selected_source,
        selected_swf_source,
        &[],
        asset_manifest.resolved_asset_refs,
        asset_manifest.missing_asset_refs,
        inputs.animation_sample_percent,
        100,
    );
    ir.warnings.extend(fallback_counter_warnings(
        style_manifest
            .fallback_counters
            .iter()
            .chain(swf_manifest.fallback_counters.iter())
            .map(|(key, value): (&String, &u32)| (key.as_str(), *value)),
    ));
    Ok(ir)
}

fn project_canvas_style_entries(
    scene: &mut crate::bb_scene::BbScene,
    raw_root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    loc_fetcher: Option<&dyn crate::bb_loc::LocFetcher>,
) {
    let record_value = raw_root_json.get("_RecordValue_").unwrap_or(raw_root_json);
    let selected_brand = resolve_brand_style(raw_root_json, manufacturer_id, None);
    let palette_source = selected_brand.map(|brand| brand.raw).unwrap_or(record_value);

    if let Some(default_entries) = record_value
        .get("defaultStyles")
        .and_then(|styles| styles.get("entries"))
        .and_then(|entries| entries.as_array())
    {
        apply_scene_style_entries(scene, default_entries, palette_source, loc_fetcher);
    }

    if let Some(brand) = resolve_brand_style(raw_root_json, manufacturer_id, None) {
        apply_brand_modifiers(scene, &brand, loc_fetcher);
    }
}

/// Render via IR compilation and IR-only rendering.
pub fn render_for_binding_ir(inputs: &PipelineInputs<'_>) -> Result<Vec<u8>, UiError> {
    let ir = compile_ir_for_binding(inputs)?;

    let mut style = load_style_for_ir(&ir, inputs)?;
    let suppresses_placeholder_screen_background = ir.selected_swf_source.is_some()
        && ir.nodes.iter().any(|node| {
            node.node_type.eq_ignore_ascii_case("widget_image")
                && !node.is_active
                && node.resolved_style_tags.iter().any(|tag| {
                    tag.tag_name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case("ScreenNameBackground"))
                })
        });
    if suppresses_placeholder_screen_background {
        style.background = crate::canvas::RgbaColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
    }
    let defaults = DefaultValueRegistry::with_pipeline_defaults(inputs.localization_map.clone());

    let swf_paths = ir
        .selected_swf_source
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let assets = load_first_swf(&swf_paths, inputs.swf_fetcher);
    let ctx = ComposeContext {
        style: &style,
        defaults: &defaults,
        assets: &assets,
    };
    let atlas_manufacturer_id = inputs.binding.manufacturer_id.or_else(|| {
        ir.selected_style_source
            .as_deref()
            .and_then(|source| source.strip_prefix("manufacturer:"))
    });
    let atlas = crate::bb_atlas::AtlasLibrary::new(inputs.asset_fetcher, atlas_manufacturer_id);

    let image = match ir.renderer_hint {
        UiRendererHint::Bb => render_ui_ir_document(&ir, &ctx, &atlas)?,
        UiRendererHint::Swf | UiRendererHint::Hybrid => render_ui_ir_with_swf_overlay(
            &ir,
            &ctx,
            &atlas,
            ir.selected_swf_source.as_ref().map(|_| &assets),
        )?,
    };
    encode_png(&image)
}

/// Main entrypoint for rendering a UI binding to PNG bytes.
pub fn render_for_binding(inputs: &PipelineInputs<'_>) -> Result<Vec<u8>, UiError> {
    render_for_binding_ir(inputs)
}

pub(super) fn fallback_counter_warnings<'a>(
    counters: impl IntoIterator<Item = (&'a str, u32)>,
) -> Vec<String> {
    counters
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(key, count)| format!("fallback path used: {key}={count}"))
        .collect()
}

pub(super) fn load_style(
    manufacturer_id: Option<&str>,
    fetcher: &dyn StyleFetcher,
) -> ManufacturerStyle {
    let id = manufacturer_id.unwrap_or("drak");
    match fetcher.fetch_manufacturer_style(id) {
        Ok(style) => style,
        Err(e) => {
            log::debug!(
                "pipeline: manufacturer style fetch failed for '{}': {}; using Drake fallback",
                id,
                e,
            );
            StyleLoader::for_manufacturer("drak").drake_amber_fallback()
        }
    }
}
