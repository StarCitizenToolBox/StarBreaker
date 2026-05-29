//! SWF shape rasterizer API.

use image::RgbaImage;
use tiny_skia::{Color, Pixmap, Rect as TskRect};

use crate::swf_assets::SwfAssetLibrary;

mod rgba;
mod shape;
mod stage;
#[cfg(test)]
mod tests;

use rgba::composite_pixmap_over_rgba;
pub use stage::{draw_swf_stage, draw_swf_symbol, draw_swf_visual_exports};

/// Render the SWF main-timeline stage (frame 0) as alpha-over composite into `img`.
pub fn draw_swf_stage_rgba(
    img: &mut RgbaImage,
    assets: &SwfAssetLibrary,
    tint: Color,
    alpha: f32,
) -> bool {
    let w = img.width();
    let h = img.height();
    let Some(mut pixmap) = Pixmap::new(w, h) else {
        return false;
    };
    let Some(dest) = TskRect::from_xywh(0.0, 0.0, w as f32, h as f32) else {
        return false;
    };
    if !stage::draw_swf_stage(&mut pixmap, assets, dest, tint, alpha) {
        return false;
    }
    composite_pixmap_over_rgba(&pixmap, img);
    true
}

/// Render all visual exports from a Flash SWF as alpha-over composite into `img`.
pub fn draw_swf_visual_exports_rgba(
    img: &mut RgbaImage,
    assets: &SwfAssetLibrary,
    tint: Color,
    alpha: f32,
) -> bool {
    let w = img.width();
    let h = img.height();
    let Some(mut pixmap) = Pixmap::new(w, h) else {
        return false;
    };
    let Some(dest) = TskRect::from_xywh(0.0, 0.0, w as f32, h as f32) else {
        return false;
    };
    if !stage::draw_swf_visual_exports(&mut pixmap, assets, dest, tint, alpha) {
        return false;
    }
    composite_pixmap_over_rgba(&pixmap, img);
    true
}
