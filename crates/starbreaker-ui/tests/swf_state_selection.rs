//! Phase 3 TDD tests: frame-label rendering and state-sprite exclusion.
//!
//! Tests are written before the implementation per TDD practice and initially
//! fail to compile.  They cover:
//!
//! - `draw_swf_at_frame_label` — renders the stage display list at the frame
//!   whose `FrameLabel` tag matches the given name.
//! - `draw_swf_symbol_excluding` — renders a named exported symbol while
//!   skipping any character whose ID is in the caller-supplied suppression set.

mod swf_helpers;

use std::collections::HashSet;

use starbreaker_ui::swf_assets::SwfAssetLibrary;
use starbreaker_ui::swf_render::{draw_swf_at_frame_label, draw_swf_symbol_excluding};
use tiny_skia::{Color, Pixmap, Rect as TskRect};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// 100×100 destination rect for the mixed-state fixture (stage 100×100 → 1:1).
fn dest_100() -> TskRect {
    TskRect::from_xywh(0.0, 0.0, 100.0, 100.0).unwrap()
}

/// 200×200 destination rect for the labeled-frames fixture (stage 200×200 → 1:1).
fn dest_200() -> TskRect {
    TskRect::from_xywh(0.0, 0.0, 200.0, 200.0).unwrap()
}

fn white() -> Color {
    Color::from_rgba8(255, 255, 255, 255)
}

fn make_pixmap_100() -> Pixmap {
    Pixmap::new(100, 100).unwrap()
}

fn make_pixmap_200() -> Pixmap {
    Pixmap::new(200, 200).unwrap()
}

fn pixel_alpha(pixmap: &Pixmap, x: u32, y: u32) -> u8 {
    pixmap.pixels()[(y * pixmap.width() + x) as usize].alpha()
}

/// Red: r > 120, g < 100 (shape color {r:220, g:50, b:50}).
fn pixel_is_reddish(pixmap: &Pixmap, x: u32, y: u32) -> bool {
    let px = pixmap.pixels()[(y * pixmap.width() + x) as usize];
    px.alpha() > 0 && px.red() > 120 && px.green() < 100
}

/// Green: g > 120, r < 100 (shape color {r:50, g:200, b:80}).
fn pixel_is_greenish(pixmap: &Pixmap, x: u32, y: u32) -> bool {
    let px = pixmap.pixels()[(y * pixmap.width() + x) as usize];
    px.alpha() > 0 && px.green() > 120 && px.red() < 100
}

// ── Frame-label tests ─────────────────────────────────────────────────────────

/// `draw_swf_at_frame_label("state_a")` renders the frame-0 display list.
/// The red 100×100 shape placed at (0,0) must produce red pixels.
#[test]
fn frame_label_state_a_renders_red_shape() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap_200();
    let drew = draw_swf_at_frame_label(&mut pixmap, &assets, "state_a", dest_200(), white(), 1.0);
    assert!(drew, "draw_swf_at_frame_label returned false for 'state_a'");
    assert!(pixel_is_reddish(&pixmap, 50, 50), "pixel (50,50) must be red for state_a");
}

/// Frame 0 (state_a) places only the red shape — the green shape from frame 1
/// must NOT appear.
#[test]
fn frame_label_state_a_suppresses_state_b_green() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap_200();
    draw_swf_at_frame_label(&mut pixmap, &assets, "state_a", dest_200(), white(), 1.0);
    assert!(
        !pixel_is_greenish(&pixmap, 50, 50),
        "state_a frame must not contain green pixels from state_b"
    );
}

/// `draw_swf_at_frame_label("state_b")` renders frame 1 — the green shape.
#[test]
fn frame_label_state_b_renders_green_shape() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap_200();
    let drew = draw_swf_at_frame_label(&mut pixmap, &assets, "state_b", dest_200(), white(), 1.0);
    assert!(drew, "draw_swf_at_frame_label returned false for 'state_b'");
    assert!(pixel_is_greenish(&pixmap, 50, 50), "pixel (50,50) must be green for state_b");
}

/// An unknown label returns `false` and leaves the pixmap fully transparent.
#[test]
fn frame_label_unknown_returns_false() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap_200();
    let drew =
        draw_swf_at_frame_label(&mut pixmap, &assets, "no_such_label", dest_200(), white(), 1.0);
    assert!(!drew, "unknown label should return false");
    let non_transparent = pixmap.pixels().iter().filter(|px| px.alpha() > 0).count();
    assert_eq!(non_transparent, 0, "unknown label should leave pixmap empty");
}

// ── State-sprite exclusion tests ──────────────────────────────────────────────

/// With an empty suppression set all three shapes are visible:
/// orange (0–30, depth 1), red StateA (35–65 x, depth 2), blue StateB (0–30 at y≥35, depth 3).
#[test]
fn draw_without_exclusion_renders_all_states() {
    let bytes = swf_helpers::make_mixed_state_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let mut pixmap = make_pixmap_100();
    let suppressed: HashSet<u16> = HashSet::new();
    let drew = draw_swf_symbol_excluding(
        &mut pixmap,
        &assets,
        "DocSprite",
        &suppressed,
        dest_100(),
        white(),
        1.0,
    );
    assert!(drew, "draw_swf_symbol_excluding should return true");
    assert!(pixel_alpha(&pixmap, 10, 10) > 0, "orange always-placed (0–30) must be visible");
    assert!(pixel_alpha(&pixmap, 45, 10) > 0, "StateA red (35–65 x) must be visible");
    assert!(pixel_alpha(&pixmap, 10, 45) > 0, "StateB blue (35–65 y) must be visible");
}

/// Suppressing `StateB_Content`'s character ID hides its blue pixels at (10,45).
#[test]
fn suppress_state_b_hides_blue_pixels() {
    let bytes = swf_helpers::make_mixed_state_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let state_b_id =
        assets.lookup_export("StateB_Content").expect("StateB_Content must be exported");
    let mut suppressed: HashSet<u16> = HashSet::new();
    suppressed.insert(state_b_id);
    let mut pixmap = make_pixmap_100();
    draw_swf_symbol_excluding(
        &mut pixmap,
        &assets,
        "DocSprite",
        &suppressed,
        dest_100(),
        white(),
        1.0,
    );
    assert_eq!(pixel_alpha(&pixmap, 10, 45), 0, "StateB (blue, 35–65 y) must be hidden");
}

/// When StateB is suppressed the orange shape and StateA (red) remain visible.
#[test]
fn suppress_state_b_keeps_orange_and_state_a() {
    let bytes = swf_helpers::make_mixed_state_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let state_b_id =
        assets.lookup_export("StateB_Content").expect("StateB_Content must be exported");
    let mut suppressed: HashSet<u16> = HashSet::new();
    suppressed.insert(state_b_id);
    let mut pixmap = make_pixmap_100();
    draw_swf_symbol_excluding(
        &mut pixmap,
        &assets,
        "DocSprite",
        &suppressed,
        dest_100(),
        white(),
        1.0,
    );
    assert!(pixel_alpha(&pixmap, 10, 10) > 0, "orange always-placed shape must remain visible");
    assert!(pixel_alpha(&pixmap, 45, 10) > 0, "StateA (red, 35–65 x) must remain visible");
}

/// When StateA is suppressed the orange shape and StateB (blue) remain; red is hidden.
#[test]
fn suppress_state_a_keeps_orange_and_state_b() {
    let bytes = swf_helpers::make_mixed_state_swf();
    let assets = SwfAssetLibrary::new(bytes).unwrap();
    let state_a_id =
        assets.lookup_export("StateA_Content").expect("StateA_Content must be exported");
    let mut suppressed: HashSet<u16> = HashSet::new();
    suppressed.insert(state_a_id);
    let mut pixmap = make_pixmap_100();
    draw_swf_symbol_excluding(
        &mut pixmap,
        &assets,
        "DocSprite",
        &suppressed,
        dest_100(),
        white(),
        1.0,
    );
    assert!(pixel_alpha(&pixmap, 10, 10) > 0, "orange always-placed shape must remain visible");
    assert!(pixel_alpha(&pixmap, 10, 45) > 0, "StateB (blue, 35–65 y) must remain visible");
    assert_eq!(pixel_alpha(&pixmap, 45, 10), 0, "StateA (red, 35–65 x) must be hidden");
}
