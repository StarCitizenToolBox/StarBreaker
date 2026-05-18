//! SWF static-atom extractor — bitmaps, shapes, fonts, sprite first-frames.
//!
//! Parses SWF tag streams using the `swf` crate as a read-only parser and collects
//! the visual atoms needed by the canvas compositor. All extracted data is owned and
//! keyed by SWF character id.
//!
//! # AVM1/AVM2 policy
//! **No action tags are decoded or executed.** `DoAction`, `DoInitAction`, `DoAbc`,
//! and `DoAbc2` tags are silently skipped — their presence is counted in debug logs but
//! no bytecode is interpreted. This module is purely structural; it reads the tag
//! stream for asset definitions only.
//!
//! # Entry point
//! [`SwfAssetLibrary`] is the primary public type: construct it from raw SWF bytes,
//! then use the `get_*` / `lookup_export` accessors to retrieve individual atoms.
//! The library is content-addressed via [`SwfAssetLibrary::content_hash`].

use std::collections::HashMap;
use std::io::Read as _;

use flate2::read::ZlibDecoder;
use image::{ImageFormat, RgbaImage};
use sha2::{Digest, Sha256};
use swf::{CharacterId, ColorTransform, Depth, Matrix, Tag};

use crate::error::UiError;

// ──────────────────────────────────────────────────────────────────────────────
// Public data types
// ──────────────────────────────────────────────────────────────────────────────

/// An extracted SWF shape character, preserving its fill/line styles and edge records.
///
/// The inner fields reuse the owned `swf` crate types directly, since `swf::Shape`,
/// `swf::ShapeRecord`, `swf::FillStyle`, and `swf::LineStyle` carry no lifetime
/// parameters.
#[derive(Clone, Debug)]
pub struct ShapeRecord {
    /// SWF character id.
    pub id: CharacterId,
    /// Shape bounds in twips.
    pub shape_bounds: swf::Rectangle<swf::Twips>,
    /// Fill styles declared for this shape.
    pub fill_styles: Vec<swf::FillStyle>,
    /// Line styles declared for this shape.
    pub line_styles: Vec<swf::LineStyle>,
    /// Sequence of shape edge/style-change records.
    pub records: Vec<swf::ShapeRecord>,
}

/// A single glyph within a [`FontGlyphSet`].
#[derive(Clone, Debug)]
pub struct FontGlyph {
    /// Unicode code point that this glyph represents, or `None` for `DefineFont` (v1)
    /// glyphs that have no accompanying `DefineFontInfo` tag.
    pub code: Option<u16>,
    /// Advance width in twips (may be absent for old v1 fonts).
    pub advance: Option<i16>,
    /// Vector shape edges that define the glyph outline.
    pub shape_records: Vec<swf::ShapeRecord>,
}

/// All glyphs for one font character, extracted from `DefineFont`, `DefineFont2`, or
/// `DefineFont3` tags (optionally enriched by `DefineFontInfo`).
#[derive(Clone, Debug)]
pub struct FontGlyphSet {
    /// SWF character id.
    pub id: CharacterId,
    /// Font family name.
    pub name: String,
    /// Whether the font is bold.
    pub is_bold: bool,
    /// Whether the font is italic.
    pub is_italic: bool,
    /// Layout metrics, if present (ascent, descent, leading in twips).
    pub ascent: Option<u16>,
    /// Descent in twips.
    pub descent: Option<u16>,
    /// Leading in twips.
    pub leading: Option<i16>,
    /// Ordered list of glyphs (glyph index = position in this vec).
    pub glyphs: Vec<FontGlyph>,
}

/// A single placed child from a sprite's first frame.
#[derive(Clone, Debug)]
pub struct PlaceRecord {
    /// Display-list depth.
    pub depth: Depth,
    /// Character placed at this depth, or `None` for a modify-only `PlaceObject`.
    pub character_id: Option<CharacterId>,
    /// Local-to-parent transform matrix.
    pub matrix: Option<Matrix>,
    /// Color/alpha multiplier+add transform.
    pub color_transform: Option<ColorTransform>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Standalone extraction functions
// ──────────────────────────────────────────────────────────────────────────────

/// Inline macro-style helper: decompresses and parses SWF bytes, then runs `$body`
/// with the parsed tag list available as `tags`. Returns on parse error.
macro_rules! with_parsed_swf {
    ($bytes:expr, |$tags:ident| $body:block) => {{
        let buf = swf::decompress_swf(std::io::Cursor::new($bytes))?;
        let parsed = swf::parse_swf(&buf)?;
        let $tags = parsed.tags;
        $body
    }};
}

/// Extract all bitmap characters from a SWF, returning them as RGBA images.
///
/// Handled tag types:
/// - `DefineBitsLossless` / `DefineBitsLossless2` — zlib-compressed palettised or
///   ARGB pixel data.
/// - `DefineBitsJpeg2` — raw JPEG data (decoded via the `image` crate).
/// - `DefineBitsJpeg3` / `DefineBitsJpeg4` — JPEG + zlib-compressed alpha channel.
///
/// Action tags (`DoAction`, `DoInitAction`, `DoAbc`, `DoAbc2`) are ignored entirely.
pub fn extract_bitmaps(swf_bytes: &[u8]) -> Result<HashMap<CharacterId, RgbaImage>, UiError> {
    let mut out: HashMap<CharacterId, RgbaImage> = HashMap::new();

    with_parsed_swf!(swf_bytes, |tags| {
        let mut skipped_action_tags = 0u32;
        for tag in &tags {
            match tag {
                Tag::DefineBitsLossless(bmp) => {
                    match decode_lossless(bmp) {
                        Ok(img) => {
                            out.insert(bmp.id, img);
                        }
                        Err(e) => {
                            log::warn!("skipping DefineBitsLossless id={}: {e}", bmp.id);
                        }
                    }
                }
                Tag::DefineBitsJpeg2 { id, jpeg_data } => {
                    match image::load_from_memory(jpeg_data) {
                        Ok(dyn_img) => {
                            out.insert(*id, dyn_img.to_rgba8());
                        }
                        Err(e) => {
                            log::warn!("skipping DefineBitsJpeg2 id={id}: {e}");
                        }
                    }
                }
                Tag::DefineBitsJpeg3(jpeg3) => {
                    match decode_jpeg3(jpeg3) {
                        Ok(img) => {
                            out.insert(jpeg3.id, img);
                        }
                        Err(e) => {
                            log::warn!("skipping DefineBitsJpeg3/4 id={}: {e}", jpeg3.id);
                        }
                    }
                }
                Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                    skipped_action_tags += 1;
                }
                _ => {}
            }
        }
        if skipped_action_tags > 0 {
            log::debug!("extract_bitmaps: skipped {skipped_action_tags} action tag(s)");
        }
        Ok(out)
    })
}

/// Extract all shape characters from a SWF.
///
/// Handles `DefineShape`, `DefineShape2`, `DefineShape3`, and `DefineShape4` — all of
/// which the `swf` crate normalises to `Tag::DefineShape(Shape)`.
pub fn extract_shapes(swf_bytes: &[u8]) -> Result<HashMap<CharacterId, ShapeRecord>, UiError> {
    let mut out: HashMap<CharacterId, ShapeRecord> = HashMap::new();

    with_parsed_swf!(swf_bytes, |tags| {
        let mut skipped_action_tags = 0u32;
        for tag in &tags {
            match tag {
                Tag::DefineShape(shape) => {
                    let rec = ShapeRecord {
                        id: shape.id,
                        shape_bounds: shape.shape_bounds.clone(),
                        fill_styles: shape.styles.fill_styles.clone(),
                        line_styles: shape.styles.line_styles.clone(),
                        records: shape.shape.clone(),
                    };
                    out.insert(shape.id, rec);
                }
                Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                    skipped_action_tags += 1;
                }
                _ => {}
            }
        }
        if skipped_action_tags > 0 {
            log::debug!("extract_shapes: skipped {skipped_action_tags} action tag(s)");
        }
        Ok(out)
    })
}

/// Extract all font characters from a SWF.
///
/// Handles `DefineFont` (v1), `DefineFont2`, `DefineFont3` (both appear as
/// `Tag::DefineFont2` in the `swf` crate), and `DefineFontInfo`/`DefineFontInfo2`
/// (which supplement v1 fonts with their code tables). `DefineFont4` (CFF) is noted
/// but the raw OpenType/CFF bytes are not decoded — the entry is created with an empty
/// glyph list and the name only.
pub fn extract_fonts(swf_bytes: &[u8]) -> Result<HashMap<CharacterId, FontGlyphSet>, UiError> {
    let mut out: HashMap<CharacterId, FontGlyphSet> = HashMap::new();

    with_parsed_swf!(swf_bytes, |tags| {
        let mut skipped_action_tags = 0u32;
        // First pass: collect all font definitions.
        for tag in &tags {
            match tag {
                Tag::DefineFont(fv1) => {
                    let glyphs = fv1
                        .glyphs
                        .iter()
                        .map(|records| FontGlyph {
                            code: None,
                            advance: None,
                            shape_records: records.clone(),
                        })
                        .collect();
                    out.insert(
                        fv1.id,
                        FontGlyphSet {
                            id: fv1.id,
                            name: String::new(),
                            is_bold: false,
                            is_italic: false,
                            ascent: None,
                            descent: None,
                            leading: None,
                            glyphs,
                        },
                    );
                }
                Tag::DefineFont2(font) => {
                    let glyphs = font
                        .glyphs
                        .iter()
                        .map(|g| FontGlyph {
                            code: Some(g.code),
                            advance: Some(g.advance),
                            shape_records: g.shape_records.clone(),
                        })
                        .collect();
                    let (ascent, descent, leading) = font
                        .layout
                        .as_ref()
                        .map(|l| (Some(l.ascent), Some(l.descent), Some(l.leading)))
                        .unwrap_or((None, None, None));
                    out.insert(
                        font.id,
                        FontGlyphSet {
                            id: font.id,
                            name: font.name.to_string_lossy(swf::UTF_8),
                            is_bold: font.flags.contains(swf::FontFlag::IS_BOLD),
                            is_italic: font.flags.contains(swf::FontFlag::IS_ITALIC),
                            ascent,
                            descent,
                            leading,
                            glyphs,
                        },
                    );
                }
                Tag::DefineFont4(font4) => {
                    // CFF/OpenType font — store name only; glyph outline decoding is
                    // deferred to a later phase.
                    out.insert(
                        font4.id,
                        FontGlyphSet {
                            id: font4.id,
                            name: font4.name.to_string_lossy(swf::UTF_8),
                            is_bold: font4.is_bold,
                            is_italic: font4.is_italic,
                            ascent: None,
                            descent: None,
                            leading: None,
                            glyphs: vec![],
                        },
                    );
                }
                Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                    skipped_action_tags += 1;
                }
                _ => {}
            }
        }

        // Second pass: apply DefineFontInfo / DefineFontInfo2 code tables to v1 fonts.
        for tag in &tags {
            if let Tag::DefineFontInfo(info) = tag {
                if let Some(font) = out.get_mut(&info.id) {
                    // Apply name from FontInfo if the font entry has none (v1 case).
                    if font.name.is_empty() {
                        font.name = info.name.to_string_lossy(swf::UTF_8);
                    }
                    font.is_bold = info.flags.contains(swf::FontInfoFlag::IS_BOLD);
                    font.is_italic = info.flags.contains(swf::FontInfoFlag::IS_ITALIC);
                    // Assign codes from the code table to the glyph slots.
                    for (glyph, &code) in font.glyphs.iter_mut().zip(info.code_table.iter()) {
                        glyph.code = Some(code);
                    }
                }
            }
        }

        if skipped_action_tags > 0 {
            log::debug!("extract_fonts: skipped {skipped_action_tags} action tag(s)");
        }
        Ok(out)
    })
}

/// Extract the first-frame display list from a `DefineSprite` character.
///
/// Walks the sprite's control tags in order, stopping **at** (not past) the first
/// `ShowFrame`. Returns all `PlaceObject`/`PlaceObject2`/`PlaceObject3` tags seen
/// before that `ShowFrame` as [`PlaceRecord`]s.
///
/// `RemoveObject` tags that appear before the first `ShowFrame` are **honoured** —
/// if a character is removed before `ShowFrame` it will not appear in the result.
/// This mirrors what a Flash player would render on frame 1.
///
/// Returns `Err` if no `DefineSprite` with `sprite_id` is found in the SWF.
pub fn extract_sprite_first_frame(
    swf_bytes: &[u8],
    sprite_id: CharacterId,
) -> Result<Vec<PlaceRecord>, UiError> {
    with_parsed_swf!(swf_bytes, |tags| {
        // Find the DefineSprite for the requested id.
        let sprite = tags.iter().find_map(|t| {
            if let Tag::DefineSprite(s) = t {
                if s.id == sprite_id {
                    return Some(s);
                }
            }
            None
        });

        let sprite = sprite.ok_or_else(|| {
            UiError::UnsupportedTag(format!("DefineSprite id={sprite_id} not found"))
        })?;

        // Walk control tags up to (and including) the first ShowFrame.
        // depth → PlaceRecord; we honour RemoveObject by removing from the map.
        let mut depth_map: HashMap<Depth, PlaceRecord> = HashMap::new();

        'walk: for tag in &sprite.tags {
            match tag {
                Tag::ShowFrame => break 'walk,

                Tag::PlaceObject(po) => {
                    let character_id = match po.action {
                        swf::PlaceObjectAction::Place(id) => Some(id),
                        swf::PlaceObjectAction::Replace(id) => Some(id),
                        swf::PlaceObjectAction::Modify => {
                            // Modify: update existing entry if present.
                            depth_map.get(&po.depth).map(|r| r.character_id).flatten()
                        }
                    };
                    depth_map.insert(
                        po.depth,
                        PlaceRecord {
                            depth: po.depth,
                            character_id,
                            matrix: po.matrix,
                            color_transform: po.color_transform,
                        },
                    );
                }

                Tag::RemoveObject(ro) => {
                    depth_map.remove(&ro.depth);
                }

                // Action tags: skip unconditionally.
                Tag::DoAction(_)
                | Tag::DoInitAction { .. }
                | Tag::DoAbc(_)
                | Tag::DoAbc2(_) => {}

                _ => {}
            }
        }

        // Sort by depth for a deterministic, painter's-order result.
        let mut records: Vec<PlaceRecord> = depth_map.into_values().collect();
        records.sort_by_key(|r| r.depth);
        Ok(records)
    })
}

/// Extract the linkage-name → character-id mapping from a SWF.
///
/// Handles both `ExportAssets` (SWF3+) and `SymbolClass` (AVM2/SWF9+) tags.
pub fn extract_exported_symbols(
    swf_bytes: &[u8],
) -> Result<HashMap<String, CharacterId>, UiError> {
    let mut out: HashMap<String, CharacterId> = HashMap::new();

    with_parsed_swf!(swf_bytes, |tags| {
        let mut skipped_action_tags = 0u32;
        for tag in &tags {
            match tag {
                Tag::ExportAssets(exports) => {
                    for asset in exports {
                        out.insert(asset.name.to_string_lossy(swf::UTF_8), asset.id);
                    }
                }
                Tag::SymbolClass(links) => {
                    for link in links {
                        out.insert(link.class_name.to_string_lossy(swf::UTF_8), link.id);
                    }
                }
                Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                    skipped_action_tags += 1;
                }
                _ => {}
            }
        }
        if skipped_action_tags > 0 {
            log::debug!("extract_exported_symbols: skipped {skipped_action_tags} action tag(s)");
        }
        Ok(out)
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// SwfAssetLibrary — content-addressed cache of all extracted atoms
// ──────────────────────────────────────────────────────────────────────────────

/// A lazily-indexed, content-addressed cache of all static visual atoms from one SWF.
///
/// Construct with [`SwfAssetLibrary::new`], which computes the SHA-256 of the raw
/// bytes (the *content hash*) and indexes all tag categories in a single parse pass.
/// Subsequent `get_*` calls are O(1) HashMap lookups.
///
/// Action tags (`DoAction`, `DoInitAction`, `DoAbc`, `DoAbc2`) are **never** decoded.
pub struct SwfAssetLibrary {
    /// SHA-256 of the raw SWF bytes, hex-encoded. Usable as a stable cache key.
    content_hash: String,
    bitmaps: HashMap<CharacterId, RgbaImage>,
    shapes: HashMap<CharacterId, ShapeRecord>,
    fonts: HashMap<CharacterId, FontGlyphSet>,
    exports: HashMap<String, CharacterId>,
    /// Raw bytes, kept so sprite first-frame queries can re-parse on demand.
    raw: Vec<u8>,
}

impl SwfAssetLibrary {
    /// Build a library from raw SWF bytes.
    ///
    /// Computes the content hash and indexes bitmaps, shapes, fonts, and export
    /// symbols eagerly. Sprite first-frames are resolved lazily via
    /// [`get_sprite_first_frame`][Self::get_sprite_first_frame].
    pub fn new(swf_bytes: Vec<u8>) -> Result<Self, UiError> {
        let content_hash = {
            let mut hasher = Sha256::new();
            hasher.update(&swf_bytes);
            format!("{:x}", hasher.finalize())
        };

        let bitmaps = extract_bitmaps(&swf_bytes)?;
        let shapes = extract_shapes(&swf_bytes)?;
        let fonts = extract_fonts(&swf_bytes)?;
        let exports = extract_exported_symbols(&swf_bytes)?;

        Ok(Self {
            content_hash,
            bitmaps,
            shapes,
            fonts,
            exports,
            raw: swf_bytes,
        })
    }

    /// SHA-256 of the raw SWF bytes (hex-encoded).
    ///
    /// Suitable for use as a content-addressed cache key shared across canvases that
    /// reference the same SWF.
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Retrieve a bitmap by character id.
    pub fn get_bitmap(&self, id: CharacterId) -> Option<&RgbaImage> {
        self.bitmaps.get(&id)
    }

    /// Retrieve a shape by character id.
    pub fn get_shape(&self, id: CharacterId) -> Option<&ShapeRecord> {
        self.shapes.get(&id)
    }

    /// Retrieve a font glyph set by character id.
    pub fn get_font(&self, id: CharacterId) -> Option<&FontGlyphSet> {
        self.fonts.get(&id)
    }

    /// Resolve a linkage name (from `ExportAssets` or `SymbolClass`) to a character id.
    pub fn lookup_export(&self, name: &str) -> Option<CharacterId> {
        self.exports.get(name).copied()
    }

    /// Return the first-frame display list of a `DefineSprite` character.
    ///
    /// This re-parses the raw SWF bytes on each call. For high-frequency access,
    /// callers should cache the result themselves.
    pub fn get_sprite_first_frame(
        &self,
        sprite_id: CharacterId,
    ) -> Result<Vec<PlaceRecord>, UiError> {
        extract_sprite_first_frame(&self.raw, sprite_id)
    }

    /// Number of bitmap characters indexed.
    pub fn bitmap_count(&self) -> usize {
        self.bitmaps.len()
    }

    /// Number of shape characters indexed.
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    /// Number of font characters indexed.
    pub fn font_count(&self) -> usize {
        self.fonts.len()
    }

    /// Number of exported symbol linkages indexed.
    pub fn export_count(&self) -> usize {
        self.exports.len()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Private decode helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Decode a `DefineBitsLossless` or `DefineBitsLossless2` tag into an `RgbaImage`.
fn decode_lossless(bmp: &swf::DefineBitsLossless<'_>) -> Result<RgbaImage, UiError> {
    let width = u32::from(bmp.width);
    let height = u32::from(bmp.height);

    // Decompress the zlib payload.
    let mut raw: Vec<u8> = Vec::new();
    ZlibDecoder::new(bmp.data.as_ref()).read_to_end(&mut raw)?;

    match bmp.format {
        swf::BitmapFormat::Rgb32 => {
            // Each pixel is 4 bytes on disk.
            // Version 1 (DefineBitsLossless):  [pad, R, G, B]
            // Version 2 (DefineBitsLossless2): [A, R, G, B]
            let expected = (width * height * 4) as usize;
            if raw.len() < expected {
                return Err(UiError::SwfParse(format!(
                    "DefineBitsLossless id={}: Rgb32 decompressed size {} < expected {expected}",
                    bmp.id,
                    raw.len()
                )));
            }
            let mut img = RgbaImage::new(width, height);
            for (i, pixel) in img.pixels_mut().enumerate() {
                let base = i * 4;
                // SWF stores ARGB; image expects RGBA.
                let a = if bmp.version == 2 { raw[base] } else { 255 };
                let r = raw[base + 1];
                let g = raw[base + 2];
                let b = raw[base + 3];
                *pixel = image::Rgba([r, g, b, a]);
            }
            Ok(img)
        }

        swf::BitmapFormat::ColorMap8 { num_colors } => {
            let palette_entries = usize::from(num_colors) + 1;
            // Version 1 palette entries are RGB (3 bytes); v2 are RGBA (4 bytes).
            let bytes_per_entry: usize = if bmp.version == 2 { 4 } else { 3 };
            let palette_bytes = palette_entries * bytes_per_entry;

            if raw.len() < palette_bytes {
                return Err(UiError::SwfParse(format!(
                    "DefineBitsLossless id={}: ColorMap8 palette truncated",
                    bmp.id
                )));
            }

            // Pixel data rows are padded to a 4-byte boundary.
            let row_stride = (width as usize + 3) & !3;
            let pixel_data = &raw[palette_bytes..];

            let mut img = RgbaImage::new(width, height);
            for row in 0..height as usize {
                for col in 0..width as usize {
                    let idx = pixel_data
                        .get(row * row_stride + col)
                        .copied()
                        .unwrap_or(0) as usize;
                    let pal_off = idx * bytes_per_entry;
                    let (r, g, b, a) = if bmp.version == 2 {
                        (
                            raw.get(palette_bytes - palette_bytes + pal_off).copied().unwrap_or(0),
                            raw.get(palette_bytes - palette_bytes + pal_off + 1).copied().unwrap_or(0),
                            raw.get(palette_bytes - palette_bytes + pal_off + 2).copied().unwrap_or(0),
                            raw.get(palette_bytes - palette_bytes + pal_off + 3).copied().unwrap_or(255),
                        )
                    } else {
                        (
                            raw.get(pal_off).copied().unwrap_or(0),
                            raw.get(pal_off + 1).copied().unwrap_or(0),
                            raw.get(pal_off + 2).copied().unwrap_or(0),
                            255,
                        )
                    };
                    img.put_pixel(col as u32, row as u32, image::Rgba([r, g, b, a]));
                }
            }
            Ok(img)
        }

        swf::BitmapFormat::Rgb15 => {
            // RGB15 (15-bit colour, SWF v1 only) is uncommon in SC assets.
            // Decode: each pixel is 2 bytes, XRRRRRGGGGGBBBBB.
            let expected = (width * height * 2) as usize;
            if raw.len() < expected {
                return Err(UiError::SwfParse(format!(
                    "DefineBitsLossless id={}: Rgb15 decompressed size {} < expected {expected}",
                    bmp.id,
                    raw.len()
                )));
            }
            let mut img = RgbaImage::new(width, height);
            for (i, pixel) in img.pixels_mut().enumerate() {
                let lo = raw[i * 2] as u16;
                let hi = raw[i * 2 + 1] as u16;
                let word = (hi << 8) | lo;
                let r = (((word >> 10) & 0x1F) * 255 / 31) as u8;
                let g = (((word >> 5) & 0x1F) * 255 / 31) as u8;
                let b = ((word & 0x1F) * 255 / 31) as u8;
                *pixel = image::Rgba([r, g, b, 255]);
            }
            Ok(img)
        }
    }
}

/// Decode a `DefineBitsJpeg3` (or Jpeg4) tag: JPEG body + optional zlib alpha channel.
fn decode_jpeg3(jpeg3: &swf::DefineBitsJpeg3<'_>) -> Result<RgbaImage, UiError> {
    let mut img = image::load_from_memory_with_format(jpeg3.data, ImageFormat::Jpeg)
        .map(|d| d.to_rgba8())?;

    if !jpeg3.alpha_data.is_empty() {
        // Alpha channel is zlib-compressed, one byte per pixel.
        let mut alpha: Vec<u8> = Vec::new();
        ZlibDecoder::new(jpeg3.alpha_data).read_to_end(&mut alpha)?;

        let pixel_count = (img.width() * img.height()) as usize;
        if alpha.len() >= pixel_count {
            for (i, pixel) in img.pixels_mut().enumerate() {
                pixel.0[3] = alpha[i];
            }
        } else {
            log::warn!(
                "DefineBitsJpeg3 id={}: alpha channel length {} < pixel count {pixel_count}",
                jpeg3.id,
                alpha.len()
            );
        }
    }

    Ok(img)
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal in-memory SWF using the `swf` crate's writer.
    ///
    /// The SWF contains:
    /// - `SetBackgroundColor` (red background)
    /// - `DefineShape` (character id=1, solid blue rectangle)
    /// - `ShowFrame`
    ///
    /// No action tags are included; this exercises the shape extractor and verifies
    /// that the library does not panic on valid input.
    fn make_minimal_swf() -> Vec<u8> {
        use swf::*;

        let header = Header {
            compression: Compression::None,
            version: 6,
            stage_size: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(200.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(200.0),
            },
            frame_rate: Fixed8::from_f32(24.0),
            num_frames: 1,
        };

        let shape = Shape {
            version: 1,
            id: 1,
            shape_bounds: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(100.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(100.0),
            },
            edge_bounds: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(100.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(100.0),
            },
            flags: ShapeFlag::empty(),
            styles: ShapeStyles {
                fill_styles: vec![FillStyle::Color(Color {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                })],
                line_styles: vec![],
            },
            shape: vec![
                ShapeRecord::StyleChange(Box::new(StyleChangeData {
                    move_to: Some(Point::new(Twips::ZERO, Twips::ZERO)),
                    fill_style_0: None,
                    fill_style_1: Some(1),
                    line_style: None,
                    new_styles: None,
                })),
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::from_pixels(100.0), Twips::ZERO),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(100.0)),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::from_pixels(-100.0), Twips::ZERO),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(-100.0)),
                },
            ],
        };

        let tags = [
            Tag::SetBackgroundColor(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
            Tag::DefineShape(shape),
            Tag::ShowFrame,
        ];

        let mut buf = Vec::new();
        swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
        buf
    }

    /// Build a SWF that includes a `DoAction` tag to verify it is silently skipped.
    fn make_swf_with_action() -> Vec<u8> {
        use swf::*;

        let header = Header {
            compression: Compression::None,
            version: 6,
            stage_size: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(100.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(100.0),
            },
            frame_rate: Fixed8::from_f32(24.0),
            num_frames: 1,
        };

        // DoAction with a trivial ActionStop opcode (0x07).
        let action_bytes: &[u8] = &[0x07, 0x00];
        let tags = [Tag::DoAction(action_bytes), Tag::ShowFrame];

        let mut buf = Vec::new();
        swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
        buf
    }

    #[test]
    fn parse_minimal_swf_without_panicking() {
        let swf_bytes = make_minimal_swf();
        // All extractors must complete without panicking.
        extract_bitmaps(&swf_bytes).expect("extract_bitmaps failed");
        extract_shapes(&swf_bytes).expect("extract_shapes failed");
        extract_fonts(&swf_bytes).expect("extract_fonts failed");
        extract_exported_symbols(&swf_bytes).expect("extract_exported_symbols failed");
    }

    #[test]
    fn shape_extractor_recognises_define_shape() {
        let swf_bytes = make_minimal_swf();
        let shapes = extract_shapes(&swf_bytes).expect("extract_shapes failed");
        assert!(!shapes.is_empty(), "expected at least one shape");
        let shape = shapes.get(&1).expect("shape id=1 not found");
        assert_eq!(shape.id, 1);
        assert!(!shape.fill_styles.is_empty(), "expected fill styles");
    }

    #[test]
    fn action_tags_not_surfaced_by_bitmap_extractor() {
        let swf_bytes = make_swf_with_action();
        // Must not panic and must return an empty bitmap map (no bitmap tags in SWF).
        let bitmaps = extract_bitmaps(&swf_bytes).expect("extract_bitmaps failed");
        assert!(
            bitmaps.is_empty(),
            "DoAction must not produce a bitmap entry"
        );
    }

    #[test]
    fn action_tags_not_surfaced_by_shape_extractor() {
        let swf_bytes = make_swf_with_action();
        let shapes = extract_shapes(&swf_bytes).expect("extract_shapes failed");
        assert!(shapes.is_empty(), "DoAction must not produce a shape entry");
    }

    #[test]
    fn library_content_hash_is_stable() {
        let bytes = make_minimal_swf();
        let lib1 = SwfAssetLibrary::new(bytes.clone()).expect("library 1 failed");
        let lib2 = SwfAssetLibrary::new(bytes).expect("library 2 failed");
        assert_eq!(lib1.content_hash(), lib2.content_hash());
        assert!(!lib1.content_hash().is_empty());
    }

    #[test]
    fn library_shape_count_matches_extract_shapes() {
        let bytes = make_minimal_swf();
        let lib = SwfAssetLibrary::new(bytes.clone()).expect("library failed");
        let direct = extract_shapes(&bytes).expect("extract_shapes failed");
        assert_eq!(lib.shape_count(), direct.len());
        assert!(lib.shape_count() >= 1);
    }

    /// Optional integration test: parse an external SWF supplied via `SB_TEST_SWF`.
    ///
    /// Skipped when the env var is absent so that CI stays self-contained.
    #[test]
    fn integration_parse_external_swf_if_present() {
        let path = match std::env::var("SB_TEST_SWF") {
            Ok(p) => p,
            Err(_) => return, // skip when unset
        };
        let bytes = std::fs::read(&path).expect("could not read SB_TEST_SWF");
        let lib = SwfAssetLibrary::new(bytes).expect("SwfAssetLibrary::new failed");
        println!(
            "SB_TEST_SWF={path}: bitmaps={} shapes={} fonts={} exports={}",
            lib.bitmap_count(),
            lib.shape_count(),
            lib.font_count(),
            lib.export_count()
        );
        // At least one of bitmaps/shapes/fonts must be present in a real SWF.
        assert!(
            lib.bitmap_count() + lib.shape_count() + lib.font_count() > 0,
            "expected at least one asset in {path}"
        );
    }
}
