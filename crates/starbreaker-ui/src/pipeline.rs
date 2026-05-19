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
    /// Localization table loaded from `global.ini` (key → display string).
    ///
    /// When `Some`, `labelProperties.label` keys (e.g. `@hud_NoTarget`) are
    /// resolved to their display strings.  When `None`, label fields are
    /// silently skipped and no localized text is emitted.
    pub localization_map: Option<std::collections::HashMap<String, String>>,
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

    // ── 3. Load SWF asset library ───────────────────────────────────────────
    //
    // When the root canvas carries `BuildingBlocks_FlashRendererPolicy` on its
    // scene items, the shapes for `WidgetCustomShape` nodes are NOT embedded in
    // per-node `swfPath` fields — they live in a standalone SWF keyed by the
    // canvas record name and manufacturer brand code.  Load that SWF first so
    // that `draw_swf_symbol` can resolve exported names (shape_Chevron, etc.).
    // Fall back to the per-node library when no Flash SWF is found or when the
    // canvas does not use Flash rendering.

    let flash_swf_opt: Option<SwfAssetLibrary> = raw_root_json
        .as_ref()
        .filter(|j| canvas_has_flash_renderer(j))
        .and_then(|j| {
            let record_name = canvas_record_name(j).unwrap_or("");
            let manufacturer_id = b.manufacturer_id.unwrap_or("drak");
            let candidates = flash_swf_candidates(record_name, manufacturer_id);
            let lib = load_first_swf(&candidates, inputs.swf_fetcher);
            let (fsw, fsh) = lib.stage_size();
            if fsw > 0.0 || lib.shape_count() > 0 {
                log::info!(
                    "flash_swf[{}]: loaded SWF ({fsw:.0}×{fsh:.0}, {} shapes) for canvas={record_name:?}",
                    b.helper_name.unwrap_or("?"),
                    lib.shape_count(),
                );
                Some(lib)
            } else {
                log::debug!(
                    "flash_swf[{}]: no SWF found for Flash canvas {record_name:?} \
                     (candidates: {candidates:?})",
                    b.helper_name.unwrap_or("?"),
                );
                None
            }
        });

    let fallback_assets = load_first_swf(&swf_paths, inputs.swf_fetcher);
    // Prefer the Flash SWF when available — it holds all the shape symbols for
    // the canvas.  Fall back to the first per-node SWF referenced by canvas
    // records.
    let assets = flash_swf_opt.as_ref().unwrap_or(&fallback_assets);

    // ── 4. Manufacturer style ───────────────────────────────────────────────

    let style = load_style(b.manufacturer_id, inputs.style_fetcher);

    // ── 5. Defaults registry ────────────────────────────────────────────────

    let mut defaults = DefaultValueRegistry::with_well_known_path_defaults();
    // Only override the built-in fallback table when the P4K map is non-empty.
    // global.ini is absent from the SC 4.x P4K, so the live map is usually
    // empty and the static fallback should remain in effect.
    if let Some(loc_map) = inputs.localization_map.clone() {
        if !loc_map.is_empty() {
            defaults.set_localization(loc_map);
        }
    }

    // ── 6. Rasterise ───────────────────────────────────────────────────────
    //
    // Prefer render_bb_scene (A3 compositor) when BbScene resolution succeeded.
    // Fall back to render_canvas (magenta placeholder) when it failed.
    // apply_postprocess is kept false until Phase A5.

    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
    let target = ComposeTarget { width: inputs.target_size.0, height: inputs.target_size.1 };

    let mut img = if let Some(ref scene) = bb_scene_opt {
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

    // ── 7. Flash SWF visual-exports overlay ────────────────────────────────────
    //
    // For Flash-backed canvases the BB nodes of type `WidgetCustomShape` carry
    // no static geometry — shapes are driven by ActionScript at runtime.  Their
    // DataCore names (e.g. `shape_Chevron`) do not match the SWF's
    // `ExportAssets` linkage names (e.g. `TargetSelection_Borders`), so the
    // per-node `draw_swf_symbol` path in `compose.rs` silently draws nothing.
    //
    // Instead, after the BB scene is composed, overlay every non-ActionScript
    // visual export from the Flash SWF scaled to the full canvas rect.  The
    // exported sprites carry internal first-frame `PlaceObject` lists that
    // position their shapes in stage coordinate space (origin = canvas top-left),
    // so the scale transform stage→canvas puts each shape in the correct place.
    //
    // Guard: only apply the overlay when the resolved BB scene actually contains
    // `WidgetCustomShape` nodes.  Brand-specific canvas variants (e.g. RSI's
    // `rsi_mc_s_target.json`) may replace all Flash shapes with WidgetCard /
    // WidgetTextField nodes; in that case the SWF overlay would paint over
    // already-correct BB content.

    let has_flash_shapes = bb_scene_opt.as_ref().map_or(false, |scene| {
        scene
            .nodes
            .values()
            .any(|n| n.ty == crate::bb_scene::BbNodeType::WidgetCustomShape)
    });

    if has_flash_shapes {
        if let Some(ref flash_swf) = flash_swf_opt {
            let pt = &style.primary_tint;
            let tint = tiny_skia::Color::from_rgba8(pt.r, pt.g, pt.b, pt.a);
            let drew =
                crate::swf_render::draw_swf_visual_exports_rgba(&mut img, flash_swf, tint, 1.0);
            log::debug!(
                "flash_overlay[{}]: drew={drew}",
                b.helper_name.unwrap_or("?"),
            );
        }
    }

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

    // Brand code: first ≤3 chars of manufacturer_id, upper-cased.
    let brand: String = manufacturer_id
        .chars()
        .take(3)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if brand.is_empty() {
        return vec![];
    }

    let base = format!(
        r"Data\UI\ShipInterface\assets\SWF\{brand}\SupportScreen16-9\"
    );

    let mut candidates = vec![
        format!("{base}{stem}Status.swf"),
        format!("{base}{stem}.swf"),
    ];

    // For generic MC_S_* canvases used by non-RSI ships, append the RSI
    // SupportScreen16-9 canonical paths as structural fallbacks.  RSI is the
    // primary host for these shared canvas SWFs in the P4K layout.
    if is_generic_mc && brand != "RSI" {
        let rsi_base = r"Data\UI\ShipInterface\assets\SWF\RSI\SupportScreen16-9\";
        candidates.push(format!("{rsi_base}{stem}Status.swf"));
        candidates.push(format!("{rsi_base}{stem}.swf"));
    }

    candidates
}
