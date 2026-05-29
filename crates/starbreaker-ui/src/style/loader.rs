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
        let fallback = self.drake_amber_fallback();
        let color_styles = record_json
            .get("_RecordValue_")
            .and_then(|v| v.get("colorStyles"))
            .and_then(|v| v.as_array())
            .or_else(|| record_json.get("colorStyles").and_then(|v| v.as_array()))
            .ok_or_else(|| {
                UiError::ParseError("BuildingBlocks_Style record missing colorStyles[]".to_string())
            })?;

        let primary_tint = parse_color_style_slot(color_styles, 0).unwrap_or(fallback.primary_tint);
        let background = parse_color_style_slot(color_styles, 8).unwrap_or(fallback.background);
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

    /// Non-authoritative Drake amber fallback used when no style record exists.
    pub fn drake_amber_fallback(&self) -> ManufacturerStyle {
        ManufacturerStyle {
            name: self.manufacturer.clone(),
            primary_tint: RgbaColor {
                r: 240,
                g: 168,
                b: 104,
                a: 255,
            },
            secondary_tint: None,
            colour_slots: vec![RgbaColor {
                r: 240,
                g: 168,
                b: 104,
                a: 255,
            }],
            background: RgbaColor {
                r: 48,
                g: 32,
                b: 16,
                a: 255,
            },
            backlight: RgbaColor {
                r: 102,
                g: 214,
                b: 255,
                a: 255,
            },
            font_family_hints: vec!["Rajdhani".into(), "Orbitron".into()],
            crt: CrtParams::default(),
        }
    }
}
