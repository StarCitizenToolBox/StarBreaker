//! StyleLoader implementation.

use crate::canvas::RgbaColor;
use crate::error::UiError;

use super::parse::{
    parse_color, parse_color_style_slot, parse_color_value, parse_color_value_lossy, parse_crt_params,
};
use super::types::{CrtParams, ManufacturerStyle};

/// Loads and parses a manufacturer style record from DataCore JSON.
pub struct StyleLoader {
    manufacturer: String,
}

impl StyleLoader {
    /// Create a loader targeting the named manufacturer.
    pub fn for_manufacturer(name: &str) -> Self {
        Self {
            manufacturer: name.to_owned(),
        }
    }

    /// Parse a `ManufacturerStyle` from a DataCore style record JSON blob.
    pub fn parse_record(&self, record_json: &serde_json::Value) -> Result<ManufacturerStyle, UiError> {
        let primary_tint = parse_color(record_json, "primaryColor")?;
        let background = parse_color(record_json, "backgroundColor")?;
        let backlight = parse_color(record_json, "backlightColor")?;

        let secondary_tint = record_json
            .get("secondaryColor")
            .map(|v| parse_color_value(v, "secondaryColor"))
            .transpose()?;

        let font_family_hints = record_json
            .get("fontFamilies")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let crt = record_json
            .get("crt")
            .map(parse_crt_params)
            .unwrap_or_default();

        Ok(ManufacturerStyle {
            name: self.manufacturer.clone(),
            primary_tint,
            secondary_tint,
            colour_slots: Vec::new(),
            background,
            backlight,
            font_family_hints,
            crt,
        })
    }

    /// Parse a `ManufacturerStyle` from a real `BuildingBlocks_Style` record.
    pub fn parse_buildingblocks_style_record(
        &self,
        record_json: &serde_json::Value,
    ) -> Result<ManufacturerStyle, UiError> {
        let fallback = self.neutral_fallback();
        let color_styles = record_json
            .get("_RecordValue_")
            .and_then(|v| v.get("colorStyles"))
            .and_then(|v| v.as_array())
            .or_else(|| record_json.get("colorStyles").and_then(|v| v.as_array()))
            .ok_or_else(|| {
                UiError::ParseError("BuildingBlocks_Style record missing colorStyles[]".to_string())
            })?;

        let primary_tint = parse_color_style_slot(color_styles, 0).unwrap_or(fallback.primary_tint);
        // Slot 9 = the BB_ColorStyle enum's Background. The previous slot-8
        // read was adjudicated WRONG against two dark-room captures of the
        // Clipper power MFD (plan P5.3, 2026-06-13): the in-game screen
        // background matches the slot-9 render on dark-region ratios
        // (13/15 pairs, ratio RMS 2.3x better, response exponent ~0.91 vs a
        // contorted 0.57 for slot 8). For drak: (38,27,10), not (20,13,5).
        let background = parse_color_style_slot(color_styles, 9).unwrap_or(fallback.background);
        let backlight = parse_color_style_slot(color_styles, 11).unwrap_or(fallback.backlight);
        let mut colour_slots: Vec<RgbaColor> = color_styles
            .iter()
            .filter_map(|slot| slot.get("color").and_then(parse_color_value_lossy))
            .collect();
        if colour_slots.is_empty() {
            colour_slots = fallback.colour_slots.clone();
        }

        Ok(ManufacturerStyle {
            name: self.manufacturer.clone(),
            primary_tint,
            secondary_tint: Some(backlight),
            colour_slots,
            background,
            backlight,
            font_family_hints: fallback.font_family_hints,
            crt: fallback.crt,
        })
    }

    /// NEUTRAL fallback used when no style record (or palette slot) exists.
    ///
    /// Pure white foreground / black background only — absence-of-data
    /// constants, deliberately NOT a brand palette: the previous "Drake
    /// amber" fallback hard-coded invented colours (240,168,104 etc. exist
    /// in no DataCore record), which masked missing-data failures as
    /// plausible-looking output. Brand colours must come from the fetched
    /// `BuildingBlocks_Style` record; if that record is missing, the render
    /// is visibly unstyled rather than silently wrong.
    pub fn neutral_fallback(&self) -> ManufacturerStyle {
        let white = RgbaColor { r: 255, g: 255, b: 255, a: 255 };
        let black = RgbaColor { r: 0, g: 0, b: 0, a: 255 };
        ManufacturerStyle {
            name: self.manufacturer.clone(),
            primary_tint: white,
            secondary_tint: None,
            colour_slots: Vec::new(),
            background: black,
            backlight: white,
            font_family_hints: Vec::new(),
            crt: CrtParams::default(),
        }
    }
}
