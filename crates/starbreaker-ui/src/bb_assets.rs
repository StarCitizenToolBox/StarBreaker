//! Asset path normalisation and fetching helpers for BuildingBlocks UI rendering.
//!
//! Provides [`UiAssetResolver`] which wraps an [`AssetFetcher`] and adds DataCore→P4K
//! path normalisation ([`UiAssetResolver::normalise_path`]):
//! - Backslash conversion (forward slashes → `\`).
//! - `.tif` → `.dds` extension substitution.
//! - `Data\` prefix addition when absent.
//!
//! Also provides reference-overlay filtering ([`UiAssetResolver::is_reference_overlay`])
//! to exclude designer-time reference overlays from rendered output.

use image::RgbaImage;
use log::debug;

use crate::bb_atlas::{AssetFetcher, AtlasLibrary, canonicalise_path};

/// Asset path resolver for BuildingBlocks UI widgets.
///
/// Wraps an [`AssetFetcher`] and provides DataCore→P4K path normalisation,
/// reference-overlay filtering, and convenience fetch methods for SVG and DDS assets.
pub struct UiAssetResolver<'a> {
    fetcher: &'a dyn AssetFetcher,
}

impl<'a> UiAssetResolver<'a> {
    /// Create a new resolver backed by `fetcher`.
    pub fn new(fetcher: &'a dyn AssetFetcher) -> Self {
        Self { fetcher }
    }

    /// Normalise a DataCore asset path to P4K canonical form.
    ///
    /// Conversions applied, in order:
    /// 1. Forward slashes replaced with backslashes.
    /// 2. `.tif` suffix replaced with `.dds` (case-insensitive).
    /// 3. `Data\` prefix prepended when absent.
    ///
    /// # Examples
    /// ```text
    /// "UI/Textures/foo.tif"     → "Data\UI\Textures\foo.dds"
    /// "Data/UI/Textures/foo.dds"→ "Data\UI\Textures\foo.dds"
    /// "Data\UI\icons\bar.svg"   → "Data\UI\icons\bar.svg"
    /// ```
    pub fn normalise_path(raw: &str) -> String {
        let s = raw.trim().replace('/', "\\");

        // Replace .tif extension with .dds (case-insensitive).
        let s = if s.to_lowercase().ends_with(".tif") {
            format!("{}.dds", &s[..s.len() - 4])
        } else {
            s
        };

        // Prepend Data\ if not already present (case-insensitive check).
        if s.to_lowercase().starts_with("data\\") {
            s
        } else {
            format!("Data\\{}", s)
        }
    }

    /// Return `true` when `raw` refers to a designer-time reference overlay.
    ///
    /// Two structural signals are recognised, both derived from the **asset
    /// path itself** (never from widget names):
    ///
    /// * Paths containing the `_references` directory — the engine's
    ///   convention for designer-time overlay sources.
    /// * UI texture paths whose filename declares itself a `mockup` (e.g.
    ///   `Data/UI/Textures/I_InteractiveScreens/Med/i_med_bioc_mockupImage.tif`).
    ///   These are semi-transparent layout-example overlays placed on parent
    ///   canvases by designers; they are not part of the production composed
    ///   output. The check is scoped to `\UI\` paths so legitimate
    ///   `mockup_*` building-set geometry (`Data\Objects\buildingsets\…`) is
    ///   unaffected.
    ///
    /// The check is path-prefix agnostic and case-insensitive.
    pub fn is_reference_overlay(raw: &str) -> bool {
        let s = raw.to_lowercase().replace('/', "\\");
        if s.contains("\\_references\\") {
            return true;
        }
        if (s.contains("\\ui\\") || s.starts_with("ui\\")) && s.contains("mockup") {
            return true;
        }
        false
    }

    /// Fetch SVG bytes for `raw_path` from the P4K archive.
    ///
    /// `raw_path` is canonicalised (lower-case, forward slashes) before the fetch.
    /// Returns `None` when the asset is not found or the path is empty.
    pub fn fetch_svg(&self, raw_path: &str) -> Option<Vec<u8>> {
        let canonical = canonicalise_path(raw_path);
        if canonical.is_empty() {
            return None;
        }
        debug!("bb_assets: fetching SVG '{}'", canonical);
        self.fetcher.fetch_svg_bytes(&canonical)
    }

    /// Fetch and decode a DDS (or other raster) asset as an RGBA image.
    ///
    /// Delegates to [`AtlasLibrary::resolve`] which handles DDS decoding, SVG
    /// rasterisation, and Lanczos3 rescaling to `target_w × target_h`.
    pub fn fetch_dds_as_rgba(
        &self,
        raw_path: &str,
        target_w: u32,
        target_h: u32,
    ) -> Option<RgbaImage> {
        let atlas = AtlasLibrary::new(self.fetcher, None);
        atlas.resolve(raw_path, target_w, target_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalise_path ────────────────────────────────────────────────────────

    #[test]
    fn normalise_tif_to_dds() {
        let result = UiAssetResolver::normalise_path("UI/Textures/foo.tif");
        assert!(
            result.to_lowercase().ends_with(".dds"),
            "expected .dds extension, got '{result}'"
        );
    }

    #[test]
    fn normalise_forward_to_back_slash() {
        let result = UiAssetResolver::normalise_path("UI/Textures/bar.svg");
        assert!(!result.contains('/'), "expected no forward slashes, got '{result}'");
        assert!(result.contains('\\'), "expected backslashes, got '{result}'");
    }

    #[test]
    fn normalise_adds_data_prefix() {
        let result = UiAssetResolver::normalise_path("UI/Textures/bar.svg");
        assert!(
            result.starts_with("Data\\"),
            "expected Data\\ prefix, got '{result}'"
        );
    }

    #[test]
    fn normalise_does_not_double_data_prefix() {
        let result = UiAssetResolver::normalise_path("Data/UI/Textures/bar.dds");
        let lower = result.to_lowercase();
        let count = lower.matches("data\\").count();
        assert_eq!(count, 1, "Data\\ prefix should appear exactly once, got '{result}'");
    }

    #[test]
    fn normalise_dds_unchanged_extension() {
        let result = UiAssetResolver::normalise_path("UI/Textures/foo.dds");
        assert!(
            result.to_lowercase().ends_with(".dds"),
            "expected .dds extension, got '{result}'"
        );
    }

    // ── is_reference_overlay ─────────────────────────────────────────────────

    #[test]
    fn reference_overlay_true_for_references_path() {
        assert!(UiAssetResolver::is_reference_overlay(
            r"Data\UI\_references\foo.svg"
        ));
    }

    #[test]
    fn reference_overlay_false_for_textures_path() {
        assert!(!UiAssetResolver::is_reference_overlay(
            r"Data\UI\Textures\foo.dds"
        ));
    }

    #[test]
    fn reference_overlay_case_insensitive() {
        assert!(UiAssetResolver::is_reference_overlay(
            "data/ui/_references/test.svg"
        ));
    }

    #[test]
    fn reference_overlay_false_for_empty_string() {
        assert!(!UiAssetResolver::is_reference_overlay(""));
    }

    #[test]
    fn reference_overlay_true_for_ui_mockup_path() {
        assert!(UiAssetResolver::is_reference_overlay(
            r"Data\UI\Textures\I_InteractiveScreens\Med\i_med_bioc_mockupImage.dds"
        ));
        assert!(UiAssetResolver::is_reference_overlay(
            "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_mockupimage.tif"
        ));
    }

    #[test]
    fn reference_overlay_false_for_non_ui_mockup_path() {
        // Building-set mockup geometry must not be filtered.
        assert!(!UiAssetResolver::is_reference_overlay(
            r"Data\Objects\buildingsets\human\lowtech\alpha\ext\floor\mockup_fake_under_01.cgfm"
        ));
    }
}
