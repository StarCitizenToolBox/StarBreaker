//! Manufacturer post-process pass.
//!
//! Applies a sequence of in-place image passes to an [`image::RgbaImage`] that
//! was produced by [`crate::compose::render_canvas`], giving the final rendered
//! screen texture the manufacturer's CRT look.

use image::RgbaImage;

use crate::style::ManufacturerStyle;

mod passes;
#[cfg(test)]
mod tests;

/// Options controlling which post-process passes are active.
///
/// All passes except [`apply_glow`][`PostProcessOptions::apply_glow`] default
/// to `true`. Glow defaults to `false` because the pass is currently a stub.
#[derive(Debug, Clone)]
pub struct PostProcessOptions {
    /// Apply the manufacturer primary-tint multiplication to lit pixels.
    pub apply_tint: bool,
    /// Apply horizontal scanline darkening.
    pub apply_scanlines: bool,
    /// Apply vertical pixel-grid darkening.
    pub apply_pixel_grid: bool,
    /// Apply radial corner vignette.
    pub apply_vignette: bool,
    /// Apply soft phosphor glow (currently a stub).
    pub apply_glow: bool,
}

impl Default for PostProcessOptions {
    fn default() -> Self {
        Self {
            apply_tint: true,
            apply_scanlines: true,
            apply_pixel_grid: true,
            apply_vignette: true,
            apply_glow: false,
        }
    }
}

/// In-place post-processor that applies CRT and tint passes to a rendered UI image.
pub struct PostProcessor<'a> {
    pub style: &'a ManufacturerStyle,
}

impl<'a> PostProcessor<'a> {
    /// Create a new post-processor bound to the given manufacturer style.
    pub fn new(style: &'a ManufacturerStyle) -> Self {
        Self { style }
    }

    /// Run all enabled passes in fixed order: tint -> scanlines -> pixel-grid
    /// -> vignette -> glow.
    pub fn run(&self, img: &mut RgbaImage, opts: &PostProcessOptions) {
        if opts.apply_tint {
            passes::pass_tint(img, self.style);
        }
        if opts.apply_scanlines {
            passes::pass_scanlines(img, self.style);
        }
        if opts.apply_pixel_grid {
            passes::pass_pixel_grid(img, self.style);
        }
        if opts.apply_vignette {
            passes::pass_vignette(img, self.style);
        }
        if opts.apply_glow {
            passes::pass_glow_stub(img, self.style);
        }
    }
}
