//! JSON parsing helpers for style records.

use crate::canvas::RgbaColor;
use crate::error::UiError;

pub(super) fn parse_color(record: &serde_json::Value, field: &str) -> Result<RgbaColor, UiError> {
    let v = record.get(field).ok_or_else(|| {
        UiError::ParseError(format!(
            "manufacturer style record missing required field '{field}'"
        ))
    })?;
    parse_color_value(v, field)
}

pub(super) fn parse_color_value(v: &serde_json::Value, field: &str) -> Result<RgbaColor, UiError> {
    let r = v["r"].as_u64().ok_or_else(|| {
        UiError::ParseError(format!("color field '{field}.r' is missing or not an integer"))
    })?;
    let g = v["g"].as_u64().ok_or_else(|| {
        UiError::ParseError(format!("color field '{field}.g' is missing or not an integer"))
    })?;
    let b = v["b"].as_u64().ok_or_else(|| {
        UiError::ParseError(format!("color field '{field}.b' is missing or not an integer"))
    })?;
    let a = v["a"].as_u64().unwrap_or(255);
    Ok(RgbaColor {
        r: r as u8,
        g: g as u8,
        b: b as u8,
        a: a as u8,
    })
}

pub(super) fn parse_crt_params(v: &serde_json::Value) -> super::types::CrtParams {
    super::types::CrtParams {
        scanline_period_px: v["scanlinePeriodPx"].as_f64().unwrap_or(3.0) as f32,
        pixel_grid_period_px: v["pixelGridPeriodPx"].as_f64().unwrap_or(3.0) as f32,
        scanline_intensity: v["scanlineIntensity"].as_f64().unwrap_or(0.15) as f32,
        vignette_strength: v["vignetteStrength"].as_f64().unwrap_or(0.3) as f32,
    }
}

pub(super) fn parse_color_style_slot(styles: &[serde_json::Value], index: usize) -> Option<RgbaColor> {
    styles
        .get(index)
        .and_then(|v| v.get("color"))
        .and_then(parse_color_value_lossy)
}

pub(super) fn parse_color_value_lossy(v: &serde_json::Value) -> Option<RgbaColor> {
    let r = v.get("r")?.as_u64()?;
    let g = v.get("g")?.as_u64()?;
    let b = v.get("b")?.as_u64()?;
    let a = v.get("a").and_then(|a| a.as_u64()).unwrap_or(255);
    Some(RgbaColor {
        r: r as u8,
        g: g as u8,
        b: b as u8,
        a: a as u8,
    })
}
