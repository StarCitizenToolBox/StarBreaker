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

use std::collections::HashSet;

use crate::canvas::{CanvasRecord, CanvasWidgetTreeResolver, ResolvedCanvas, SceneItem};
use crate::compose::{ComposeContext, ComposeTarget, encode_png, render_canvas_with_postprocess};
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::postprocess::PostProcessOptions;
use crate::style::{ManufacturerStyle, StyleLoader};
use crate::swf_assets::SwfAssetLibrary;

pub use crate::bb_atlas::AssetFetcher;

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
}

// ──────────────────────────────────────────────────────────────────────────────
// Main entry point
// ──────────────────────────────────────────────────────────────────────────────

/// Render a UI canvas described by `inputs` and return the PNG bytes.
///
/// Prefers `content_canvas_guid` over `canvas_guid` when both are present.
/// Returns [`UiError::RenderError`] when no canvas GUID is available.
pub fn render_for_binding(inputs: &PipelineInputs<'_>) -> Result<Vec<u8>, UiError> {
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

    // ── 1. Resolve canvas tree ──────────────────────────────────────────────

    let resolver = CanvasWidgetTreeResolver::new();
    let raw_root_json = inputs.canvas_fetcher.fetch_canvas_json(effective_guid).ok();
    let resolved: ResolvedCanvas = resolver.resolve(effective_guid, |guid| {
        inputs.canvas_fetcher.fetch_canvas_json(guid)
    })?;

    // ── 1b. Resolve BbScene (used for diagnostics, probe, and rendering) ──────

    let bb_scene_opt: Option<crate::bb_scene::BbScene> =
        raw_root_json.as_ref().and_then(|root_json| {
            crate::bb_resolve::resolve_canvas_graph(root_json, b.manufacturer_id, &|p| {
                inputs
                    .canvas_fetcher
                    .fetch_canvas_by_path(p)
                    .map_err(|e| e.to_string())
            })
            .map_err(|e| {
                log::warn!(
                    "bb_scene resolve failed for helper {:?} canvas {}: {}",
                    b.helper_name,
                    effective_guid,
                    e,
                );
            })
            .ok()
        });

    if let Some(ref scene) = bb_scene_opt {
        let mut type_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for node in scene.nodes.values() {
            let key = format!("{:?}", node.ty);
            *type_counts.entry(key).or_insert(0) += 1;
        }
        log::info!(
            "bb_scene[{}]: canvas={:?} size=({:.0}x{:.0}) merged nodes={} roots={} types={:?}",
            b.helper_name.unwrap_or("?"),
            effective_guid,
            scene.canvas_size.0,
            scene.canvas_size.1,
            scene.nodes.len(),
            scene.roots.len(),
            type_counts,
        );

        // A3 probe: per-node summary for first pilot binding.
        // Guarded by env var BB_A3_PROBE=1; removed after probe run.
        if std::env::var("BB_A3_PROBE").as_deref() == Ok("1")
            && b.helper_name == Some("Screen_Right_Upper_RTT")
        {
            let probe_layout =
                crate::bb_layout::layout(scene, inputs.target_size.0, inputs.target_size.1);
            for (&node_id, node) in &scene.nodes {
                let rect = probe_layout
                    .rects
                    .get(&node_id)
                    .copied()
                    .unwrap_or_default();
                eprintln!(
                    "A3-probe: id=ptr:{node_id} parent={} type={:?} name={:?} \
                     rect=({:.0},{:.0},{:.0},{:.0}) bg={} icon={} text={}",
                    node.parent
                        .map(|p| format!("ptr:{p}"))
                        .unwrap_or_else(|| "none".to_string()),
                    node.ty,
                    node.name,
                    rect.x,
                    rect.y,
                    rect.w,
                    rect.h,
                    node.background.is_some(),
                    node.icon.is_some(),
                    node.text.is_some(),
                );
            }
        }
    }

    // ── 1c. Atlas diagnostic ────────────────────────────────────────────────
    if let Some(ref scene) = bb_scene_opt {
        let atlas = crate::bb_atlas::AtlasLibrary::new(inputs.asset_fetcher, b.manufacturer_id);
        let mut resolved_count = 0usize;
        let mut first_miss: Option<String> = None;

        for node in scene.nodes.values() {
            let w = bb_value_dimension(&node.sizing.width).round() as u32;
            let h = bb_value_dimension(&node.sizing.height).round() as u32;
            let tw = if w > 0 { w } else { 64 };
            let th = if h > 0 { h } else { 64 };
            if atlas.resolve_for_node(node, tw, th).is_some() {
                resolved_count += 1;
            } else {
                let miss_path = node
                    .icon
                    .as_ref()
                    .and_then(|i| i.image_record.as_deref())
                    .or_else(|| {
                        node.background
                            .as_ref()
                            .and_then(|bg| bg.svg_fill_path.as_deref())
                    })
                    .unwrap_or("");
                if !miss_path.is_empty() && first_miss.is_none() {
                    first_miss = Some(miss_path.to_string());
                }
            }
        }

        let with_ref = scene
            .nodes
            .values()
            .filter(|n| {
                n.icon
                    .as_ref()
                    .and_then(|i| i.image_record.as_deref())
                    .filter(|p| !p.is_empty())
                    .is_some()
                    || n.background
                        .as_ref()
                        .and_then(|bg| bg.svg_fill_path.as_deref())
                        .filter(|p| !p.is_empty())
                        .is_some()
            })
            .count();
        let missed = with_ref.saturating_sub(resolved_count);
        log::info!(
            "atlas[helper={}]: nodes_with_image={}, resolved={}, missed={} (first miss: {})",
            b.helper_name.unwrap_or("?"),
            with_ref,
            resolved_count,
            missed,
            first_miss.as_deref().unwrap_or("none"),
        );
    }

    // ── 2. Collect SWF paths ────────────────────────────────────────────────

    let swf_paths = collect_swf_paths(&resolved);

    // ── 3. Load first available SWF (lazy) ─────────────────────────────────

    let assets = load_first_swf(&swf_paths, inputs.swf_fetcher);

    // ── 4. Manufacturer style ───────────────────────────────────────────────

    let style = load_style(b.manufacturer_id, inputs.style_fetcher);

    // ── 5. Defaults registry ────────────────────────────────────────────────

    let defaults = DefaultValueRegistry::with_well_known_path_defaults();

    // ── 6. Rasterise ───────────────────────────────────────────────────────
    //
    // Prefer render_bb_scene (A3 compositor) when BbScene resolution succeeded.
    // Fall back to render_canvas (magenta placeholder) when it failed.
    // apply_postprocess is kept false until Phase A5.

    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let target = ComposeTarget { width: inputs.target_size.0, height: inputs.target_size.1 };

    let img = if let Some(ref scene) = bb_scene_opt {
        let atlas = crate::bb_atlas::AtlasLibrary::new(inputs.asset_fetcher, b.manufacturer_id);
        crate::compose::render_bb_scene(scene, &ctx, &atlas, target)?
    } else {
        // BbScene resolution failed — fall back to magenta placeholder.
        let opts = PostProcessOptions::default();
        if inputs.apply_postprocess {
            render_canvas_with_postprocess(&resolved, &ctx, target, &opts)?
        } else {
            crate::compose::render_canvas(&resolved, &ctx, target)?
        }
    };

    encode_png(&img)
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
            Ok(bytes) => match SwfAssetLibrary::new(bytes) {
                Ok(lib) => return lib,
                Err(e) => {
                    log::warn!("pipeline: failed to parse SWF '{}': {}", path, e);
                }
            },
            Err(e) => {
                log::debug!("pipeline: SWF fetch failed for '{}': {}", path, e);
            }
        }
    }
    // Minimal valid uncompressed SWF — no tags.
    let minimal: Vec<u8> = vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    SwfAssetLibrary::new(minimal).expect("minimal SWF is always valid")
}

fn bb_value_dimension(value: &crate::bb_scene::BbValue) -> f32 {
    match value {
        crate::bb_scene::BbValue::Fixed(v)
        | crate::bb_scene::BbValue::Percent(v)
        | crate::bb_scene::BbValue::Other { value: v, .. } => *v,
    }
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
