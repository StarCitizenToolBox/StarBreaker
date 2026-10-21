//! Phase 0 regression tests for the SWF fixture infrastructure.
//!
//! These tests verify that:
//!   1. The two canonical fixture SWFs parse without error (0.1).
//!   2. Each fixture contains the expected tags, exports, and frame labels (0.1).
//!   3. The production `draw_swf_symbol` path produces non-empty pixel coverage
//!      for a sprite-wrapped shape (0.3).
//!
//! The `swf_helpers` module provides the builders and the coverage helper; later
//! phases include `mod swf_helpers;` to reuse them.

mod swf_helpers;

use swf::{CharacterId, Tag};
use starbreaker_ui::swf_assets::{extract_exported_symbols, SwfAssetLibrary};

// ── Fixture 1: labeled frames ────────────────────────────────────────────────

#[test]
fn labeled_frames_fixture_parses_without_error() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    SwfAssetLibrary::new(bytes).expect("make_labeled_frames_swf must produce a valid SWF");
}

#[test]
fn labeled_frames_fixture_has_two_main_timeline_frames() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    let buf = swf::decompress_swf(std::io::Cursor::new(&bytes)).expect("decompress");
    let parsed = swf::parse_swf(&buf).expect("parse");
    assert_eq!(parsed.header.num_frames(), 2, "expected 2 main-timeline frames");
}

#[test]
fn labeled_frames_fixture_contains_expected_frame_labels() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    let buf = swf::decompress_swf(std::io::Cursor::new(&bytes)).expect("decompress");
    let parsed = swf::parse_swf(&buf).expect("parse");

    let labels: Vec<String> = parsed
        .tags
        .iter()
        .filter_map(|t| {
            if let Tag::FrameLabel(fl) = t {
                Some(fl.label.to_string_lossy(swf::UTF_8))
            } else {
                None
            }
        })
        .collect();

    assert!(
        labels.contains(&"state_a".to_string()),
        "expected FrameLabel 'state_a', got: {labels:?}"
    );
    assert!(
        labels.contains(&"state_b".to_string()),
        "expected FrameLabel 'state_b', got: {labels:?}"
    );
}

#[test]
fn labeled_frames_fixture_exports_shape_a_and_shape_b() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    let exports = extract_exported_symbols(&bytes).expect("extract_exported_symbols");
    assert!(exports.contains_key("shape_a"), "expected export 'shape_a', got: {exports:?}");
    assert!(exports.contains_key("shape_b"), "expected export 'shape_b', got: {exports:?}");
}

// ── Fixture 2: state sprites ─────────────────────────────────────────────────

#[test]
fn state_sprites_fixture_parses_without_error() {
    let bytes = swf_helpers::make_state_sprites_swf();
    SwfAssetLibrary::new(bytes).expect("make_state_sprites_swf must produce a valid SWF");
}

#[test]
fn state_sprites_fixture_exports_state_a_and_state_b() {
    let bytes = swf_helpers::make_state_sprites_swf();
    let exports = extract_exported_symbols(&bytes).expect("extract_exported_symbols");
    assert!(exports.contains_key("StateA"), "expected export 'StateA', got: {exports:?}");
    assert!(exports.contains_key("StateB"), "expected export 'StateB', got: {exports:?}");
}

#[test]
fn state_sprites_fixture_has_two_define_sprites() {
    let bytes = swf_helpers::make_state_sprites_swf();
    let buf = swf::decompress_swf(std::io::Cursor::new(&bytes)).expect("decompress");
    let parsed = swf::parse_swf(&buf).expect("parse");

    let sprite_ids: Vec<CharacterId> = parsed
        .tags
        .iter()
        .filter_map(|t| {
            if let Tag::DefineSprite(s) = t { Some(s.id) } else { None }
        })
        .collect();

    assert_eq!(sprite_ids.len(), 2, "expected 2 DefineSprite tags, got: {sprite_ids:?}");
}

#[test]
fn state_sprites_fixture_has_import_assets_font_reference() {
    let bytes = swf_helpers::make_state_sprites_swf();
    let buf = swf::decompress_swf(std::io::Cursor::new(&bytes)).expect("decompress");
    let parsed = swf::parse_swf(&buf).expect("parse");

    let imported: Vec<String> = parsed
        .tags
        .iter()
        .flat_map(|t| {
            if let Tag::ImportAssets { imports, .. } = t {
                imports
                    .iter()
                    .map(|imp| imp.name.to_string_lossy(swf::UTF_8))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        })
        .collect();

    assert!(
        imported.contains(&"$Furore".to_string()),
        "expected ImportAssets '$Furore', got: {imported:?}"
    );
    assert!(
        imported.contains(&"$OrbitronLight".to_string()),
        "expected ImportAssets '$OrbitronLight', got: {imported:?}"
    );
}

#[test]
fn state_sprites_fixture_has_define_text_record() {
    let bytes = swf_helpers::make_state_sprites_swf();
    let buf = swf::decompress_swf(std::io::Cursor::new(&bytes)).expect("decompress");
    let parsed = swf::parse_swf(&buf).expect("parse");

    let define_text_count = parsed
        .tags
        .iter()
        .filter(|t| matches!(t, Tag::DefineText(_) | Tag::DefineText2(_)))
        .count();

    assert_eq!(define_text_count, 1, "expected exactly 1 DefineText tag");
}

#[test]
fn state_sprites_fixture_has_html_edit_text_with_loc_key() {
    let bytes = swf_helpers::make_state_sprites_swf();
    let buf = swf::decompress_swf(std::io::Cursor::new(&bytes)).expect("decompress");
    let parsed = swf::parse_swf(&buf).expect("parse");

    let edit_text = parsed.tags.iter().find_map(|t| {
        if let Tag::DefineEditText(e) = t { Some(e) } else { None }
    });

    let edit = edit_text.expect("expected a DefineEditText tag in state_sprites fixture");
    assert!(edit.is_html(), "DefineEditText must have HTML flag set");

    let initial = edit
        .initial_text()
        .map(|s| s.to_string_lossy(swf::UTF_8))
        .unwrap_or_default();

    assert!(
        initial.contains("$Furore"),
        "HTML initial_text must reference $Furore, got: {initial:?}"
    );
    assert!(
        initial.contains("@hud_NoTarget"),
        "HTML initial_text must contain @hud_NoTarget loc key, got: {initial:?}"
    );
}

// ── Coverage assertion test (Phase 0.3) ──────────────────────────────────────

/// Render `"StateA"` (a DefineSprite whose child is a red rectangle) via the
/// production `draw_swf_symbol` path and verify that actual pixels were drawn.
/// This locks in the sprite→shape rendering path and gives later phases a
/// reference helper they can call with new fixture symbols.
#[test]
fn state_a_sprite_renders_with_non_empty_pixel_coverage() {
    let bytes = swf_helpers::make_state_sprites_swf();
    swf_helpers::assert_swf_symbol_has_non_empty_coverage(&bytes, "StateA", 200, 200);
}

/// Same check for `"StateB"` (blue rectangle in the second sprite).
#[test]
fn state_b_sprite_renders_with_non_empty_pixel_coverage() {
    let bytes = swf_helpers::make_state_sprites_swf();
    swf_helpers::assert_swf_symbol_has_non_empty_coverage(&bytes, "StateB", 200, 200);
}

/// Shape exported directly (not wrapped in a sprite) also renders correctly.
#[test]
fn shape_a_from_labeled_frames_renders_with_non_empty_pixel_coverage() {
    let bytes = swf_helpers::make_labeled_frames_swf();
    swf_helpers::assert_swf_symbol_has_non_empty_coverage(&bytes, "shape_a", 200, 200);
}
