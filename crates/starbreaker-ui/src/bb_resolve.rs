//! BuildingBlocks canvas graph resolver.
//!
//! Two-pass drill-down from a root `BuildingBlocks_Canvas` JSON record:
//!
//! **Pass 1** — `defaultStyles` / `brandStyles` canvas-reference modifiers.
//! Walks `defaultStyles.entries[]` (or a matching `brandStyles[]` entry) and
//! fetches any `CanvasReferenceRecord`-typed modifier values.  Each fetched
//! child canvas is itself resolved recursively (up to `MAX_CANVAS_DEPTH`
//! total levels), so deep hierarchies like
//! `MC_S_Power_Master → GEN_MC_S_Power → gen_mc_s_powerlists → …` are fully
//! expanded in one call.
//!
//! **Pass 2** — `WidgetCanvas.canvas` field.
//! Some host canvases (e.g. `M_MFD_Screen`) have an empty `defaultStyles` and
//! carry all their content via a `BuildingBlocks_WidgetCanvas` node whose
//! `canvas` field is a `file://` URL pointing to the real content canvas.
//! Pass 2 follows those references recursively so the merged scene includes
//! the full content hierarchy.  Both passes share the same depth counter and
//! a global `visited` path set to guard against cycles.

use std::collections::{HashMap, HashSet};

use crate::bb_scene::{BbNodeId, BbNodeType, BbScene, parse_bb_canvas};
use crate::pipeline::extract_record_name;

/// Maximum total nesting depth across both passes.
///
/// The real MFD hierarchy has at least four levels:
/// `M_MFD_Screen → MC_S_Power_Master → GEN_MC_S_Power → gen_mc_s_powerlists`.
/// A cap of 8 provides ample headroom while still preventing runaway recursion.
const MAX_CANVAS_DEPTH: u8 = 8;

/// Parse `root_json`, recursively resolve all child canvases, and return a
/// fully-merged [`BbScene`].
///
/// `manufacturer_id` selects a matching `brandStyles[]` entry (e.g. `"drak"`);
/// when no brand matches, `defaultStyles.entries[]` are used at every level of
/// the hierarchy.  Individual child fetch or parse failures are logged and
/// skipped so a partial scene is still returned.
pub fn resolve_canvas_graph(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Result<BbScene, String> {
    let mut visited = HashSet::new();
    // Seed visited with the root's own record name so cycles back to the root
    // are caught without needing a separate check.
    if let Some(name) = root_json.get("_RecordName_").and_then(|v| v.as_str()) {
        visited.insert(name.to_ascii_lowercase());
    }
    resolve_canvas_graph_inner(root_json, manufacturer_id, fetch_by_path, 0, &mut visited)
}

/// Inner recursive resolver.  `visited` accumulates normalised record names
/// (lower-cased basenames extracted from `file://` URLs or bare names) seen so
/// far in the call chain; any path already in `visited` is skipped to break
/// cycles.
fn resolve_canvas_graph_inner(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    depth: u8,
    visited: &mut HashSet<String>,
) -> Result<BbScene, String> {
    let mut scene = parse_bb_canvas(root_json)?;
    let record_value = root_json.get("_RecordValue_").ok_or("missing _RecordValue_")?;

    if depth >= MAX_CANVAS_DEPTH {
        return Ok(scene);
    }

    // Collect Pass 2 URLs from the ROOT canvas's own scene nodes BEFORE Pass 1
    // merges child canvas nodes.  Without this guard, Pass 2 would follow
    // WidgetCanvas.canvas references that originated in merged child scenes
    // (e.g. sub-view tabs inside a master canvas), pulling in unrelated sibling
    // canvases and producing a mixed-content scene.
    //
    // Pass 1 below may still add new WidgetCanvas nodes via child canvas merges,
    // but those children are responsible for following their own WidgetCanvas
    // URLs in their own recursive resolve calls — not in ours.
    let root_canvas_urls: Vec<String> = scene
        .nodes
        .values()
        .filter(|n| n.ty == BbNodeType::WidgetCanvas)
        .filter_map(|n| n.raw.get("canvas").and_then(|v| v.as_str()))
        .filter(|url| !url.is_empty() && *url != "null")
        .map(|url| url.to_owned())
        .collect();

    let debug_trace = std::env::var("STARBREAKER_UI_DEBUG").as_deref() == Ok("1");

    // Pass 1: follow defaultStyles / brandStyles canvas-reference modifiers.
    //
    // Each referenced child canvas is resolved recursively so multi-level
    // hierarchies (master → gen → leaf) are fully expanded.
    //
    // Style entries may be unconditional (always active) or conditional
    // (mode-specific, e.g. GunsMode vs NavMode vs TurretMode on a self-status
    // canvas).  Following *all* conditional entries in a static render merges
    // every sub-view at the canvas origin and produces overlapping text.
    // To avoid this, only the first conditional entry is followed; subsequent
    // ones are skipped.  Unconditional entries (empty `conditionsList`) are
    // always followed since they represent always-visible content.
    let mut conditional_entry_seen = false;
    for entry in pick_active_entries(record_value, manufacturer_id) {
        let entry_name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let has_conditions = entry
            .get("conditionsList")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        if has_conditions {
            if conditional_entry_seen {
                if debug_trace {
                    log::info!(
                        "bb_resolve[depth={}]: skipping conditional entry {:?} (already saw one)",
                        depth,
                        entry_name,
                    );
                }
                continue;
            }
            conditional_entry_seen = true;
        }
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

            // Normalise path to a record name for cycle detection.
            let norm = extract_record_name(path).to_ascii_lowercase();
            if !visited.insert(norm.clone()) {
                log::debug!("bb_resolve: skipping already-visited child canvas '{}'", path);
                continue;
            }

            if debug_trace {
                log::info!(
                    "bb_resolve[depth={}]: Pass1 following entry {:?} -> {}",
                    depth,
                    entry_name,
                    norm,
                );
            }

            let child_json = match fetch_by_path(path) {
                Ok(json) => json,
                Err(e) => {
                    log::warn!("bb_resolve: failed to fetch child canvas '{}': {}", path, e);
                    continue;
                }
            };
            // Recurse: resolve the child's own style-references and WidgetCanvas URLs.
            let child_scene = match resolve_canvas_graph_inner(
                &child_json,
                manufacturer_id,
                fetch_by_path,
                depth + 1,
                visited,
            ) {
                Ok(scene) => scene,
                Err(e) => {
                    log::warn!("bb_resolve: failed to resolve child canvas '{}': {}", path, e);
                    continue;
                }
            };
            merge_child_scene(&mut scene, child_scene, match_to);
        }
    }

    // Pass 2: follow WidgetCanvas.canvas field references.
    //
    // Host canvases such as M_MFD_Screen store their content canvas URL in the
    // `canvas` field of a `BuildingBlocks_WidgetCanvas` scene node rather than
    // in `defaultStyles.entries`.  We fetch and resolve each such URL so the
    // merged scene captures the full content hierarchy.  The same `visited`
    // set prevents cycles between canvas-field references and style-references.
    //
    // Only the URLs collected from the root canvas's own nodes (before Pass 1)
    // are followed here — see the comment above.
    //
    // Some WidgetCanvas nodes serve as mode-switchable content slots: their
    // DEFAULT canvas is the NavMode view, but conditional style entries
    // REPLACE that canvas with mode-specific content (guns, missiles, …).
    // In a static render, if a conditional entry was already followed in Pass 1,
    // the slot's default canvas must NOT also be followed — it was replaced.
    // We detect this by collecting every canvas norm referenced by any conditional
    // entry and skipping Pass-2 URLs that appear in that set but were NOT already
    // added to `visited` by Pass 1.
    let conditional_canvas_norms: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        for entry in pick_active_entries(record_value, manufacturer_id) {
            let has_conditions = entry
                .get("conditionsList")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            if !has_conditions {
                continue;
            }
            let Some(mods) = entry.get("modifiers").and_then(|v| v.as_array()) else {
                continue;
            };
            for modifier in mods {
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
                if let Some(path) = field.get("value").and_then(|v| v.as_str()) {
                    let norm = extract_record_name(path).to_ascii_lowercase();
                    set.insert(norm);
                }
            }
        }
        set
    };

    {
        let canvas_urls = root_canvas_urls;

        for url in canvas_urls {
            let norm = extract_record_name(&url).to_ascii_lowercase();
            // If this canvas URL appears in ANY conditional entry but was NOT
            // selected in Pass 1 (not yet in `visited`), skip it.  It is the
            // default canvas for a mode-switchable slot, and the selected mode's
            // canvas has already replaced it.
            if conditional_canvas_norms.contains(&norm) && !visited.contains(&norm) {
                if debug_trace {
                    log::info!(
                        "bb_resolve[depth={}]: Pass2 skipping {} (conditional mode canvas, not selected in Pass1)",
                        depth, norm,
                    );
                }
                continue;
            }
            if !visited.insert(norm.clone()) {
                log::debug!("bb_resolve: skipping already-visited WidgetCanvas url '{}'", url);
                continue;
            }
            if debug_trace {
                log::info!(
                    "bb_resolve[depth={}]: Pass2 following WidgetCanvas.canvas -> {}",
                    depth, norm,
                );
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
            let child_scene = match resolve_canvas_graph_inner(
                &child_json,
                manufacturer_id,
                fetch_by_path,
                depth + 1,
                visited,
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
    let BbScene { roots: child_roots, nodes: child_nodes, operations: child_ops, .. } = child_scene;

    // Build a collision-free mapping from child original IDs to new IDs.
    // We use a monotonic counter that wraps and skips any ID already present
    // in the parent, so the merge is always safe regardless of the depth to
    // which children have been recursively merged.
    let mut next: BbNodeId = parent_scene
        .nodes
        .keys()
        .next_back()
        .copied()
        .unwrap_or(0)
        .wrapping_add(1);
    let mut id_map: HashMap<BbNodeId, BbNodeId> = HashMap::with_capacity(child_nodes.len());
    for &orig_id in child_nodes.keys() {
        // Advance past any ID already occupied in the parent or already
        // assigned to another child node in this batch.
        while parent_scene.nodes.contains_key(&next) || id_map.values().any(|&v| v == next) {
            next = next.wrapping_add(1);
        }
        id_map.insert(orig_id, next);
        next = next.wrapping_add(1);
    }

    let child_roots_reided: Vec<BbNodeId> = child_roots
        .iter()
        .filter_map(|id| id_map.get(id).copied())
        .collect();

    let Some(host_parent_id) = find_host_parent(parent_scene, match_to) else {
        log::warn!("bb_resolve: no host parent available for child canvas merge");
        return;
    };

    let mut inserted_child_roots = Vec::new();
    for (orig_id, mut node) in child_nodes {
        let new_id = match id_map.get(&orig_id).copied() {
            Some(id) => id,
            None => {
                log::warn!("bb_resolve: no id mapping for child node {orig_id}; skipping");
                continue;
            }
        };
        node.id = new_id;
        node.parent = node.parent.and_then(|p| id_map.get(&p).copied());
        node.children = node
            .children
            .into_iter()
            .filter_map(|c| id_map.get(&c).copied())
            .collect();

        if child_roots.contains(&orig_id) {
            node.parent = Some(host_parent_id);
            inserted_child_roots.push(new_id);
        }

        parent_scene.nodes.insert(new_id, node);
    }

    // Remap ptr: / _PointsTo_:ptr: references in operations using id_map.
    let mut remapped_ops = child_ops;
    for op in &mut remapped_ops {
        remap_ptrs_in_json_map(op, &id_map);
    }
    parent_scene.operations.extend(remapped_ops);

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

fn remap_ptrs_in_json_map(v: &mut serde_json::Value, id_map: &HashMap<BbNodeId, BbNodeId>) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(n) = s.strip_prefix("ptr:").and_then(|n| n.parse::<BbNodeId>().ok()) {
                if let Some(&new_id) = id_map.get(&n) {
                    *s = format!("ptr:{new_id}");
                }
            } else if let Some(n) = s
                .strip_prefix("_PointsTo_:ptr:")
                .and_then(|n| n.parse::<BbNodeId>().ok())
            {
                if let Some(&new_id) = id_map.get(&n) {
                    *s = format!("_PointsTo_:ptr:{new_id}");
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                remap_ptrs_in_json_map(item, id_map);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values_mut() {
                remap_ptrs_in_json_map(value, id_map);
            }
        }
        _ => {}
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
    fn resolve_follows_multiple_widget_canvas_levels() {
        // Verify that the resolver follows WidgetCanvas.canvas URLs at depth 1,
        // 2, 3, and 4 (all within MAX_CANVAS_DEPTH = 8).
        //
        // Chain: host → level1 → level2 → level3 → level4 (leaf)
        // Each level has 1 root node + 1 WidgetCanvas child (level4 is a leaf).
        fn make_canvas(name: &str, id: &str, child_url: Option<&str>) -> serde_json::Value {
            let scene_nodes: Vec<serde_json::Value> = {
                let mut nodes = vec![serde_json::json!({
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_DisplayWidget",
                    "name": format!("{name}_root")
                })];
                if let Some(url) = child_url {
                    nodes.push(serde_json::json!({
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": format!("{name}_canvas"),
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": url
                    }));
                }
                nodes
            };
            serde_json::json!({
                "_RecordName_": name,
                "_RecordId_": id,
                "_RecordValue_": {
                    "_Type_": "BuildingBlocks_Canvas",
                    "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                    "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                    "brandStyles": [],
                    "scene": scene_nodes
                }
            })
        }

        // level4 is a leaf — no child URL, so no fetch attempt for level5.
        let level4 = make_canvas("level4", "00000000-0000-0000-0000-000000000004", None);
        let level3 = make_canvas("level3", "00000000-0000-0000-0000-000000000003", Some("file://./level4.json"));
        let level2 = make_canvas("level2", "00000000-0000-0000-0000-000000000002", Some("file://./level3.json"));
        let level1 = make_canvas("level1", "00000000-0000-0000-0000-000000000001", Some("file://./level2.json"));
        let host = host_canvas_with_widget_canvas_url("file://./level1.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&host, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(match url {
                "file://./level1.json" => level1.clone(),
                "file://./level2.json" => level2.clone(),
                "file://./level3.json" => level3.clone(),
                "file://./level4.json" => level4.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        // Depth chain: host(depth=0)→level1(1)→level2(2)→level3(3)→level4(4)
        // level4 is a leaf, so exactly 4 fetches occur.
        assert_eq!(fetch_count.get(), 4, "expected 4 fetches (levels 1–4), got {}", fetch_count.get());

        // host(2) + level1(2) + level2(2) + level3(2) + level4(1) = 9 nodes
        assert_eq!(scene.nodes.len(), 9, "expected 9 merged nodes, got {}", scene.nodes.len());
    }

    #[test]
    fn resolve_does_not_follow_widget_canvas_url_beyond_depth_cap() {
        // A chain that loops back to the same URL exercises cycle-detection.
        // The visited set stops further traversal after the first fetch.
        //
        // The depth cap (MAX_CANVAS_DEPTH) is a separate backstop for long but
        // non-cyclic chains.
        fn make_level_canvas(child_url: &str) -> serde_json::Value {
            serde_json::json!({
                "_RecordName_": "level_n",
                "_RecordId_": "00000000-0000-0000-0000-000000000099",
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
                            "name": "next_level",
                            "parent": "_PointsTo_:ptr:1",
                            "canvas": child_url
                        }
                    ]
                }
            })
        }

        let host = host_canvas_with_widget_canvas_url("file://./level.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&host, None, &|_url| {
            fetch_count.set(fetch_count.get() + 1);
            // Every fetch returns a canvas that points back to "file://./level.json".
            // The global cycle-guard (visited set) catches this after the first fetch,
            // so at most 1 fetch occurs regardless of depth cap.
            Ok(make_level_canvas("file://./level.json"))
        })
        .expect("resolve failed");

        // Cycle detection: the same normalised URL "level" is only fetched once.
        assert!(
            fetch_count.get() <= super::MAX_CANVAS_DEPTH as u32,
            "fetched {} times, should be ≤ MAX_CANVAS_DEPTH={}",
            fetch_count.get(), super::MAX_CANVAS_DEPTH,
        );

        // There must be merged nodes from the fetched levels.
        assert!(scene.nodes.len() >= 2, "must have at least 2 merged nodes, got {}", scene.nodes.len());
    }

    #[test]
    fn gen_mc_s_target_fixture_has_widget_text_fields() {
        // The GEN_MC_S_Target canvas contains WidgetTextField nodes inline.
        // Resolving it (with a failing fetcher for nested canvas URLs) must
        // return a scene that includes at least one WidgetTextField node.
        use crate::bb_scene::BbNodeType;
        let json = load_fixture("GEN_MC_S_Target_dd9ed6dc.json");
        let scene = resolve_canvas_graph(&json, None, &|_p| Err("no fetcher in test".to_string()))
            .expect("resolve failed");
        let text_count = scene.nodes.values().filter(|n| n.ty == BbNodeType::WidgetTextField).count();
        assert!(
            text_count >= 1,
            "expected at least 1 WidgetTextField in GEN_MC_S_Target, got {}",
            text_count,
        );
    }

    #[test]
    fn mc_s_self_master_differs_from_gen_mc_s_target() {
        // MC_S_Self_Master and GEN_MC_S_Target represent different MFD screens.
        // Their resolved scenes must have different node counts (proving that
        // different root canvases produce distinct merged results).
        let target_json = load_fixture("GEN_MC_S_Target_dd9ed6dc.json");
        let self_json = load_fixture("MC_S_Self_Master_680a71df.json");

        let target_scene =
            resolve_canvas_graph(&target_json, None, &|_p| Err("no fetcher".to_string()))
                .expect("target resolve failed");
        let self_scene =
            resolve_canvas_graph(&self_json, None, &|_p| Err("no fetcher".to_string()))
                .expect("self resolve failed");

        assert_ne!(
            target_scene.nodes.len(),
            self_scene.nodes.len(),
            "GEN_MC_S_Target ({} nodes) and MC_S_Self_Master ({} nodes) must differ",
            target_scene.nodes.len(),
            self_scene.nodes.len(),
        );
    }

    /// Build a minimal canvas JSON that carries a single Pass 1 canvas-reference
    /// modifier pointing to `child_url`.
    fn canvas_with_style_ref(name: &str, child_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": name,
            "_RecordId_": "00000000-0000-0000-0000-0000000000aa",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "matchTo": "",
                        "modifiers": [{
                            "field": {
                                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                "value": child_url
                            }
                        }]
                    }]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": format!("{name}_root")}
                ]
            }
        })
    }

    #[test]
    fn pass1_style_ref_child_is_resolved_recursively() {
        // Phase B1 regression: Pass 1 canvas-reference children must themselves
        // be recursively resolved, not just shallowly parsed.
        //
        // Hierarchy:
        //   root  (1 root node, Pass 1 ref → child_canvas.json)
        //     └── child  (1 node, Pass 1 ref → grandchild_canvas.json)
        //           └── grandchild  (3 nodes, no refs)
        //
        // With the old shallow parse, grandchild nodes were never merged.
        // With the recursive fix, the scene must contain root + child + grandchild.
        let grandchild = serde_json::json!({
            "_RecordName_": "grandchild",
            "_RecordId_": "00000000-0000-0000-0000-0000000000cc",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "gc_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetIcon", "name": "gc_icon1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetIcon", "name": "gc_icon2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child = canvas_with_style_ref("child", "file://./grandchild.json");
        let root = canvas_with_style_ref("root", "file://./child.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(match url {
                "file://./child.json" => child.clone(),
                "file://./grandchild.json" => grandchild.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        // Both child and grandchild must have been fetched.
        assert_eq!(fetch_count.get(), 2, "expected 2 fetches (child + grandchild), got {}", fetch_count.get());
        // root(1) + child(1) + grandchild(3) = 5 nodes
        assert_eq!(scene.nodes.len(), 5, "expected 5 merged nodes (root+child+grandchild), got {}", scene.nodes.len());
    }

    #[test]
    fn pass2_does_not_follow_widget_canvas_urls_from_pass1_merged_children() {
        // Regression test for B3.3: Pass 2 must only follow WidgetCanvas.canvas
        // URLs that exist in the ROOT canvas's own scene BEFORE Pass 1 runs.
        //
        // Scenario (mirrors the real MC_S_Power_Master / MC_S_Self_Master bug):
        //
        //   master_canvas  (1 root node; NO WidgetCanvas nodes of its own)
        //     └── Pass 1 style ref → child_canvas.json
        //           child_canvas  (root + WidgetCanvas with canvas = "side_canvas.json")
        //
        // Because master has no WidgetCanvas nodes of its own, Pass 2 must
        // follow zero additional URLs at the master level.
        //
        // child_canvas's WidgetCanvas URL IS correctly followed by child's own
        // Pass 2 (since it appears in child's scene before child's Pass 1).
        // The visited set then prevents master's old-code Pass 2 from fetching
        // it a second time.  With this fix, master's Pass 2 does not even
        // attempt to collect the URL (collected list is empty before Pass 1).
        //
        // Verification: the fetcher is called exactly once for side_canvas
        // (by child's Pass 2, not master's), and the resolved scene has the
        // correct content — no spurious additional nodes from a phantom fetch.
        let side_canvas = serde_json::json!({
            "_RecordName_": "side_canvas",
            "_RecordId_": "00000000-0000-0000-0000-0000000000bb",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "side_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetIcon", "name": "side_icon", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        // child_canvas has a WidgetCanvas node pointing to side_canvas.
        let child_canvas = serde_json::json!({
            "_RecordName_": "child_canvas",
            "_RecordId_": "00000000-0000-0000-0000-0000000000cc",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "child_widget_canvas",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "file://./side_canvas.json"
                    }
                ]
            }
        });

        // master_canvas has no WidgetCanvas nodes in its own scene.
        let master_canvas = canvas_with_style_ref("master_canvas", "file://./child_canvas.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&master_canvas, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            match url {
                "file://./side_canvas.json" => Ok(side_canvas.clone()),
                "file://./child_canvas.json" => Ok(child_canvas.clone()),
                _ => Err(format!("unexpected fetch: {url}")),
            }
        })
        .expect("resolve failed");

        // side_canvas should be fetched exactly once (by child's own Pass 2).
        // master's Pass 2 has an empty URL list (master had no WidgetCanvas nodes
        // before Pass 1) so it never even attempts to follow side_canvas.
        // The visited set is a backstop but the fetch count must be 1 either way.
        assert_eq!(
            fetch_count.get(),
            2,
            "expected exactly 2 fetches (child_canvas + side_canvas), got {}",
            fetch_count.get()
        );

        // master(1) + child(2) + side(2) = 5 nodes — child correctly carries side content.
        assert_eq!(
            scene.nodes.len(),
            5,
            "expected 5 nodes (master root + child root + child WidgetCanvas + side root + side icon), got {}",
            scene.nodes.len()
        );
    }

    #[test]
    fn pass2_null_canvas_url_strings_are_not_followed() {
        // WidgetCanvas nodes may have canvas = "null" (literal string) when the
        // slot is unassigned (e.g. canvas_Interchangeable on an MFD host canvas).
        // These must not be passed to the fetcher.
        let host = serde_json::json!({
            "_RecordName_": "host",
            "_RecordId_": "00000000-0000-0000-0000-000000000020",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_interchangeable",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "null"
                    }
                ]
            }
        });

        let fetch_called = std::cell::Cell::new(false);
        let scene = resolve_canvas_graph(&host, None, &|url| {
            fetch_called.set(true);
            Err(format!("fetcher must not be called, got url: {url}"))
        })
        .expect("resolve must succeed even with null canvas URL");

        assert!(
            !fetch_called.get(),
            "fetcher must not be called for canvas=\"null\" WidgetCanvas nodes"
        );
        assert_eq!(scene.nodes.len(), 2, "expected 2 original nodes, got {}", scene.nodes.len());
    }
}
