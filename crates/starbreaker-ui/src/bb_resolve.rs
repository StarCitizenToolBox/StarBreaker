//! BuildingBlocks canvas graph resolver.
//!
//! Two-pass drill-down from a root `BuildingBlocks_Canvas` JSON record:
//!
//! **Pass 1** — `defaultStyles` / `brandStyles` canvas-reference modifiers.
//! Selects the *default-state* entry from `defaultStyles.entries[]` (or a
//! matching `brandStyles[]` entry) and fetches its
//! `CanvasReferenceRecord`-typed modifier value.  The default-state entry is:
//! 1. The first entry whose `conditionsList` is absent or empty
//!    (unconditional — always active regardless of game state).
//! 2. Failing that, the entry whose condition tag matches the scene node with
//!    the highest style-tag count ("most-tagged node" heuristic — the primary
//!    content slot carries more tags than sub-component slots).
//! 3. Last resort: the first entry overall (used when no scene-node heuristic
//!    applies, e.g. `MC_S_Target_Master` which has exactly one conditional
//!    entry that is effectively the default).
//!
//! Selecting a single entry prevents runtime mode-switching canvases (e.g.
//! `GunsMode`, `NavMode`, `TurretMode` on `MC_S_Self_Master`) from all being
//! merged into the static render at once.  Each fetched child canvas is
//! itself resolved recursively (up to `MAX_CANVAS_DEPTH` total levels), so
//! deep hierarchies like
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
//!
//! **Mode-switch guard** — `conditional_canvas_norms`.
//! All canvas URLs referenced by *any* conditional entry (regardless of which
//! entry Pass 1 selected) are collected into `conditional_canvas_norms`.
//! Pass 2 skips any URL in this set that was not visited by Pass 1, preventing
//! WidgetCanvas nodes that serve as mode-switchable slots (e.g. `canvas_NavMode`
//! pointing to `gen_mc_s_nav.json`) from being followed when a different mode
//! was selected in Pass 1.

use std::collections::{HashMap, HashSet};

use crate::bb_loc::LocFetcher;
use crate::bb_scene::{BbNodeId, BbNodeType, BbScene, BbValue, parse_bb_canvas};
use crate::bb_brand_style;
use crate::pipeline::extract_record_name;

fn is_linear_progress_meter(node: &crate::bb_scene::BbNode) -> bool {
    matches!(
        &node.ty,
        BbNodeType::Other(kind)
            if kind.eq_ignore_ascii_case("BuildingBlocks_WidgetLinearProgressMeter")
    )
}

fn modular_linearprogress_style_path(style_identifier: &str) -> Option<String> {
    let normalized = style_identifier.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let module_id = if let Some(rest) = normalized.strip_prefix("s_") {
        format!("sk_{}", rest)
    } else if normalized.starts_with("sk_") {
        normalized
    } else {
        return None;
    };

    Some(format!(
        "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/styles/modularkitstyles/{0}/{0}_linearprogressmeterstyles.json",
        module_id
    ))
}

fn modular_buttonsecondary_style_path(style_identifier: &str) -> Option<String> {
    let normalized = style_identifier.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let module_id = if let Some(rest) = normalized.strip_prefix("s_") {
        format!("sk_{}", rest)
    } else if normalized.starts_with("sk_") {
        normalized
    } else {
        return None;
    };

    Some(format!(
        "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/styles/modularkitstyles/{0}/{0}_buttonsecondarystyles.json",
        module_id
    ))
}

fn extract_rootghost_button_secondary_corner_radius(style_entries: &[serde_json::Value]) -> Option<f32> {
    let root_ghost_entry = style_entries.iter().find(|entry| {
        entry
            .get("name")
            .and_then(|v| v.as_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("RootGhost"))
    })?;

    let modifiers = root_ghost_entry.get("modifiers").and_then(|v| v.as_array())?;
    let radius_fields = [
        "BorderTopLeftRadius",
        "BorderTopRightRadius",
        "BorderBottomLeftRadius",
        "BorderBottomRightRadius",
    ];

    let mut radii = Vec::with_capacity(radius_fields.len());
    for field_name in radius_fields {
        let value = modifiers
            .iter()
            .find(|modifier| {
                modifier
                    .get("field")
                    .and_then(|field| field.as_str())
                    .is_some_and(|field| field.eq_ignore_ascii_case(field_name))
            })
            .and_then(|modifier| modifier.get("value"))
            .and_then(|value| value.as_f64())? as f32;
        radii.push(value);
    }

    let first = *radii.first()?;
    if first <= 0.0 || radii.iter().any(|radius| (*radius - first).abs() > f32::EPSILON) {
        return None;
    }

    Some(first)
}

fn set_uniform_border_radius(node: &mut crate::bb_scene::BbNode, radius: f32) {
    let Some(raw_obj) = node.raw.as_object_mut() else {
        return;
    };
    let border = raw_obj
        .entry("border".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(border_obj) = border.as_object_mut() else {
        return;
    };

    for corner in ["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"] {
        let corner_value = border_obj
            .entry(corner.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let Some(corner_obj) = corner_value.as_object_mut() else {
            continue;
        };

        let radius_value = corner_obj
            .entry("radius".to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let Some(radius_obj) = radius_value.as_object_mut() else {
            continue;
        };

        radius_obj.insert(
            "value".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(radius as f64).unwrap()),
        );
        radius_obj.insert(
            "behavior".to_string(),
            serde_json::Value::String("Fixed".to_string()),
        );
    }
}

fn apply_buttonsecondary_modular_styles(
    scene: &mut BbScene,
    style_entries: &[serde_json::Value],
) {
    let Some(radius) = extract_rootghost_button_secondary_corner_radius(style_entries) else {
        return;
    };

    for node in scene.nodes.values_mut() {
        if !matches!(node.ty, BbNodeType::ComponentGeneralButtonSecondary) {
            continue;
        }
        let is_ghost = node
            .raw
            .get("fillStyle")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("Ghost"));
        if !is_ghost {
            continue;
        }
        set_uniform_border_radius(node, radius);
    }
}

fn collect_style_condition_tags(
    condition: &serde_json::Value,
    out: &mut Vec<(String, Option<String>, Option<String>)>,
) {
    if let Some(tag) = condition.get("tag") {
        let tag_id = tag
            .get("_RecordId_")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| {
                tag.as_str().and_then(|s| {
                    s.strip_prefix("Tag.")
                        .map(str::to_owned)
                        .or_else(|| Some(s.to_owned()))
                })
            });

        if let Some(tag_id) = tag_id {
            let tag_record_name = tag
                .get("_RecordName_")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .or_else(|| Some(format!("Tag.{tag_id}")));
            let tag_record_path = tag
                .get("_RecordPath_")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            out.push((tag_id, tag_record_name, tag_record_path));
        }
    }

    if let Some(conditions) = condition.get("conditions").and_then(|v| v.as_array()) {
        for nested in conditions {
            collect_style_condition_tags(nested, out);
        }
    }
    if let Some(break_conditions) = condition.get("breakConditions").and_then(|v| v.as_array()) {
        for nested in break_conditions {
            collect_style_condition_tags(nested, out);
        }
    }
}

fn find_tag_name_in_tree(tags: &[serde_json::Value], tag_id: &str) -> Option<String> {
    for tag in tags {
        if tag
            .get("_RecordId_")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id.eq_ignore_ascii_case(tag_id))
        {
            return tag
                .get("tagName")
                .and_then(|v| v.as_str())
                .map(|name| name.to_ascii_lowercase());
        }

        if let Some(children) = tag.get("children").and_then(|v| v.as_array()) {
            if let Some(name) = find_tag_name_in_tree(children, tag_id) {
                return Some(name);
            }
        }
    }
    None
}

fn resolve_tag_name(
    tag_id: &str,
    tag_record_name: Option<&str>,
    tag_record_path: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Option<String> {
    if let Some(record_name) = tag_record_name {
        if let Ok(tag_record) = fetch_by_path(record_name) {
            if let Some(tag_name) = tag_record
                .get("_RecordValue_")
                .and_then(|v| v.get("tagName"))
                .and_then(|v| v.as_str())
            {
                return Some(tag_name.to_ascii_lowercase());
            }
        }
    }

    if let Some(record_path) = tag_record_path {
        if let Ok(tag_database_record) = fetch_by_path(record_path) {
            if let Some(tags) = tag_database_record
                .get("_RecordValue_")
                .and_then(|v| v.get("tags"))
                .and_then(|v| v.as_array())
            {
                return find_tag_name_in_tree(tags, tag_id);
            }
        }
    }

    None
}

fn seed_implicit_linearprogress_style_tags(
    scene: &mut BbScene,
    style_entries: &[serde_json::Value],
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) {
    let mut implicit_tags: Vec<(String, String)> = Vec::new();

    for entry in style_entries {
        let Some(condition_lists) = entry.get("conditionsList").and_then(|v| v.as_array()) else {
            continue;
        };
        for list in condition_lists {
            let Some(conditions) = list.get("conditions").and_then(|v| v.as_array()) else {
                continue;
            };
            for condition in conditions {
                let mut found = Vec::new();
                collect_style_condition_tags(condition, &mut found);
                for (tag_id, tag_record_name, tag_record_path) in found {
                    let tag_name = resolve_tag_name(
                        &tag_id,
                        tag_record_name.as_deref(),
                        tag_record_path.as_deref(),
                        fetch_by_path,
                    );
                    if let Some(tag_name) = tag_name {
                        implicit_tags.push((tag_id.clone(), tag_name));
                    }
                }
            }
        }
    }

    for (tag_id, tag_name) in implicit_tags {
        match tag_name.as_str() {
            "meter-element-instance" => {
                for node in scene.nodes.values_mut() {
                    if is_linear_progress_meter(node)
                        && !node.style_tag_uuids.iter().any(|id| id == &tag_id)
                    {
                        node.style_tag_uuids.push(tag_id.clone());
                    }
                }
            }
            "progress-meter-state-active" => {
                for node in scene.nodes.values_mut() {
                    let is_active_progress = is_linear_progress_meter(node)
                        && node
                            .raw
                            .get("progress")
                            .and_then(|v| v.as_f64())
                            .map(|v| v > 0.0)
                            .unwrap_or(false);
                    if is_active_progress && !node.style_tag_uuids.iter().any(|id| id == &tag_id)
                    {
                        node.style_tag_uuids.push(tag_id.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

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
///
/// This is a backwards-compatible wrapper around [`resolve_canvas_graph_with_loc`]
/// that passes `None` for the localization fetcher.
pub fn resolve_canvas_graph(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Result<BbScene, String> {
    resolve_canvas_graph_with_loc(root_json, manufacturer_id, fetch_by_path, None)
}

/// Like [`resolve_canvas_graph`] but accepts an optional localization fetcher.
///
/// When `loc_fetcher` is `Some`, brand-applied string modifier values that start
/// with `@` are resolved through the fetcher before being written to nodes.
pub fn resolve_canvas_graph_with_loc(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
) -> Result<BbScene, String> {
    let mut visited = HashSet::new();
    // Seed visited with the root's own record name so cycles back to the root
    // are caught without needing a separate check.
    if let Some(name) = root_json.get("_RecordName_").and_then(|v| v.as_str()) {
        visited.insert(name.to_ascii_lowercase());
    }
    resolve_canvas_graph_inner(
        root_json,
        manufacturer_id,
        fetch_by_path,
        loc_fetcher,
        0,
        &mut visited,
        None,
        None,
        None,
    )
}

/// Inner recursive resolver.  `visited` accumulates normalised record names
/// (lower-cased basenames extracted from `file://` URLs or bare names) seen so
/// far in the call chain; any path already in `visited` is skipped to break
/// cycles.
fn resolve_canvas_graph_inner(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
    depth: u8,
    visited: &mut HashSet<String>,
    inherited_style: Option<&serde_json::Value>,
    inherited_style_identifier: Option<&str>,
    inherited_param_inputs: Option<&[serde_json::Value]>,
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
    // Also capture paramInputValues so child scenes can inherit localized-param
    // overrides (e.g. annunciator chiclet labels PWR/WPN/THR/SHLD/COOL).
    let root_canvas_urls: Vec<(BbNodeId, String, Vec<serde_json::Value>)> = scene
        .nodes
        .values()
        .filter(|n| n.ty == BbNodeType::WidgetCanvas)
        .filter_map(|n| {
            let url = n.raw.get("canvas").and_then(|v| v.as_str())?;
            if url.is_empty() || url == "null" {
                return None;
            }
            let param_inputs: Vec<serde_json::Value> = n
                .raw
                .get("paramInputValues")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            Some((n.id, url.to_owned(), param_inputs))
        })
        .collect();

    let debug_trace = std::env::var("STARBREAKER_UI_DEBUG").as_deref() == Ok("1");

    let mut local_style_identifier = inherited_style_identifier.map(ToOwned::to_owned);
    let local_style_value = record_value.get("style").and_then(|style| {
        if let Some(style_url) = style.as_str().filter(|s| !s.is_empty()) {
            local_style_identifier = Some(extract_record_name(style_url));
            match fetch_by_path(style_url) {
                Ok(style_json) => Some(style_json.get("_RecordValue_").cloned().unwrap_or(style_json)),
                Err(e) => {
                    log::warn!(
                        "bb_resolve: failed to fetch canvas-level style record '{}': {}",
                        style_url,
                        e
                    );
                    None
                }
            }
        } else if style.is_object() && !style.is_null() {
            Some(style.clone())
        } else {
            None
        }
    });
    let palette_source = local_style_value.as_ref().or(inherited_style);

    // Pass 1: follow the default-state canvas-reference modifier.
    //
    // Only ONE style entry is selected (see `pick_default_entry`) to prevent
    // mode-switching canvases (e.g. GunsMode / NavMode / TurretMode on
    // MC_S_Self_Master) from all being merged together into the static render.
    //
    // The selected child canvas is resolved recursively so multi-level
    // hierarchies (master → gen → leaf) are fully expanded.
    if let Some(entry) = pick_default_entry(record_value, root_json, manufacturer_id) {
        let entry_name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let match_to = entry.get("matchTo").and_then(|v| v.as_str()).unwrap_or("");

        if let Some(modifiers) = entry.get("modifiers").and_then(|v| v.as_array()) {
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
                let match_to_param_inputs = if match_to.is_empty() {
                    None
                } else {
                    param_inputs_for_match_to(&scene, match_to)
                };
                // Recurse: resolve the child's own style-references and WidgetCanvas URLs.
                let mut child_scene = match resolve_canvas_graph_inner(
                    &child_json,
                    manufacturer_id,
                fetch_by_path,
                loc_fetcher,
                depth + 1,
                visited,
                palette_source,
                local_style_identifier.as_deref(),
                match_to_param_inputs.as_deref(),
            ) {
                    Ok(scene) => scene,
                    Err(e) => {
                        log::warn!(
                            "bb_resolve: failed to resolve child canvas '{}': {}",
                            path,
                            e
                        );
                        continue;
                    }
                };
                // Pass1 canvas-reference merges can also carry localized
                // paramInputValues on the matched host node (same mechanism as
                // Pass2 WidgetCanvas.canvas references). Inject those overrides
                // so localized component-parameter defaults resolve correctly.
                if let Some(param_inputs) = match_to_param_inputs.as_ref() {
                    inject_param_overrides(param_inputs, &mut child_scene);
                }
                merge_child_scene(&mut scene, child_scene, match_to, None);
            }
        }
    }

    // Pass 2: follow WidgetCanvas.canvas field references.
    //
    // Host canvases such as M_MFD_Screen store their content canvas URL in the
    // `canvas` field of a `BuildingBlocks_WidgetCanvas` scene node rather than
    // in `defaultStyles.entries`.  We fetch and resolve each such URL so the
    // merged scene captures the full content hierarchy.
    //
    // Only the URLs collected from the root canvas's own nodes (before Pass 1)
    // are followed here — see the comment above.
    //
    // Some WidgetCanvas nodes serve as mode-switchable content slots whose
    // DEFAULT canvas matches a canvas already followed by Pass 1.  To prevent
    // Pass 2 from fetching those canvases again, we collect every canvas norm
    // referenced by a conditional Pass-1 entry.
    //
    // **Snapshot semantics**: we capture `visited` immediately after Pass 1 so
    // we can distinguish "already merged by Pass 1" from "first time seen in
    // Pass 2".  The outer Pass 2 loop does NOT insert into `visited`, which
    // allows the same template URL to appear multiple times in `root_canvas_urls`
    // with different `paramInputValues` (e.g. the 5 chiclet slots in the
    // annunciator screen all share one `h_eng_annunciator` template canvas).
    // Cycle protection is provided by the MAX_CANVAS_DEPTH depth limit on
    // recursive `resolve_canvas_graph_inner` calls — no cycle can run forever.
    let conditional_canvas_norms: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        for entry in pick_active_entries(record_value, root_json, manufacturer_id) {
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

        // Compute the set of WidgetCanvas node pointers whose `Instantiated`
        // field binding evaluates to `false` under static defaults.  These
        // canvases are inactive at startup and must not be followed in Pass 2.
        // For canvases without any `Instantiated` bindings (e.g. MFD screens)
        // the set is empty and all WidgetCanvas URLs are followed normally.
        let instantiated_false = crate::bb_state_filter::instantiated_false_widgets_with_param_inputs(
            record_value,
            inherited_param_inputs.unwrap_or(&[]),
        );

        deactivate_subtrees(&mut scene, &instantiated_false);

        // Snapshot the visited set as it stands right after Pass 1.  Used
        // below to test "was this canvas already merged by Pass 1?" without
        // modifying the set (so the same URL may appear multiple times in the
        // loop with different paramInputValues and each instance is processed).
        let post_pass1_visited: std::collections::HashSet<String> = visited.clone();

        for (node_id, url, param_inputs) in canvas_urls {
            // Skip WidgetCanvas nodes whose Instantiated binding is false.
            // This prevents inactive state sub-canvases (e.g. Attract, LogIn,
            // MainMenu on a medical kiosk) from being merged into the static render.
            if instantiated_false.contains(&node_id) {
                if debug_trace {
                    log::info!(
                        "bb_resolve[depth={}]: Pass2 skipping ptr:{} {} (Instantiated=false)",
                        depth, node_id, url,
                    );
                }
                continue;
            }
            let norm = extract_record_name(&url).to_ascii_lowercase();
            // If this canvas URL appears in ANY conditional entry but was NOT
            // selected in Pass 1 (not present in the post-Pass-1 snapshot),
            // skip it.  It is the default canvas for a mode-switchable slot,
            // and the selected mode's canvas has already replaced it.
            if conditional_canvas_norms.contains(&norm) && !post_pass1_visited.contains(&norm) {
                if debug_trace {
                    log::info!(
                        "bb_resolve[depth={}]: Pass2 skipping {} (not selected by Pass1 conditional)",
                        depth, norm,
                    );
                }
                continue;
            }
            // Skip canvases that were already merged by Pass 1.  We do NOT
            // insert into `visited` here so that multiple WidgetCanvas nodes
            // referencing the same template URL (e.g. chiclet slots all sharing
            // `h_eng_annunciator`) are each resolved independently.
            if post_pass1_visited.contains(&norm) {
                log::debug!(
                    "bb_resolve: skipping already-pass1-merged WidgetCanvas url '{}'",
                    url
                );
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
            let mut child_scene = match resolve_canvas_graph_inner(
                &child_json,
                manufacturer_id,
                    fetch_by_path,
                    loc_fetcher,
                    depth + 1,
                    visited,
                    palette_source,
                    local_style_identifier.as_deref(),
                    Some(param_inputs.as_slice()),
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
            inject_param_overrides(&param_inputs, &mut child_scene);
            merge_child_scene(&mut scene, child_scene, "", Some(node_id));
        }
    }

    // Apply brand modifiers after scene resolution is complete.
    // This is the R2 phase — mutate node fields based on brand-style modifiers.
    //
    // Resolution order:
    //   1. `brandStyles[]` on the canvas (per-brand override for MC_*, single-entry
    //      for IC_*). Resolved by `resolve_brand_style`.
    //   2. Canvas-level `style` field (a file:// URL pointing to a
    //      `BuildingBlocks_Style` record — e.g. `s_bioc` on medical canvases).
    //      The linked Style record has the same `entries[]` (StyleEntry) shape
    //      as a brandStyles entry, so we can apply it identically.
    let preferred_brand = local_style_identifier.as_deref();
    if let Some(brand_style) =
        bb_brand_style::resolve_brand_style(root_json, manufacturer_id, preferred_brand)
    {
        crate::bb_brand_apply::apply_brand_modifiers(&mut scene, &brand_style, loc_fetcher);
    } else if let Some(style_value) = local_style_value.as_ref() {
        if let Some(entries) = style_value.get("entries").and_then(|v| v.as_array()) {
            let identifier = record_value
                .get("style")
                .and_then(|v| v.as_str())
                .map(crate::pipeline::extract_record_name)
                .unwrap_or_else(|| "linked_style".to_string());
            let brand = bb_brand_style::BrandStyle {
                identifier,
                entries: entries.as_slice(),
                raw: style_value,
            };
            crate::bb_brand_apply::apply_brand_modifiers(&mut scene, &brand, loc_fetcher);
        }
    }

    if let Some(entries) = record_value.get("embeddedStyles").and_then(|v| v.as_array()) {
        let palette_source = palette_source.unwrap_or(record_value);
        let brand = bb_brand_style::BrandStyle {
            identifier: "embeddedStyles".to_string(),
            entries: entries.as_slice(),
            raw: palette_source,
        };
        crate::bb_brand_apply::apply_brand_modifiers(&mut scene, &brand, loc_fetcher);
    }

    if let Some(style_id) = local_style_identifier.as_deref() {
        let module_paths = [
            modular_linearprogress_style_path(style_id),
            modular_buttonsecondary_style_path(style_id),
        ];

        for module_path in module_paths.into_iter().flatten() {
            match fetch_by_path(&module_path) {
                Ok(module_style_json) => {
                    let module_style_value = module_style_json
                        .get("_RecordValue_")
                        .unwrap_or(&module_style_json);
                    if let Some(entries) = module_style_value.get("entries").and_then(|v| v.as_array()) {
                        if module_path.contains("_linearprogressmeterstyles") {
                            seed_implicit_linearprogress_style_tags(
                                &mut scene,
                                entries,
                                fetch_by_path,
                            );
                        }
                        if module_path.contains("_buttonsecondarystyles") {
                            apply_buttonsecondary_modular_styles(&mut scene, entries);
                        }

                        let brand = bb_brand_style::BrandStyle {
                            identifier: extract_record_name(&module_path),
                            entries: entries.as_slice(),
                            raw: module_style_value,
                        };
                        crate::bb_brand_apply::apply_brand_modifiers(
                            &mut scene,
                            &brand,
                            loc_fetcher,
                        );
                    }
                }
                Err(e) => {
                    log::debug!(
                        "bb_resolve: no modular style '{}' for '{}': {}",
                        module_path,
                        style_id,
                        e
                    );
                }
            }
        }
    }

    let animated_roots: std::collections::HashSet<BbNodeId> = scene
        .nodes
        .iter()
        .filter(|(_, node)| node.name.eq_ignore_ascii_case("base_animatedelements"))
        .map(|(id, _)| *id)
        .collect();
    if !animated_roots.is_empty() {
        deactivate_subtrees(&mut scene, &animated_roots);
    }

    Ok(scene)
}

fn deactivate_subtrees(scene: &mut BbScene, roots: &std::collections::HashSet<BbNodeId>) {
    let mut stack: Vec<BbNodeId> = roots.iter().copied().collect();
    let mut seen: std::collections::HashSet<BbNodeId> = std::collections::HashSet::new();
    while let Some(node_id) = stack.pop() {
        if !seen.insert(node_id) {
            continue;
        }
        let Some(node) = scene.nodes.get_mut(&node_id) else {
            continue;
        };
        node.is_active = false;
        stack.extend(node.children.iter().copied());
    }
}

fn pick_active_entries<'a>(
    record_value: &'a serde_json::Value,
    record_root: &'a serde_json::Value,
    manufacturer_id: Option<&str>,
) -> Vec<&'a serde_json::Value> {
    let preferred_brand = record_value
        .get("style")
        .and_then(|v| v.as_str())
        .map(extract_record_name);
    // Use new brand-style resolver (R1 phase) which handles IC_* per-canvas override + generic fallback
    if let Some(brand_style) = bb_brand_style::resolve_brand_style(
        record_root,
        manufacturer_id,
        preferred_brand.as_deref(),
    ) {
        return brand_style.entries.iter().collect();
    }

    // Fall back to defaultStyles when no brand match
    record_value
        .get("defaultStyles")
        .map(entries_from)
        .unwrap_or_default()
}

/// Return the single default-state entry to follow in Pass 1.
///
/// Prefers the first entry whose `conditionsList` is absent or empty
/// (unconditional — always active).  Falls back to the entry that targets
/// the scene node with the highest style-tag count when all entries are
/// conditional (the "most-tagged" node heuristic selects the primary content
/// slot over sub-component slots — e.g. `canvas_GunsMode` with 2 tags wins
/// over `canvas_AmmoNumbers` with 1 tag on `MC_S_Self_Master`).  Falls back
/// to the first entry overall as a last resort.
fn pick_default_entry<'a>(
    record_value: &'a serde_json::Value,
    record_root: &'a serde_json::Value,
    manufacturer_id: Option<&str>,
) -> Option<&'a serde_json::Value> {
    let entries = pick_active_entries(record_value, record_root, manufacturer_id);
    // Prefer first unconditional (empty/absent conditionsList).
    if let Some(entry) = entries.iter().copied().find(|e| {
        e.get("conditionsList")
            .and_then(|v| v.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true)
    }) {
        return Some(entry);
    }

    // When all entries are conditional, use style-tag count on the matched
    // scene node as a tiebreaker.  Entries that target scene nodes with MORE
    // tags are more specifically annotated (e.g. the primary weapon-info slot
    // carries both a system tag and a content-type tag), so they are preferred.
    if entries.len() > 1 {
        let scene = record_value
            .get("scene")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        // Build a map from tag RecordId → number of style tags on the scene
        // node that carries that tag.
        let mut tag_to_node_tag_count: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for node in scene {
            let style_tags = node
                .get("styleTags")
                .and_then(|v| v.as_array())
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            let count = style_tags.len();
            for tag in style_tags {
                if let Some(rid) = tag.get("_RecordId_").and_then(|v| v.as_str()) {
                    // Prefer the higher count if a tag appears on multiple nodes.
                    let e = tag_to_node_tag_count.entry(rid).or_insert(0);
                    if count > *e {
                        *e = count;
                    }
                }
            }
        }

        // Score each entry by the max style-tag count of its condition's tag.
        let mut best_entry = entries[0];
        let mut best_score = 0usize;
        for entry in &entries {
            let score = condition_tag_score(entry, &tag_to_node_tag_count);
            if score > best_score {
                best_score = score;
                best_entry = entry;
            }
        }
        if best_score > 0 {
            return Some(best_entry);
        }
    }

    // Last resort: first entry (structural default when all entries are
    // conditional and no scene-node heuristic applies, e.g. MC_S_Target_Master
    // which has exactly one entry used at runtime despite being tagged
    // conditional in the data).
    entries.into_iter().next()
}

/// Extract the maximum style-tag count of any scene node matched by an
/// entry's `conditionsList` tag conditions.
///
/// Returns 0 when the entry has no recognisable tag conditions or none of its
/// tags appear in `tag_to_count`.
fn condition_tag_score(
    entry: &serde_json::Value,
    tag_to_count: &std::collections::HashMap<&str, usize>,
) -> usize {
    let cond_lists = match entry.get("conditionsList").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return 0,
    };
    let mut max = 0usize;
    for cond_list in cond_lists {
        let conditions = cond_list
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        for cond in conditions {
            max = max.max(score_condition_node(cond, tag_to_count));
        }
    }
    max
}

/// Recursively walk a condition node to find tag RecordIds and return the
/// maximum style-tag count found in `tag_to_count`.
fn score_condition_node(
    cond: &serde_json::Value,
    tag_to_count: &std::collections::HashMap<&str, usize>,
) -> usize {
    // Direct tag condition: {"_Type_": "…ConditionTag", "tag": {"_RecordId_": "…"}}
    if let Some(tag_id) = cond
        .get("tag")
        .and_then(|t| t.get("_RecordId_"))
        .and_then(|v| v.as_str())
    {
        return tag_to_count.get(tag_id).copied().unwrap_or(0);
    }
    // Compound (AllOf/AnyOf): recurse into "conditions" children.
    let mut max = 0usize;
    if let Some(children) = cond.get("conditions").and_then(|v| v.as_array()) {
        for child in children {
            max = max.max(score_condition_node(child, tag_to_count));
        }
    }
    max
}

fn entries_from(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    value
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|entries| entries.iter().collect())
        .unwrap_or_default()
}

fn merge_child_scene(
    parent_scene: &mut BbScene,
    child_scene: BbScene,
    match_to: &str,
    host_parent_override: Option<BbNodeId>,
) {
    let BbScene {
        canvas_size: child_canvas_size,
        roots: child_roots,
        nodes: child_nodes,
        operations: child_ops,
    } = child_scene;

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

    let host_parent_id = if let Some(id) = host_parent_override {
        if parent_scene.nodes.contains_key(&id) {
            Some(id)
        } else {
            None
        }
    } else {
        find_host_parent(parent_scene, match_to)
    };
    let Some(host_parent_id) = host_parent_id else {
        log::warn!("bb_resolve: no host parent available for child canvas merge");
        return;
    };
    let canvas_scale = child_canvas_scale_for_host(parent_scene, host_parent_id, child_canvas_size);

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
        if let Some((sx, sy)) = canvas_scale {
            scale_node_from_child_canvas(&mut node, sx, sy);
        }

        parent_scene.nodes.insert(new_id, node);
    }

    // Remap operation-pointer namespace to avoid collisions across merged child
    // canvases. Node IDs and operation IDs are distinct domains in source
    // records, but both use ptr:N string encoding and can collide numerically.
    let op_id_map = remap_child_operation_ids(parent_scene, &child_ops);

    // Remap ptr: / _PointsTo_:ptr: references in operations using both node-id
    // and operation-id maps.
    let mut remapped_ops = child_ops;
    for op in &mut remapped_ops {
        remap_ptrs_in_json_map(op, &id_map, &op_id_map);
    }
    parent_scene.operations.extend(remapped_ops);

    if let Some(host) = parent_scene.nodes.get_mut(&host_parent_id) {
        let roots_to_add = if inserted_child_roots.is_empty() {
            child_roots_reided
        } else {
            inserted_child_roots
        };
        host.children.extend(roots_to_add);
        let mut seen = std::collections::BTreeSet::new();
        host.children.retain(|id| seen.insert(*id));
    }
}

fn child_canvas_scale_for_host(
    parent_scene: &BbScene,
    host_parent_id: BbNodeId,
    child_canvas_size: (f32, f32),
) -> Option<(f32, f32)> {
    let host = parent_scene.nodes.get(&host_parent_id)?;
    let child_w = child_canvas_size.0;
    let child_h = child_canvas_size.1;
    if child_w <= 0.0 || child_h <= 0.0 {
        return None;
    }
    let host_w = match host.sizing.width {
        BbValue::Fixed(v) if v > 0.0 => v,
        _ => return None,
    };
    let host_h = match host.sizing.height {
        BbValue::Fixed(v) if v > 0.0 => v,
        _ => return None,
    };
    let sx = host_w / child_w;
    let sy = host_h / child_h;
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return None;
    }
    if sx > 4.0 || sy > 4.0 || sx < 0.25 || sy < 0.25 {
        log::debug!(
            "bb_resolve: skipping child-canvas scaling for host ptr:{} (child {:.0}x{:.0} -> host {:.0}x{:.0}, scale {:.3}x{:.3})",
            host_parent_id,
            child_w,
            child_h,
            host_w,
            host_h,
            sx,
            sy,
        );
        return None;
    }
    if (sx - 1.0).abs() < 0.0001 && (sy - 1.0).abs() < 0.0001 {
        return None;
    }
    Some((sx, sy))
}

fn scale_node_from_child_canvas(node: &mut crate::bb_scene::BbNode, sx: f32, sy: f32) {
    node.position.x *= sx;
    node.position.y *= sy;
    node.position_offset.x *= sx;
    node.position_offset.y *= sy;
    scale_bb_value(&mut node.sizing.width, sx);
    scale_bb_value(&mut node.sizing.height, sy);
    node.padding.left *= sx;
    node.padding.right *= sx;
    node.padding.top *= sy;
    node.padding.bottom *= sy;
    node.margin.left *= sx;
    node.margin.right *= sx;
    node.margin.top *= sy;
    node.margin.bottom *= sy;
    if let Some(text) = node.text.as_mut() {
        scale_bb_value(&mut text.font_size, sy);
    }
    if let Some(border) = node.border.as_mut() {
        let sw = sx.min(sy);
        border.top.width *= sw;
        border.right.width *= sw;
        border.bottom.width *= sw;
        border.left.width *= sw;
    }
}

fn scale_bb_value(value: &mut BbValue, scale: f32) {
    if let BbValue::Fixed(v) = value {
        *v *= scale;
    }
}

fn remap_child_operation_ids(
    parent_scene: &BbScene,
    child_ops: &[serde_json::Value],
) -> HashMap<BbNodeId, BbNodeId> {
    let mut occupied: std::collections::BTreeSet<BbNodeId> = std::collections::BTreeSet::new();
    for id in parent_scene.nodes.keys().copied() {
        occupied.insert(id);
    }
    for op in &parent_scene.operations {
        if let Some(id) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("ptr:"))
            .and_then(|n| n.parse::<BbNodeId>().ok())
        {
            occupied.insert(id);
        }
    }

    let mut next: BbNodeId = occupied.iter().next_back().copied().unwrap_or(0).wrapping_add(1);
    let mut out = HashMap::new();
    for op in child_ops {
        let Some(old_id) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("ptr:"))
            .and_then(|n| n.parse::<BbNodeId>().ok())
        else {
            continue;
        };
        if out.contains_key(&old_id) {
            continue;
        }
        while occupied.contains(&next) || out.values().any(|&v| v == next) {
            next = next.wrapping_add(1);
        }
        out.insert(old_id, next);
        next = next.wrapping_add(1);
    }
    out
}

fn remap_ptrs_in_json_map(
    v: &mut serde_json::Value,
    id_map: &HashMap<BbNodeId, BbNodeId>,
    op_id_map: &HashMap<BbNodeId, BbNodeId>,
) {
    remap_ptrs_in_json_map_keyed(v, None, id_map, op_id_map);
}

fn remap_ptrs_in_json_map_keyed(
    v: &mut serde_json::Value,
    key: Option<&str>,
    id_map: &HashMap<BbNodeId, BbNodeId>,
    op_id_map: &HashMap<BbNodeId, BbNodeId>,
) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(n) = s.strip_prefix("ptr:").and_then(|n| n.parse::<BbNodeId>().ok()) {
                let remapped = remap_ptr_with_key(n, key, id_map, op_id_map);
                if let Some(new_id) = remapped {
                    *s = format!("ptr:{new_id}");
                }
            } else if let Some(n) = s
                .strip_prefix("_PointsTo_:ptr:")
                .and_then(|n| n.parse::<BbNodeId>().ok())
            {
                let remapped = remap_ptr_with_key(n, key, id_map, op_id_map);
                if let Some(new_id) = remapped {
                    *s = format!("_PointsTo_:ptr:{new_id}");
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                remap_ptrs_in_json_map_keyed(item, key, id_map, op_id_map);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, value) in map {
                remap_ptrs_in_json_map_keyed(value, Some(k.as_str()), id_map, op_id_map);
            }
        }
        _ => {}
    }
}

fn remap_ptr_with_key(
    old: BbNodeId,
    key: Option<&str>,
    id_map: &HashMap<BbNodeId, BbNodeId>,
    op_id_map: &HashMap<BbNodeId, BbNodeId>,
) -> Option<BbNodeId> {
    let k = key.unwrap_or("");
    let prefer_node = matches!(k, "widget" | "parent" | "target");
    let prefer_op = k == "_Pointer_" || k.starts_with("input") || k == "nZeros";

    if prefer_node {
        if let Some(&n) = id_map.get(&old) {
            return Some(n);
        }
        if let Some(&n) = op_id_map.get(&old) {
            return Some(n);
        }
        return None;
    }
    if prefer_op {
        if let Some(&n) = op_id_map.get(&old) {
            return Some(n);
        }
        if let Some(&n) = id_map.get(&old) {
            return Some(n);
        }
        return None;
    }
    op_id_map.get(&old).copied().or_else(|| id_map.get(&old).copied())
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

fn param_inputs_for_match_to(
    scene: &BbScene,
    match_to: &str,
) -> Option<Vec<serde_json::Value>> {
    for root_id in &scene.roots {
        let Some(root) = scene.nodes.get(root_id) else {
            continue;
        };
        for child_id in &root.children {
            let Some(child) = scene.nodes.get(child_id) else {
                continue;
            };
            if !child.name.eq_ignore_ascii_case(match_to) {
                continue;
            }
            let inputs = child
                .raw
                .get("paramInputValues")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if !inputs.is_empty() {
                return Some(inputs);
            }
        }
    }
    None
}

/// Synthesize `_SynthLocalizedParam_` operations from a parent `WidgetCanvas`
/// node's `paramInputValues` array into the child scene's operations.
///
/// When a `WidgetCanvas` node declares `paramInputValues` entries of type
/// `BuildingBlocks_ComponentParameterInputLocalization`, those entries override
/// the `defaultValue` of the matching `BuildingBlocks_BindingsLocalizedComponentParameter`
/// operations inside the child canvas.  This function injects a synthetic
/// `_SynthLocalizedParam_` operation for each such override so that
/// `BindingResolver` can map widget pointers to localization keys without
/// requiring ActionScript execution.
///
/// Synthetic ops have the same `_Pointer_` value as the matching
/// `BuildingBlocks_BindingsLocalizedComponentParameter` op; they are remapped
/// by `merge_child_scene` together with the rest of the child's operations.
fn inject_param_overrides(
    param_inputs: &[serde_json::Value],
    child_scene: &mut crate::bb_scene::BbScene,
) {
    if param_inputs.is_empty() {
        return;
    }

    // Build param_name → override maps from parent paramInputValues.
    let mut param_to_loc: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut param_to_string: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut param_to_bool: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut param_to_int: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for entry in param_inputs {
        let ty = entry
            .get("_Type_")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let Some(param) = entry.get("parameter").and_then(|v| v.as_str()) else {
            continue;
        };
        if param.is_empty() {
            continue;
        }
        let key = param.to_ascii_lowercase();
        if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputLocalization") {
            let Some(value) = entry.get("value").and_then(|v| v.as_str()) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            param_to_loc.insert(key, value.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputString") {
            let Some(value) = entry.get("value").and_then(|v| v.as_str()) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            param_to_string.insert(key, value.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputBoolean") {
            if let Some(v) = entry.get("value").and_then(|v| v.as_bool()) {
                param_to_bool.insert(param.to_ascii_lowercase(), v);
            }
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputInteger") {
            if let Some(v) = entry.get("value").and_then(|v| v.as_i64()) {
                param_to_int.insert(param.to_ascii_lowercase(), v);
            }
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputNumber") {
            if let Some(v) = entry
                .get("value")
                .and_then(|v| v.as_f64())
                .map(|v| v.round() as i64)
            {
                param_to_int.insert(param.to_ascii_lowercase(), v);
            }
        }
    }
    if param_to_loc.is_empty()
        && param_to_string.is_empty()
        && param_to_bool.is_empty()
        && param_to_int.is_empty()
    {
        return;
    }

    // Scan existing ops for BuildingBlocks_BindingsLocalizedComponentParameter
    // entries and inject a synthetic _SynthLocalizedParam_ op for each match.
    let mut synthetics: Vec<serde_json::Value> = Vec::new();
    let mut param_ptr_to_loc: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut param_ptr_to_string: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for op in &child_scene.operations {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        let param_name_lc = op
            .get("parameter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let Some(ptr_str) = op.get("_Pointer_").and_then(|v| v.as_str()) else {
            continue;
        };
        if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedComponentParameter") {
            let Some(loc_key) = param_to_loc.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthLocalizedParam_",
                "_Pointer_": ptr_str,
                "resolvedLocKey": loc_key,
            }));
            param_ptr_to_loc.insert(ptr_str.to_owned(), loc_key.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringComponentParameter") {
            let Some(value) = param_to_string.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthStringParam_",
                "_Pointer_": ptr_str,
                "resolvedString": value,
            }));
            param_ptr_to_string.insert(ptr_str.to_owned(), value.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsBooleanComponentParameter") {
            let Some(value) = param_to_bool.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthBooleanParam_",
                "_Pointer_": ptr_str,
                "resolvedBool": value,
            }));
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsIntegerComponentParameter") {
            let Some(value) = param_to_int.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthIntegerParam_",
                "_Pointer_": ptr_str,
                "resolvedInt": value,
            }));
        }
    }

    // Also synthesize direct widget→loc mappings for LocalizedField ops that
    // consume those component-parameter pointers. This avoids losing the
    // mapping when intermediate pointer graphs are ambiguous after deep merges.
    if !param_ptr_to_loc.is_empty() || !param_ptr_to_string.is_empty() {
        for op in &child_scene.operations {
            let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if !ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedField")
                && !ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringField")
            {
                continue;
            }
            let Some(widget_ptr) = ptr_ref_str(op.get("widget")) else {
                continue;
            };
            let Some(input_ptr_raw) = ptr_ref_str(op.get("input")) else {
                continue;
            };
            if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedField") {
                let Some(loc_key) = param_ptr_to_loc.get(input_ptr_raw) else {
                    continue;
                };
                synthetics.push(serde_json::json!({
                    "_Type_": "_SynthLocalizedWidget_",
                    "widget": widget_ptr,
                    "resolvedLocKey": loc_key,
                }));
            } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringField") {
                let Some(value) = param_ptr_to_string.get(input_ptr_raw) else {
                    continue;
                };
                synthetics.push(serde_json::json!({
                    "_Type_": "_SynthStringWidget_",
                    "widget": widget_ptr,
                    "resolvedString": value,
                }));
            }
        }
    }

    if !synthetics.is_empty() {
        child_scene.operations.extend(synthetics);
    }
}

fn ptr_ref_str(value: Option<&serde_json::Value>) -> Option<&str> {
    match value {
        Some(serde_json::Value::String(s)) => s.strip_prefix("_PointsTo_:"),
        Some(serde_json::Value::Object(obj)) => obj.get("_Pointer_").and_then(|v| v.as_str()),
        _ => None,
    }
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

    #[test]
    fn merge_scales_child_canvas_units_to_host_slot_size() {
        // Parent canvas authored at 1920x1080 with a 400x400 WidgetCanvas slot.
        let host = serde_json::json!({
            "_RecordName_": "host_touch_slot",
            "_RecordId_": "00000000-0000-0000-0000-000000000111",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_":"Vec3","x":1920.0,"y":1080.0,"z":0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "_Type_":"BuildingBlocks_StyleEntry",
                        "name":"Default",
                        "matchTo":"touch_slot",
                        "conditionsList":[],
                        "modifiers":[
                            {"_Type_":"BuildingBlocks_FieldModifier",
                             "field":{"_Type_":"BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord","value":"file://./child_touch.json"}}
                        ]
                    }]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_":"ptr:1","_Type_":"BuildingBlocks_DisplayWidget","name":"root"},
                    {"_Pointer_":"ptr:2","_Type_":"BuildingBlocks_WidgetCanvas","name":"touch_slot","parent":"_PointsTo_:ptr:1",
                     "sizing":{"_Type_":"BuildingBlocks_Size",
                        "width":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":400.0,"behavior":"Fixed"},
                        "height":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":400.0,"behavior":"Fixed"}}}
                ]
            }
        });
        // Child canvas authored at 1024x1024 with a 512px image.
        let child = serde_json::json!({
            "_RecordName_": "child_touch",
            "_RecordId_": "00000000-0000-0000-0000-000000000112",
            "_RecordValue_": {
                "_Type_":"BuildingBlocks_Canvas",
                "size":{"_Type_":"Vec3","x":1024.0,"y":1024.0,"z":0.0},
                "scene":[
                    {"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_DisplayWidget","name":"child_root",
                     "sizing":{"_Type_":"BuildingBlocks_Size",
                        "width":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":1024.0,"behavior":"Fixed"},
                        "height":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":1024.0,"behavior":"Fixed"}}},
                    {"_Pointer_":"ptr:11","_Type_":"BuildingBlocks_WidgetImage","name":"child_image","parent":"_PointsTo_:ptr:10",
                     "sizing":{"_Type_":"BuildingBlocks_Size",
                        "width":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":512.0,"behavior":"Fixed"},
                        "height":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":256.0,"behavior":"Fixed"}}}
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|path| {
            if path.to_ascii_lowercase().contains("child_touch.json") {
                Ok(child.clone())
            } else {
                Err(format!("unexpected fetch path: {path}"))
            }
        })
        .expect("resolve failed");

        let image_node = scene
            .nodes
            .values()
            .find(|n| n.name == "child_image")
            .expect("child image node missing after merge");
        let w = match image_node.sizing.width {
            BbValue::Fixed(v) => v,
            _ => panic!("child image width must stay fixed"),
        };
        let h = match image_node.sizing.height {
            BbValue::Fixed(v) => v,
            _ => panic!("child image height must stay fixed"),
        };
        assert!(
            (w - 200.0).abs() < 0.01 && (h - 100.0).abs() < 0.01,
            "expected 512x256 in 1024-child scaled into 400x400 slot => 200x100, got {w}x{h}"
        );
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
    fn instantiated_false_widget_is_deactivated_and_not_merged() {
        let host = serde_json::json!({
            "_RecordName_": "test_host_instantiated_false",
            "_RecordId_": "00000000-0000-0000-0000-000000000120",
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
                        "name": "attract_canvas",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "file://./content_canvas.json"
                    }
                ],
                "operations": [
                    {"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_BindingsBooleanVariable","value":"Bed/state.BaseScreens.Attract"},
                    {"_Pointer_":"ptr:11","_Type_":"BuildingBlocks_BindingsBooleanVariable","value":"Bed/state.BaseScreens.MainMenu"},
                    {"_Pointer_":"ptr:12","_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:2","field":"Instantiated","input":"_PointsTo_:ptr:10"}
                ]
            }
        });
        let content = serde_json::json!({
            "_RecordName_": "content_canvas",
            "_RecordId_": "00000000-0000-0000-0000-000000000121",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "content_root"}
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|url| {
            assert_eq!(url, "file://./content_canvas.json");
            Ok(content.clone())
        })
        .expect("resolve failed");

        let widget = scene
            .nodes
            .get(&2)
            .expect("host widget canvas ptr:2 must exist");
        assert!(!widget.is_active, "Instantiated=false widget must be deactivated");
        assert_eq!(
            scene.nodes.len(),
            2,
            "inactive widget should not merge child canvas content"
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
        //
        // Since the Pass 2 outer loop no longer deduplicates by URL (to allow
        // intentional multi-instantiation of template canvases), cycles are
        // now bounded purely by MAX_CANVAS_DEPTH.  Each recursive level fetches
        // the canvas once, so the total fetch count equals MAX_CANVAS_DEPTH.
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
            // The depth cap terminates traversal at MAX_CANVAS_DEPTH levels.
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
    fn pass2_instantiates_same_template_url_multiple_times() {
        // Regression test for B11: when multiple WidgetCanvas nodes in the
        // root canvas all reference the same template URL with different
        // paramInputValues (e.g. the 5 chiclet slots in the annunciator screen
        // all use `h_eng_annunciator`), each slot must produce an independent
        // set of merged nodes.
        //
        // Before the fix, the Pass 2 outer loop deduped by URL — only the first
        // slot was resolved and the other 4 produced no nodes.
        let template = serde_json::json!({
            "_RecordName_": "chiclet_template",
            "_RecordId_": "00000000-0000-0000-0000-000000000090",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "chiclet_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetText", "name": "chiclet_label",
                     "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        // Host canvas has 3 WidgetCanvas nodes all pointing to the same template.
        let host = serde_json::json!({
            "_RecordName_": "annunciator_host",
            "_RecordId_": "00000000-0000-0000-0000-000000000091",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "host_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "slot_a",
                     "parent": "_PointsTo_:ptr:1", "canvas": "file://./chiclet_template.json"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "slot_b",
                     "parent": "_PointsTo_:ptr:1", "canvas": "file://./chiclet_template.json"},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "slot_c",
                     "parent": "_PointsTo_:ptr:1", "canvas": "file://./chiclet_template.json"}
                ]
            }
        });

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&host, None, &|url| {
            assert_eq!(url, "file://./chiclet_template.json", "unexpected url: {url}");
            fetch_count.set(fetch_count.get() + 1);
            Ok(template.clone())
        })
        .expect("resolve failed");

        // The template must be fetched once per slot (3 slots → 3 fetches).
        assert_eq!(
            fetch_count.get(),
            3,
            "expected 3 fetches (one per template slot), got {}",
            fetch_count.get()
        );

        // host(4) + 3 × template(2) = 10 nodes.
        assert_eq!(
            scene.nodes.len(),
            10,
            "expected 10 merged nodes (4 host + 3×2 template), got {}",
            scene.nodes.len()
        );
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

    #[test]
    fn inject_param_overrides_synthesizes_boolean_component_parameter_values() {
        let mut child_scene = crate::bb_scene::BbScene {
            canvas_size: (1920.0, 1080.0),
            roots: Vec::new(),
            nodes: std::collections::BTreeMap::new(),
            operations: vec![serde_json::json!({
                "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                "_Pointer_": "ptr:42",
                "parameter": "ParamInput0",
                "defaultValue": false
            })],
        };
        let param_inputs = vec![serde_json::json!({
            "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
            "parameter": "ParamInput0",
            "value": true
        })];

        inject_param_overrides(&param_inputs, &mut child_scene);

        let synth = child_scene
            .operations
            .iter()
            .find(|op| {
                op.get("_Type_").and_then(|v| v.as_str()) == Some("_SynthBooleanParam_")
                    && op.get("_Pointer_").and_then(|v| v.as_str()) == Some("ptr:42")
            })
            .expect("expected _SynthBooleanParam_ for ptr:42");
        assert_eq!(synth.get("resolvedBool").and_then(|v| v.as_bool()), Some(true));
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

    /// Build a canvas with multiple conditional style entries, each pointing to
    /// a different child canvas URL.
    fn canvas_with_multi_conditional_refs(urls: &[(&str, &str)]) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = urls
            .iter()
            .map(|(name, url)| {
                serde_json::json!({
                    "name": name,
                    "matchTo": "",
                    "conditionsList": [{"_Type_": "BuildingBlocks_StyleConditionList", "conditions": [{"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"}]}],
                    "modifiers": [{
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                            "value": url
                        }
                    }]
                })
            })
            .collect();
        serde_json::json!({
            "_RecordName_": "multi_cond_root",
            "_RecordId_": "00000000-0000-0000-0000-000000000030",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": entries
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"}
                ]
            }
        })
    }

    #[test]
    fn pass1_follows_only_first_conditional_as_default() {
        // B7.2b: Pass 1 must follow only the FIRST (default-state) entry when
        // all entries are conditional.  Previously all 3 were followed, which
        // caused mode-mixing (GunsMode + NavMode + TurretMode merged together).
        //
        // Scenario: root has 3 conditional entries → child_a, child_b, child_c.
        // Each child has 2 nodes.  Only child_a (first entry) must be merged.
        let child_a = serde_json::json!({
            "_RecordName_": "child_a",
            "_RecordId_": "00000000-0000-0000-0000-000000000031",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "a_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_A", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child_b = serde_json::json!({
            "_RecordName_": "child_b",
            "_RecordId_": "00000000-0000-0000-0000-000000000032",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "b_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_B", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child_c = serde_json::json!({
            "_RecordName_": "child_c",
            "_RecordId_": "00000000-0000-0000-0000-000000000033",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "c_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_C", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        let root = canvas_with_multi_conditional_refs(&[
            ("Entry A", "file://./child_a.json"),
            ("Entry B", "file://./child_b.json"),
            ("Entry C", "file://./child_c.json"),
        ]);

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(match url {
                "file://./child_a.json" => child_a.clone(),
                "file://./child_b.json" => child_b.clone(),
                "file://./child_c.json" => child_c.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        // Only the first conditional entry (child_a) must be fetched.
        assert_eq!(
            fetch_count.get(),
            1,
            "expected 1 fetch (first-entry fallback only), got {}",
            fetch_count.get()
        );

        // root(1) + child_a(2) = 3 nodes total.
        assert_eq!(
            scene.nodes.len(),
            3,
            "expected 3 merged nodes (root + child_a(2)), got {}",
            scene.nodes.len()
        );

        let names: std::collections::HashSet<&str> = scene
            .nodes
            .values()
            .filter_map(|n| if n.name.is_empty() { None } else { Some(n.name.as_str()) })
            .collect();
        assert!(names.contains("a_root"), "a_root not found in merged scene");
        assert!(!names.contains("b_root"), "b_root must NOT appear (conditional entry skipped)");
        assert!(!names.contains("c_root"), "c_root must NOT appear (conditional entry skipped)");
    }

    #[test]
    fn pass1_prefers_unconditional_over_first_conditional() {
        // B7.2b: when entries have a mix of conditional and unconditional,
        // Pass 1 must prefer the first UNCONDITIONAL entry over the first
        // overall.
        //
        // Scenario: root has [conditional→child_a, unconditional→child_b].
        // Only child_b must be merged.
        let child_a = serde_json::json!({
            "_RecordName_": "child_a",
            "_RecordId_": "00000000-0000-0000-0000-000000000041",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "a_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_A", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child_b = serde_json::json!({
            "_RecordName_": "child_b",
            "_RecordId_": "00000000-0000-0000-0000-000000000042",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "b_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_B", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        // Build a root canvas with two entries: [0] conditional, [1] unconditional.
        let root = serde_json::json!({
            "_RecordName_": "mixed_root",
            "_RecordId_": "00000000-0000-0000-0000-000000000040",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [
                        // Conditional entry (has conditionsList with one condition).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "ConditionalEntry",
                            "conditionsList": [{"_Type_": "BuildingBlocks_StyleConditionList", "name": "cond", "conditions": [{"key": "mode", "value": "guns"}]}],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./child_a.json"
                                }
                            }]
                        },
                        // Unconditional entry (empty conditionsList).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "UnconditionalEntry",
                            "conditionsList": [],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./child_b.json"
                                }
                            }]
                        }
                    ]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"}
                ]
            }
        });

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(match url {
                "file://./child_a.json" => child_a.clone(),
                "file://./child_b.json" => child_b.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        // Only child_b (unconditional) must be fetched.
        assert_eq!(
            fetch_count.get(),
            1,
            "expected 1 fetch (unconditional entry preferred), got {}",
            fetch_count.get()
        );

        let names: std::collections::HashSet<&str> = scene
            .nodes
            .values()
            .filter_map(|n| if n.name.is_empty() { None } else { Some(n.name.as_str()) })
            .collect();
        assert!(names.contains("b_root"), "b_root (unconditional) must be in merged scene");
        assert!(!names.contains("a_root"), "a_root (conditional) must NOT appear");
    }

    /// When every entry in `defaultStyles.entries` is conditional, the
    /// most-tagged-node heuristic must select the entry whose condition tag
    /// appears on the scene node that carries the highest number of style tags.
    ///
    /// This mirrors the `MC_S_Self_Master` case where `canvas_GunsMode` has 2
    /// style tags while all other canvas nodes have 1, so the GunsMode entry
    /// (→ `gen_mc_s_weaponinfo`) must win over the first entry (→ `gen_mc_s_ammolists`).
    #[test]
    fn pick_default_entry_most_tagged_node_wins() {
        // Child canvases with a single content node each.
        let child_ammo = serde_json::json!({
            "_RecordName_": "ammo",
            "_RecordId_": "00000000-0000-0000-0000-000000000010",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "ammo_root"}
                ]
            }
        });
        let child_guns = serde_json::json!({
            "_RecordName_": "guns",
            "_RecordId_": "00000000-0000-0000-0000-000000000020",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "guns_root"}
                ]
            }
        });

        // Root: two conditional entries.
        // Scene has a DisplayWidget root (ptr:1) with two WidgetCanvas children
        // (ptr:2 = AmmoNumbers with 1 tag, ptr:3 = GunsMode with 2 tags).
        let root = serde_json::json!({
            "_RecordName_": "self_master",
            "_RecordId_": "00000000-0000-0000-0000-000000000001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [
                        // Entry 0 — condition tag matches canvas_AmmoNumbers (1 style tag).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "AmmoNumbers Canvas",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": "tag_ammo"}
                                }]
                            }],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./ammo.json"
                                }
                            }]
                        },
                        // Entry 1 — condition tag matches canvas_GunsMode (2 style tags).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "GunsMode Canvas",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": "tag_guns_mode"}
                                }]
                            }],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./guns.json"
                                }
                            }]
                        }
                    ]
                },
                "brandStyles": [],
                "scene": [
                    // Root DisplayWidget (ptr:1).
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "self_master_root"},
                    // canvas_AmmoNumbers: 1 style tag.
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_AmmoNumbers",
                        "parent": "_PointsTo_:ptr:1",
                        "styleTags": [{"_RecordId_": "tag_ammo"}]
                    },
                    // canvas_GunsMode: 2 style tags — the "most-tagged" node.
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_GunsMode",
                        "parent": "_PointsTo_:ptr:1",
                        "styleTags": [
                            {"_RecordId_": "tag_guns_mode"},
                            {"_RecordId_": "tag_extra"}
                        ]
                    }
                ]
            }
        });

        let chosen = std::cell::Cell::new(None::<&'static str>);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            let label: &'static str = if url.contains("guns") { "guns" } else { "ammo" };
            chosen.set(Some(label));
            Ok(match url {
                "file://./ammo.json" => child_ammo.clone(),
                "file://./guns.json" => child_guns.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        assert_eq!(
            chosen.get(),
            Some("guns"),
            "most-tagged-node heuristic must pick GunsMode entry"
        );
        let names: std::collections::HashSet<&str> = scene
            .nodes
            .values()
            .filter_map(|n| if n.name.is_empty() { None } else { Some(n.name.as_str()) })
            .collect();
        assert!(names.contains("guns_root"), "guns_root must be in merged scene");
        assert!(!names.contains("ammo_root"), "ammo_root must NOT appear");
    }
}
