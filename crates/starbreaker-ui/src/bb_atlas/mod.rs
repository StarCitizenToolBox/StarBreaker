//! Bitmap and SVG atlas library for BuildingBlocks canvas rendering.

use std::{collections::HashMap, sync::RwLock};

use image::RgbaImage;
use log::debug;

use crate::bb_scene::BbNode;

mod decode;
mod path;
#[cfg(test)]
mod tests;

pub use path::canonicalise_path;

use decode::{decode_bytes, source_dimensions};
use path::{extension_of, replace_mfd_segment};

/// Read raw asset bytes from a P4K archive (or test fixture).
pub trait AssetFetcher: Send + Sync {
    /// Fetch DDS / PNG / JPG bytes by archive path.
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>>;

    /// Fetch SVG bytes by archive path. Defaults to [`Self::fetch_image_bytes`].
    fn fetch_svg_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        self.fetch_image_bytes(p4k_path)
    }
}

type CacheKey = (String, u32, u32);

/// Cached atlas of decoded images.
pub struct AtlasLibrary<'a> {
    fetcher: &'a dyn AssetFetcher,
    manufacturer_id: Option<&'a str>,
    cache: RwLock<HashMap<CacheKey, RgbaImage>>,
}

impl<'a> AtlasLibrary<'a> {
    /// Create a new library.
    pub fn new(fetcher: &'a dyn AssetFetcher, manufacturer_id: Option<&'a str>) -> Self {
        Self {
            fetcher,
            manufacturer_id,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Resolve `raw_path` to a decoded RGBA image at `(target_w, target_h)`.
    pub fn resolve(&self, raw_path: &str, target_w: u32, target_h: u32) -> Option<RgbaImage> {
        if target_w == 0 || target_h == 0 {
            return None;
        }

        let canonical = canonicalise_path(raw_path);
        if canonical.is_empty() {
            return None;
        }

        {
            let guard = self.cache.read().ok()?;
            if let Some(img) = guard.get(&(canonical.clone(), target_w, target_h)) {
                return Some(img.clone());
            }
        }

        let img = self.fetch_and_decode(&canonical, target_w, target_h)?;

        if let Ok(mut guard) = self.cache.write() {
            guard.insert((canonical, target_w, target_h), img.clone());
        }
        Some(img)
    }

    /// Resolve any primary image reference from `node`.
    pub fn resolve_for_node(
        &self,
        node: &BbNode,
        target_w: u32,
        target_h: u32,
    ) -> Option<RgbaImage> {
        if let Some(icon) = &node.icon
            && let Some(path) = &icon.image_record
            && !path.is_empty()
            && let Some(img) = self.resolve(path, target_w, target_h)
        {
            return Some(img);
        }

        if let Some(bg) = &node.background
            && let Some(path) = &bg.svg_fill_path
            && !path.is_empty()
            && let Some(img) = self.resolve(path, target_w, target_h)
        {
            return Some(img);
        }

        None
    }

    /// Fetch raw bytes for `raw_path` from the archive.
    pub fn fetch_raw(&self, raw_path: &str) -> Option<Vec<u8>> {
        let canonical = canonicalise_path(raw_path);
        if canonical.is_empty() {
            return None;
        }
        self.fetch_with_mfd_fallback(&canonical)
    }

    /// Return the intrinsic pixel dimensions for `raw_path` without resizing.
    pub fn source_dimensions(&self, raw_path: &str) -> Option<(u32, u32)> {
        let canonical = canonicalise_path(raw_path);
        if canonical.is_empty() {
            return None;
        }

        let ext = extension_of(&canonical);
        let bytes = self.fetch_with_mfd_fallback(&canonical)?;
        source_dimensions(&bytes, ext)
    }

    fn fetch_and_decode(&self, canonical: &str, target_w: u32, target_h: u32) -> Option<RgbaImage> {
        let ext = extension_of(canonical);
        let bytes = self.fetch_with_mfd_fallback(canonical)?;
        decode_bytes(&bytes, ext, target_w, target_h)
    }

    fn fetch_with_mfd_fallback(&self, canonical: &str) -> Option<Vec<u8>> {
        let ext = extension_of(canonical);
        let fetch = |path: &str| -> Option<Vec<u8>> {
            if ext == "svg" {
                self.fetcher.fetch_svg_bytes(path)
            } else {
                self.fetcher.fetch_image_bytes(path)
            }
        };

        if let Some(b) = fetch(canonical) {
            return Some(b);
        }

        if !canonical.contains("/mfd/") {
            debug!("atlas: '{}' not found", canonical);
            return None;
        }

        for mfr in [self.manufacturer_id, Some("GEN")].into_iter().flatten() {
            let alt = replace_mfd_segment(canonical, mfr);
            if alt == canonical {
                continue;
            }
            debug!(
                "atlas: '{}' not found; trying MFD fallback '{}'",
                canonical, alt
            );
            if let Some(b) = fetch(&alt) {
                return Some(b);
            }
        }

        debug!("atlas: '{}' not found (all fallbacks exhausted)", canonical);
        None
    }
}
