//! High-level rendering pipeline entry point.
//!
//! Provides [`render_for_binding`] — a single function that takes a
//! borrowed snapshot of a `UiBinding`'s canvas / style / SWF references
//! together with caller-supplied fetcher traits and produces a PNG byte
//! vector.
//!
//! The fetcher traits are deliberately thin so that `starbreaker-3d` can
//! implement them over the existing DataCore + P4K abstractions without
//! coupling this crate to those dependencies.
//!
//! # Rendering sequence
//! 1. Pick the effective canvas GUID: `content_canvas_guid` preferred over
//!    `canvas_guid`.
//! 2. Resolve the full widget tree via [`CanvasWidgetTreeResolver`].
//! 3. Collect all SWF paths referenced in the resolved tree.
//! 4. Load the first found SWF via [`SwfFetcher`]; fall back to a minimal
//!    empty library when no SWF is present.
//! 5. Load the manufacturer style via [`StyleFetcher`]; fall back to the
//!    Drake amber defaults when the manufacturer id is `None` or the fetch
//!    fails.
//! 6. Rasterise the canvas.  If `apply_postprocess` is `true`, run the
//!    manufacturer post-process pass.
//! 7. Encode to PNG and return.

use std::collections::{BTreeMap, HashSet};
use std::io::Cursor;

use crate::canvas::{CanvasRecord, CanvasWidgetTreeResolver, ResolvedCanvas, SceneItem};
use crate::compose::{ComposeContext, encode_png};
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::hybrid_compose::render_ui_ir_with_swf_overlay;
use crate::ir_compose::render_ui_ir_document;
use crate::style::{ManufacturerStyle, StyleLoader};
use crate::swf_assets::SwfAssetLibrary;
use crate::ui_ir::{UiIrDocument, UiRendererHint};
use swf::Tag;

pub use crate::bb_atlas::AssetFetcher;

/// Static UI captures use the midpoint of authored BuildingBlocks animations.
/// This matches the accepted `*-current.png` regression artifacts and keeps live
/// ship-generated textures on the same sampled layout path.
pub const DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT: f32 = 50.0;

// ──────────────────────────────────────────────────────────────────────────────
// Fetcher traits
// ──────────────────────────────────────────────────────────────────────────────

/// Extract a DataCore record name from a BuildingBlocks file URL or bare name.
///
/// Strips an optional `file://` prefix, keeps only the final path component, and
/// removes a trailing `.json` extension case-insensitively.
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

/// Fetch a BuildingBlocks canvas record as a JSON [`serde_json::Value`].
///
/// Implementations look up `guid` in DataCore (or a test fixture map) and
/// return the record body JSON.
pub trait CanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError>;

    /// Fetch a canvas by DataCore file URL, path, or bare record name.
    fn fetch_canvas_by_path(&self, path_or_name: &str) -> Result<serde_json::Value, UiError> {
        let name = extract_record_name(path_or_name);
        self.fetch_canvas_by_name(&name)
    }

    /// Fetch a canvas by exact record name.
    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        Err(UiError::FetchFailed {
            guid: record_name.into(),
            source: "fetch_canvas_by_name not implemented".into(),
        })
    }
}

/// Fetch raw SWF bytes by their P4K archive path.
pub trait SwfFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError>;
}

/// Resolve a manufacturer style by its short manufacturer id (e.g. `"drak"`).
pub trait StyleFetcher {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError>;
}

// ──────────────────────────────────────────────────────────────────────────────
// Input types
// ──────────────────────────────────────────────────────────────────────────────

/// Borrowed snapshot of the fields from `UiBinding` that the pipeline needs.
///
/// `helper_name` is included for diagnostic messages **only** — it must never
/// be used to switch rendering behaviour.
pub struct UiBindingView<'a> {
    /// Container canvas GUID.
    pub canvas_guid: Option<&'a str>,
    /// Per-helper content canvas GUID (preferred when present).
    pub content_canvas_guid: Option<&'a str>,
    /// Binding category: `"mfd"`, `"physical"`, `"radar"`, `"door"`, etc.
    pub binding_kind: Option<&'a str>,
    /// Short manufacturer identifier for style lookup (e.g. `"drak"`).
    pub manufacturer_id: Option<&'a str>,
    /// Helper name — diagnostics only; never used to switch rendering logic.
    pub helper_name: Option<&'a str>,
    pub default_view_index: Option<u32>,
    pub default_screen_slot: Option<u32>,
}

/// All inputs required by [`render_for_binding`].
pub struct PipelineInputs<'a> {
    pub binding: &'a UiBindingView<'a>,
    pub canvas_fetcher: &'a dyn CanvasFetcher,
    pub swf_fetcher: &'a dyn SwfFetcher,
    pub style_fetcher: &'a dyn StyleFetcher,
    /// Asset fetcher for bitmap/SVG images referenced in BB nodes.
    pub asset_fetcher: &'a dyn crate::bb_atlas::AssetFetcher,
    /// Output raster size `(width, height)` in pixels.
    pub target_size: (u32, u32),
    /// Apply manufacturer post-process (tint, scanlines, vignette) after rasterisation.
    pub apply_postprocess: bool,
    /// Optional static animation sample as a percentage in `[0, 100]`.
    pub animation_sample_percent: Option<f32>,
    /// Localization table loaded from `global.ini` (key → display string).
    ///
    /// When `Some`, `labelProperties.label` keys (e.g. `@hud_NoTarget`) are
    /// resolved to their display strings.  When `None`, label fields are
    /// silently skipped and no localized text is emitted.
    pub localization_map: Option<std::collections::HashMap<String, String>>,
    /// Optional localization fetcher for brand-applied `@KEY` string resolution.
    ///
    /// When `Some`, string modifier values that start with `@` in brand-style
    /// modifiers are resolved to their display strings during scene construction.
    /// When `None`, `@KEY` strings are passed through as-is.
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StyleSelectionManifest {
    selected_source: Option<String>,
    fallback_counters: BTreeMap<String, u32>,
}

/// Compile canonical UI IR for the binding described by `inputs`.
///
/// This uses the same canvas-guid selection and BuildingBlocks scene resolve
/// path as [`render_for_binding`], but stops before rasterization.
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

    let scene = crate::bb_resolve::resolve_canvas_graph_with_loc(
        &raw_root_json,
        b.manufacturer_id,
        &|p| {
            inputs
                .canvas_fetcher
                .fetch_canvas_by_path(p)
                .map_err(|e| e.to_string())
        },
        inputs.loc_fetcher,
    )
    .map_err(UiError::RenderError)?;

    let swf_manifest = build_swf_selection_manifest(
        &raw_root_json,
        &resolved,
        b.manufacturer_id.unwrap_or("drak"),
        inputs.swf_fetcher,
    );
    let selected_swf_source = swf_manifest
        .valid_candidates
        .first()
        .map(|candidate| candidate.path.clone());

    let defaults = DefaultValueRegistry::with_pipeline_defaults(inputs.localization_map.clone());

    let asset_manifest = build_asset_reference_manifest(&scene, inputs.asset_fetcher);

    let mut ir = crate::ui_ir::compile_ui_ir_from_scene_with_animation_sample(
        &scene,
        Some(inputs.canvas_fetcher),
        effective_guid,
        canvas_name,
        inputs.target_size,
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
            .map(|(key, value)| (key.as_str(), *value)),
    ));
    Ok(ir)
}

/// Render a UI binding by first compiling canonical IR and then consuming only IR.
///
/// This entrypoint is the Phase 2 bridge toward deterministic IR consumption:
/// it shares binding/canvas resolution with [`compile_ir_for_binding`] and then
/// renders from [`UiIrDocument`] without consulting raw BB scene records.
pub fn render_for_binding_ir(inputs: &PipelineInputs<'_>) -> Result<Vec<u8>, UiError> {
    let ir = compile_ir_for_binding(inputs)?;

    let style = load_style_for_ir(&ir, inputs)?;
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
    let atlas = crate::bb_atlas::AtlasLibrary::new(
        inputs.asset_fetcher,
        inputs.binding.manufacturer_id,
    );

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

fn load_style_for_ir(
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

fn build_style_selection_manifest(
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
        .map(crate::pipeline::extract_record_name)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssetReferenceManifest {
    all_asset_refs: Vec<String>,
    resolved_asset_refs: Vec<String>,
    missing_asset_refs: Vec<String>,
}

fn build_asset_reference_manifest(
    scene: &crate::bb_scene::BbScene,
    asset_fetcher: &dyn crate::bb_atlas::AssetFetcher,
) -> AssetReferenceManifest {
    let mut all_asset_refs = Vec::new();
    let mut resolved_asset_refs = Vec::new();
    let mut missing_asset_refs = Vec::new();
    let mut seen_assets = std::collections::BTreeSet::new();

    for node in scene.nodes.values() {
        for asset_ref in crate::ui_ir::collect_node_asset_refs(node) {
            if !seen_assets.insert(asset_ref.clone()) {
                continue;
            }
            all_asset_refs.push(asset_ref.clone());
            let resolved = asset_fetcher.fetch_image_bytes(&asset_ref).is_some()
                || asset_fetcher.fetch_svg_bytes(&asset_ref).is_some();
            if resolved {
                resolved_asset_refs.push(asset_ref);
            } else {
                missing_asset_refs.push(asset_ref);
            }
        }
    }

    AssetReferenceManifest {
        all_asset_refs,
        resolved_asset_refs,
        missing_asset_refs,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Main entry point
// ──────────────────────────────────────────────────────────────────────────────

/// Render a UI canvas described by `inputs` and return the PNG bytes.
///
/// Prefers `content_canvas_guid` over `canvas_guid` when both are present.
/// Returns [`UiError::RenderError`] when no canvas GUID is available.
pub fn render_for_binding(inputs: &PipelineInputs<'_>) -> Result<Vec<u8>, UiError> {
    render_for_binding_ir(inputs)
}


// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Collect all distinct SWF paths referenced by `canvas` (root + children).
fn collect_swf_paths(canvas: &ResolvedCanvas) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    collect_from_record(&canvas.root, &mut seen, &mut paths);
    for child in canvas.children.values() {
        collect_from_record(child, &mut seen, &mut paths);
    }
    paths
}

fn collect_from_record(
    record: &CanvasRecord,
    seen: &mut HashSet<String>,
    paths: &mut Vec<String>,
) {
    for item in &record.scene {
        collect_from_scene_item(item, seen, paths);
    }
}

fn collect_from_scene_item(
    item: &SceneItem,
    seen: &mut HashSet<String>,
    paths: &mut Vec<String>,
) {
    let suffix = item.kind.strip_prefix("BuildingBlocks_").unwrap_or(&item.kind);
    match suffix {
        "Sprite" | "WidgetSprite" | "SpriteInstance" | "MovieClip" => {
            let path = item
                .properties
                .get("swfPath")
                .or_else(|| item.properties.get("swf"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !path.is_empty() && seen.insert(path.clone()) {
                paths.push(path);
            }
        }
        _ => {}
    }
}

/// Try to load the first SWF in `paths` from the fetcher.
///
/// Falls back to a minimal empty [`SwfAssetLibrary`] if `paths` is empty or
/// all fetches fail.
fn load_first_swf(paths: &[String], fetcher: &dyn SwfFetcher) -> SwfAssetLibrary {
    for path in paths {
        match fetcher.fetch_swf_bytes(path) {
            Ok(bytes) => {
                let mut root_lib = match SwfAssetLibrary::new(bytes.clone()) {
                    Ok(lib) => lib,
                    Err(e) => {
                        log::warn!("pipeline: failed to parse SWF '{}': {}", path, e);
                        continue;
                    }
                };
                let mut pending = vec![(normalize_p4k_swf_path(path), bytes)];
                let mut seen = HashSet::new();

                // BuildingBlocks text fields use Canvas.swf as the font-template
                // source and fonts_en.gfx as the shared glyph source. Some game
                // SWFs reference these indirectly, so include both sources even
                // when the primary SWF has no explicit imports.
                let canvas_path = "Data/UI/BuildingBlocks/assets/SWF/Canvas.swf".to_string();
                if let Ok(canvas_bytes) = fetcher.fetch_swf_bytes(&canvas_path) {
                    if let Err(e) = root_lib.merge_swf_bytes(&canvas_bytes) {
                        log::debug!(
                            "pipeline: failed to merge BuildingBlocks canvas SWF '{}': {}",
                            canvas_path,
                            e
                        );
                    }
                    pending.push((canvas_path, canvas_bytes));
                } else if std::env::var("SB_UI_FONT_TELEMETRY").is_ok() {
                    eprintln!("pipeline-font: canvas SWF fetch failed for Data/UI/BuildingBlocks/assets/SWF/Canvas.swf");
                }

                let shared_fonts_path = "Data/UI/fonts/Shared/fonts_en.gfx".to_string();
                if let Ok(shared_bytes) = fetcher.fetch_swf_bytes(&shared_fonts_path) {
                    if let Err(e) = root_lib.merge_swf_bytes(&shared_bytes) {
                        log::debug!(
                            "pipeline: failed to merge shared fonts SWF '{}': {}",
                            shared_fonts_path,
                            e
                        );
                    }
                    pending.push((shared_fonts_path, shared_bytes));
                } else if std::env::var("SB_UI_FONT_TELEMETRY").is_ok() {
                    eprintln!("pipeline-font: shared fonts fetch failed for Data/UI/fonts/Shared/fonts_en.gfx");
                }

                while let Some((current_path, current_bytes)) = pending.pop() {
                    if !seen.insert(current_path.clone()) {
                        continue;
                    }
                    for import_path in collect_import_swf_paths(&current_path, &current_bytes) {
                        if seen.contains(&import_path) {
                            continue;
                        }
                        match fetcher.fetch_swf_bytes(&import_path) {
                            Ok(import_bytes) => {
                                if let Err(e) = root_lib.merge_swf_bytes(&import_bytes) {
                                    log::debug!(
                                        "pipeline: failed to merge imported SWF '{}': {}",
                                        import_path,
                                        e
                                    );
                                }
                                pending.push((import_path, import_bytes));
                            }
                            Err(e) => {
                                log::debug!(
                                    "pipeline: import SWF fetch failed for '{}': {}",
                                    import_path,
                                    e
                                );
                            }
                        }
                    }
                }
                if std::env::var("SB_UI_FONT_TELEMETRY").is_ok() {
                    eprintln!(
                        "pipeline-font: selected='{}' fonts={} exports={}",
                        path,
                        root_lib.font_count(),
                        root_lib.export_count()
                    );
                }
                return root_lib;
            }
            Err(e) => {
                log::debug!("pipeline: SWF fetch failed for '{}': {}", path, e);
            }
        }
    }

    let canvas_path = "Data/UI/BuildingBlocks/assets/SWF/Canvas.swf".to_string();
    let shared_fonts_path = "Data/UI/fonts/Shared/fonts_en.gfx".to_string();
    if let Ok(shared_bytes) = fetcher.fetch_swf_bytes(&shared_fonts_path) {
        let lib = if let Ok(canvas_bytes) = fetcher.fetch_swf_bytes(&canvas_path) {
            match SwfAssetLibrary::new(canvas_bytes) {
                Ok(mut canvas_lib) => {
                    if let Err(e) = canvas_lib.merge_swf_bytes(&shared_bytes) {
                        log::debug!(
                            "pipeline: failed to merge shared fonts SWF '{}' into fallback canvas library: {}",
                            shared_fonts_path,
                            e
                        );
                    }
                    canvas_lib
                }
                Err(e) => {
                    log::debug!(
                        "pipeline: failed to parse fallback canvas SWF '{}': {}",
                        canvas_path,
                        e
                    );
                    match SwfAssetLibrary::new(shared_bytes) {
                        Ok(lib) => lib,
                        Err(_) => break_fallback_swf_load(),
                    }
                }
            }
        } else {
            match SwfAssetLibrary::new(shared_bytes) {
                Ok(lib) => lib,
                Err(_) => break_fallback_swf_load(),
            }
        };
        if std::env::var("SB_UI_FONT_TELEMETRY").is_ok() {
            eprintln!(
                "pipeline-font: fallback='{}' fonts={} exports={}",
                shared_fonts_path,
                lib.font_count(),
                lib.export_count()
            );
        }
        return lib;
    }

    fn break_fallback_swf_load() -> SwfAssetLibrary {
        let minimal: Vec<u8> = vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        SwfAssetLibrary::new(minimal).expect("minimal SWF is always valid")
    }

    // Minimal valid uncompressed SWF — no tags.
    let minimal: Vec<u8> = vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    SwfAssetLibrary::new(minimal).expect("minimal SWF is always valid")
}

fn normalize_p4k_swf_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    if replaced.to_ascii_lowercase().starts_with("data/") {
        replaced
    } else {
        format!("Data/{replaced}")
    }
}

fn resolve_relative_swf_path(base_path: &str, import_url: &str) -> String {
    let import = import_url.replace('\\', "/");
    if import.is_empty() {
        return String::new();
    }
    if import.to_ascii_lowercase().starts_with("data/") {
        return import;
    }

    let base_norm = base_path.replace('\\', "/");
    let mut base_parts: Vec<&str> = base_norm.split('/').collect();
    if !base_parts.is_empty() {
        base_parts.pop();
    }

    for part in import.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !base_parts.is_empty() {
                    base_parts.pop();
                }
            }
            other => base_parts.push(other),
        }
    }

    let joined = base_parts.join("/");
    if joined.to_ascii_lowercase().starts_with("data/") {
        joined
    } else {
        format!("Data/{joined}")
    }
}

fn collect_import_swf_paths(current_path: &str, bytes: &[u8]) -> Vec<String> {
    let Ok(swf_buf) = swf::decompress_swf(Cursor::new(bytes)) else {
        return Vec::new();
    };
    let Ok(parsed) = swf::parse_swf(&swf_buf) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for tag in &parsed.tags {
        if let Tag::ImportAssets { url, .. } = tag {
            let url_text = url.to_string_lossy(swf::UTF_8);
            let resolved = resolve_relative_swf_path(current_path, &url_text);
            if !resolved.is_empty() {
                out.push(resolved);
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SwfPathCandidate {
    path: String,
    reason: &'static str,
    rank: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SwfSelectionManifest {
    flash_candidates: Vec<SwfPathCandidate>,
    resolved_scene_candidates: Vec<SwfPathCandidate>,
    ordered_candidates: Vec<SwfPathCandidate>,
    valid_candidates: Vec<SwfPathCandidate>,
    fallback_counters: BTreeMap<String, u32>,
}

fn build_swf_selection_manifest(
    raw_root_json: &serde_json::Value,
    resolved: &ResolvedCanvas,
    manufacturer_id: &str,
    fetcher: &dyn SwfFetcher,
) -> SwfSelectionManifest {
    let flash_candidates = if canvas_has_flash_renderer(raw_root_json) {
        let mut source_candidates = flash_swf_candidates_from_canvas_refs(raw_root_json, manufacturer_id);
        let record_name = canvas_record_name(raw_root_json).unwrap_or("");
        source_candidates.extend(flash_swf_candidates(record_name, manufacturer_id));
        let deduped = source_candidates
            .into_iter()
            .fold(Vec::<String>::new(), |mut acc, path| {
                if !acc.iter().any(|existing| existing.eq_ignore_ascii_case(&path)) {
                    acc.push(path);
                }
                acc
            });
        deduped
            .into_iter()
            .enumerate()
            .map(|(index, path)| SwfPathCandidate {
                path,
                reason: "flash_structural_candidate",
                rank: index as u32,
            })
            .collect()
    } else {
        Vec::new()
    };

    let resolved_scene_candidates = collect_swf_paths(resolved)
        .into_iter()
        .enumerate()
        .map(|(index, path)| SwfPathCandidate {
            path,
            reason: "resolved_scene_reference",
            rank: 1000 + index as u32,
        })
        .collect::<Vec<_>>();
    let ordered_candidates = merge_unique_candidates(&flash_candidates, &resolved_scene_candidates);
    // Keep selection deterministic but avoid validating every candidate path
    // once a viable source is found.
    let mut valid_candidates = Vec::new();
    for candidate in &ordered_candidates {
        let Ok(bytes) = fetcher.fetch_swf_bytes(&candidate.path) else {
            continue;
        };
        if SwfAssetLibrary::new(bytes).is_ok() {
            valid_candidates.push(candidate.clone());
            break;
        }
    }

    let mut fallback_counters = BTreeMap::new();
    if let Some(selected) = valid_candidates.first() {
        if selected.reason == "resolved_scene_reference" {
            fallback_counters.insert("swf_resolved_scene_fallback".to_string(), 1);
        }
    } else if !ordered_candidates.is_empty() {
        fallback_counters.insert("swf_candidate_miss".to_string(), 1);
    }

    SwfSelectionManifest {
        flash_candidates,
        resolved_scene_candidates,
        ordered_candidates,
        valid_candidates,
        fallback_counters,
    }
}

fn merge_unique_candidates(
    primary: &[SwfPathCandidate],
    secondary: &[SwfPathCandidate],
) -> Vec<SwfPathCandidate> {
    let mut out = Vec::with_capacity(primary.len() + secondary.len());
    let mut seen = HashSet::new();
    for candidate in primary.iter().chain(secondary.iter()) {
        if seen.insert(candidate.path.clone()) {
            out.push(candidate.clone());
        }
    }
    out
}

fn fallback_counter_warnings<'a>(
    counters: impl IntoIterator<Item = (&'a str, u32)>,
) -> Vec<String> {
    counters
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(key, count)| format!("fallback path used: {key}={count}"))
        .collect()
}

/// Load the manufacturer style for `manufacturer_id` via `fetcher`.
///
/// Falls back to Drake amber defaults when `manufacturer_id` is `None` or
/// when the fetch / parse fails.
fn load_style(manufacturer_id: Option<&str>, fetcher: &dyn StyleFetcher) -> ManufacturerStyle {
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

// ─────────────────────────────────────────────────────────────────────────────
// Flash canvas detection and SWF path derivation
// ─────────────────────────────────────────────────────────────────────────────

/// Extract the `_RecordName_` from a canvas root JSON.
fn canvas_record_name(root_json: &serde_json::Value) -> Option<&str> {
    root_json.get("_RecordName_")?.as_str()
}

/// Return `true` when any top-level scene item in the canvas JSON declares
/// `rendererType: "Flash"`, which indicates that the canvas content is driven
/// by a standalone SWF rather than the BB widget system.
fn canvas_has_flash_renderer(root_json: &serde_json::Value) -> bool {
    let Some(rv) = root_json.get("_RecordValue_") else {
        return false;
    };
    let Some(scene) = rv.get("scene") else {
        return false;
    };
    let Some(items) = scene.as_array() else {
        return false;
    };
    items.iter().any(|item| {
        item.get("rendererType")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("Flash"))
    })
}

/// Derive candidate P4K paths for a standalone SWF given the canvas record
/// name and the manufacturer id.
///
/// The derivation is structural:
/// 1. Extract the **stem** from the canvas record name by stripping the
///    `BuildingBlocks_Canvas.` prefix (if present), then stripping an
///    optional leading segment and trailing `_Master` suffix.  For example
///    `MC_S_Target_Master` → `Target`.
/// 2. Derive the P4K brand code as the first three characters of the
///    manufacturer id, upper-cased (e.g. `"rsi"` → `"RSI"`, `"drak"` →
///    `"DRA"`, `"aegs"` → `"AEG"`).  This mapping is a structural rule
///    observed across the `Data\UI\ShipInterface\assets\SWF\` layout.
/// 3. Build candidates under the `SupportScreen16-9` subdir:
///    - `{stem}Status.swf` (matches `TargetStatus.swf`, `OwnShipStatus.swf`, …)
///    - `{stem}.swf`        (fallback for SWFs without "Status" suffix)
///
/// **Annunciator screens** use a completely different layout:
/// `{BRAND}\{SHIP}\AnnunciatorScreen\AnnunciatorHalve{N}.swf`.
/// When a ship has no dedicated Annunciator SWF (e.g. DRAK_Clipper), the
/// renderer falls back to a brand-representative ship's SWFs, following the
/// same structural principle used for the RSI SupportScreen16-9 fallback.
///
/// The generic `MC_S_*_Master` canvas family is manufacturer-independent and
/// its Flash SWFs are canonically hosted under `RSI\SupportScreen16-9\`.
/// When the primary brand-specific path does not exist (e.g. `DRA` ships that
/// share these generic canvases without a ship-specific override), the RSI
/// canonical path is appended as a structural fallback.
///
/// Returns the list in preference order; the caller tries each in turn.
pub fn flash_swf_candidates(record_name: &str, manufacturer_id: &str) -> Vec<String> {
    // Strip optional DataCore prefix.
    let name = record_name
        .strip_prefix("BuildingBlocks_Canvas.")
        .unwrap_or(record_name);

    // Brand code: first ≤3 chars of manufacturer_id, upper-cased.
    let brand: String = manufacturer_id
        .chars()
        .take(3)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if brand.is_empty() {
        return vec![];
    }

    // ── Annunciator screens ────────────────────────────────────────────────
    // Canvas names of the form `H_Eng_Annunciator_Master_Left` /
    // `H_Eng_Annunciator_Master_Right` map to dedicated SWF halves.
    // These live under `{BRAND}\{optional_ship}\AnnunciatorScreen\AnnunciatorHalve{N}.swf`
    // rather than under `SupportScreen16-9\`.
    if name.to_ascii_lowercase().contains("annunciator") {
        return annunciator_swf_candidates(name, &brand);
    }

    // Detect whether this is a generic multi-ship canvas (MC_S_ prefix).  The
    // generic canvases use Flash SWFs that live under RSI\SupportScreen16-9\
    // regardless of which manufacturer's ship embeds them.
    let is_generic_mc = name.starts_with("MC_S_") || name.starts_with("GEN_MC_S_");

    // Extract the stem: strip a leading "MC_S_" or "GEN_MC_S_" compound and
    // a trailing "_Master" or "_master" suffix, case-insensitively.
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

    let mut candidates = support_screen_candidates_for_brand(&brand, stem);

    // For generic MC_S_* canvases used by non-RSI ships, append the RSI
    // support-screen variants as structural fallbacks. RSI is the primary host
    // for many shared generic MC_S_* canvas SWFs in the P4K layout.
    if is_generic_mc && brand != "RSI" {
        candidates.extend(support_screen_candidates_for_brand("RSI", stem));
    }

    candidates
}

fn flash_swf_candidates_from_canvas_refs(
    raw_root_json: &serde_json::Value,
    manufacturer_id: &str,
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
            candidates.extend(flash_swf_candidates_from_stem(&stem, manufacturer_id));
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

fn flash_swf_candidates_from_stem(stem: &str, manufacturer_id: &str) -> Vec<String> {
    let brand: String = manufacturer_id
        .chars()
        .take(3)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if brand.is_empty() || stem.is_empty() {
        return Vec::new();
    }
    support_screen_candidates_for_brand(&brand, stem)
}

fn support_screen_candidates_for_brand(brand: &str, stem: &str) -> Vec<String> {
    if brand.is_empty() || stem.is_empty() {
        return Vec::new();
    }
    let bases = [
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\SupportScreen16-9\"),
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\SupportScreen1-1\"),
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\SupportScreenBespoke2\"),
        format!(r"Data\UI\ShipInterface\assets\SWF\{brand}\Support_Bespoke_2\"),
    ];
    bases
        .into_iter()
        .flat_map(|base| {
            [
                format!("{base}{stem}Status.swf"),
                format!("{base}{stem}.swf"),
            ]
        })
        .collect()
}

/// Build candidate P4K paths for an annunciator canvas.
///
/// Annunciator SWFs live under:
/// `Data\UI\ShipInterface\assets\SWF\{BRAND}\{optional_ship}\AnnunciatorScreen\AnnunciatorHalve{N}.swf`
///
/// The halve number is derived from the canvas name:
/// - names containing `_Left`  → Halve1
/// - names containing `_Right` → Halve2
///
/// When a brand has no direct `{BRAND}\AnnunciatorScreen\` path, the
/// candidates include brand-representative ships known from the P4K layout.
/// This is a content-addressing lookup table — the same ships serve as the
/// canonical SWF source for all ships of that brand that lack their own
/// Annunciator SWF (structural fallback, analogous to the RSI
/// SupportScreen16-9 fallback for generic MC_S_* canvases).
fn annunciator_swf_candidates(canvas_name: &str, brand: &str) -> Vec<String> {
    let name_lower = canvas_name.to_ascii_lowercase();

    // Determine which halve from Left/Right suffix.
    let halve = if name_lower.contains("_left") {
        1u8
    } else if name_lower.contains("_right") {
        2u8
    } else {
        1u8 // default to Halve1 when indeterminate
    };

    let halve_file = format!("AnnunciatorHalve{halve}.swf");
    let swf_root = r"Data\UI\ShipInterface\assets\SWF\";

    let mut candidates = Vec::new();

    // Brand-level direct path — present for some brands (e.g. AEG).
    candidates.push(format!(
        r"{swf_root}{brand}\AnnunciatorScreen\{halve_file}"
    ));

    // Brand-representative ships for brands that host Annunciator SWFs under
    // a ship subdirectory.  These are structural representatives observed in
    // the P4K layout; all ships of the same brand that lack their own SWF
    // fall back to these.
    let ship_fallbacks: &[&str] = match brand {
        "DRA" => &["DRAK_Buccaneer", "DRAK_Dragonfly"],
        "ORI" => &["ORIG_85X"],
        "MIS" => &["MISC_Freelancer_Base"],
        _ => &[],
    };
    for ship in ship_fallbacks {
        candidates.push(format!(
            r"{swf_root}{brand}\{ship}\AnnunciatorScreen\{halve_file}"
        ));
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyFetcher;

    impl SwfFetcher for EmptyFetcher {
        fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
            Err(UiError::RenderError("missing swf".to_string()))
        }
    }

    #[test]
    fn merge_unique_candidates_preserves_reason_and_rank_of_first_path() {
        let primary = vec![SwfPathCandidate {
            path: "A.swf".to_string(),
            reason: "primary",
            rank: 1,
        }];
        let secondary = vec![
            SwfPathCandidate {
                path: "A.swf".to_string(),
                reason: "secondary-duplicate",
                rank: 99,
            },
            SwfPathCandidate {
                path: "B.swf".to_string(),
                reason: "secondary",
                rank: 2,
            },
        ];

        let merged = merge_unique_candidates(&primary, &secondary);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].reason, "primary");
        assert_eq!(merged[0].rank, 1);
        assert_eq!(merged[1].path, "B.swf");
    }

    #[test]
    fn swf_selection_manifest_contains_structural_flash_candidates() {
        let root = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.MC_S_Target_Master",
            "_RecordValue_": {
                "scene": [{"rendererType": "Flash"}]
            }
        });
        let resolved = ResolvedCanvas {
            root: crate::canvas::CanvasRecord {
                guid: "root-guid".to_string(),
                name: "Root".to_string(),
                views: Vec::new(),
                scene: Vec::new(),
                operations: Vec::new(),
            },
            children: std::collections::HashMap::new(),
        };
        let fetcher = EmptyFetcher;

        let manifest = build_swf_selection_manifest(&root, &resolved, "drak", &fetcher);

        assert!(!manifest.flash_candidates.is_empty());
        assert_eq!(manifest.ordered_candidates, manifest.flash_candidates);
        assert_eq!(manifest.flash_candidates[0].reason, "flash_structural_candidate");
        assert!(manifest.valid_candidates.is_empty());
        assert_eq!(manifest.fallback_counters.get("swf_candidate_miss"), Some(&1));
    }

    #[test]
    fn flash_candidates_prefer_canvas_reference_stem_when_present() {
        let root = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.MC_S_MissionData_Master",
            "_RecordValue_": {
                "scene": [{"rendererType": "Flash"}],
                "defaultStyles": {
                    "entries": [{
                        "modifiers": [{
                            "field": {
                                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                "value": "file://./types/gen_mc_s_target.json"
                            }
                        }]
                    }]
                },
                "brandStyles": []
            }
        });
        let resolved = ResolvedCanvas {
            root: crate::canvas::CanvasRecord {
                guid: "root-guid".to_string(),
                name: "Root".to_string(),
                views: Vec::new(),
                scene: Vec::new(),
                operations: Vec::new(),
            },
            children: std::collections::HashMap::new(),
        };
        let fetcher = EmptyFetcher;

        let manifest = build_swf_selection_manifest(&root, &resolved, "rsi", &fetcher);

        assert!(manifest.flash_candidates[0]
            .path
            .to_ascii_lowercase()
            .ends_with("targetstatus.swf"));
        assert!(manifest
            .flash_candidates
            .iter()
            .any(|candidate| candidate.path.to_ascii_lowercase().ends_with("missiondatastatus.swf")));
    }

    #[test]
    fn flash_candidates_cover_structural_supportscreen_variants() {
        let candidates = flash_swf_candidates("BuildingBlocks_Canvas.MC_S_Target_Master", "aegs");

        assert!(candidates.iter().any(|path| path.contains("SupportScreen16-9\\TargetStatus.swf")));
        assert!(candidates.iter().any(|path| path.contains("SupportScreen1-1\\TargetStatus.swf")));
        assert!(candidates.iter().any(|path| path.contains("SupportScreenBespoke2\\TargetStatus.swf")));
        assert!(candidates.iter().any(|path| path.contains("Support_Bespoke_2\\TargetStatus.swf")));
    }

    struct MissingCanvasStyleFetcher;

    impl CanvasFetcher for MissingCanvasStyleFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
            Err(UiError::FetchFailed {
                guid: guid.to_string(),
                source: "missing canvas".to_string().into(),
            })
        }
    }

    struct MissingManufacturerStyleFetcher;

    impl StyleFetcher for MissingManufacturerStyleFetcher {
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

    #[test]
    fn pipeline_defaults_seeds_placeholder_keys_and_merges_localization() {
        let defaults = DefaultValueRegistry::with_pipeline_defaults(Some(std::collections::HashMap::from([(
            "hud_custom".to_string(),
            "CUSTOM".to_string(),
        )])));

        assert_eq!(defaults.lookup_localization("@hud_custom"), Some("CUSTOM"));
        assert_eq!(defaults.lookup_localization("@loc_placeholder"), Some(""));
        assert_eq!(defaults.lookup_localization("@loc_empty"), Some(""));
    }

    #[test]
    fn fallback_counter_warnings_emit_human_readable_messages() {
        let warnings = fallback_counter_warnings([
            ("swf_candidate_miss", 1),
            ("manufacturer_style_fallback_drak", 2),
            ("ignored_zero", 0),
        ]);

        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0], "fallback path used: swf_candidate_miss=1");
        assert_eq!(warnings[1], "fallback path used: manufacturer_style_fallback_drak=2");
    }
}
