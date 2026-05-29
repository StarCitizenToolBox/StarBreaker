//! SWF candidate manifest building and selection.

use std::collections::{BTreeMap, HashSet};

use crate::canvas::{CanvasRecord, ResolvedCanvas, SceneItem};
use crate::swf_assets::SwfAssetLibrary;

use super::flash_paths::{flash_swf_candidates, flash_swf_candidates_from_canvas_refs};
use super::super::SwfFetcher;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwfPathCandidate {
    pub path: String,
    pub reason: &'static str,
    pub rank: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwfSelectionManifest {
    pub flash_candidates: Vec<SwfPathCandidate>,
    pub resolved_scene_candidates: Vec<SwfPathCandidate>,
    pub ordered_candidates: Vec<SwfPathCandidate>,
    pub valid_candidates: Vec<SwfPathCandidate>,
    pub fallback_counters: BTreeMap<String, u32>,
}

pub(crate) fn build_swf_selection_manifest(
    raw_root_json: &serde_json::Value,
    resolved: &ResolvedCanvas,
    manufacturer_id: &str,
    fetcher: &dyn SwfFetcher,
) -> SwfSelectionManifest {
    let flash_candidates = if canvas_has_flash_renderer(raw_root_json) {
        let mut source_candidates = flash_swf_candidates_from_canvas_refs(raw_root_json, manufacturer_id);
        let record_name = canvas_record_name(raw_root_json).unwrap_or("");
        source_candidates.extend(flash_swf_candidates(record_name, manufacturer_id));
        let deduped = source_candidates
            .into_iter()
            .fold(Vec::<String>::new(), |mut acc, path| {
                if !acc.iter().any(|existing| existing.eq_ignore_ascii_case(&path)) {
                    acc.push(path);
                }
                acc
            });
        deduped
            .into_iter()
            .enumerate()
            .map(|(index, path)| SwfPathCandidate {
                path,
                reason: "flash_structural_candidate",
                rank: index as u32,
            })
            .collect()
    } else {
        Vec::new()
    };

    let resolved_scene_candidates = collect_swf_paths(resolved)
        .into_iter()
        .enumerate()
        .map(|(index, path)| SwfPathCandidate {
            path,
            reason: "resolved_scene_reference",
            rank: 1000 + index as u32,
        })
        .collect::<Vec<_>>();
    let ordered_candidates = merge_unique_candidates(&flash_candidates, &resolved_scene_candidates);

    let mut valid_candidates = Vec::new();
    for candidate in &ordered_candidates {
        let Ok(bytes) = fetcher.fetch_swf_bytes(&candidate.path) else {
            continue;
        };
        if SwfAssetLibrary::new(bytes).is_ok() {
            valid_candidates.push(candidate.clone());
            break;
        }
    }

    let mut fallback_counters = BTreeMap::new();
    if let Some(selected) = valid_candidates.first() {
        if selected.reason == "resolved_scene_reference" {
            fallback_counters.insert("swf_resolved_scene_fallback".to_string(), 1);
        }
    } else if !ordered_candidates.is_empty() {
        fallback_counters.insert("swf_candidate_miss".to_string(), 1);
    }

    SwfSelectionManifest {
        flash_candidates,
        resolved_scene_candidates,
        ordered_candidates,
        valid_candidates,
        fallback_counters,
    }
}

pub(super) fn merge_unique_candidates(
    primary: &[SwfPathCandidate],
    secondary: &[SwfPathCandidate],
) -> Vec<SwfPathCandidate> {
    let mut out = Vec::with_capacity(primary.len() + secondary.len());
    let mut seen = HashSet::new();
    for candidate in primary.iter().chain(secondary.iter()) {
        if seen.insert(candidate.path.clone()) {
            out.push(candidate.clone());
        }
    }
    out
}

fn collect_swf_paths(canvas: &ResolvedCanvas) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    collect_from_record(&canvas.root, &mut seen, &mut paths);
    for child in canvas.children.values() {
        collect_from_record(child, &mut seen, &mut paths);
    }
    paths
}

fn collect_from_record(record: &CanvasRecord, seen: &mut HashSet<String>, paths: &mut Vec<String>) {
    for item in &record.scene {
        collect_from_scene_item(item, seen, paths);
    }
}

fn collect_from_scene_item(item: &SceneItem, seen: &mut HashSet<String>, paths: &mut Vec<String>) {
    let suffix = item.kind.strip_prefix("BuildingBlocks_").unwrap_or(&item.kind);
    match suffix {
        "Sprite" | "WidgetSprite" | "SpriteInstance" | "MovieClip" => {
            let path = item
                .properties
                .get("swfPath")
                .or_else(|| item.properties.get("swf"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !path.is_empty() && seen.insert(path.clone()) {
                paths.push(path);
            }
        }
        _ => {}
    }
}

fn canvas_record_name(root_json: &serde_json::Value) -> Option<&str> {
    root_json.get("_RecordName_")?.as_str()
}

fn canvas_has_flash_renderer(root_json: &serde_json::Value) -> bool {
    let Some(rv) = root_json.get("_RecordValue_") else {
        return false;
    };
    let Some(scene) = rv.get("scene") else {
        return false;
    };
    let Some(items) = scene.as_array() else {
        return false;
    };
    items.iter().any(|item| {
        item.get("rendererType")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("Flash"))
    })
}
