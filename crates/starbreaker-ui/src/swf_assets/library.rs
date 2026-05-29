use std::collections::HashMap;

use image::RgbaImage;
use sha2::{Digest, Sha256};
use swf::CharacterId;

use crate::error::UiError;

use super::extract::{
    extract_bitmaps, extract_exported_symbols, extract_font_edit_text_metrics, extract_fonts,
    extract_shapes,
};
use super::stage::{extract_sprite_first_frame, extract_stage_frame, extract_stage_size};
use super::types::{FontGlyphSet, PlaceRecord, ShapeRecord, SwfEditTextMetrics};

/// Content-addressed cache of static visual atoms from one SWF.
pub struct SwfAssetLibrary {
    content_hash: String,
    bitmaps: HashMap<CharacterId, RgbaImage>,
    shapes: HashMap<CharacterId, ShapeRecord>,
    fonts: HashMap<CharacterId, FontGlyphSet>,
    exports: HashMap<String, CharacterId>,
    font_edit_text_metrics: HashMap<String, SwfEditTextMetrics>,
    raw: Vec<u8>,
}

impl SwfAssetLibrary {
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
        let font_edit_text_metrics = extract_font_edit_text_metrics(&swf_bytes)?;

        Ok(Self {
            content_hash,
            bitmaps,
            shapes,
            fonts,
            exports,
            font_edit_text_metrics,
            raw: swf_bytes,
        })
    }

    pub fn merge_swf_bytes(&mut self, swf_bytes: &[u8]) -> Result<(), UiError> {
        self.bitmaps.extend(extract_bitmaps(swf_bytes)?);
        self.shapes.extend(extract_shapes(swf_bytes)?);
        self.fonts.extend(extract_fonts(swf_bytes)?);

        for (symbol, metrics) in extract_font_edit_text_metrics(swf_bytes)? {
            self.font_edit_text_metrics.entry(symbol).or_insert(metrics);
        }

        for (name, id) in extract_exported_symbols(swf_bytes)? {
            self.exports.entry(name).or_insert(id);
        }

        Ok(())
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn get_bitmap(&self, id: CharacterId) -> Option<&RgbaImage> {
        self.bitmaps.get(&id)
    }

    pub fn get_shape(&self, id: CharacterId) -> Option<&ShapeRecord> {
        self.shapes.get(&id)
    }

    pub fn get_font(&self, id: CharacterId) -> Option<&FontGlyphSet> {
        self.fonts.get(&id)
    }

    pub fn find_font_by_name(&self, query: &str) -> Option<&FontGlyphSet> {
        let q = query.to_ascii_lowercase();
        self.fonts.values().find(|font| {
            font.name.eq_ignore_ascii_case(&q) || font.name.to_ascii_lowercase().contains(&q)
        })
    }

    pub fn lookup_export(&self, name: &str) -> Option<CharacterId> {
        self.exports.get(name).copied()
    }

    pub fn export_entries(&self) -> impl Iterator<Item = (&str, CharacterId)> {
        self.exports.iter().map(|(name, &id)| (name.as_str(), id))
    }

    pub fn font_edit_text_metrics(&self, symbol: &str) -> Option<&SwfEditTextMetrics> {
        self.font_edit_text_metrics.get(symbol)
    }

    pub fn get_sprite_first_frame(&self, sprite_id: CharacterId) -> Result<Vec<PlaceRecord>, UiError> {
        extract_sprite_first_frame(&self.raw, sprite_id)
    }

    pub fn extract_sprite_first_frame(&self, character_id: CharacterId) -> Vec<PlaceRecord> {
        self.get_sprite_first_frame(character_id).unwrap_or_default()
    }

    pub fn export_name_for(&self, character_id: CharacterId) -> Option<String> {
        self.exports
            .iter()
            .find_map(|(name, &id)| (id == character_id).then(|| name.clone()))
    }

    pub fn stage_frame(&self, frame_index: u32) -> Vec<PlaceRecord> {
        extract_stage_frame(&self.raw, frame_index)
    }

    pub fn stage_size(&self) -> (f32, f32) {
        extract_stage_size(&self.raw)
    }

    pub fn bitmap_count(&self) -> usize {
        self.bitmaps.len()
    }

    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    pub fn font_count(&self) -> usize {
        self.fonts.len()
    }

    pub fn export_count(&self) -> usize {
        self.exports.len()
    }

    pub fn visual_exports(&self) -> impl Iterator<Item = CharacterId> + '_ {
        self.exports
            .iter()
            .filter(|(name, _)| !name.starts_with("__Packages."))
            .map(|(_, &id)| id)
    }
}
