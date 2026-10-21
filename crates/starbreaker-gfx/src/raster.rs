//! Display-list rasterizer that renders actual GFX content instead of procedural placeholders.

use std::collections::HashMap;

use image::{Rgba, RgbaImage};

use crate::error::{GfxError, GfxResult};
use crate::types::{ColorTransform, Matrix, PlaceObject};

/// Context for rendering a GFX display-list to an image.
pub struct RasterContext {
    /// Bitmap cache keyed by character ID.
    bitmaps: HashMap<u16, RgbaImage>,
}

impl RasterContext {
    /// Create a new raster context.
    pub fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
        }
    }

    /// Register a bitmap by character ID.
    pub fn add_bitmap(&mut self, character_id: u16, bitmap: RgbaImage) {
        self.bitmaps.insert(character_id, bitmap);
    }

    /// Render a display-list to an output image.
    pub fn render(&self, width: u32, height: u32, placements: &[PlaceObject]) -> GfxResult<RgbaImage> {
        if width == 0 || height == 0 {
            return Err(GfxError::malformed("output dimensions must be non-zero"));
        }

        let mut output = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

        for placement in placements {
            if let Some(character_id) = placement.character_id {
                if let Some(bitmap) = self.bitmaps.get(&character_id) {
                    self.composite_bitmap(
                        &mut output,
                        bitmap,
                        placement.matrix.as_ref(),
                        placement.color_transform.as_ref(),
                    )?;
                }
            }
        }

        Ok(output)
    }

    /// Composite a bitmap onto the output with optional transforms.
    fn composite_bitmap(
        &self,
        output: &mut RgbaImage,
        bitmap: &RgbaImage,
        matrix: Option<&Matrix>,
        color_transform: Option<&ColorTransform>,
    ) -> GfxResult<()> {
        // For now, simple case: no transforms, just alpha blend
        if matrix.is_none() && color_transform.is_none() {
            // Simple copy with alpha blending
            for (x, y, pixel) in bitmap.enumerate_pixels() {
                if x < output.width() && y < output.height() {
                    let dst = output.get_pixel_mut(x, y);
                    let src_alpha = f32::from(pixel[3]) / 255.0;
                    for channel in 0..3 {
                        dst[channel] = ((f32::from(dst[channel]) * (1.0 - src_alpha))
                            + (f32::from(pixel[channel]) * src_alpha))
                            as u8;
                    }
                    dst[3] = 255;
                }
            }
        } else {
            // TODO: Handle transforms and color transforms
            // For now, fall back to simple copy
            for (x, y, pixel) in bitmap.enumerate_pixels() {
                if x < output.width() && y < output.height() {
                    let dst = output.get_pixel_mut(x, y);
                    let mut src_pixel = *pixel;

                    // Apply color transform if present
                    if let Some(ct) = color_transform {
                        src_pixel[0] = apply_color_transform(src_pixel[0], ct.multiply_r, ct.add_r);
                        src_pixel[1] = apply_color_transform(src_pixel[1], ct.multiply_g, ct.add_g);
                        src_pixel[2] = apply_color_transform(src_pixel[2], ct.multiply_b, ct.add_b);
                        src_pixel[3] = apply_color_transform(src_pixel[3], ct.multiply_a, ct.add_a);
                    }

                    let src_alpha = f32::from(src_pixel[3]) / 255.0;
                    for channel in 0..3 {
                        dst[channel] = ((f32::from(dst[channel]) * (1.0 - src_alpha))
                            + (f32::from(src_pixel[channel]) * src_alpha))
                            as u8;
                    }
                    dst[3] = 255;
                }
            }
        }

        Ok(())
    }
}

/// Apply color transform to a single channel.
fn apply_color_transform(channel: u8, multiply: u8, add: i16) -> u8 {
    let multiplied = u32::from(channel) * u32::from(multiply) / 255;
    let transformed = (multiplied as i32) + i32::from(add);
    transformed.clamp(0, 255) as u8
}

impl Default for RasterContext {
    fn default() -> Self {
        Self::new()
    }
}
