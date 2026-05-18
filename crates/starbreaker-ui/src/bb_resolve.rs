//! BuildingBlocks canvas graph resolver.
//!
//! Two-pass drill-down from a root `BuildingBlocks_Canvas` JSON record:
//!
//! **Pass 1** — `defaultStyles` / `brandStyles` canvas-reference modifiers.
//! Walks `defaultStyles.entries[]` (or a matching `brandStyles[]` entry) and
//! fetches any `CanvasReferenceRecord`-typed modifier values.
//!
//! **Pass 2** — `WidgetCanvas.canvas` field.
//! Some host canvases (e.g. `M_MFD_Screen`) have an empty `defaultStyles` and
//! carry all their content via a `BuildingBlocks_WidgetCanvas` node whose
//! `canvas` field is a `file://` URL pointing to the real content canvas.
//! Pass 2 follows those references one level deep so the merged scene includes
//! the content canvas nodes.  Recursion is capped at depth 1 to prevent cycles.

use std::collections::HashSet;

use crate::bb_scene::{BbNodeId, BbNodeType, BbScene, parse_bb_canvas};
use crate::pipeline::extract_record_name;

/// Maximum depth for `WidgetCanvas.canvas` recursion.
const MAX_WIDGET_CANVAS_DEPTH: u8 = 1;

/// Parse `root_json`, fetch active style-referenced child canvases, and merge them.
///
/// `manufacturer_id` selects a matching `brandStyles[]` entry when present;
/// otherwise, or when no brand matches, `defaultStyles.entries[]` are used.
/// Individual child fetch or parse failures are logged and skipped so a partial
/// scene can still be inspected.
pub fn resolve_canvas_graph(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Result<BbScene, String> {
    resolve_canvas_graph_depth(root_json, manufacturer_id, fetch_by_path, 0)
}

fn resolve_canvas_graph_depth(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    depth: u8,
) -> Result<BbScene, String> {
    let mut scene = parse_bb_canvas(root_json)?;
    let record_value = root_json.get("_RecordValue_").ok_or("missing _RecordValue_")?;

    // Pass 1: follow defaultStyles / brandStyles canvas-reference modifiers.
    for entry in pick_active_entries(record_value, manufacturer_id) {
        let match_to = entry.get("matchTo").and_then(|v| v.as_str()).unwrap_or("");
        let Some(modifiers) = entry.get("modifiers").and_then(|v| v.as_array()) else {
            continue;
        };

        for modifier in modifiers {
            let Some(field) = modifier.get("field") else {
                continue;
            };
            let is_canvas_ref = field
                .get("_Type_")
                .and_then(|v| v.as_str())
                == Some("BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord");
            if !is_canvas_ref {
                continue;
            }
            let Some(path) = field.get("value").and_then(|v| v.as_str()) else {
                continue;
            };

            let child_json = match fetch_by_path(path) {
                Ok(json) => json,
                Err(e) => {
                    log::warn!("bb_resolve: failed to fetch child canvas '{}': {}", path, e);
                    continue;
                }
            };
            let child_scene = match parse_bb_canvas(&child_json) {
                Ok(scene) => scene,
                Err(e) => {
                    log::warn!("bb_resolve: failed to parse child canvas '{}': {}", path, e);
                    continue;
                }
            };
            merge_child_scene(&mut scene, child_scene, match_to);
        }
    }

    // Pass 2: follow WidgetCanvas.canvas field references (depth-limited).
    //
    // Host canvases such as M_MFD_Screen store their content canvas URL in the
    // `canvas` field of a `BuildingBlocks_WidgetCanvas` scene node rather than
    // in `defaultStyles.entries`.  We fetch and resolve each such URL so the
    // merged scene captures the full content hierarchy.
    if depth < MAX_WIDGET_CANVAS_DEPTH {
        let canvas_urls: Vec<String> = scene
            .nodes
            .values()
            .filter(|n| n.ty == BbNodeType::WidgetCanvas)
            .filter_map(|n| n.raw.get("canvas").and_then(|v| v.as_str()))
            .filter(|url| !url.is_empty())
            .map(|url| url.to_owned())
            .collect();

        let mut seen_urls: HashSet<String> = HashSet::new();
        for url in canvas_urls {
            if !seen_urls.insert(url.clone()) {
                continue;
            }
            let child_json = match fetch_by_path(&url) {
                Ok(json) => json,
                Err(e) => {
                    log::warn!(
                        "bb_resolve: failed to fetch WidgetCanvas canvas '{}': {}",
                        url,
                        e
                    );
                    continue;
                }
            };
            let child_scene = match resolve_canvas_graph_depth(
                &child_json,
                manufacturer_id,
                fetch_by_path,
                depth + 1,
            ) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!(
                        "bb_resolve: failed to resolve WidgetCanvas content '{}': {}",
                        url,
                        e
                    );
                    continue;
                }
            };
            merge_child_scene(&mut scene, child_scene, "");
        }
    }

    Ok(scene)
}

fn pick_active_entries<'a>(
    record_value: &'a serde_json::Value,
    manufacturer_id: Option<&str>,
) -> Vec<&'a serde_json::Value> {
    if let Some(mfr) = manufacturer_id {
        let prefix = format!("s_{}_", mfr.to_ascii_lowercase());
        if let Some(brand_styles) = record_value.get("brandStyles").and_then(|v| v.as_array()) {
            for brand in brand_styles {
                let brand_base = brand
                    .get("brandIdentifier")
                    .and_then(|v| v.as_str())
                    .map(extract_record_name)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if brand_base.starts_with(&prefix) {
                    return entries_from(brand);
                }
            }
        }
    }

    record_value
        .get("defaultStyles")
        .map(entries_from)
        .unwrap_or_default()
}

fn entries_from(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    value
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|entries| entries.iter().collect())
        .unwrap_or_default()
}

fn merge_child_scene(parent_scene: &mut BbScene, child_scene: BbScene, match_to: &str) {
    let offset = parent_scene
        .nodes
        .keys()
        .next_back()
        .copied()
        .unwrap_or(0)
        .saturating_add(1);

    let BbScene { roots: child_roots, nodes: child_nodes, .. } = child_scene;
    let child_roots_reided: Vec<BbNodeId> = child_roots
        .iter()
        .map(|&id| id.saturating_add(offset))
        .collect();

    let Some(host_parent_id) = find_host_parent(parent_scene, match_to) else {
        log::warn!("bb_resolve: no host parent available for child canvas merge");
        return;
    };

    let mut inserted_child_roots = Vec::new();
    for (_, mut node) in child_nodes {
        let original_id = node.id;
        node.id = node.id.saturating_add(offset);
        node.parent = node.parent.map(|parent| parent.saturating_add(offset));
        node.children = node
            .children
            .into_iter()
            .map(|child| child.saturating_add(offset))
            .collect();

        if child_roots.contains(&original_id) {
            node.parent = Some(host_parent_id);
            inserted_child_roots.push(node.id);
        }

        if parent_scene.nodes.contains_key(&node.id) {
            log::warn!("bb_resolve: skipping child node id collision at {}", node.id);
            continue;
        }
        parent_scene.nodes.insert(node.id, node);
    }

    if let Some(host) = parent_scene.nodes.get_mut(&host_parent_id) {
        let roots_to_add = if inserted_child_roots.is_empty() {
            child_roots_reided
        } else {
            inserted_child_roots
        };
        host.children.extend(roots_to_add);
        host.children.sort_unstable();
        host.children.dedup();
    }
}

fn find_host_parent(parent_scene: &BbScene, match_to: &str) -> Option<BbNodeId> {
    if !match_to.is_empty() {
        for root_id in &parent_scene.roots {
            let Some(root) = parent_scene.nodes.get(root_id) else {
                continue;
            };
            for child_id in &root.children {
                let Some(child) = parent_scene.nodes.get(child_id) else {
                    continue;
                };
                if child.name.eq_ignore_ascii_case(match_to) {
                    return Some(*child_id);
                }
            }
        }
    }

    parent_scene.roots.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::extract_record_name;

    #[test]
    fn extract_record_name_long_file_url_uses_basename_without_json() {
        assert_eq!(
            extract_record_name("file://./a/b/c/gen_mc_s_target.json"),
            "gen_mc_s_target"
        );
    }

    #[test]
    fn extract_record_name_short_file_url_uses_local_basename() {
        assert_eq!(extract_record_name("file://./local.json"), "local");
    }

    #[test]
    fn extract_record_name_bare_json_name_strips_extension() {
        assert_eq!(extract_record_name("my_canvas.json"), "my_canvas");
    }

    #[test]
    fn extract_record_name_bare_name_is_unchanged() {
        assert_eq!(extract_record_name("my_canvas"), "my_canvas");
    }

    #[test]
    fn extract_record_name_fixture_path_returns_target_name() {
        assert_eq!(
            extract_record_name("file://./../../../../../../../../../../../libs/foundry/records/ui/buildingblocks/ships/displays/mfdscreens/mc_mfdcomponents/screens/target/types/gen_mc_s_target.json"),
            "gen_mc_s_target"
        );
    }

    #[test]
    fn extract_record_name_mixed_case_json_extension_strips_extension() {
        assert_eq!(extract_record_name("file://./local.Json"), "local");
    }

    #[test]
    fn resolve_with_rsi_manufacturer_fetches_brand_child() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let child = one_node_canvas();
        let scene = resolve_canvas_graph(&json, Some("rsi"), &|_p| Ok(child.clone()))
            .expect("resolve failed");
        assert!(
            scene.nodes.len() > 2,
            "expected >2 nodes after merge, got {}",
            scene.nodes.len()
        );
    }

    #[test]
    fn resolve_with_no_manufacturer_uses_default_styles() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let child = three_node_canvas();
        let scene = resolve_canvas_graph(&json, None, &|_p| Ok(child.clone()))
            .expect("resolve failed");
        assert!(
            scene.nodes.len() >= 5,
            "expected >=5 nodes after merge, got {}",
            scene.nodes.len()
        );
    }

    #[test]
    fn resolve_with_error_fetcher_does_not_panic() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = resolve_canvas_graph(&json, None, &|_p| Err("stub error".to_string()))
            .expect("resolve must not fail even when fetcher errors");
        assert!(
            scene.nodes.len() >= 2,
            "expected at least the 2 original nodes, got {}",
            scene.nodes.len()
        );
    }

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/fixtures/canvas/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {name}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("failed to parse {name}: {e}"))
    }

    fn one_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child",
            "_RecordId_": "00000000-0000-0000-0000-000000000001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"}
                ]
            }
        })
    }

    fn three_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child3",
            "_RecordId_": "00000000-0000-0000-0000-000000000002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        })
    }

    /// A host canvas (like M_MFD_Screen) with empty defaultStyles but a
    /// WidgetCanvas node that carries a `canvas` URL to a content canvas.
    fn host_canvas_with_widget_canvas_url(content_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_host",
            "_RecordId_": "00000000-0000-0000-0000-000000000010",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": []
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_content",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": content_url
                    }
                ]
            }
        })
    }

    #[test]
    fn resolve_follows_widget_canvas_canvas_field() {
        let content_url = "file://./content_canvas.json";
        let host = host_canvas_with_widget_canvas_url(content_url);

        // Content canvas has 5 nodes
        let content = serde_json::json!({
            "_RecordName_": "content_canvas",
            "_RecordId_": "00000000-0000-0000-0000-000000000011",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "content_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetIcon", "name": "icon1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetIcon", "name": "icon2", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|url| {
            assert_eq!(url, content_url, "fetcher called with unexpected url");
            Ok(content.clone())
        })
        .expect("resolve failed");

        // host (2) + content (5) = 7 nodes
        assert_eq!(
            scene.nodes.len(),
            7,
            "expected 7 merged nodes (2 host + 5 content), got {}",
            scene.nodes.len()
        );
    }

    #[test]
    fn resolve_does_not_follow_widget_canvas_url_at_depth_1() {
        // A content canvas that itself has a WidgetCanvas.canvas URL.
        // At depth=1 the recursion must not follow it further.
        let inner_content_url = "file://./inner.json";
        let content_with_nested_widget_canvas = serde_json::json!({
            "_RecordName_": "content_nested",
            "_RecordId_": "00000000-0000-0000-0000-000000000012",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "nested",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": inner_content_url
                    }
                ]
            }
        });

        let host = host_canvas_with_widget_canvas_url("file://./content_nested.json");

        let mut fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&host, None, &|_url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(content_with_nested_widget_canvas.clone())
        })
        .expect("resolve failed");

        // The inner_content_url must NOT be followed (depth cap).
        // Only one fetch should happen (the content_nested.json canvas).
        assert_eq!(fetch_count.get(), 1, "expected exactly 1 fetch, got {}", fetch_count.get());
        // host (2) + content_nested (2) = 4 nodes
        assert_eq!(
            scene.nodes.len(),
            4,
            "expected 4 merged nodes, got {}",
            scene.nodes.len()
        );
    }
}
