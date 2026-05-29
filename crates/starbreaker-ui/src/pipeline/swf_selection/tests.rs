use crate::canvas::ResolvedCanvas;

use super::candidates::{
    SwfPathCandidate, build_swf_selection_manifest, merge_unique_candidates,
};
use super::flash_paths::flash_swf_candidates;
use crate::pipeline::SwfFetcher;

struct EmptyFetcher;

impl SwfFetcher for EmptyFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, crate::error::UiError> {
        Err(crate::error::UiError::RenderError("missing swf".to_string()))
    }
}

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
    let candidates = flash_swf_candidates("BuildingBlocks_Canvas.MC_S_Target_Master", "aegs");

    assert!(candidates.iter().any(|path| path.contains("SupportScreen16-9\\TargetStatus.swf")));
    assert!(candidates.iter().any(|path| path.contains("SupportScreen1-1\\TargetStatus.swf")));
    assert!(candidates.iter().any(|path| path.contains("SupportScreenBespoke2\\TargetStatus.swf")));
    assert!(candidates.iter().any(|path| path.contains("Support_Bespoke_2\\TargetStatus.swf")));
}
