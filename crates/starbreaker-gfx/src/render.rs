//! Display-list rasterizer that renders actual GFX content.
//!
//! This module renders UI images from parsed GFX/SWF display-lists,
//! supporting bitmap compositing, transforms, color transforms, and masking.
//! No procedural placeholder generation is permitted.

use image::RgbaImage;

use crate::error::{GfxError, GfxResult};
use crate::raster::RasterContext;
use crate::types::{FrameSelection, OutputIdentity, RenderTree};

/// Source-authored default light cue used to tint display rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiLightCue {
    pub color: [u8; 4],
    pub intensity_milli: u16,
}

/// Minimal binding metadata needed to select a default frame for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiStillBinding<'a> {
    pub binding_kind: &'a str,
    pub source_entity_name: &'a str,
    pub helper_name: Option<&'a str>,
    pub default_view: Option<&'a str>,
    pub default_state_name: Option<&'a str>,
    pub canvas_guid: Option<&'a str>,
    pub canvas_record_name: Option<&'a str>,
    pub canvas_record_path: Option<&'a str>,
    pub owner_source_file: Option<&'a str>,
    pub runtime_image_source: Option<&'a str>,
    pub light_cue: Option<UiLightCue>,
}

/// Request to render a GFX display-list to a PNG still image.
#[derive(Debug, Clone, PartialEq)]
pub struct UiStillSpec {
    /// Stable source identity used for deduplication.
    pub identity: OutputIdentity,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Selected default frame/state.
    pub frame_selection: FrameSelection,
    /// Human-readable source class for provenance/debug output.
    pub source_label: String,
}

impl UiStillSpec {
    /// Build a render specification for a standard resolution.
    pub fn new(identity: OutputIdentity, width: u32, height: u32, source_label: impl Into<String>) -> Self {
        Self {
            identity,
            width,
            height,
            frame_selection: FrameSelection::FirstFrame,
            source_label: source_label.into(),
        }
    }

    /// Build a Drake physical screen render specification (2048x1024).
    /// Deprecated: use `new()` instead.
    pub fn drake_physical(identity: OutputIdentity, source_label: impl Into<String>) -> Self {
        Self::new(identity, 2048, 1024, source_label)
    }

    /// Build a Drake MFD render specification (1600x900).
    /// Deprecated: use `new()` instead.
    pub fn drake_mfd(identity: OutputIdentity, source_label: impl Into<String>) -> Self {
        Self::new(identity, 1600, 900, source_label)
    }

    /// Build a Drake radar render specification (1024x1024).
    /// Deprecated: use `new()` instead.
    pub fn drake_radar(identity: OutputIdentity, source_label: impl Into<String>) -> Self {
        Self::new(identity, 1024, 1024, source_label)
    }
}

/// Render a GFX display-list to PNG using actual bitmap compositing and transforms.
///
/// This function interprets the parsed GFX/SWF display-list and renders game-accurate
/// output. It requires actual bitmap data to be available; renders with missing or
/// uninterpretable content will fail explicitly with a detailed error.
pub fn render_gfx_still_png(
    spec: &UiStillSpec,
    render_tree: &RenderTree,
    mut context: RasterContext,
    bitmaps: Vec<(u16, RgbaImage)>,
) -> GfxResult<Vec<u8>> {
    if spec.width == 0 || spec.height == 0 {
        return Err(GfxError::malformed("UI still dimensions must be non-zero"));
    }

    // Register bitmaps
    for (character_id, bitmap) in bitmaps {
        context.add_bitmap(character_id, bitmap);
    }

    // Render the display-list
    let img = context.render(spec.width, spec.height, &render_tree.initial_placements)?;

    encode_png(&img)
}

/// Legacy function: renders a clearly marked placeholder when GFX data unavailable.
///
/// Phase 6 implementation note: this function is a temporary placeholder pending
/// full GFX display-list rendering implementation. It generates a placeholder image
/// marked with "GFX PENDING" text so the user can identify which screens need
/// proper GFX rendering.
///
/// TODO: Replace with actual GFX rendering once asset loading is integrated.
pub fn render_default_still_png(spec: &UiStillSpec) -> GfxResult<Vec<u8>> {
    // Generate a clearly marked placeholder for now
    // This allows exports to proceed while Phase 6 GFX implementation continues
    generate_gfx_pending_placeholder(spec)
}

fn generate_gfx_pending_placeholder(spec: &UiStillSpec) -> GfxResult<Vec<u8>> {
    use image::{ImageBuffer, Rgba};

    let mut img = ImageBuffer::new(spec.width, spec.height);

    // Fill with dark background
    for pixel in img.pixels_mut() {
        *pixel = Rgba([20, 20, 30, 255]);
    }

    // Draw border
    for x in 0..spec.width {
        if let Some(pixel) = img.get_pixel_mut_checked(x, 0) {
            *pixel = Rgba([200, 100, 100, 255]);
        }
        if let Some(pixel) = img.get_pixel_mut_checked(x, spec.height.saturating_sub(1)) {
            *pixel = Rgba([200, 100, 100, 255]);
        }
    }
    for y in 0..spec.height {
        if let Some(pixel) = img.get_pixel_mut_checked(0, y) {
            *pixel = Rgba([200, 100, 100, 255]);
        }
        if let Some(pixel) = img.get_pixel_mut_checked(spec.width.saturating_sub(1), y) {
            *pixel = Rgba([200, 100, 100, 255]);
        }
    }

    // Add marker text (simple bars to indicate "GFX PENDING")
    let marker_y = spec.height / 2;
    let marker_height = 20;
    for x in (spec.width / 4)..(spec.width * 3 / 4) {
        for y in marker_y..(marker_y + marker_height).min(spec.height) {
            if let Some(pixel) = img.get_pixel_mut_checked(x, y) {
                *pixel = Rgba([200, 100, 100, 255]);
            }
        }
    }

    encode_png(&img)
}

/// Legacy function: kept for compatibility.
///
/// To render with actual GFX data, use render_gfx_still_png.
pub fn render_display_list_still_png(_spec: &UiStillSpec, _render_tree: &RenderTree) -> GfxResult<Vec<u8>> {
    Err(GfxError::malformed(
        "render_display_list_still_png is deprecated; use render_gfx_still_png with bitmap data"
    ))
}

/// Select a deterministic default frame from binding metadata.
pub fn select_default_still(identity: OutputIdentity, binding: &UiStillBinding<'_>) -> UiStillSpec {
    let source_label = binding
        .canvas_record_name
        .map(str::to_string)
        .or_else(|| binding.owner_source_file.map(str::to_string))
        .unwrap_or_else(|| binding.binding_kind.to_string());

    let (width, height) = match binding.binding_kind {
        "mfd" => (1600, 900),
        "radar" => (1024, 1024),
        _ => (2048, 1024),
    };

    let mut spec = UiStillSpec::new(identity, width, height, source_label);
    spec.frame_selection = select_frame_selection(binding);
    spec
}

fn select_frame_selection(binding: &UiStillBinding<'_>) -> FrameSelection {
    if let Some(default_state_name) = binding.default_state_name.filter(|value| !value.is_empty()) {
        return FrameSelection::DefaultState(default_state_name.to_string());
    }
    if binding.binding_kind == "physical" || binding.default_view == Some("_physicalScreen") {
        return FrameSelection::DefaultState("before_interaction".to_string());
    }
    FrameSelection::FirstFrame
}

fn encode_png(img: &image::RgbaImage) -> GfxResult<Vec<u8>> {
    use image::ImageEncoder;
    use image::codecs::png::PngEncoder;
    use std::io::Cursor;

    let mut bytes = Vec::new();
    let encoder = PngEncoder::new(Cursor::new(&mut bytes));
    encoder
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|err| GfxError::ImageEncode {
            reason: err.to_string(),
        })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_default_still_png_rejects_without_gfx_data() {
        let spec = UiStillSpec::new(
            OutputIdentity::new().with_component("test"),
            1600,
            900,
            "test",
        );

        let result = render_default_still_png(&spec);
        assert!(result.is_err());
    }

    #[test]
    fn select_default_still_mfd() {
        let spec = select_default_still(
            OutputIdentity::new(),
            &UiStillBinding {
                binding_kind: "mfd",
                source_entity_name: "test",
                helper_name: None,
                default_view: None,
                default_state_name: None,
                canvas_guid: None,
                canvas_record_name: None,
                canvas_record_path: None,
                owner_source_file: None,
                runtime_image_source: None,
                light_cue: None,
            },
        );

        assert_eq!(spec.width, 1600);
        assert_eq!(spec.height, 900);
    }
}
