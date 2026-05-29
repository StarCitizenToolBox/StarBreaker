//! Style schema types shared by loading and rendering.

use serde::{Deserialize, Serialize};

use crate::canvas::RgbaColor;

/// CRT post-process hints for the canvas compositor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrtParams {
    pub scanline_period_px: f32,
    pub pixel_grid_period_px: f32,
    pub scanline_intensity: f32,
    pub vignette_strength: f32,
}

impl Default for CrtParams {
    fn default() -> Self {
        Self {
            scanline_period_px: 3.0,
            pixel_grid_period_px: 3.0,
            scanline_intensity: 0.15,
            vignette_strength: 0.3,
        }
    }
}

/// All per-manufacturer visual properties used by the compositor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManufacturerStyle {
    pub name: String,
    pub primary_tint: RgbaColor,
    pub secondary_tint: Option<RgbaColor>,
    pub colour_slots: Vec<RgbaColor>,
    pub background: RgbaColor,
    pub backlight: RgbaColor,
    pub font_family_hints: Vec<String>,
    pub crt: CrtParams,
}
