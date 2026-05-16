//! Asset resolution helpers for GFx/SWF imports, textures, BuildingBlocks JSON,
//! and related UI source files.
//!
//! The resolver keeps Phase 4 deterministic and testable by working over caller-
//! supplied bytes rather than a live archive. It normalizes DataCore/UI paths,
//! resolves `.tif` requests to `.dds` siblings when needed, decodes TIFF/DDS
//! textures to PNG payloads for downstream renderers, and returns explicit
//! missing-resource errors instead of silent fallbacks.

use std::collections::BTreeMap;
use std::io::Cursor;

use image::ImageEncoder;
use starbreaker_dds::DdsFile;

use crate::error::{GfxError, GfxResult};
use crate::types::{ImportedResource, ImportedResourceKind};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredAsset {
    resolved_path: String,
    bytes: Vec<u8>,
}

/// Source-resolved asset payload returned by the Phase 4 resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    /// Caller-supplied path before normalization.
    pub requested_path: String,
    /// Canonical normalized path that matched the resolver registry.
    pub resolved_path: String,
    /// Asset class after resolution/decoding.
    pub kind: ResolvedAssetKind,
    /// Resolved bytes. Textures are returned as PNG bytes.
    pub bytes: Vec<u8>,
}

/// Asset classes materialized by the Phase 4 resolver.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ResolvedAssetKind {
    /// Nested GFx/SWF movie payload.
    Movie,
    /// BuildingBlocks JSON or related UI metadata.
    Json,
    /// Texture payload normalized to PNG bytes.
    TexturePng,
    /// Font bytes for later text rendering.
    Font,
    /// Other source payloads resolved by normalized path.
    Binary,
}

/// In-memory dependency resolver for UI assets.
#[derive(Debug, Default, Clone)]
pub struct AssetResolver {
    assets: BTreeMap<String, RegisteredAsset>,
}

impl AssetResolver {
    /// Register source bytes at a DataCore/UI-relative path.
    pub fn register_bytes(&mut self, path: impl AsRef<str>, bytes: impl Into<Vec<u8>>) {
        let resolved_path = normalize_requested_source_path(path.as_ref());
        self.assets.insert(
            resolved_path.to_ascii_lowercase(),
            RegisteredAsset {
                resolved_path,
                bytes: bytes.into(),
            },
        );
    }

    /// Resolve an imported GFx resource by its parsed kind and path.
    pub fn resolve_import(&self, import: &ImportedResource) -> GfxResult<ResolvedAsset> {
        match import.kind {
            ImportedResourceKind::Movie => self.resolve_movie_path(&import.source),
            ImportedResourceKind::Texture => self.resolve_texture_reference(&import.source),
            ImportedResourceKind::Font => self.resolve_font_reference(&import.source),
            ImportedResourceKind::Unknown => self.resolve_source_path(&import.source),
        }
    }

    /// Resolve a nested `.gfx` / `.swf` import.
    pub fn resolve_movie_path(&self, path: &str) -> GfxResult<ResolvedAsset> {
        self.resolve_bytes(
            path,
            &[normalize_requested_source_path(path)],
            ResolvedAssetKind::Movie,
            GfxError::MissingImportedAsset {
                path: normalize_requested_source_path(path),
            },
        )
    }

    /// Resolve a texture reference from a `.tif`, `.dds`, or already-rasterized source.
    pub fn resolve_texture_reference(&self, path: &str) -> GfxResult<ResolvedAsset> {
        let requested_path = normalize_requested_source_path(path);
        let resolved = self.lookup(
            texture_candidate_paths(path),
            GfxError::MissingReferencedTexture {
                path: requested_path.clone(),
            },
        )?;
        let png = transcode_texture_to_png(&resolved.resolved_path, &resolved.bytes)?;
        Ok(ResolvedAsset {
            requested_path,
            resolved_path: resolved.resolved_path.clone(),
            kind: ResolvedAssetKind::TexturePng,
            bytes: png,
        })
    }

    /// Resolve BuildingBlocks JSON or other structured UI metadata.
    pub fn resolve_buildingblocks_json(&self, path: &str) -> GfxResult<ResolvedAsset> {
        self.resolve_bytes(
            path,
            &[normalize_requested_source_path(path)],
            ResolvedAssetKind::Json,
            GfxError::MissingImportedAsset {
                path: normalize_requested_source_path(path),
            },
        )
    }

    /// Resolve a font payload for later text rendering.
    pub fn resolve_font_reference(&self, path: &str) -> GfxResult<ResolvedAsset> {
        self.resolve_bytes(
            path,
            &[normalize_requested_source_path(path)],
            ResolvedAssetKind::Font,
            GfxError::MissingImportedAsset {
                path: normalize_requested_source_path(path),
            },
        )
    }

    /// Resolve an arbitrary source material/UI path using the shared normalization rules.
    pub fn resolve_source_path(&self, path: &str) -> GfxResult<ResolvedAsset> {
        self.resolve_bytes(
            path,
            &[normalize_requested_source_path(path)],
            ResolvedAssetKind::Binary,
            GfxError::MissingImportedAsset {
                path: normalize_requested_source_path(path),
            },
        )
    }

    fn resolve_bytes(
        &self,
        path: &str,
        candidates: &[String],
        kind: ResolvedAssetKind,
        missing_error: GfxError,
    ) -> GfxResult<ResolvedAsset> {
        let requested_path = normalize_requested_source_path(path);
        let resolved = self.lookup(candidates.iter().cloned(), missing_error)?;
        Ok(ResolvedAsset {
            requested_path,
            resolved_path: resolved.resolved_path.clone(),
            kind,
            bytes: resolved.bytes.clone(),
        })
    }

    fn lookup(
        &self,
        candidates: impl IntoIterator<Item = String>,
        missing_error: GfxError,
    ) -> GfxResult<&RegisteredAsset> {
        for candidate in candidates {
            if let Some(asset) = self.assets.get(&candidate.to_ascii_lowercase()) {
                return Ok(asset);
            }
        }
        Err(missing_error)
    }
}

fn texture_candidate_paths(path: &str) -> Vec<String> {
    let normalized = normalize_requested_source_path(path);
    let lower = normalized.to_ascii_lowercase();
    let mut candidates = vec![normalized.clone()];
    if lower.ends_with(".tif") {
        candidates.push(format!("{}.dds", normalized.trim_end_matches(".tif")));
    } else if lower.ends_with(".tiff") {
        candidates.push(format!("{}.dds", normalized.trim_end_matches(".tiff")));
    }
    candidates
}

fn normalize_requested_source_path(path: &str) -> String {
    let trimmed = path.trim().replace('\\', "/");
    let trimmed = trimmed.trim_start_matches('/');
    if trimmed.is_empty() {
        return "Data/".to_string();
    }
    if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("data/") {
        return format!("Data/{}", &trimmed[5..]);
    }
    format!("Data/{trimmed}")
}

fn transcode_texture_to_png(path: &str, bytes: &[u8]) -> GfxResult<Vec<u8>> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".dds") {
        let dds = DdsFile::from_bytes(bytes).map_err(|err| GfxError::MalformedFile {
            reason: format!("failed to decode DDS texture {path}: {err}"),
        })?;
        let rgba = dds.decode_rgba(0).map_err(|err| GfxError::MalformedFile {
            reason: format!("failed to decode DDS mip for {path}: {err}"),
        })?;
        let (width, height) = dds.dimensions(0);
        return encode_png_rgba(width, height, rgba);
    }

    let format = if lower.ends_with(".tif") || lower.ends_with(".tiff") {
        image::ImageFormat::Tiff
    } else if lower.ends_with(".png") {
        image::ImageFormat::Png
    } else {
        image::guess_format(bytes).map_err(|err| GfxError::MalformedFile {
            reason: format!("failed to infer image format for {path}: {err}"),
        })?
    };
    let image = image::load_from_memory_with_format(bytes, format).map_err(|err| GfxError::MalformedFile {
        reason: format!("failed to decode texture {path}: {err}"),
    })?;
    encode_png_rgba(image.width(), image.height(), image.to_rgba8().into_raw())
}

fn encode_png_rgba(width: u32, height: u32, rgba: Vec<u8>) -> GfxResult<Vec<u8>> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(Cursor::new(&mut bytes))
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|err| GfxError::ImageEncode {
            reason: err.to_string(),
        })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbaImage};

    #[test]
    fn resolves_tiff_texture_to_png_with_normalized_path() {
        let mut resolver = AssetResolver::default();
        let tiff_bytes = encode_tiff([17, 34, 51, 255]);
        resolver.register_bytes("UI\\Textures\\example_screen.tif", tiff_bytes);

        let asset = resolver
            .resolve_texture_reference("UI/Textures/example_screen.tif")
            .expect("resolve texture");

        assert_eq!(asset.resolved_path, "Data/UI/Textures/example_screen.tif");
        assert_eq!(asset.kind, ResolvedAssetKind::TexturePng);
        assert!(asset.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn resolves_dds_sibling_for_tif_requests() {
        let mut resolver = AssetResolver::default();
        resolver.register_bytes("UI/Textures/example_screen.dds", minimal_rgba_dds([0, 128, 255, 255]));

        let asset = resolver
            .resolve_texture_reference("UI/Textures/example_screen.tif")
            .expect("resolve texture through dds sibling");

        assert_eq!(asset.resolved_path, "Data/UI/Textures/example_screen.dds");
        assert!(asset.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn resolves_buildingblocks_json_fonts_and_generic_source_paths() {
        let mut resolver = AssetResolver::default();
        resolver.register_bytes("UI/BuildingBlocks/example.json", br#"{"screen":"default"}"#.to_vec());
        resolver.register_bytes("Fonts/example.ttf", vec![0, 1, 2, 3]);
        resolver.register_bytes("Materials/example.gfx", b"GFXDATA".to_vec());

        let json = resolver
            .resolve_buildingblocks_json("UI/BuildingBlocks/example.json")
            .expect("resolve json");
        let font = resolver
            .resolve_font_reference("Fonts/example.ttf")
            .expect("resolve font");
        let generic = resolver
            .resolve_source_path("Materials/example.gfx")
            .expect("resolve generic source");

        assert_eq!(json.kind, ResolvedAssetKind::Json);
        assert_eq!(font.kind, ResolvedAssetKind::Font);
        assert_eq!(generic.kind, ResolvedAssetKind::Binary);
        assert_eq!(font.bytes, vec![0, 1, 2, 3]);
        assert_eq!(generic.resolved_path, "Data/Materials/example.gfx");
    }

    #[test]
    fn resolves_imports_by_parsed_kind() {
        let mut resolver = AssetResolver::default();
        resolver.register_bytes("UI/BuildingBlocks/example.gfx", b"movie".to_vec());
        resolver.register_bytes("Fonts/example.ttf", vec![1, 2, 3]);

        let movie = resolver
            .resolve_import(&ImportedResource {
                source: "UI/BuildingBlocks/example.gfx".to_string(),
                kind: ImportedResourceKind::Movie,
            })
            .expect("resolve movie import");
        let font = resolver
            .resolve_import(&ImportedResource {
                source: "Fonts/example.ttf".to_string(),
                kind: ImportedResourceKind::Font,
            })
            .expect("resolve font import");

        assert_eq!(movie.kind, ResolvedAssetKind::Movie);
        assert_eq!(font.kind, ResolvedAssetKind::Font);
    }

    #[test]
    fn returns_explicit_missing_resource_errors() {
        let resolver = AssetResolver::default();

        let texture_err = resolver
            .resolve_texture_reference("UI/Textures/missing_screen.tif")
            .expect_err("missing texture should fail");
        let font_err = resolver
            .resolve_font_reference("Fonts/missing.ttf")
            .expect_err("missing font should fail");

        assert_eq!(
            texture_err,
            GfxError::MissingReferencedTexture {
                path: "Data/UI/Textures/missing_screen.tif".to_string(),
            }
        );
        assert_eq!(
            font_err,
            GfxError::MissingImportedAsset {
                path: "Data/Fonts/missing.ttf".to_string(),
            }
        );
    }

    fn encode_tiff(pixel: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, image::Rgba(pixel)));
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Tiff)
            .expect("encode tiff");
        bytes
    }

    fn minimal_rgba_dds(pixel: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DDS ");
        bytes.extend_from_slice(&124u32.to_le_bytes());
        bytes.extend_from_slice(&0x0002_100Fu32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 44]);
        bytes.extend_from_slice(&32u32.to_le_bytes());
        bytes.extend_from_slice(&0x41u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&32u32.to_le_bytes());
        bytes.extend_from_slice(&0x00ff_0000u32.to_le_bytes());
        bytes.extend_from_slice(&0x0000_ff00u32.to_le_bytes());
        bytes.extend_from_slice(&0x0000_00ffu32.to_le_bytes());
        bytes.extend_from_slice(&0xff00_0000u32.to_le_bytes());
        bytes.extend_from_slice(&0x1000u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        bytes
    }
}
