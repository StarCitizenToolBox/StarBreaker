//! Phase 2 TDD tests for the SWF display-list renderer.
//!
//! Tests verify correct multi-level sprite recursion, composed matrix
//! transforms, and cycle detection.  These tests are written first (TDD) and
//! fail until the Phase 2 implementation is complete.

mod swf_helpers;

use starbreaker_ui::swf_assets::SwfAssetLibrary;
use starbreaker_ui::swf_render::draw_swf_symbol;
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

fn pixel_alpha(pixmap: &Pixmap, x: u32, y: u32) -> u8 {
    pixmap.pixels()[(y * pixmap.width() + x) as usize].alpha()
}

// ── Multi-level nesting ───────────────────────────────────────────────────────

/// `draw_swf_symbol` must recurse into sprites more than one level deep.
///
/// The fixture has outer(id=3) → inner(id=2) → shape(id=1).  The current
/// implementation only handles one level (sprite → shapes) so it silently
/// skips inner and returns false.
#[test]
fn doubly_nested_sprite_renders_inner_shape() {
    let bytes = swf_helpers::make_doubly_nested_sprite_swf();
    swf_helpers::assert_swf_symbol_has_non_empty_coverage(&bytes, "DoubleNested", 100, 100);
}

// ── Matrix composition ────────────────────────────────────────────────────────

/// When an outer sprite places an inner at 2× scale, child shapes must be
/// rendered at the composed (2×) size, not at their local (1×) size.
///
/// The shape is 20×20 at (0,0) in a 100×100 stage.  With correct composition
/// it covers (0,0)–(40,40); without it only (0,0)–(20,20).  Pixel (30,30)
/// distinguishes the two cases.
#[test]
fn scaled_outer_sprite_composes_matrix_with_inner() {
    let bytes = swf_helpers::make_scaled_nested_sprite_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap();

    draw_swf_symbol(&mut pixmap, &assets, "ScaledOuter", dest_100(), white(), 1.0);

    assert!(
        pixel_alpha(&pixmap, 30, 30) > 0,
        "pixel (30,30) must be non-transparent: with 2× scale the 20×20 shape spans 0–40 \
         in a 100×100 stage mapped to a 100×100 dest"
    );
}

/// The 2× scaled shape must NOT extend beyond 40 pixels.  Pixel (50,50)
/// should remain transparent (the shape is at origin and its 2× extent is 40).
#[test]
fn scaled_outer_sprite_does_not_extend_beyond_composed_bounds() {
    let bytes = swf_helpers::make_scaled_nested_sprite_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap();

    draw_swf_symbol(&mut pixmap, &assets, "ScaledOuter", dest_100(), white(), 1.0);

    assert_eq!(
        pixel_alpha(&pixmap, 50, 50),
        0,
        "pixel (50,50) must be transparent: shape covers only 0–40 with 2× scale"
    );
}

// ── Cycle detection ───────────────────────────────────────────────────────────

/// A sprite that places itself must not cause a panic or infinite loop.
///
/// With the old max-depth-only guard it recurses 4 times then stops.  With
/// explicit cycle detection it stops at the first self-reference.  Either way
/// it must complete quickly and not panic.
#[test]
fn self_referential_sprite_does_not_panic() {
    let bytes = swf_helpers::make_self_referential_sprite_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap();

    // Must complete without panic.
    let drew = draw_swf_symbol(&mut pixmap, &assets, "SelfRef", dest_100(), white(), 1.0);

    // The self-referential sprite has no shape, so it must return false.
    assert!(!drew, "self-referential sprite has no shape — draw must return false");
}
