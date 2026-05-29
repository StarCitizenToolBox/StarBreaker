//! Tiny-skia shape and border drawing helpers.

use tiny_skia::{Paint, PathBuilder, Pixmap, Rect as TskRect, Stroke, Transform};

use crate::bb_scene::BbBorder;

use super::{ComposeContext, to_skia_color};

pub(crate) fn fill_rect_ts(pixmap: &mut Pixmap, rect: TskRect, rgba: [f32; 4], alpha: f32) {
    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
    paint.anti_alias = false;
    pixmap
        .as_mut()
        .fill_rect(rect, &paint, Transform::identity(), None);
}

pub(crate) fn draw_border_ts(
    pixmap: &mut Pixmap,
    rect: TskRect,
    border: &BbBorder,
    ctx: &ComposeContext<'_>,
    alpha: f32,
    raw: &serde_json::Value,
) {
    let max_width = border
        .top
        .width
        .max(border.right.width)
        .max(border.bottom.width)
        .max(border.left.width);
    if max_width < 0.01 {
        return;
    }

    let radius = raw
        .get("border")
        .and_then(|b| b.get("radius"))
        .and_then(|r| r.as_f64())
        .unwrap_or(0.0) as f32;

    let pt = &ctx.style.primary_tint;
    let fallback = [
        pt.r as f32 / 255.0,
        pt.g as f32 / 255.0,
        pt.b as f32 / 255.0,
        0.4,
    ];
    let border_rgba = border
        .top
        .colour
        .or(border.right.colour)
        .or(border.bottom.colour)
        .or(border.left.colour)
        .unwrap_or(fallback);

    let mut paint = Paint::default();
    paint.set_color(to_skia_color(border_rgba, alpha));
    paint.anti_alias = radius > 0.5;

    let path = if radius > 0.5 {
        let k = 0.5523 * radius;
        let l = rect.x() + radius;
        let r = rect.x() + rect.width() - radius;
        let t = rect.y() + radius;
        let b = rect.y() + rect.height() - radius;
        let mut pb = PathBuilder::new();
        pb.move_to(l, rect.y());
        pb.line_to(r, rect.y());
        pb.cubic_to(
            r + k,
            rect.y(),
            rect.x() + rect.width(),
            t - k,
            rect.x() + rect.width(),
            t,
        );
        pb.line_to(rect.x() + rect.width(), b);
        pb.cubic_to(
            rect.x() + rect.width(),
            b + k,
            r + k,
            rect.y() + rect.height(),
            r,
            rect.y() + rect.height(),
        );
        pb.line_to(l, rect.y() + rect.height());
        pb.cubic_to(l - k, rect.y() + rect.height(), rect.x(), b + k, rect.x(), b);
        pb.line_to(rect.x(), t);
        pb.cubic_to(rect.x(), t - k, l - k, rect.y(), l, rect.y());
        pb.close();
        pb.finish()
    } else {
        Some(PathBuilder::from_rect(rect))
    };

    if let Some(p) = path {
        let stroke = Stroke {
            width: max_width.max(1.0),
            ..Stroke::default()
        };
        pixmap
            .as_mut()
            .stroke_path(&p, &paint, &stroke, Transform::identity(), None);
    }
}
