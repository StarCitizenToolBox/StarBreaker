//! Unit tests for SWF candidate derivation and selection.

use std::collections::HashMap;

use crate::canvas::ResolvedCanvas;

use super::candidates::{
    SwfPathCandidate, build_swf_selection_manifest, merge_unique_candidates,
};
use super::flash_paths::flash_swf_candidates;
use crate::pipeline::SwfFetcher;

// ── Mock fetchers ─────────────────────────────────────────────────────────────

struct EmptyFetcher;

impl SwfFetcher for EmptyFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, crate::error::UiError> {
        Err(crate::error::UiError::RenderError("missing swf".to_string()))
    }
}

/// Fetcher whose `list_swf_dirs` returns a fixed map of prefix → subdirs.
struct MockDirFetcher {
    dirs: HashMap<String, Vec<String>>,
}

impl SwfFetcher for MockDirFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, crate::error::UiError> {
        Err(crate::error::UiError::RenderError("mock".to_string()))
    }

    fn list_swf_dirs(&self, prefix: &str) -> Vec<String> {
        self.dirs.get(prefix).cloned().unwrap_or_default()
    }
}

fn dra_root() -> &'static str {
    r"Data\UI\ShipInterface\assets\SWF\DRA\"
}

// ── Existing tests (updated for new fetcher parameter) ────────────────────────

#[test]
fn merge_unique_candidates_preserves_reason_and_rank_of_first_path() {
    let primary = vec![SwfPathCandidate {
        path: "A.swf".to_string(),
        reason: "primary",
        rank: 1,
    }];
    let secondary = vec![
        SwfPathCandidate {
            path: "A.swf".to_string(),
            reason: "secondary-duplicate",
            rank: 99,
        },
        SwfPathCandidate {
            path: "B.swf".to_string(),
            reason: "secondary",
            rank: 2,
        },
    ];

    let merged = merge_unique_candidates(&primary, &secondary);

    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].reason, "primary");
    assert_eq!(merged[0].rank, 1);
    assert_eq!(merged[1].path, "B.swf");
}

#[test]
fn swf_selection_manifest_contains_structural_flash_candidates() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.MC_S_Target_Master",
        "_RecordValue_": {
            "scene": [{"rendererType": "Flash"}]
        }
    });
    let resolved = ResolvedCanvas {
        root: crate::canvas::CanvasRecord {
            guid: "root-guid".to_string(),
            name: "Root".to_string(),
            views: Vec::new(),
            scene: Vec::new(),
            operations: Vec::new(),
        },
        children: std::collections::HashMap::new(),
    };
    let fetcher = EmptyFetcher;

    let manifest = build_swf_selection_manifest(&root, &resolved, "drak", &fetcher);

    assert!(!manifest.flash_candidates.is_empty());
    assert_eq!(manifest.ordered_candidates, manifest.flash_candidates);
    assert_eq!(manifest.flash_candidates[0].reason, "flash_structural_candidate");
    assert!(manifest.valid_candidates.is_empty());
    assert_eq!(manifest.fallback_counters.get("swf_candidate_miss"), Some(&1));
}

#[test]
fn flash_candidates_prefer_canvas_reference_stem_when_present() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.MC_S_MissionData_Master",
        "_RecordValue_": {
            "scene": [{"rendererType": "Flash"}],
            "defaultStyles": {
                "entries": [{
                    "modifiers": [{
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                            "value": "file://./types/gen_mc_s_target.json"
                        }
                    }]
                }]
            },
            "brandStyles": []
        }
    });
    let resolved = ResolvedCanvas {
        root: crate::canvas::CanvasRecord {
            guid: "root-guid".to_string(),
            name: "Root".to_string(),
            views: Vec::new(),
            scene: Vec::new(),
            operations: Vec::new(),
        },
        children: std::collections::HashMap::new(),
    };
    let fetcher = EmptyFetcher;

    let manifest = build_swf_selection_manifest(&root, &resolved, "rsi", &fetcher);

    assert!(manifest.flash_candidates[0]
        .path
        .to_ascii_lowercase()
        .ends_with("targetstatus.swf"));
    assert!(manifest
        .flash_candidates
        .iter()
        .any(|candidate| candidate.path.to_ascii_lowercase().ends_with("missiondatastatus.swf")));
}

#[test]
fn flash_candidates_cover_structural_supportscreen_variants() {
    // Brand-level screen-set dirs are always generated regardless of fetcher.
    let candidates = flash_swf_candidates(
        "BuildingBlocks_Canvas.MC_S_Target_Master",
        "aegs",
        &EmptyFetcher,
    );

    assert!(candidates.iter().any(|path| path.contains("SupportScreen16-9\\TargetStatus.swf")));
    assert!(candidates.iter().any(|path| path.contains("SupportScreen1-1\\TargetStatus.swf")));
    assert!(candidates.iter().any(|path| path.contains("SupportScreenBespoke2\\TargetStatus.swf")));
    assert!(candidates.iter().any(|path| path.contains("Support_Bespoke_2\\TargetStatus.swf")));
}

// ── Phase 1 tests: P4K-driven ship-subdir enumeration ────────────────────────

/// When no dirs are returned by the fetcher, no ship-subdir candidates are
/// generated (no hard-coded fallback).
#[test]
fn no_ship_dirs_produces_only_brand_level_candidates() {
    let candidates = flash_swf_candidates("MC_S_Target_Master", "drak", &EmptyFetcher);

    assert!(
        !candidates.is_empty(),
        "brand-level candidates must still be generated"
    );
    assert!(
        candidates.iter().all(|p| !p.contains("DRAK_")),
        "no ship-subdir candidates expected when fetcher returns empty dirs: {candidates:?}"
    );
}

/// Only dirs returned by the fetcher appear in candidates; unlisted ship dirs
/// (even if they were previously hard-coded) must not appear.
#[test]
fn p4k_enumeration_excludes_dirs_not_returned_by_fetcher() {
    let mut dirs = HashMap::new();
    // Only Buccaneer — Dragonfly and Caterpillar are absent
    dirs.insert(dra_root().to_string(), vec!["DRAK_Buccaneer".to_string()]);
    let fetcher = MockDirFetcher { dirs };

    let candidates = flash_swf_candidates("MC_S_Target_Master", "drak", &fetcher);

    assert!(
        candidates.iter().any(|p| p.contains("DRAK_Buccaneer")),
        "DRAK_Buccaneer must be in candidates: {candidates:?}"
    );
    assert!(
        candidates.iter().all(|p| !p.contains("DRAK_Dragonfly")),
        "DRAK_Dragonfly must not appear (not returned by fetcher): {candidates:?}"
    );
    assert!(
        candidates.iter().all(|p| !p.contains("DRAK_Caterpillar")),
        "DRAK_Caterpillar must not appear (not returned by fetcher): {candidates:?}"
    );
}

/// Ship-subdir candidates appear in lexicographic (alphabetical) order
/// regardless of the order returned by the fetcher.
#[test]
fn ship_dir_candidates_appear_in_alphabetical_order() {
    let mut dirs = HashMap::new();
    // Fetcher returns dirs in reverse alphabetical order
    dirs.insert(
        dra_root().to_string(),
        vec!["DRAK_Dragonfly".to_string(), "DRAK_Buccaneer".to_string()],
    );
    let fetcher = MockDirFetcher { dirs };

    let candidates = flash_swf_candidates("MC_S_Target_Master", "drak", &fetcher);

    let buck_pos = candidates
        .iter()
        .position(|p| p.contains("DRAK_Buccaneer") && p.to_lowercase().contains("targetstatus"));
    let drag_pos = candidates
        .iter()
        .position(|p| p.contains("DRAK_Dragonfly") && p.to_lowercase().contains("targetstatus"));

    match (buck_pos, drag_pos) {
        (Some(b), Some(d)) => assert!(
            b < d,
            "DRAK_Buccaneer ({b}) must appear before DRAK_Dragonfly ({d}) (alphabetical)"
        ),
        _ => panic!("expected both DRAK_Buccaneer and DRAK_Dragonfly in candidates: {candidates:?}"),
    }
}

/// Annunciator candidates include the correct halve file and enumerate
/// ship subdirs alphabetically, matching the expected DRA thin-strip SWF.
#[test]
fn annunciator_candidates_enumerate_ship_dirs_alphabetically() {
    let mut dirs = HashMap::new();
    // Fetcher returns in reverse order; result must be sorted
    dirs.insert(
        dra_root().to_string(),
        vec!["DRAK_Dragonfly".to_string(), "DRAK_Buccaneer".to_string()],
    );
    let fetcher = MockDirFetcher { dirs };

    let candidates = flash_swf_candidates("MC_S_Annunciator_Left_Master", "drak", &fetcher);

    assert!(
        candidates.iter().any(|p| p.ends_with("AnnunciatorHalve1.swf")),
        "expected AnnunciatorHalve1.swf in candidates: {candidates:?}"
    );

    let buck_pos = candidates
        .iter()
        .position(|p| p.contains("DRAK_Buccaneer") && p.ends_with("AnnunciatorHalve1.swf"));
    let drag_pos = candidates
        .iter()
        .position(|p| p.contains("DRAK_Dragonfly") && p.ends_with("AnnunciatorHalve1.swf"));

    match (buck_pos, drag_pos) {
        (Some(b), Some(d)) => assert!(
            b < d,
            "DRAK_Buccaneer ({b}) must appear before DRAK_Dragonfly ({d}) (alphabetical)"
        ),
        _ => panic!("expected both Buccaneer and Dragonfly annunciator candidates: {candidates:?}"),
    }
}

/// A new ship dir returned by the fetcher (not previously hard-coded) must
/// produce candidates — validates forward-compatibility with new ships.
#[test]
fn new_ship_dir_from_fetcher_produces_candidates() {
    let mut dirs = HashMap::new();
    dirs.insert(
        dra_root().to_string(),
        vec!["DRAK_NewShip".to_string()],
    );
    let fetcher = MockDirFetcher { dirs };

    let candidates = flash_swf_candidates("MC_S_Target_Master", "drak", &fetcher);

    assert!(
        candidates.iter().any(|p| p.contains("DRAK_NewShip")),
        "a new ship dir from the fetcher must appear in candidates: {candidates:?}"
    );
}

/// Empty or unknown manufacturer_id must return an empty list without panicking.
#[test]
fn flash_candidates_no_panic_on_empty_manufacturer() {
    let candidates = flash_swf_candidates("MC_S_Target_Master", "", &EmptyFetcher);
    assert!(candidates.is_empty());
}
