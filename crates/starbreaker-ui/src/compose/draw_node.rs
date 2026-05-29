//! Per-node drawing dispatch.

use std::collections::BTreeMap;

use tiny_skia::{Pixmap, Rect as TskRect};

use crate::bb_atlas::AtlasLibrary;
use crate::bb_bindings::BindingResolver;
use crate::bb_layout::Rect;
use crate::bb_scene::{BbNode, BbNodeId, BbNodeType, BbScene};
use crate::swf_render;

use super::{
    ComposeContext, blit_atlas_image, blit_atlas_image_tinted, draw_border_ts, draw_manufacturer_logo,
    draw_raw_asset, fill_rect_ts, node_fill_rgba,
};

pub(crate) fn draw_node(
    node: &BbNode,
    rect: Rect,
    resolver: &BindingResolver,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
    _scene: &BbScene,
    _layout_rects: &BTreeMap<BbNodeId, Rect>,
) {
    let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
        return;
    };
    let alpha = node.alpha.clamp(0.0, 1.0);
    let iw = rect.w.round().max(1.0) as u32;
    let ih = rect.h.round().max(1.0) as u32;

    match &node.ty {
        BbNodeType::DisplayWidget => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if !draw_raw_asset(node, rect, resolver, atlas, pixmap, alpha)
                && let Some(img) = atlas.resolve_for_node(node, iw, ih)
            {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }
        BbNodeType::WidgetBodyBackground => {
            let body_rect = Rect {
                x: 0.0,
                y: 0.0,
                w: pixmap.width() as f32,
                h: pixmap.height() as f32,
            };
            let Some(body_tsk_rect) =
                TskRect::from_xywh(body_rect.x, body_rect.y, body_rect.w, body_rect.h)
            else {
                return;
            };
            let background_type = node
                .raw
                .get("backgroundType")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if background_type.eq_ignore_ascii_case("Texture")
                && draw_raw_asset(node, body_rect, resolver, atlas, pixmap, alpha)
            {
                return;
            }

            let bl = &ctx.style.backlight;
            let fill = [
                bl.r as f32 / 255.0,
                bl.g as f32 / 255.0,
                bl.b as f32 / 255.0,
                0.50,
            ];
            fill_rect_ts(pixmap, body_tsk_rect, fill, alpha);
        }
        BbNodeType::WidgetCanvas => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }
        BbNodeType::WidgetCard
        | BbNodeType::ComponentGeneralButton
        | BbNodeType::ComponentGeneralButtonSecondary => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if !draw_raw_asset(node, rect, resolver, atlas, pixmap, alpha)
                && let Some(img) = atlas.resolve_for_node(node, iw, ih)
            {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }
        BbNodeType::WidgetIcon | BbNodeType::WidgetImage => {
            if draw_raw_asset(node, rect, resolver, atlas, pixmap, alpha) {
            } else if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                let tint = node
                    .icon
                    .as_ref()
                    .and_then(|i| i.tint_colour)
                    .unwrap_or([1.0, 1.0, 1.0, 1.0]);
                blit_atlas_image_tinted(pixmap, &img, rect.x as i32, rect.y as i32, tint, alpha);
            }
        }
        BbNodeType::Other(kind) if kind == "BuildingBlocks_WidgetManufacturerLogo" => {
            if !draw_raw_asset(node, rect, resolver, atlas, pixmap, alpha)
                && !draw_manufacturer_logo(node, rect, atlas, pixmap, alpha, ctx)
                && let Some(img) = atlas.resolve_for_node(node, iw, ih)
            {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
        }
        BbNodeType::WidgetTextField | BbNodeType::WidgetText => {}
        BbNodeType::WidgetCustomShape => {
            let drew_raw = draw_raw_asset(node, rect, resolver, atlas, pixmap, alpha);
            if drew_raw {
                if let Some(border) = &node.border {
                    draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
                }
            } else if let Some(img) = atlas.resolve_for_node(node, iw, ih) {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
                if let Some(border) = &node.border {
                    draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
                }
            } else {
                let pt = &ctx.style.primary_tint;
                let tint = tiny_skia::Color::from_rgba8(pt.r, pt.g, pt.b, pt.a);
                let drew_swf = swf_render::draw_swf_symbol(
                    pixmap,
                    ctx.assets,
                    &node.name,
                    tsk_rect,
                    tint,
                    alpha,
                );

                if !drew_swf {
                    let bg_fill = node_fill_rgba(node);
                    if bg_fill[3] > 0.005 {
                        fill_rect_ts(pixmap, tsk_rect, bg_fill, alpha);
                    }
                    if let Some(border) = &node.border {
                        let max_w = border
                            .top
                            .width
                            .max(border.right.width)
                            .max(border.bottom.width)
                            .max(border.left.width);
                        if max_w > 0.5 {
                            draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
                        }
                    }
                }
            }
        }
        BbNodeType::Other(_) => {
            let fill = node_fill_rgba(node);
            if fill[3] > 0.005 {
                fill_rect_ts(pixmap, tsk_rect, fill, alpha);
            }
            if !draw_raw_asset(node, rect, resolver, atlas, pixmap, alpha)
                && let Some(img) = atlas.resolve_for_node(node, iw, ih)
            {
                blit_atlas_image(pixmap, &img, rect.x as i32, rect.y as i32, alpha);
            }
            if let Some(border) = &node.border {
                draw_border_ts(pixmap, tsk_rect, border, ctx, alpha, &node.raw);
            }
        }
    }
}
