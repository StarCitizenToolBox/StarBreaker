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

// ──────────────────────────────────────────────────────────────────────────────
// Fetcher traits
// ──────────────────────────────────────────────────────────────────────────────

/// Fetch a BuildingBlocks canvas record as a JSON [`serde_json::Value`].
///
/// Implementations look up `guid` in DataCore (or a test fixture map) and
/// return the record body JSON.
pub trait CanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError>;
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
    let resolved: ResolvedCanvas = resolver.resolve(effective_guid, |guid| {
        inputs.canvas_fetcher.fetch_canvas_json(guid)
    })?;

    // ── 2. Collect SWF paths ────────────────────────────────────────────────

    let swf_paths = collect_swf_paths(&resolved);

    // ── 3. Load first available SWF (lazy) ─────────────────────────────────

    let assets = load_first_swf(&swf_paths, inputs.swf_fetcher);

    // ── 4. Manufacturer style ───────────────────────────────────────────────

    let style = load_style(b.manufacturer_id, inputs.style_fetcher);

    // ── 5. Defaults registry ────────────────────────────────────────────────

    let defaults = DefaultValueRegistry::with_well_known_path_defaults();

    // ── 6. Rasterise ───────────────────────────────────────────────────────

    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let target = ComposeTarget { width: inputs.target_size.0, height: inputs.target_size.1 };
    let opts = PostProcessOptions::default();

    let img = if inputs.apply_postprocess {
        render_canvas_with_postprocess(&resolved, &ctx, target, &opts)?
    } else {
        crate::compose::render_canvas(&resolved, &ctx, target)?
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
