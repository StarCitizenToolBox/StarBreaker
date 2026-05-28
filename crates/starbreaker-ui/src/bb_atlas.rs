//! Bitmap / SVG atlas library for BuildingBlocks canvas rendering.
//!
//! Resolves image references that appear in a [`BbNode`] — SVG icon paths, SVG
//! fill paths, and DDS / PNG / JPG bitmap paths — to an in-memory
//! [`image::RgbaImage`] at a caller-supplied target size.
//!
//! # Key types
//! - [`AssetFetcher`] — thin trait for reading raw bytes from the P4K archive
//!   (or a test stub).
//! - [`AtlasLibrary`] — caches decoded images keyed on
//!   `(canonicalised_path, target_w, target_h)`.

use std::{collections::HashMap, sync::RwLock};

use image::{GenericImageView, RgbaImage, imageops};
use log::{debug, warn};
use tiny_skia_011 as tiny_skia;

use crate::bb_scene::BbNode;

/// Read raw asset bytes from a P4K archive (or test fixture).
///
/// Path argument uses the archive's native convention. Implementations are
/// expected to perform case-insensitive matching where the source supports it.
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
///
/// Constructed with a reference to an [`AssetFetcher`] and an optional
/// manufacturer id (for example, `"drak"`) used for MFD manufacturer-template
/// fallback resolution.
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
    ///
    /// Returns `None` if the path is empty, the asset is not found in the
    /// archive, or decoding fails. Decode errors are logged as warnings.
    /// Results are cached on `(canonicalised_path, target_w, target_h)`.
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
    ///
    /// Checks, in order: `icon.image_record` (custom SVG icon path), then
    /// `background.svg_fill_path` (SVG fill path). Returns `None` if the node
    /// has no resolvable primary image reference.
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
    ///
    /// The path is canonicalised and manufacturer MFD fallback is applied identically
    /// to [`Self::resolve`].  Returns `None` when the asset is not found.
    ///
    /// Use this when the caller needs to post-process the raw bytes (e.g. SVG
    /// rasterisation with a fill-colour override) rather than receiving a decoded image.
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

fn decode_bytes(bytes: &[u8], ext: &str, target_w: u32, target_h: u32) -> Option<RgbaImage> {
    let img = match ext {
        "svg" => decode_svg(bytes, target_w, target_h)?,
        "dds" => decode_dds(bytes)?,
        _ => {
            let dyn_img = image::load_from_memory(bytes)
                .map_err(|e| {
                    warn!("atlas: image decode failed: {}", e);
                    e
                })
                .ok()?;
            dyn_img.to_rgba8()
        }
    };

    Some(resize_to(img, target_w, target_h))
}

fn decode_svg(bytes: &[u8], target_w: u32, target_h: u32) -> Option<RgbaImage> {
    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opts)
        .map_err(|e| {
            warn!("atlas: SVG parse failed: {}", e);
            e
        })
        .ok()?;

    let source_w = tree.size().width();
    let source_h = tree.size().height();
    if source_w <= 0.0 || source_h <= 0.0 {
        warn!("atlas: SVG has invalid size {}x{}", source_w, source_h);
        return None;
    }

    let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)?;
    let transform = tiny_skia::Transform::from_scale(
        target_w as f32 / source_w,
        target_h as f32 / source_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    RgbaImage::from_raw(target_w, target_h, pixmap.take())
}

fn decode_dds(bytes: &[u8]) -> Option<RgbaImage> {
    let dds = starbreaker_dds::DdsFile::from_bytes(bytes)
        .map_err(|e| {
            warn!("atlas: DDS decode failed: {}", e);
            e
        })
        .ok()?;
    let (w, h) = dds.dimensions(0);
    let rgba = dds
        .decode_rgba(0)
        .map_err(|e| {
            warn!("atlas: DDS RGBA extract failed: {}", e);
            e
        })
        .ok()?;
    RgbaImage::from_raw(w, h, rgba)
}

fn source_dimensions(bytes: &[u8], ext: &str) -> Option<(u32, u32)> {
    match ext {
        "dds" => {
            let dds = starbreaker_dds::DdsFile::from_bytes(bytes)
                .map_err(|e| {
                    warn!("atlas: DDS dimension read failed: {}", e);
                    e
                })
                .ok()?;
            Some(dds.dimensions(0))
        }
        "svg" => {
            let opts = usvg::Options::default();
            let tree = usvg::Tree::from_data(bytes, &opts)
                .map_err(|e| {
                    warn!("atlas: SVG parse failed: {}", e);
                    e
                })
                .ok()?;
            let size = tree.size();
            let w = size.width().round().max(1.0) as u32;
            let h = size.height().round().max(1.0) as u32;
            Some((w, h))
        }
        _ => {
            let dyn_img = image::load_from_memory(bytes)
                .map_err(|e| {
                    warn!("atlas: image dimension read failed: {}", e);
                    e
                })
                .ok()?;
            Some(dyn_img.dimensions())
        }
    }
}

fn resize_to(img: RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.width() == w && img.height() == h {
        return img;
    }
    imageops::resize(&img, w, h, imageops::FilterType::Lanczos3)
}

/// Canonicalise a BuildingBlocks asset path.
///
/// Lowercases, converts backslashes to forward slashes, strips leading `./`,
/// and collapses a doubled `data/data/` prefix.
pub fn canonicalise_path(raw: &str) -> String {
    let s = raw.trim().replace('\\', "/").to_lowercase();
    let s = s.strip_prefix("./").unwrap_or(&s);
    let s = if s.starts_with("data/data/") {
        s.strip_prefix("data/").unwrap_or(s)
    } else {
        s
    };
    s.to_string()
}

fn extension_of(path: &str) -> &str {
    path.rfind('.').map(|i| &path[i + 1..]).unwrap_or("")
}

fn replace_mfd_segment(canonical: &str, replacement: &str) -> String {
    let marker = "/mfd/";
    let Some(mfd_pos) = canonical.find(marker) else {
        return canonical.to_string();
    };
    let after_mfd = &canonical[mfd_pos + marker.len()..];
    if after_mfd.is_empty() {
        return canonical.to_string();
    }
    let next_slash = after_mfd.find('/').unwrap_or(after_mfd.len());
    let rest = &after_mfd[next_slash..];
    format!(
        "{}{}{}{}",
        &canonical[..mfd_pos],
        marker,
        replacement.to_uppercase(),
        rest
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use image::{ImageFormat, Rgba};

    use super::*;

    struct StubFetcher {
        files: HashMap<String, Vec<u8>>,
        fetches: AtomicUsize,
    }

    impl StubFetcher {
        fn new(files: HashMap<String, Vec<u8>>) -> Self {
            Self {
                files,
                fetches: AtomicUsize::new(0),
            }
        }
    }

    impl AssetFetcher for StubFetcher {
        fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            self.files
                .iter()
                .find(|(path, _)| path.eq_ignore_ascii_case(p4k_path))
                .map(|(_, bytes)| bytes.clone())
        }
    }

    fn red_svg() -> Vec<u8> {
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>"#.to_vec()
    }

    fn red_png() -> Vec<u8> {
        let img = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]));
        let mut cursor = std::io::Cursor::new(Vec::new());
        img.write_to(&mut cursor, ImageFormat::Png)
            .expect("PNG encode should succeed");
        cursor.into_inner()
    }

    #[test]
    fn resolves_svg_to_target_size() {
        let fetcher = StubFetcher::new(HashMap::from([("test/red.svg".to_string(), red_svg())]));
        let atlas = AtlasLibrary::new(&fetcher, None);

        let img = atlas
            .resolve("test/red.svg", 32, 32)
            .expect("SVG should resolve");

        assert_eq!((img.width(), img.height()), (32, 32));
        let px = img.get_pixel(16, 16).0;
        assert!(
            px[0] > 200 && px[1] < 50 && px[2] < 50 && px[3] > 200,
            "centre pixel was {px:?}"
        );
    }

    #[test]
    fn resolves_png_and_scales_to_target_size() {
        let fetcher = StubFetcher::new(HashMap::from([("test/red.png".to_string(), red_png())]));
        let atlas = AtlasLibrary::new(&fetcher, None);

        let img = atlas
            .resolve("test/red.png", 8, 8)
            .expect("PNG should resolve");

        assert_eq!((img.width(), img.height()), (8, 8));
    }

    #[test]
    fn applies_mfd_manufacturer_and_gen_fallbacks() {
        let fetcher = StubFetcher::new(HashMap::from([
            ("data/ui/textures/mfd/drak/foo.svg".to_string(), red_svg()),
            ("data/ui/textures/mfd/gen/foo.svg".to_string(), red_svg()),
        ]));

        let drak_atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let drak_img = drak_atlas
            .resolve("data/ui/textures/mfd/whatever/foo.svg", 4, 4)
            .expect("manufacturer fallback should resolve");
        assert_eq!((drak_img.width(), drak_img.height()), (4, 4));

        let anvl_atlas = AtlasLibrary::new(&fetcher, Some("anvl"));
        let gen_img = anvl_atlas
            .resolve("data/ui/textures/mfd/whatever/foo.svg", 4, 4)
            .expect("GEN fallback should resolve");
        assert_eq!((gen_img.width(), gen_img.height()), (4, 4));
    }

    #[test]
    fn caches_successful_resolves() {
        let fetcher = StubFetcher::new(HashMap::from([("test/red.svg".to_string(), red_svg())]));
        let atlas = AtlasLibrary::new(&fetcher, None);

        assert!(atlas.resolve("test/red.svg", 16, 16).is_some());
        assert!(atlas.resolve("test/red.svg", 16, 16).is_some());

        assert_eq!(fetcher.fetches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn canonicalises_common_path_variants() {
        assert_eq!(
            canonicalise_path(r#".\Data\Data\UI\foo.SVG"#),
            "data/ui/foo.svg"
        );
    }
}
