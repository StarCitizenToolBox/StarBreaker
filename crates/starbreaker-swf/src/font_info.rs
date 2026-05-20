//! Enumerate every embedded font in an SWF.
//!
//! Walks the tag stream of an SWF (CWS / ZWS / FWS) and produces one
//! [`FontEntry`] per `DefineFont2` / `DefineFont3` tag, augmented with
//! information from any associated `DefineFontInfo*` and `DefineFontName`
//! tags. Day 1 of the SC font extraction work — Day 2 will convert each
//! entry's glyph shapes to TrueType contours.

use crate::SwfError;
use std::collections::HashMap;
use swf::{Tag, decompress_swf, parse_swf};

/// One font discovered in an SWF.
///
/// The `swf` crate exposes the raw `Font` struct (glyph outlines etc.); this
/// type is a lightweight summary suitable for diagnostic dumps and downstream
/// conversion.
#[derive(Debug, Clone)]
pub struct FontEntry {
    /// SWF character ID (unique within the file).
    pub id: u16,
    /// `DefineFont` version: 2 or 3. Version 3 uses 20× finer coordinates.
    pub version: u8,
    /// Embedded font name (from `DefineFont2/3.name`).
    pub name: String,
    /// Display name from a `DefineFontName` tag, if one references this font.
    pub display_name: Option<String>,
    /// Copyright string from `DefineFontName`, if any.
    pub copyright: Option<String>,
    /// Number of glyph shapes embedded.
    pub glyph_count: usize,
    /// `true` if the font carries a `FontLayout` (ascent/descent/advances).
    pub has_layout: bool,
    /// `true` if the font's flags mark it bold.
    pub is_bold: bool,
    /// `true` if the font's flags mark it italic.
    pub is_italic: bool,
    /// `true` if codepoint mapping (`Glyph.code`) is available either inline
    /// in `DefineFont2/3` or via a companion `DefineFontInfo*` tag.
    pub has_code_table: bool,
    /// Min and max Unicode codepoints covered (None if `has_code_table` is false).
    pub codepoint_range: Option<(u16, u16)>,
}

/// Parse an SWF and return one [`FontEntry`] per embedded font.
pub fn analyze_fonts(bytes: &[u8]) -> Result<Vec<FontEntry>, SwfError> {
    let buf =
        decompress_swf(bytes).map_err(|e| SwfError::Decompress(e.to_string()))?;
    let swf =
        parse_swf(&buf).map_err(|e| SwfError::Tags(e.to_string()))?;

    // First pass: collect any DefineFontName / DefineFontInfo tags keyed by id.
    let mut names: HashMap<u16, (String, String)> = HashMap::new();
    let mut info_code_tables: HashMap<u16, Vec<u16>> = HashMap::new();
    for tag in &swf.tags {
        match tag {
            Tag::DefineFontName { id, name, copyright_info } => {
                names.insert(
                    *id,
                    (
                        name.to_string_lossy(swf::UTF_8),
                        copyright_info.to_string_lossy(swf::UTF_8),
                    ),
                );
            }
            Tag::DefineFontInfo(info) => {
                if !info.code_table.is_empty() {
                    info_code_tables.insert(info.id, info.code_table.clone());
                }
            }
            _ => {}
        }
    }

    let mut entries = Vec::new();
    for tag in &swf.tags {
        if let Tag::DefineFont2(font) = tag {
            let id = font.id;
            let (display_name, copyright) = match names.remove(&id) {
                Some((n, c)) => (Some(n), Some(c)),
                None => (None, None),
            };

            // Codepoint info: prefer the inline `code` per glyph (carried in
            // DefineFont2/3 when the font has a code table), otherwise fall
            // back to a companion DefineFontInfo tag.
            let mut codepoints: Vec<u16> =
                font.glyphs.iter().map(|g| g.code).filter(|c| *c != 0).collect();
            if codepoints.is_empty() {
                if let Some(extra) = info_code_tables.get(&id) {
                    codepoints = extra.iter().copied().filter(|c| *c != 0).collect();
                }
            }
            let codepoint_range = if codepoints.is_empty() {
                None
            } else {
                let mn = *codepoints.iter().min().expect("non-empty");
                let mx = *codepoints.iter().max().expect("non-empty");
                Some((mn, mx))
            };

            entries.push(FontEntry {
                id,
                version: font.version,
                name: font.name.to_string_lossy(swf::UTF_8),
                display_name,
                copyright,
                glyph_count: font.glyphs.len(),
                has_layout: font.layout.is_some(),
                is_bold: font.flags.contains(swf::FontFlag::IS_BOLD),
                is_italic: font.flags.contains(swf::FontFlag::IS_ITALIC),
                has_code_table: codepoint_range.is_some(),
                codepoint_range,
            });
        }
    }

    Ok(entries)
}
