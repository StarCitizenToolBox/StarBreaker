use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform, Rect as TskRect};

use crate::swf_assets::ShapeRecord;

pub(super) fn draw_shape(
    pixmap: &mut Pixmap,
    shape: &ShapeRecord,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let bounds = &shape.shape_bounds;
    let bx0 = bounds.x_min.to_pixels() as f32;
    let by0 = bounds.y_min.to_pixels() as f32;
    let bw = (bounds.x_max - bounds.x_min).to_pixels() as f32;
    let bh = (bounds.y_max - bounds.y_min).to_pixels() as f32;

    if bw <= 0.0 || bh <= 0.0 {
        return false;
    }

    let sx = dest.width() / bw;
    let sy = dest.height() / bh;
    let dx = dest.left() - bx0 * sx;
    let dy = dest.top() - by0 * sy;

    let mut drew_any = false;

    let num_fill = shape.fill_styles.len();
    for style_idx in 1..=(num_fill as u32) {
        let fill = &shape.fill_styles[(style_idx - 1) as usize];
        let color = match fill {
            swf::FillStyle::Color(c) => *c,
            _ => continue,
        };

        if let Some(path) = build_path_for_fill(shape, style_idx, sx, sy, dx, dy) {
            let rgba = tinted_color(color, tint, alpha);
            let mut paint = Paint::default();
            paint.set_color(rgba);
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            drew_any = true;
        }
    }

    for (ls_idx, line_style) in shape.line_styles.iter().enumerate() {
        let color = match line_style.fill_style() {
            swf::FillStyle::Color(c) => *c,
            _ => continue,
        };

        let ls_index = (ls_idx + 1) as u32;
        if let Some(path) = build_path_for_line(shape, ls_index, sx, sy, dx, dy) {
            let stroke_width_px = (line_style.width().to_pixels() as f32 * sx.abs()).max(0.5);
            let rgba = tinted_color(color, tint, alpha);
            let mut paint = Paint::default();
            paint.set_color(rgba);
            paint.anti_alias = true;
            let stroke = Stroke {
                width: stroke_width_px,
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            drew_any = true;
        }
    }

    drew_any
}

pub(super) fn matrix_to_dest(
    shape: &ShapeRecord,
    matrix: &swf::Matrix,
    _sw: f32,
    _sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
) -> TskRect {
    let b = &shape.shape_bounds;
    let bx0 = b.x_min.to_pixels() as f32;
    let by0 = b.y_min.to_pixels() as f32;
    let bx1 = b.x_max.to_pixels() as f32;
    let by1 = b.y_max.to_pixels() as f32;

    let a = matrix.a.to_f32();
    let b_coef = matrix.b.to_f32();
    let c = matrix.c.to_f32();
    let d = matrix.d.to_f32();
    let tx = matrix.tx.to_pixels() as f32;
    let ty = matrix.ty.to_pixels() as f32;

    let corners = [(bx0, by0), (bx1, by0), (bx0, by1), (bx1, by1)];
    let trans: Vec<(f32, f32)> = corners
        .iter()
        .map(|&(x, y)| (a * x + c * y + tx, b_coef * x + d * y + ty))
        .collect();

    let mx0 = trans.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let my0 = trans.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let mx1 = trans.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let my1 = trans.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);

    let dest_x = origin.left() + mx0 * sx;
    let dest_y = origin.top() + my0 * sy;
    let dest_w = ((mx1 - mx0) * sx).max(1.0);
    let dest_h = ((my1 - my0) * sy).max(1.0);

    TskRect::from_xywh(dest_x, dest_y, dest_w, dest_h).unwrap_or(origin)
}

fn build_path_for_fill(
    shape: &ShapeRecord,
    target_fill_idx: u32,
    sx: f32,
    sy: f32,
    dx: f32,
    dy: f32,
) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut active_fill1: u32 = 0;
    let mut active_fill0: u32 = 0;
    let mut in_sub = false;

    for rec in &shape.records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if in_sub {
                    pb.close();
                    in_sub = false;
                }
                if let Some(pt) = sc.move_to {
                    cx = pt.x.to_pixels() as f32 * sx + dx;
                    cy = pt.y.to_pixels() as f32 * sy + dy;
                }
                if let Some(fi) = sc.fill_style_1 {
                    active_fill1 = fi;
                }
                if let Some(fi) = sc.fill_style_0 {
                    active_fill0 = fi;
                }
                if sc.new_styles.is_some() {
                    active_fill0 = 0;
                    active_fill1 = 0;
                }
                if active_fill1 == target_fill_idx || active_fill0 == target_fill_idx {
                    pb.move_to(cx, cy);
                    in_sub = true;
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                if active_fill1 != target_fill_idx && active_fill0 != target_fill_idx {
                    let ex = cx + delta.dx.to_pixels() as f32 * sx;
                    let ey = cy + delta.dy.to_pixels() as f32 * sy;
                    cx = ex;
                    cy = ey;
                    continue;
                }
                if !in_sub {
                    pb.move_to(cx, cy);
                    in_sub = true;
                }
                let ex = cx + delta.dx.to_pixels() as f32 * sx;
                let ey = cy + delta.dy.to_pixels() as f32 * sy;
                pb.line_to(ex, ey);
                cx = ex;
                cy = ey;
            }
            swf::ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                if active_fill1 != target_fill_idx && active_fill0 != target_fill_idx {
                    let ctrl_x = cx + control_delta.dx.to_pixels() as f32 * sx;
                    let ctrl_y = cy + control_delta.dy.to_pixels() as f32 * sy;
                    let anch_x = ctrl_x + anchor_delta.dx.to_pixels() as f32 * sx;
                    let anch_y = ctrl_y + anchor_delta.dy.to_pixels() as f32 * sy;
                    cx = anch_x;
                    cy = anch_y;
                    continue;
                }
                if !in_sub {
                    pb.move_to(cx, cy);
                    in_sub = true;
                }
                let ctrl_x = cx + control_delta.dx.to_pixels() as f32 * sx;
                let ctrl_y = cy + control_delta.dy.to_pixels() as f32 * sy;
                let anch_x = ctrl_x + anchor_delta.dx.to_pixels() as f32 * sx;
                let anch_y = ctrl_y + anchor_delta.dy.to_pixels() as f32 * sy;
                pb.quad_to(ctrl_x, ctrl_y, anch_x, anch_y);
                cx = anch_x;
                cy = anch_y;
            }
        }
    }
    if in_sub {
        pb.close();
    }

    pb.finish()
}

fn build_path_for_line(
    shape: &ShapeRecord,
    target_ls_idx: u32,
    sx: f32,
    sy: f32,
    dx: f32,
    dy: f32,
) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut active_ls: u32 = 0;
    let mut in_sub = false;

    for rec in &shape.records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if in_sub && active_ls != target_ls_idx {
                    in_sub = false;
                }
                if let Some(pt) = sc.move_to {
                    cx = pt.x.to_pixels() as f32 * sx + dx;
                    cy = pt.y.to_pixels() as f32 * sy + dy;
                    in_sub = false;
                }
                if let Some(ls) = sc.line_style {
                    active_ls = ls;
                }
                if sc.new_styles.is_some() {
                    active_ls = 0;
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                let ex = cx + delta.dx.to_pixels() as f32 * sx;
                let ey = cy + delta.dy.to_pixels() as f32 * sy;
                if active_ls == target_ls_idx {
                    if !in_sub {
                        pb.move_to(cx, cy);
                        in_sub = true;
                    }
                    pb.line_to(ex, ey);
                }
                cx = ex;
                cy = ey;
            }
            swf::ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                let ctrl_x = cx + control_delta.dx.to_pixels() as f32 * sx;
                let ctrl_y = cy + control_delta.dy.to_pixels() as f32 * sy;
                let anch_x = ctrl_x + anchor_delta.dx.to_pixels() as f32 * sx;
                let anch_y = ctrl_y + anchor_delta.dy.to_pixels() as f32 * sy;
                if active_ls == target_ls_idx {
                    if !in_sub {
                        pb.move_to(cx, cy);
                        in_sub = true;
                    }
                    pb.quad_to(ctrl_x, ctrl_y, anch_x, anch_y);
                }
                cx = anch_x;
                cy = anch_y;
            }
        }
    }

    pb.finish()
}

fn tinted_color(swf_color: swf::Color, tint: Color, alpha: f32) -> Color {
    let is_tint_white = tint.red() >= 1.0 && tint.green() >= 1.0 && tint.blue() >= 1.0;
    let swf_is_white = swf_color.r == 255 && swf_color.g == 255 && swf_color.b == 255;

    let (r, g, b) = if !is_tint_white && swf_is_white {
        (tint.red(), tint.green(), tint.blue())
    } else {
        (
            swf_color.r as f32 / 255.0,
            swf_color.g as f32 / 255.0,
            swf_color.b as f32 / 255.0,
        )
    };

    let a = (swf_color.a as f32 / 255.0) * alpha.clamp(0.0, 1.0);
    Color::from_rgba(r, g, b, a).unwrap_or(Color::TRANSPARENT)
}
