//! TDD tests for SWF colour-transform composition.
//!
//! Colour transforms must compose down the sprite tree the same way matrices
//! do: folded into the inherited tint exactly once per placement and propagated
//! to children. These tests guard the two regressions the renderer previously
//! had:
//! - a top-level stage shape's transform applied twice (`draw_stage_at_frame`
//!   pre-applied it, then `draw_character` applied it again);
//! - an intermediate sprite's transform dropped entirely (the sprite branch
//!   passed the parent tint through without folding its own transform in).

mod swf_helpers;

use starbreaker_ui::swf_assets::SwfAssetLibrary;
use starbreaker_ui::swf_render::draw_swf_stage;
use tiny_skia::{Color, Pixmap, Rect as TskRect};

fn dest_100() -> TskRect {
    TskRect::from_xywh(0.0, 0.0, 100.0, 100.0).unwrap()
}

fn white() -> Color {
    Color::from_rgba8(255, 255, 255, 255)
}

fn make_pixmap() -> Pixmap {
    Pixmap::new(100, 100).unwrap()
}

fn pixel_rgba(pixmap: &Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
    let p = pixmap.pixels()[(y * pixmap.width() + x) as usize];
    // Pixels are premultiplied; with full alpha this equals straight alpha.
    (p.red(), p.green(), p.blue(), p.alpha())
}

/// A white shape placed directly on the stage with `r_multiply = 0.5` must have
/// its colour transform applied exactly once: red ≈ 128, not ≈ 64.
#[test]
fn top_level_color_transform_applied_once() {
    let bytes = swf_helpers::make_stage_color_transform_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap();

    draw_swf_stage(&mut pixmap, &assets, dest_100(), white(), 1.0);

    let (r, g, b, a) = pixel_rgba(&pixmap, 10, 10);
    assert!(a > 0, "shape pixel must be opaque");
    assert!(
        (110..=145).contains(&r),
        "r_multiply=0.5 applied once → red≈128; got {r} (≈64 means it was double-applied)"
    );
    assert!(g > 200 && b > 200, "green/blue must stay full; got g={g} b={b}");
}

/// Nested colour transforms must both compose: stage → spriteA (blue→0) →
/// spriteB (green→0) → white shape yields pure red.  If the intermediate
/// spriteB transform is dropped, green survives (yellow).
#[test]
fn nested_color_transforms_compose_down_tree() {
    let bytes = swf_helpers::make_nested_color_transform_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap();

    draw_swf_stage(&mut pixmap, &assets, dest_100(), white(), 1.0);

    let (r, g, b, a) = pixel_rgba(&pixmap, 50, 50);
    assert!(a > 0, "shape pixel at (50,50) must be opaque");
    assert!(r > 200, "red must survive both transforms; got {r}");
    assert!(
        g < 40,
        "green must be zeroed by the intermediate spriteB transform; got {g} (≈255 means it was dropped)"
    );
    assert!(b < 40, "blue must be zeroed by the spriteA transform; got {b}");
}
