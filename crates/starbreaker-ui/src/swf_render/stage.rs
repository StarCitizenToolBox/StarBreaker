use tiny_skia::{Color, Pixmap, Rect as TskRect};

use crate::swf_assets::{PlaceRecord, ShapeRecord, SwfAssetLibrary};

use super::shape::{draw_shape, matrix_to_dest};

/// Rasterise the SWF shape named `symbol_name` into `pixmap`, mapped into `dest`.
pub fn draw_swf_symbol(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    symbol_name: &str,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let Some(char_id) = assets.lookup_export(symbol_name) else {
        log::debug!("draw_swf_symbol: symbol '{symbol_name}' not found in exports");
        return false;
    };

    if let Some(shape) = assets.get_shape(char_id) {
        draw_shape(pixmap, shape, dest, tint, alpha)
    } else {
        let places: Vec<PlaceRecord> = assets.extract_sprite_first_frame(char_id);
        if places.is_empty() {
            log::debug!(
                "draw_swf_symbol: char id={char_id} for '{symbol_name}' is not a shape or sprite"
            );
            return false;
        }

        let mut drew_any = false;
        for place in &places {
            if let Some(shape) = assets.get_shape(place.character_id) {
                let effective_dest = apply_place_matrix_to_dest(shape, &place.matrix, dest);
                if draw_shape(pixmap, shape, effective_dest, tint, alpha) {
                    drew_any = true;
                }
            }
        }
        drew_any
    }
}

/// Rasterise the SWF main-timeline stage frame 0 into `pixmap`, mapped into `dest`.
pub fn draw_swf_stage(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let (sw, sh) = assets.stage_size();
    if sw <= 0.0 || sh <= 0.0 {
        log::debug!("draw_swf_stage: degenerate stage size ({sw}x{sh}), skipping");
        return false;
    }

    let stage_places = assets.stage_frame(0);
    if stage_places.is_empty() {
        log::debug!("draw_swf_stage: stage frame 0 is empty");
        return false;
    }

    let sx = dest.width() / sw;
    let sy = dest.height() / sh;

    let mut drew_any = false;
    for place in &stage_places {
        let ct_tint = color_transform_tint(tint, place.color_transform.as_ref());
        if draw_stage_character(pixmap, assets, place, sw, sh, sx, sy, dest, ct_tint, alpha) {
            drew_any = true;
        }
    }

    drew_any
}

/// Render all visual exports from a Flash SWF, plus stage frame 0 content.
pub fn draw_swf_visual_exports(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let (sw, sh) = assets.stage_size();
    if sw <= 0.0 || sh <= 0.0 {
        log::debug!("draw_swf_visual_exports: degenerate stage size ({sw}x{sh}), skipping");
        return false;
    }

    let sx = dest.width() / sw;
    let sy = dest.height() / sh;

    let mut drew_any = false;
    let mut seen: std::collections::HashSet<swf::CharacterId> = std::collections::HashSet::new();

    let stage_places = assets.stage_frame(0);
    for place in &stage_places {
        if !seen.insert(place.character_id) {
            continue;
        }
        let ct_tint = color_transform_tint(tint, place.color_transform.as_ref());
        if draw_stage_character(pixmap, assets, place, sw, sh, sx, sy, dest, ct_tint, alpha) {
            drew_any = true;
        }
    }

    let char_ids: Vec<swf::CharacterId> = assets.visual_exports().collect();
    for char_id in char_ids {
        if !seen.insert(char_id) {
            continue;
        }
        let place = PlaceRecord {
            depth: 0,
            character_id: char_id,
            matrix: swf::Matrix::IDENTITY,
            color_transform: None,
            name: None,
        };
        if draw_stage_character(pixmap, assets, &place, sw, sh, sx, sy, dest, tint, alpha) {
            drew_any = true;
        }
    }

    drew_any
}

fn draw_stage_character(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    place: &PlaceRecord,
    sw: f32,
    sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    const MAX_SPRITE_DEPTH: u8 = 4;
    draw_stage_character_depth(
        pixmap,
        assets,
        place,
        sw,
        sh,
        sx,
        sy,
        origin,
        tint,
        alpha,
        MAX_SPRITE_DEPTH,
    )
}

fn draw_stage_character_depth(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    place: &PlaceRecord,
    sw: f32,
    sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
    tint: Color,
    alpha: f32,
    max_depth: u8,
) -> bool {
    let char_id = place.character_id;

    if let Some(shape) = assets.get_shape(char_id) {
        let shape_dest = matrix_to_dest(shape, &place.matrix, sw, sh, sx, sy, origin);
        draw_shape(pixmap, shape, shape_dest, tint, alpha)
    } else if max_depth > 0 {
        let sprite_places = assets.extract_sprite_first_frame(char_id);
        if sprite_places.is_empty() {
            return false;
        }
        let sprite_origin = sprite_origin_in_dest(&place.matrix, sw, sh, sx, sy, origin);
        let mut drew_any = false;
        for sp_place in &sprite_places {
            let sp_tint = color_transform_tint(tint, sp_place.color_transform.as_ref());
            if draw_stage_character_depth(
                pixmap,
                assets,
                sp_place,
                sw,
                sh,
                sx,
                sy,
                sprite_origin,
                sp_tint,
                alpha,
                max_depth - 1,
            ) {
                drew_any = true;
            }
        }
        drew_any
    } else {
        false
    }
}

fn sprite_origin_in_dest(
    matrix: &swf::Matrix,
    _sw: f32,
    _sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
) -> TskRect {
    let tx = matrix.tx.to_pixels() as f32;
    let ty = matrix.ty.to_pixels() as f32;
    let dest_x = origin.left() + tx * sx;
    let dest_y = origin.top() + ty * sy;
    let dest_w = origin.width().max(1.0);
    let dest_h = origin.height().max(1.0);
    TskRect::from_xywh(dest_x, dest_y, dest_w, dest_h).unwrap_or(origin)
}

fn color_transform_tint(tint: Color, ct: Option<&swf::ColorTransform>) -> Color {
    let Some(ct) = ct else { return tint };
    let rm = ct.r_multiply.to_f32().clamp(0.0, 1.0);
    let gm = ct.g_multiply.to_f32().clamp(0.0, 1.0);
    let bm = ct.b_multiply.to_f32().clamp(0.0, 1.0);
    let am = ct.a_multiply.to_f32().clamp(0.0, 1.0);
    Color::from_rgba(
        (tint.red() * rm).clamp(0.0, 1.0),
        (tint.green() * gm).clamp(0.0, 1.0),
        (tint.blue() * bm).clamp(0.0, 1.0),
        (tint.alpha() * am).clamp(0.0, 1.0),
    )
    .unwrap_or(tint)
}

fn apply_place_matrix_to_dest(
    shape: &ShapeRecord,
    matrix: &swf::Matrix,
    parent_dest: TskRect,
) -> TskRect {
    let b = &shape.shape_bounds;
    let bx0 = b.x_min.to_pixels() as f32;
    let by0 = b.y_min.to_pixels() as f32;
    let bx1 = b.x_max.to_pixels() as f32;
    let by1 = b.y_max.to_pixels() as f32;

    let corners = [(bx0, by0), (bx1, by0), (bx0, by1), (bx1, by1)];
    let a = matrix.a.to_f32();
    let b_coef = matrix.b.to_f32();
    let c = matrix.c.to_f32();
    let d = matrix.d.to_f32();
    let tx = matrix.tx.to_pixels() as f32;
    let ty = matrix.ty.to_pixels() as f32;

    let transformed: Vec<(f32, f32)> = corners
        .iter()
        .map(|&(x, y)| (a * x + c * y + tx, b_coef * x + d * y + ty))
        .collect();

    let mx0 = transformed.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let my0 = transformed.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let mx1 = transformed.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let my1 = transformed.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);

    let mw = (mx1 - mx0).max(1.0);
    let mh = (my1 - my0).max(1.0);

    let sx = parent_dest.width() / (bx1 - bx0).max(1.0);
    let sy = parent_dest.height() / (by1 - by0).max(1.0);

    let x0_px = parent_dest.left() + (mx0 - bx0) * sx;
    let y0_px = parent_dest.top() + (my0 - by0) * sy;
    let w_px = (mw * sx).max(1.0);
    let h_px = (mh * sy).max(1.0);

    TskRect::from_xywh(x0_px, y0_px, w_px, h_px).unwrap_or(parent_dest)
}
