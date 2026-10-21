//! SWF shape rasterizer API.
//!
//! Renders SWF display lists (shapes, sprites, EditText) via `tiny-skia`.
//! All public entry points produce either a `Pixmap` (stage paths) or
//! composite into an existing `RgbaImage` (the `_rgba` variants).
//!
//! Phase 4: the `edit_text` submodule exposes the Flash HTML parser and
//! `draw_edit_text` rendering path for `DefineEditText` characters.
//!
//! Phase 5: `state_select` implements data-driven sample-data export detection;
//! `draw_swf_stage_with_state` and `draw_swf_stage_rgba_in_rect` add suppression
//! + loc resolution to the stage render path.

use std::collections::HashSet;

use image::RgbaImage;
use swf::CharacterId;
use tiny_skia::{Color, Pixmap, Rect as TskRect};

use crate::swf_assets::SwfAssetLibrary;

pub mod edit_text;
pub mod state_select;
mod rgba;
mod shape;
mod stage;
#[cfg(test)]
mod tests;

use rgba::composite_pixmap_over_rgba;
pub use edit_text::{FlashTextRun, parse_swf_html};
pub use stage::{
    draw_swf_at_frame_label, draw_swf_stage, draw_swf_stage_with_state, draw_swf_symbol,
    draw_swf_symbol_excluding, draw_swf_visual_exports,
};

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

/// Render the SWF main-timeline stage (frame 0) with state suppression and loc resolution
/// as alpha-over composite into `img`.  The stage is scaled to fill `dest`.
pub fn draw_swf_stage_rgba_in_rect(
    img: &mut RgbaImage,
    assets: &SwfAssetLibrary,
    dest: TskRect,
    tint: Color,
    alpha: f32,
    suppressed: &HashSet<CharacterId>,
    loc_fn: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let iw = img.width();
    let ih = img.height();
    let Some(mut pixmap) = Pixmap::new(iw, ih) else {
        return false;
    };
    if !stage::draw_swf_stage_with_state(&mut pixmap, assets, dest, tint, alpha, suppressed, loc_fn) {
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
