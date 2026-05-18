//! Manufacturer style loader — tint, CRT parameters, widget palette.
//!
//! [`ManufacturerStyle`] captures all per-manufacturer visual properties:
//! the primary content tint (e.g. Drake amber), the backlight color (cyan
//! for Drake — sourced from `SCItemDisplayScreenComponentParams.screenStates`),
//! background fill, font family hints, and CRT post-process parameters.
//!
//! [`StyleLoader`] loads a manufacturer style record from a parsed DataCore
//! JSON blob and extracts the fields above.  When no record is available,
//! `drake_amber_fallback()` provides documented non-authoritative values
//! derived from Phase 1 reference-image observation and DataCore research.
//!
//! # Color source notes
//! - **`primary_tint` (amber):** observed from the Drake Clipper reference
//!   images.  The amber color (~#FFB04C) is produced by Drake's CRT material
//!   shader applied on top of the white/neutral SWF content color.  It does
//!   **not** appear directly in any DataCore record — it is a material
//!   property. The fallback value here is an approximation for test and
//!   development use.
//! - **`backlight` (cyan):** from `SCItemDisplayScreenComponentParams
//!   .screenStates[Normal].color = SRGBA8(r:102, g:214, b:255, a:255)`.
//!   This is the screen-frame backlight glow color, **not** the UI content
//!   color.  Phase 1 explicitly documents that these must not be confused.

use serde::{Deserialize, Serialize};

use crate::canvas::RgbaColor;
use crate::error::UiError;

// ──────────────────────────────────────────────────────────────────────────────
// CRT post-process parameters
// ──────────────────────────────────────────────────────────────────────────────

/// CRT post-process hints for the canvas compositor.
///
/// All period values are in canvas pixels (before any viewport scale).
/// Intensity / strength values are in `[0.0, 1.0]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrtParams {
    /// Scanline period in pixels (vertical distance between dark scanlines).
    pub scanline_period_px: f32,
    /// Pixel-grid period in pixels (horizontal distance between pixel columns).
    pub pixel_grid_period_px: f32,
    /// Scanline darkness intensity (0 = invisible, 1 = fully black scanlines).
    pub scanline_intensity: f32,
    /// Vignette corner darkening strength (0 = none, 1 = fully black corners).
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

// ──────────────────────────────────────────────────────────────────────────────
// Manufacturer style
// ──────────────────────────────────────────────────────────────────────────────

/// All per-manufacturer visual properties used by the canvas compositor.
///
/// Fields prefixed with `//` comments below document the authoritative data
/// source for each value so the compositor does not need to guess origins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManufacturerStyle {
    /// Short manufacturer identifier as used in DataCore style record names
    /// (e.g. `"drak"` for Drake Interplanetary).
    pub name: String,

    /// Primary UI content tint color.
    ///
    /// For Drake: warm amber (~#FFB04C) observed from reference images and
    /// the CRT material shader.  This is applied to emissive/lit content
    /// elements; transparent areas are preserved.
    pub primary_tint: RgbaColor,

    /// Optional secondary/accent tint.
    ///
    /// Present when the style record provides a second named color (e.g. a
    /// highlight or inactive state color).
    pub secondary_tint: Option<RgbaColor>,

    /// Canvas background fill color (behind all widgets).
    pub background: RgbaColor,

    /// Screen backlight glow color.
    ///
    /// For Drake: cyan — `SRGBA8(r:102, g:214, b:255, a:255)`.
    /// Source: Phase 1 research,
    /// `SCItemDisplayScreenComponentParams.screenStates[Normal].color`.
    /// This is the physical screen emission color, **not** the content tint.
    pub backlight: RgbaColor,

    /// Font family names to try in order when looking up SWF font records.
    ///
    /// These are hints for canvas composition; the SWF glyph outlines take
    /// precedence when available.
    pub font_family_hints: Vec<String>,

    /// CRT post-process parameters for Phase 8 application.
    pub crt: CrtParams,
}

// ──────────────────────────────────────────────────────────────────────────────
// Style loader
// ──────────────────────────────────────────────────────────────────────────────

/// Loads and parses a manufacturer style record from DataCore JSON.
pub struct StyleLoader {
    manufacturer: String,
}

impl StyleLoader {
    /// Create a loader targeting the named manufacturer.
    ///
    /// `name` should be the short identifier as used in DataCore style record
    /// names, e.g. `"drak"`.
    pub fn for_manufacturer(name: &str) -> Self {
        Self { manufacturer: name.to_owned() }
    }

    /// Parse a `ManufacturerStyle` from a DataCore style record JSON blob.
    ///
    /// The record must contain at minimum:
    /// - `"primaryColor"` → object with `"r"`, `"g"`, `"b"` integer fields.
    /// - `"backgroundColor"` → same shape.
    /// - `"backlightColor"` → same shape.
    ///
    /// Optional fields (absent fields are given sensible defaults):
    /// - `"secondaryColor"` → same shape.
    /// - `"fontFamilies"` → array of strings.
    /// - `"crt"` → object with `"scanlinePeriodPx"`, `"pixelGridPeriodPx"`,
    ///   `"scanlineIntensity"`, `"vignetteStrength"` float fields.
    ///
    /// Returns [`UiError::ParseError`] if a required field is absent or has
    /// the wrong type.
    pub fn parse_record(
        &self,
        record_json: &serde_json::Value,
    ) -> Result<ManufacturerStyle, UiError> {
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
            background,
            backlight,
            font_family_hints,
            crt,
        })
    }

    /// Non-authoritative Drake amber fallback used when no DataCore record is
    /// available (e.g. in unit tests or during early development).
    ///
    /// Values:
    /// - `primary_tint`: ~#FFB04C (warm amber) — approximated from Phase 1
    ///   reference-image observation of the Drake Clipper MFD screens.
    /// - `backlight`: SRGBA8(r:102, g:214, b:255, a:255) — directly from
    ///   Phase 1 DataCore research:
    ///   `SCItemDisplayScreenComponentParams.screenStates[Normal].color`.
    /// - `background`: near-black (#0A0A0A) — the dark CRT screen background.
    ///
    /// Production code must prefer `parse_record` with a real DataCore record.
    pub fn drake_amber_fallback(&self) -> ManufacturerStyle {
        ManufacturerStyle {
            name: self.manufacturer.clone(),

            // Warm amber ~#FFB04C — Phase 1 reference-image observation.
            // Drake CRT material shader produces this tint on the SWF content.
            primary_tint: RgbaColor { r: 255, g: 176, b: 76, a: 255 },

            secondary_tint: None,

            // Near-black CRT screen background.
            background: RgbaColor { r: 10, g: 10, b: 10, a: 255 },

            // Cyan screen-frame backlight.
            // Source: Phase 1 — SCItemDisplayScreenComponentParams
            //   .screenStates[Normal].color = SRGBA8(r:102,g:214,b:255,a:255)
            backlight: RgbaColor { r: 102, g: 214, b: 255, a: 255 },

            font_family_hints: vec!["Rajdhani".into(), "Orbitron".into()],

            crt: CrtParams::default(),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

fn parse_color(
    record: &serde_json::Value,
    field: &str,
) -> Result<RgbaColor, UiError> {
    let v = record.get(field).ok_or_else(|| {
        UiError::ParseError(format!("manufacturer style record missing required field '{field}'"))
    })?;
    parse_color_value(v, field)
}

fn parse_color_value(v: &serde_json::Value, field: &str) -> Result<RgbaColor, UiError> {
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
    Ok(RgbaColor { r: r as u8, g: g as u8, b: b as u8, a: a as u8 })
}

fn parse_crt_params(v: &serde_json::Value) -> CrtParams {
    CrtParams {
        scanline_period_px: v["scanlinePeriodPx"].as_f64().unwrap_or(3.0) as f32,
        pixel_grid_period_px: v["pixelGridPeriodPx"].as_f64().unwrap_or(3.0) as f32,
        scanline_intensity: v["scanlineIntensity"].as_f64().unwrap_or(0.15) as f32,
        vignette_strength: v["vignetteStrength"].as_f64().unwrap_or(0.3) as f32,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn loader() -> StyleLoader {
        StyleLoader::for_manufacturer("drak")
    }

    // ── drake_amber_fallback ─────────────────────────────────────────────────

    #[test]
    fn drake_amber_fallback_name() {
        assert_eq!(loader().drake_amber_fallback().name, "drak");
    }

    #[test]
    fn drake_amber_fallback_primary_tint_is_amber() {
        // Amber hue: R > G > B and all R > 0.6 (normalised).
        let style = loader().drake_amber_fallback();
        let r = style.primary_tint.r as f32 / 255.0;
        let g = style.primary_tint.g as f32 / 255.0;
        let b = style.primary_tint.b as f32 / 255.0;
        assert!(r > 0.6, "expected R > 0.6, got {r}");
        assert!(r > g, "expected R > G ({r} > {g})");
        assert!(g > b, "expected G > B ({g} > {b})");
    }

    #[test]
    fn drake_amber_fallback_backlight_is_cyan() {
        // The backlight is documented as SRGBA8(r:102, g:214, b:255, a:255).
        let style = loader().drake_amber_fallback();
        assert_eq!(style.backlight, RgbaColor { r: 102, g: 214, b: 255, a: 255 });
    }

    // ── parse_record — happy path ────────────────────────────────────────────

    fn full_fixture() -> serde_json::Value {
        serde_json::json!({
            "primaryColor":    { "r": 255, "g": 176, "b": 76,  "a": 255 },
            "secondaryColor":  { "r": 200, "g": 100, "b": 50,  "a": 255 },
            "backgroundColor": { "r": 10,  "g": 10,  "b": 10,  "a": 255 },
            "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 },
            "fontFamilies":    ["Rajdhani", "Orbitron"],
            "crt": {
                "scanlinePeriodPx":  4.0,
                "pixelGridPeriodPx": 3.5,
                "scanlineIntensity": 0.2,
                "vignetteStrength":  0.4
            }
        })
    }

    #[test]
    fn parse_record_primary_tint() {
        let style = loader().parse_record(&full_fixture()).unwrap();
        assert_eq!(style.primary_tint, RgbaColor { r: 255, g: 176, b: 76, a: 255 });
    }

    #[test]
    fn parse_record_secondary_tint() {
        let style = loader().parse_record(&full_fixture()).unwrap();
        assert_eq!(style.secondary_tint, Some(RgbaColor { r: 200, g: 100, b: 50, a: 255 }));
    }

    #[test]
    fn parse_record_background() {
        let style = loader().parse_record(&full_fixture()).unwrap();
        assert_eq!(style.background, RgbaColor { r: 10, g: 10, b: 10, a: 255 });
    }

    #[test]
    fn parse_record_backlight() {
        let style = loader().parse_record(&full_fixture()).unwrap();
        assert_eq!(style.backlight, RgbaColor { r: 102, g: 214, b: 255, a: 255 });
    }

    #[test]
    fn parse_record_font_families() {
        let style = loader().parse_record(&full_fixture()).unwrap();
        assert_eq!(style.font_family_hints, vec!["Rajdhani", "Orbitron"]);
    }

    #[test]
    fn parse_record_crt_params() {
        let style = loader().parse_record(&full_fixture()).unwrap();
        assert!((style.crt.scanline_period_px - 4.0).abs() < 1e-5);
        assert!((style.crt.pixel_grid_period_px - 3.5).abs() < 1e-5);
        assert!((style.crt.scanline_intensity - 0.2).abs() < 1e-5);
        assert!((style.crt.vignette_strength - 0.4).abs() < 1e-5);
    }

    #[test]
    fn parse_record_optional_fields_absent() {
        // secondaryColor and fontFamilies omitted → sensible defaults.
        let minimal = serde_json::json!({
            "primaryColor":    { "r": 255, "g": 176, "b": 76, "a": 255 },
            "backgroundColor": { "r": 0,   "g": 0,   "b": 0,  "a": 255 },
            "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 }
        });
        let style = loader().parse_record(&minimal).unwrap();
        assert_eq!(style.secondary_tint, None);
        assert!(style.font_family_hints.is_empty());
    }

    // ── parse_record — negative cases ───────────────────────────────────────

    #[test]
    fn parse_record_missing_primary_color_returns_error() {
        let bad = serde_json::json!({
            "backgroundColor": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 }
        });
        assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
    }

    #[test]
    fn parse_record_missing_background_color_returns_error() {
        let bad = serde_json::json!({
            "primaryColor":   { "r": 255, "g": 176, "b": 76, "a": 255 },
            "backlightColor": { "r": 102, "g": 214, "b": 255, "a": 255 }
        });
        assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
    }

    #[test]
    fn parse_record_missing_backlight_color_returns_error() {
        let bad = serde_json::json!({
            "primaryColor":    { "r": 255, "g": 176, "b": 76, "a": 255 },
            "backgroundColor": { "r": 0,   "g": 0,   "b": 0,  "a": 255 }
        });
        assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
    }

    #[test]
    fn parse_record_color_missing_r_channel_returns_error() {
        let bad = serde_json::json!({
            "primaryColor":    { "g": 176, "b": 76 },
            "backgroundColor": { "r": 0,   "g": 0, "b": 0, "a": 255 },
            "backlightColor":  { "r": 102, "g": 214, "b": 255, "a": 255 }
        });
        assert!(matches!(loader().parse_record(&bad), Err(UiError::ParseError(_))));
    }

    // ── snapshot / print test (ignored by default) ───────────────────────────

    /// Visual inspection snapshot: prints the Drake style and a small screen-
    /// state registry for manual review.  Run with:
    ///   cargo test -p starbreaker-ui -- --ignored snapshot_drake_style_print
    #[test]
    #[ignore]
    fn snapshot_drake_style_print() {
        use crate::defaults::DefaultValueRegistry;

        let loader = StyleLoader::for_manufacturer("drak");
        let style = loader.drake_amber_fallback();

        println!("=== Drake Manufacturer Style (fallback) ===");
        println!("name:          {}", style.name);
        println!(
            "primary_tint:  #{:02X}{:02X}{:02X} (a={})",
            style.primary_tint.r,
            style.primary_tint.g,
            style.primary_tint.b,
            style.primary_tint.a
        );
        println!(
            "backlight:     #{:02X}{:02X}{:02X} (a={})",
            style.backlight.r, style.backlight.g, style.backlight.b, style.backlight.a
        );
        println!(
            "background:    #{:02X}{:02X}{:02X} (a={})",
            style.background.r,
            style.background.g,
            style.background.b,
            style.background.a
        );
        println!("font_hints:    {:?}", style.font_family_hints);
        println!(
            "crt:           scanline_period={}px  grid_period={}px  \
             scanline_intensity={}  vignette={}",
            style.crt.scanline_period_px,
            style.crt.pixel_grid_period_px,
            style.crt.scanline_intensity,
            style.crt.vignette_strength
        );

        // Simulate ingesting the Clipper screenStates Normal entry.
        let screen_states = serde_json::json!([{
            "name": "Normal",
            "lightOn": true,
            "color": { "r": 102, "g": 214, "b": 255, "a": 255 },
            "intensity": 0.025
        }]);
        let mut reg = DefaultValueRegistry::with_well_known_path_defaults();
        reg.ingest_screen_states(&screen_states);

        println!("\n=== DefaultValueRegistry (well-known paths + Clipper screenStates) ===");
        println!("path_count:   {}", reg.path_count());
        for path in [
            "/vehicle/targetname",
            "/vehicle/target/distance",
            "/vehicle/target/bearing",
            "/ship/hp/current",
            "/ship/hp/max",
            "/seatdashboard/powerstate",
            "/seatdashboard/powercurrent",
            "/seatdashboard/powermax",
            "/vehicle/gungroup",
        ] {
            println!("  {path:<35} → {:?}", reg.lookup_path(path));
        }
        println!(
            "  Normal backlight               → {:?}",
            reg.screen_state_color("Normal")
        );
    }
}
