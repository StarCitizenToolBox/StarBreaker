//! SWF display-list renderer.
//!
//! Key public entry points:
//! - `draw_swf_symbol`          — render a named exported symbol into a dest rect.
//! - `draw_swf_symbol_excluding`— same, but skips specified character IDs (state suppression).
//! - `draw_swf_stage`           — render main-timeline frame 0 into a dest rect.
//! - `draw_swf_at_frame_label`  — render the stage at a named `FrameLabel` frame.
//! - `draw_swf_visual_exports`  — render all visual exports.
//!
//! Phase 2: matrices are composed as the renderer descends the sprite tree;
//! cycle detection (visited set on the call stack) prevents infinite loops.
//! Phase 3: `suppressed` set enables caller-driven state-sprite exclusion;
//! `draw_swf_at_frame_label` selects a named frame for SWFs that use labels.
//! Phase 4: `draw_character` handles `DefineEditText` via `edit_text::draw_edit_text`;
//! `loc_fn` threads through to resolve `@key` loc strings.

use std::collections::HashSet;

use swf::CharacterId;
use tiny_skia::{Color, Pixmap, Rect as TskRect};

use crate::swf_assets::{PlaceRecord, SwfAssetLibrary};

use super::shape::{draw_shape, matrix_to_dest};

const MAX_SPRITE_DEPTH: u8 = 8;

// ── Public rendering entry points ─────────────────────────────────────────────

/// Rasterise the SWF shape or sprite named `symbol_name` into `pixmap`,
/// mapped into `dest`.  Returns `true` if at least one pixel was drawn.
///
/// The symbol is rendered at its position in SWF stage coordinates, scaled
/// so the full stage fits in `dest`.  For a symbol that spans the full stage
/// this fills `dest`; for a smaller symbol it occupies a proportional area.
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

    let (sw, sh) = assets.stage_size();
    // Fall back to dest dimensions if the SWF has a degenerate stage header.
    let (sw, sh) = if sw > 0.0 && sh > 0.0 {
        (sw, sh)
    } else {
        (dest.width().max(1.0), dest.height().max(1.0))
    };
    let sx = dest.width() / sw;
    let sy = dest.height() / sh;

    let place = PlaceRecord {
        depth: 0,
        character_id: char_id,
        matrix: swf::Matrix::IDENTITY,
        color_transform: None,
        name: None,
        clip_depth: None,
    };

    let empty = HashSet::new();
    let mut visited = HashSet::new();
    draw_character(pixmap, assets, &place, sw, sh, sx, sy, dest, tint, alpha, MAX_SPRITE_DEPTH, &mut visited, &empty, &|_| None)
}

/// Rasterise the SWF main-timeline stage frame 0 into `pixmap`, mapped into `dest`.
pub fn draw_swf_stage(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let empty = HashSet::new();
    draw_stage_at_frame(pixmap, assets, 0, dest, tint, alpha, &empty, &|_| None)
}

/// Rasterise the SWF main-timeline stage frame 0 with state suppression and loc resolution.
///
/// `suppressed` is the set of CharacterIds to skip (and their subtrees); populated
/// from `compute_sample_data_export_ids` for static renders.  `loc_fn` resolves
/// `@key` loc strings in EditText.
pub fn draw_swf_stage_with_state(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    dest: TskRect,
    tint: Color,
    alpha: f32,
    suppressed: &HashSet<CharacterId>,
    loc_fn: &dyn Fn(&str) -> Option<String>,
) -> bool {
    draw_stage_at_frame(pixmap, assets, 0, dest, tint, alpha, suppressed, loc_fn)
}

/// Rasterise the SWF stage at the frame whose `FrameLabel` matches `label`.
///
/// Returns `false` when the label is not found or the display list is empty.
pub fn draw_swf_at_frame_label(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    label: &str,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let Some(frame_index) = assets.frame_label_index(label) else {
        log::debug!("draw_swf_at_frame_label: label '{label}' not found");
        return false;
    };
    let empty = HashSet::new();
    draw_stage_at_frame(pixmap, assets, frame_index, dest, tint, alpha, &empty, &|_| None)
}

/// Render the named exported symbol, skipping any character whose ID is in
/// `suppressed` (and recursively all its children).
///
/// Callers use this to suppress inactive-state sprites while still rendering
/// always-placed siblings and the active-state sprite.
pub fn draw_swf_symbol_excluding(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    symbol_name: &str,
    suppressed: &HashSet<CharacterId>,
    dest: TskRect,
    tint: Color,
    alpha: f32,
) -> bool {
    let Some(char_id) = assets.lookup_export(symbol_name) else {
        log::debug!("draw_swf_symbol_excluding: symbol '{symbol_name}' not found");
        return false;
    };

    let (sw, sh) = assets.stage_size();
    let (sw, sh) = if sw > 0.0 && sh > 0.0 {
        (sw, sh)
    } else {
        (dest.width().max(1.0), dest.height().max(1.0))
    };
    let sx = dest.width() / sw;
    let sy = dest.height() / sh;

    let place = PlaceRecord {
        depth: 0,
        character_id: char_id,
        matrix: swf::Matrix::IDENTITY,
        color_transform: None,
        name: None,
        clip_depth: None,
    };

    let mut visited = HashSet::new();
    draw_character(pixmap, assets, &place, sw, sh, sx, sy, dest, tint, alpha, MAX_SPRITE_DEPTH, &mut visited, suppressed, &|_| None)
}

/// Render all visual exports from a Flash SWF at their stage-space positions.
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
    let mut seen: HashSet<CharacterId> = HashSet::new();

    // Stage frame 0 shapes are ActionScript-controlled at runtime. The game
    // dynamically shows or hides them based on state (e.g. target acquired vs
    // no target). In static renders we do not know the runtime state, so we
    // skip stage shapes entirely and rely on the BB IR layer for structural
    // content. We only draw explicitly exported visual symbols below.
    // Do NOT add stage IDs to `seen` — some stage characters are also exported
    // symbols that we DO want to draw at their canonical (identity) position.

    // Sort by ascending CharacterId (SWF definition order) so overlapping
    // exports composite in a deterministic order; `visual_exports()` iterates a
    // `HashMap` whose order varies per process.
    let mut char_ids: Vec<CharacterId> = assets.visual_exports().collect();
    char_ids.sort_unstable();
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
            clip_depth: None,
        };
        let empty = HashSet::new();
        let mut visited = HashSet::new();
        if draw_character(pixmap, assets, &place, sw, sh, sx, sy, dest, tint, alpha, MAX_SPRITE_DEPTH, &mut visited, &empty, &|_| None) {
            drew_any = true;
        }
    }

    drew_any
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Render all placed characters in the stage display list at `frame_index`.
fn draw_stage_at_frame(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    frame_index: u32,
    dest: TskRect,
    tint: Color,
    alpha: f32,
    suppressed: &HashSet<CharacterId>,
    loc_fn: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let (sw, sh) = assets.stage_size();
    if sw <= 0.0 || sh <= 0.0 {
        log::debug!("draw_stage_at_frame: degenerate stage size ({sw}x{sh}), skipping");
        return false;
    }

    let stage_places = assets.stage_frame(frame_index);
    if stage_places.is_empty() {
        log::debug!("draw_stage_at_frame: frame {frame_index} is empty");
        return false;
    }

    let sx = dest.width() / sw;
    let sy = dest.height() / sh;

    let mut drew_any = false;
    for place in &stage_places {
        // `draw_character` folds each place's own colour transform into the tint,
        // so pass the unmodified stage tint here (no pre-application).
        let mut visited = HashSet::new();
        if draw_character(pixmap, assets, place, sw, sh, sx, sy, dest, tint, alpha, MAX_SPRITE_DEPTH, &mut visited, suppressed, loc_fn) {
            drew_any = true;
        }
    }
    drew_any
}

// ── Core recursive renderer ────────────────────────────────────────────────────

/// Render one character (shape, sprite, or EditText) into `pixmap`.
///
/// `place.matrix` is the **fully composed** transform from the stage origin to
/// this character.  For top-level calls this is the character's own matrix;
/// for recursive calls it is `parent_matrix × child_matrix`.
///
/// `visited` tracks character IDs on the current call stack to break cycles.
/// `suppressed` contains character IDs to skip entirely (state suppression).
/// `loc_fn` resolves `@key` strings in EditText initial_text (Phase 4).
fn draw_character(
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
    visited: &mut HashSet<CharacterId>,
    suppressed: &HashSet<CharacterId>,
    loc_fn: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let char_id = place.character_id;

    // State suppression: caller-nominated IDs and their subtrees are skipped.
    if suppressed.contains(&char_id) {
        return false;
    }

    // Cycle detection: stop if this character is already on the call stack.
    if !visited.insert(char_id) {
        return false;
    }

    // Fold this character's colour transform into the inherited tint exactly
    // once. The folded tint is used for the leaf draw and propagated to sprite
    // children, so nested colour transforms compose down the tree the same way
    // matrices do — without being dropped (intermediate sprites) or applied
    // twice (top-level shapes).
    let local_tint = color_transform_tint(tint, place.color_transform.as_ref());

    let result = if let Some(shape) = assets.get_shape(char_id) {
        let shape_dest = matrix_to_dest(shape, &place.matrix, sw, sh, sx, sy, origin);
        draw_shape(pixmap, shape, shape_dest, local_tint, alpha)
    } else if let Some(edit) = assets.get_edit_text(char_id) {
        super::edit_text::draw_edit_text(
            pixmap, assets, edit, &place.matrix,
            sw, sh, sx, sy, origin, alpha, loc_fn,
        )
    } else if max_depth > 0 {
        let sprite_places = assets.extract_sprite_first_frame(char_id);
        let mut drew_any = false;
        for sp_place in &sprite_places {
            // Compose accumulated parent matrix with this child's local matrix
            // so the full transform chain is reflected in every leaf draw call.
            let composed_matrix = compose_matrix(&place.matrix, &sp_place.matrix);
            let composed_place = PlaceRecord {
                depth: sp_place.depth,
                character_id: sp_place.character_id,
                matrix: composed_matrix,
                color_transform: sp_place.color_transform,
                name: sp_place.name.clone(),
                clip_depth: sp_place.clip_depth,
            };
            // Propagate the folded tint so this sprite's own colour transform
            // affects its children; each child folds its own transform in turn.
            if draw_character(
                pixmap,
                assets,
                &composed_place,
                sw, sh, sx, sy,
                origin, // unchanged — composed_matrix handles positioning
                local_tint,
                alpha,
                max_depth - 1,
                visited,
                suppressed,
                loc_fn,
            ) {
                drew_any = true;
            }
        }
        drew_any
    } else {
        false
    };

    visited.remove(&char_id);
    result
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compose two SWF matrices: `result = parent × child`.
///
/// Points in the child's local space are transformed by `child` first, then
/// by `parent`, giving the composed transform from the child's space to the
/// grandparent's space.
fn compose_matrix(parent: &swf::Matrix, child: &swf::Matrix) -> swf::Matrix {
    let pa = parent.a.to_f32();
    let pb = parent.b.to_f32();
    let pc = parent.c.to_f32();
    let pd = parent.d.to_f32();
    let ptx = parent.tx.to_pixels() as f32;
    let pty = parent.ty.to_pixels() as f32;

    let ca = child.a.to_f32();
    let cb = child.b.to_f32();
    let cc = child.c.to_f32();
    let cd = child.d.to_f32();
    let ctx = child.tx.to_pixels() as f32;
    let cty = child.ty.to_pixels() as f32;

    swf::Matrix {
        a: swf::Fixed16::from_f32(pa * ca + pc * cb),
        b: swf::Fixed16::from_f32(pb * ca + pd * cb),
        c: swf::Fixed16::from_f32(pa * cc + pc * cd),
        d: swf::Fixed16::from_f32(pb * cc + pd * cd),
        tx: swf::Twips::from_pixels((pa * ctx + pc * cty + ptx) as f64),
        ty: swf::Twips::from_pixels((pb * ctx + pd * cty + pty) as f64),
    }
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
