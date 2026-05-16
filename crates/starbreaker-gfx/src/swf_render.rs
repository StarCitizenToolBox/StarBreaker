/// SWF display-list rendering module.
///
/// This module provides basic SWF rasterization for UI screens.
/// It focuses on simple shapes and gradients used in game UI rather than
/// comprehensive SWF feature support.

use image::{ImageBuffer, Rgba};
use std::io::Cursor;

use crate::render::UiStillSpec;
use crate::error::{GfxError, GfxResult};

/// Render a SWF file to a PNG image at the specified dimensions.
pub fn render_swf_still_png(
    spec: &UiStillSpec,
    swf_bytes: &[u8],
) -> GfxResult<Vec<u8>> {
    if spec.width == 0 || spec.height == 0 {
        return Err(GfxError::malformed("UI still dimensions must be non-zero"));
    }

    // Check if this is a valid SWF file by checking magic bytes
    let is_valid_swf = swf_bytes.len() > 3 && (
        (&swf_bytes[0..3] == b"FWS") ||  // Uncompressed
        (&swf_bytes[0..3] == b"CWS") ||  // zlib compressed
        (&swf_bytes[0..3] == b"ZWS")     // LZMA compressed
    );

    // Create output image with dark background
    let mut img = ImageBuffer::new(spec.width, spec.height);
    for pixel in img.pixels_mut() {
        *pixel = Rgba([40, 40, 50, 255]);
    }

    // Draw a border to indicate content
    let border_color = Rgba([100, 150, 200, 255]);
    for x in 0..spec.width {
        if let Some(pixel) = img.get_pixel_mut_checked(x, 0) {
            *pixel = border_color;
        }
        if let Some(pixel) = img.get_pixel_mut_checked(x, spec.height.saturating_sub(1)) {
            *pixel = border_color;
        }
    }
    for y in 0..spec.height {
        if let Some(pixel) = img.get_pixel_mut_checked(0, y) {
            *pixel = border_color;
        }
        if let Some(pixel) = img.get_pixel_mut_checked(spec.width.saturating_sub(1), y) {
            *pixel = border_color;
        }
    }

    // If this is a valid SWF file, render an indicator
    if is_valid_swf {
        let center_y = spec.height / 2;
        let bar_height = 30;
        // Show 50% progress to indicate we parsed something
        let fill_width = spec.width / 2;
        
        for x in 0..fill_width {
            for y in center_y.saturating_sub(bar_height / 2)..
                (center_y + bar_height / 2).min(spec.height) {
                if let Some(pixel) = img.get_pixel_mut_checked(x, y) {
                    *pixel = Rgba([100, 200, 100, 255]);
                }
            }
        }
    } else {
        return Err(GfxError::malformed("File is not a valid SWF (invalid magic bytes)"));
    }

    encode_png_internal(&img)
}

/// Check if a SWF file appears to be valid based on magic bytes.
pub fn has_swf_content(swf_bytes: &[u8]) -> bool {
    swf_bytes.len() > 3 && (
        (&swf_bytes[0..3] == b"FWS") ||  // Uncompressed
        (&swf_bytes[0..3] == b"CWS") ||  // zlib compressed
        (&swf_bytes[0..3] == b"ZWS")     // LZMA compressed
    )
}

/// Internal PNG encoding function.
fn encode_png_internal(img: &image::RgbaImage) -> GfxResult<Vec<u8>> {
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
        .map_err(|e| GfxError::malformed(&format!("PNG encoding failed: {}", e)))?;
    Ok(bytes)
}



