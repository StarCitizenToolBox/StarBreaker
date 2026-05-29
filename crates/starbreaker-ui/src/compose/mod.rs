//! Canvas compositor entry points.

use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::bb_atlas::AtlasLibrary;
use crate::bb_bindings::BindingResolver;
use crate::bb_layout;
use crate::bb_scene::{BbNode, BbNodeType, BbScene};
use crate::canvas::ResolvedCanvas;
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::postprocess::PostProcessOptions;
use crate::style::ManufacturerStyle;
use crate::swf_assets::SwfAssetLibrary;
use crate::text::TextRenderer;

mod blit;
mod draw_node;
mod draw_primitives;
mod raw_assets;
#[cfg(test)]
mod tests;
mod text_draw;

use blit::{blit_atlas_image, blit_atlas_image_alpha_mask_tinted, blit_atlas_image_tinted};
use draw_primitives::{draw_border_ts, fill_rect_ts};
use raw_assets::{draw_manufacturer_logo, draw_raw_asset};
use text_draw::draw_text_node;
use blit::{magenta_placeholder, pixmap_to_rgba_image};
use draw_node::draw_node;

/// Shared references needed by the compositor.
pub struct ComposeContext<'a> {
    pub style: &'a ManufacturerStyle,
    pub defaults: &'a DefaultValueRegistry,
    pub assets: &'a SwfAssetLibrary,
}

/// Output canvas dimensions in pixels.
pub struct ComposeTarget {
    pub width: u32,
    pub height: u32,
}

/// Intermediate render state produced by pass1.
pub struct BbRenderState {
    pub img: RgbaImage,
    pub(crate) layout: bb_layout::LayoutResult,
    pub(crate) resolver: BindingResolver,
    pub(crate) renderer: TextRenderer,
}

/// Rasterise a merged [`BbScene`] to RGBA.
pub fn render_bb_scene(
    scene: &BbScene,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    target: ComposeTarget,
) -> Result<RgbaImage, UiError> {
    let mut state = render_bb_scene_pass1(scene, ctx, atlas, target)?;
    render_bb_scene_pass2(&mut state, scene, ctx);
    Ok(state.img)
}

/// Pass 1: fill background, draw non-text nodes.
pub fn render_bb_scene_pass1(
    scene: &BbScene,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    target: ComposeTarget,
) -> Result<BbRenderState, UiError> {
    if target.width == 0 || target.height == 0 {
        return Err(UiError::RenderError(format!(
            "invalid target size {}x{}",
            target.width, target.height
        )));
    }

    let layout = bb_layout::layout(scene, target.width, target.height);
    let text_renderer = TextRenderer::new();
    let resolver = BindingResolver::from_operations(&scene.operations);

    let mut pixmap = Pixmap::new(target.width, target.height)
        .ok_or_else(|| UiError::RenderError("pixmap allocation failed".into()))?;

    let bg = &ctx.style.background;
    pixmap.fill(tiny_skia::Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    for &node_id in &layout.draw_order {
        let Some(node) = scene.nodes.get(&node_id) else {
            continue;
        };
        let Some(&rect) = layout.rects.get(&node_id) else {
            continue;
        };
        if rect.w < 0.5 || rect.h < 0.5 {
            continue;
        }
        draw_node(node, rect, &resolver, ctx, atlas, &mut pixmap, scene, &layout.rects);
    }

    let img = pixmap_to_rgba_image(pixmap)?;
    Ok(BbRenderState {
        img,
        layout,
        resolver,
        renderer: text_renderer,
    })
}

/// Pass 2: draw text nodes atop pass1 output.
pub fn render_bb_scene_pass2(state: &mut BbRenderState, scene: &BbScene, ctx: &ComposeContext<'_>) {
    let mut seen_text_rects: std::collections::HashSet<(i32, i32, i32)> =
        std::collections::HashSet::new();
    for &node_id in &state.layout.draw_order {
        let Some(node) = scene.nodes.get(&node_id) else {
            continue;
        };
        if !matches!(node.ty, BbNodeType::WidgetTextField | BbNodeType::WidgetText) {
            continue;
        }
        let Some(&rect) = state.layout.rects.get(&node_id) else {
            continue;
        };
        if rect.w < 0.5 || rect.h < 0.5 {
            continue;
        }
        let key = (
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.w.round() as i32,
        );
        if !seen_text_rects.insert(key) {
            continue;
        }
        draw_text_node(
            &mut state.img,
            node,
            rect,
            &state.renderer,
            &state.resolver,
            state.layout.canvas_scale,
            ctx,
        );
    }
}

/// Fallback rasterise path when BbScene resolution fails.
pub fn render_canvas(
    _canvas: &ResolvedCanvas,
    _ctx: &ComposeContext<'_>,
    target: ComposeTarget,
) -> Result<RgbaImage, UiError> {
    if target.width == 0 || target.height == 0 {
        return Err(UiError::RenderError(format!(
            "invalid target size {}x{}",
            target.width, target.height
        )));
    }
    magenta_placeholder(target.width, target.height)
}

/// Encode an [`RgbaImage`] to PNG bytes.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, UiError> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| UiError::RenderError(format!("PNG encode failed: {e}")))?;
    Ok(buf)
}

/// Rasterise + post-process wrapper.
pub fn render_canvas_with_postprocess(
    canvas: &ResolvedCanvas,
    ctx: &ComposeContext<'_>,
    target: ComposeTarget,
    _opts: &PostProcessOptions,
) -> Result<RgbaImage, UiError> {
    render_canvas(canvas, ctx, target)
}

/// Return explicit fill colour for a node, or transparent.
pub(crate) fn node_fill_rgba(node: &BbNode) -> [f32; 4] {
    node.background
        .as_ref()
        .and_then(|bg| bg.fill_colour)
        .unwrap_or([0.0, 0.0, 0.0, 0.0])
}

/// Build a tiny-skia `Color` from [f32; 4] RGBA and global alpha.
pub(crate) fn to_skia_color(rgba: [f32; 4], global_alpha: f32) -> tiny_skia::Color {
    let a = (rgba[3] * global_alpha).clamp(0.0, 1.0);
    tiny_skia::Color::from_rgba8(
        (rgba[0].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0) as u8,
        (a * 255.0) as u8,
    )
}
