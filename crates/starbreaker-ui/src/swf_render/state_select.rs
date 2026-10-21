//! SWF state selection: identifies sample-data exports to suppress in static renders.
//!
//! The key function is `compute_sample_data_export_ids`, which scans the
//! exported sprites of a `SwfAssetLibrary` and returns those that should be
//! suppressed in static renders because all of their `EditText` content is
//! ActionScript-driven sample data (no `@`-prefixed loc keys, no static
//! literals). This implements the Phase 3.2 suppression rule generically from
//! SWF content — no ship/symbol name hard-coding.

use std::collections::HashSet;

use swf::CharacterId;

use crate::swf_assets::SwfAssetLibrary;

use super::edit_text::parse_swf_html;

/// Return the set of exported sprite CharacterIds whose EditText subtree
/// contains only AS-driven sample data.
///
/// Rule (data-driven, from §3.2 of the plan):
/// - A sprite with no EditText in its subtree is always-visible → NOT suppressed.
/// - A sprite with at least one `@`-key loc text in its subtree is a static
///   placeholder state → NOT suppressed.
/// - A sprite whose non-empty EditText fields contain no `@` keys is an
///   AS-driven live state → SUPPRESSED.
///
/// Called before `draw_swf_stage_with_state`; pass the result as `suppressed`.
pub fn compute_sample_data_export_ids(assets: &SwfAssetLibrary) -> HashSet<CharacterId> {
    let mut result = HashSet::new();
    let export_ids: Vec<CharacterId> = assets.visual_exports().collect();
    for char_id in export_ids {
        if is_sample_data_sprite(assets, char_id) {
            result.insert(char_id);
        }
    }
    result
}

/// Returns `true` when the sprite subtree has EditText but all non-empty
/// `initial_text` values are sample data (no `@`-key loc references).
fn is_sample_data_sprite(assets: &SwfAssetLibrary, char_id: CharacterId) -> bool {
    let mut visited = HashSet::new();
    let (has_et, has_loc) = collect_edit_text_stats(assets, char_id, &mut visited);
    has_et && !has_loc
}

/// Returns `(has_any_edit_text, has_any_loc_key)` for the subtree of `char_id`.
fn collect_edit_text_stats(
    assets: &SwfAssetLibrary,
    char_id: CharacterId,
    visited: &mut HashSet<CharacterId>,
) -> (bool, bool) {
    if !visited.insert(char_id) {
        return (false, false);
    }

    let mut has_et = false;
    let mut has_loc = false;

    if let Some(edit) = assets.get_edit_text(char_id) {
        has_et = true;
        let text = edit.initial_text.as_deref().unwrap_or("");
        if edit.is_html {
            let runs = parse_swf_html(text);
            if runs.iter().any(|r| r.is_loc_key()) {
                has_loc = true;
            }
        } else if text.starts_with('@') {
            has_loc = true;
        }
    }

    for place in assets.extract_sprite_first_frame(char_id) {
        let (child_et, child_loc) = collect_edit_text_stats(assets, place.character_id, visited);
        has_et |= child_et;
        has_loc |= child_loc;
    }

    (has_et, has_loc)
}
