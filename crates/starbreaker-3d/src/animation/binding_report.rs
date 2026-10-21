//! Report-only animation binding diagnostics shared by the CLI
//! (`starbreaker animation binding-report`) and the MCP `animation_binding_report`
//! tool.
//!
//! The logic compares the hash-only CAF/DBA track keys in exported animation
//! JSON sidecars against the node names StarBreaker can recover from the
//! original CryEngine rig files (`.cga`/`.cgam` NMC nodes, `.meshsetup` joints,
//! `.chrparams` references) and from converted `.glb` output. It never mutates
//! sidecars or guesses bindings — callers own all file/P4k I/O and feed bytes,
//! text, or parsed JSON to the pure builders here.
//!
//! Key entry points: [`NameSources`] (accumulator), the `*_source_report`
//! helpers (one per rig source kind), [`clips_from_value`], [`build_report`],
//! and [`build_markdown_report`].

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::bone_name_hash;

/// Accumulates candidate node names grouped by the source they came from.
#[derive(Default)]
pub struct NameSources {
    by_source: BTreeMap<String, BTreeSet<String>>,
}

impl NameSources {
    pub fn add(&mut self, source: impl Into<String>, name: impl AsRef<str>) {
        let name = name.as_ref().trim();
        if name.is_empty() {
            return;
        }
        self.by_source
            .entry(source.into())
            .or_default()
            .insert(name.to_string());
    }

    fn source_counts_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        for (source, names) in &self.by_source {
            map.insert(source.clone(), json!(names.len()));
        }
        Value::Object(map)
    }

    fn all_names(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for names in self.by_source.values() {
            out.extend(names.iter().cloned());
        }
        out
    }
}

/// Add NMC/rig node names from a `.cga`/`.cgam` file's raw bytes, returning a
/// per-source summary value. `source_ref` is a display string for the report.
pub fn rig_source_report(
    data: &[u8],
    label: &str,
    source_ref: &str,
    names: &mut NameSources,
) -> Value {
    let rig_names = crate::skeleton::parse_rig_node_names(data).unwrap_or_default();
    let nmc = crate::nmc::parse_nmc_full(data);
    let (nmc_node_count, nmc_geom_node_count, nmc_non_geom_node_count) = match &nmc {
        Some((nodes, _)) => {
            let geom = nodes.iter().filter(|node| node.geometry_type == 0).count();
            (nodes.len(), geom, nodes.len().saturating_sub(geom))
        }
        None => (0, 0, 0),
    };
    for name in &rig_names {
        names.add(label, name);
    }
    json!({
        "source": label,
        "path": source_ref,
        "file_size": data.len(),
        "rig_node_name_count": rig_names.len(),
        "nmc_node_count": nmc_node_count,
        "nmc_geometry_node_count": nmc_geom_node_count,
        "nmc_non_geometry_node_count": nmc_non_geom_node_count,
        "sample_names": sample_strings(rig_names.iter().map(|s| s.as_str()), 40),
    })
}

/// Add joint names from a `.meshsetup` file's text.
pub fn meshsetup_source_report(text: &str, source_ref: &str, names: &mut NameSources) -> Value {
    let joints = extract_attr_values(text, "Joint");
    for name in &joints {
        names.add("meshsetup", name);
    }
    json!({
        "source": "meshsetup",
        "path": source_ref,
        "joint_count": joints.len(),
        "sample_names": sample_strings(joints.iter().map(|s| s.as_str()), 40),
    })
}

/// Add animation name/path references from a `.chrparams` file's text.
pub fn chrparams_source_report(text: &str, source_ref: &str, names: &mut NameSources) -> Value {
    let animation_names = extract_attr_values(text, "name");
    let animation_paths = extract_attr_values(text, "path");
    for name in &animation_names {
        names.add("chrparams_animation_name", name);
    }
    for path_value in &animation_paths {
        if let Some(stem) = path_value.rsplit(['/', '\\']).next() {
            names.add("chrparams_animation_path_stem", stem.trim_end_matches(".caf"));
        }
    }
    json!({
        "source": "chrparams",
        "path": source_ref,
        "animation_name_count": animation_names.len(),
        "animation_path_count": animation_paths.len(),
        "sample_animation_names": sample_strings(animation_names.iter().map(|s| s.as_str()), 40),
        "sample_animation_paths": sample_strings(animation_paths.iter().map(|s| s.as_str()), 40),
    })
}

/// Add exported node names from a `.glb` file's raw bytes. Returns `None` if the
/// GLB JSON chunk cannot be parsed.
pub fn glb_source_report(data: &[u8], source_ref: &str, names: &mut NameSources) -> Option<Value> {
    let json = read_glb_json(data)?;
    let mut node_names = Vec::new();
    if let Some(nodes) = json.get("nodes").and_then(|v| v.as_array()) {
        for node in nodes {
            if let Some(name) = node.get("name").and_then(|v| v.as_str()) {
                node_names.push(name.to_string());
                names.add("glb_node", name);
            }
        }
    }
    let mesh_count = json.get("meshes").and_then(|v| v.as_array()).map_or(0, Vec::len);
    let skin_count = json.get("skins").and_then(|v| v.as_array()).map_or(0, Vec::len);
    let animation_count = json.get("animations").and_then(|v| v.as_array()).map_or(0, Vec::len);
    Some(json!({
        "source": "glb",
        "path": source_ref,
        "node_count": node_names.len(),
        "mesh_count": mesh_count,
        "skin_count": skin_count,
        "animation_count": animation_count,
        "sample_names": sample_strings(node_names.iter().map(|s| s.as_str()), 40),
    }))
}

/// Per-clip statistics extracted from one animation sidecar.
#[derive(Default)]
pub struct ClipStats {
    file: String,
    name: String,
    source_skeleton_paths: BTreeSet<String>,
    bone_entries: usize,
    channel_variants: usize,
    animated_track_entries: usize,
    hash_entries: BTreeSet<u32>,
    source_node_names: BTreeSet<String>,
}

/// Parse all clips out of one animation sidecar JSON value. `file_label`
/// identifies the sidecar (path or name) in the report; it is also used to
/// derive a fallback clip name.
pub fn clips_from_value(file_label: &str, value: &Value) -> Vec<ClipStats> {
    let mut clips = Vec::new();
    if let Some(array) = value.as_array() {
        for (i, clip) in array.iter().enumerate() {
            clips.push(clip_stats_from_value(file_label, Some(i), clip));
        }
    } else {
        clips.push(clip_stats_from_value(file_label, None, value));
    }
    clips
}

fn clip_stats_from_value(file_label: &str, array_index: Option<usize>, clip: &Value) -> ClipStats {
    let fallback_name = file_label
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_label)
        .trim_end_matches(".json")
        .to_string();
    let mut stats = ClipStats {
        file: file_label.to_string(),
        name: clip
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| match array_index {
                Some(i) => format!("{fallback_name}[{i}]"),
                None => fallback_name,
            }),
        ..Default::default()
    };

    let Some(bones) = clip.get("bones").and_then(|v| v.as_object()) else {
        return stats;
    };

    stats.bone_entries = bones.len();
    for (bone_key, channel_value) in bones {
        if let Some(hash) = parse_hash_key(bone_key) {
            stats.hash_entries.insert(hash);
        }
        if let Some(obj) = channel_value.as_object() {
            stats.channel_variants += 1;
            collect_channel_metadata(obj, &mut stats);
        } else if let Some(array) = channel_value.as_array() {
            for variant in array {
                if let Some(obj) = variant.as_object() {
                    stats.channel_variants += 1;
                    collect_channel_metadata(obj, &mut stats);
                }
            }
        }
    }

    stats
}

fn collect_channel_metadata(obj: &serde_json::Map<String, Value>, stats: &mut ClipStats) {
    let has_position = obj.get("has_position").and_then(|v| v.as_bool()).unwrap_or(false);
    let has_rotation = obj.get("has_rotation").and_then(|v| v.as_bool()).unwrap_or(false);
    if has_position || has_rotation {
        stats.animated_track_entries += 1;
    }
    if let Some(path) = obj.get("source_skeleton_path").and_then(|v| v.as_str()) {
        stats.source_skeleton_paths.insert(path.to_string());
    }
    if let Some(name) = obj.get("source_node_name").and_then(|v| v.as_str()) {
        stats.source_node_names.insert(name.to_string());
    }
}

fn parse_hash_key(key: &str) -> Option<u32> {
    let trimmed = key.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    trimmed.parse::<u32>().ok()
}

#[derive(Clone)]
struct HashCandidate {
    source: String,
    name: String,
    variant: &'static str,
}

type HashIndex = BTreeMap<u32, Vec<HashCandidate>>;

fn build_name_hash_index(names: &NameSources) -> HashIndex {
    let mut index: HashIndex = BTreeMap::new();
    for (source, source_names) in &names.by_source {
        for name in source_names {
            add_hash_candidate(&mut index, source, name, name, "exact");
            let lower = name.to_ascii_lowercase();
            if lower != *name {
                add_hash_candidate(&mut index, source, name, &lower, "lowercase");
            }
            let upper = name.to_ascii_uppercase();
            if upper != *name {
                add_hash_candidate(&mut index, source, name, &upper, "uppercase");
            }
        }
    }
    index
}

fn add_hash_candidate(
    index: &mut HashIndex,
    source: &str,
    original_name: &str,
    hashed_name: &str,
    variant: &'static str,
) {
    let hash = bone_name_hash(hashed_name);
    index.entry(hash).or_default().push(HashCandidate {
        source: source.to_string(),
        name: original_name.to_string(),
        variant,
    });
}

/// Build the machine-readable binding report from accumulated name sources,
/// per-source summaries, the total animation-file count, and per-clip stats.
pub fn build_report(
    names: &NameSources,
    source_reports: Vec<Value>,
    animation_file_count: usize,
    clip_stats: &[ClipStats],
) -> Value {
    let name_hash_index = build_name_hash_index(names);
    let mut all_track_hashes = BTreeSet::<u32>::new();
    let mut clips_with_any_source_names = 0usize;
    let mut clips_with_unresolved_hashes = 0usize;
    let mut exact_hits = 0usize;
    let mut case_variant_hits = 0usize;
    let mut unresolved_hashes = BTreeSet::<u32>::new();

    let clip_values: Vec<Value> = clip_stats
        .iter()
        .map(|clip| {
            if !clip.source_node_names.is_empty() {
                clips_with_any_source_names += 1;
            }
            let mut hash_reports = Vec::new();
            let mut clip_unresolved = false;
            for hash in &clip.hash_entries {
                all_track_hashes.insert(*hash);
                let candidates = name_hash_index.get(hash).cloned().unwrap_or_default();
                let exact: Vec<Value> = candidates
                    .iter()
                    .filter(|c| c.variant == "exact")
                    .map(candidate_json)
                    .collect();
                let variants: Vec<Value> = candidates
                    .iter()
                    .filter(|c| c.variant != "exact")
                    .map(candidate_json)
                    .collect();
                if !exact.is_empty() {
                    exact_hits += 1;
                } else if !variants.is_empty() {
                    case_variant_hits += 1;
                } else {
                    unresolved_hashes.insert(*hash);
                    clip_unresolved = true;
                }
                hash_reports.push(json!({
                    "hash_hex": format!("0x{hash:08X}"),
                    "hash_decimal": *hash,
                    "exact_name_hash_matches": exact,
                    "case_variant_name_hash_matches": variants,
                }));
            }
            if clip_unresolved {
                clips_with_unresolved_hashes += 1;
            }
            json!({
                "file": clip.file,
                "name": clip.name,
                "source_skeleton_paths": clip.source_skeleton_paths.iter().cloned().collect::<Vec<_>>(),
                "bone_entries": clip.bone_entries,
                "channel_variants": clip.channel_variants,
                "animated_track_entries": clip.animated_track_entries,
                "hash_entry_count": clip.hash_entries.len(),
                "source_node_name_count": clip.source_node_names.len(),
                "source_node_names": clip.source_node_names.iter().cloned().collect::<Vec<_>>(),
                "hashes": hash_reports,
            })
        })
        .collect();

    let all_candidate_names = names.all_names();
    json!({
        "report_type": "StarBreaker animation binding report",
        "summary": {
            "animation_json_files": animation_file_count,
            "clips": clip_stats.len(),
            "clips_with_any_source_node_name": clips_with_any_source_names,
            "clips_with_unresolved_hashes": clips_with_unresolved_hashes,
            "unique_track_hashes": all_track_hashes.len(),
            "unique_candidate_names": all_candidate_names.len(),
            "unique_unresolved_hashes": unresolved_hashes.len(),
            "track_hashes_with_exact_candidate_name_hits": exact_hits,
            "track_hashes_with_case_variant_candidate_name_hits": case_variant_hits,
            "candidate_name_counts_by_source": names.source_counts_json(),
        },
        "source_reports": source_reports,
        "unresolved_hashes": unresolved_hashes.iter().map(|h| format!("0x{h:08X}")).collect::<Vec<_>>(),
        "clips": clip_values,
    })
}

fn candidate_json(candidate: &HashCandidate) -> Value {
    json!({
        "source": candidate.source,
        "name": candidate.name,
        "hash_variant": candidate.variant,
    })
}

/// Render the Markdown summary for a report produced by [`build_report`].
pub fn build_markdown_report(report: &Value) -> String {
    let summary = &report["summary"];
    let mut out = String::new();
    out.push_str("# StarBreaker Animation Binding Report\n\n");
    out.push_str("## Summary\n\n");
    out.push_str(&format!("- Animation JSON files: {}\n", summary["animation_json_files"]));
    out.push_str(&format!("- Clips: {}\n", summary["clips"]));
    out.push_str(&format!("- Clips with source_node_name: {}\n", summary["clips_with_any_source_node_name"]));
    out.push_str(&format!("- Clips with unresolved hashes: {}\n", summary["clips_with_unresolved_hashes"]));
    out.push_str(&format!("- Unique track hashes: {}\n", summary["unique_track_hashes"]));
    out.push_str(&format!("- Unique unresolved hashes: {}\n", summary["unique_unresolved_hashes"]));
    out.push_str(&format!("- Exact candidate-name hash hits: {}\n", summary["track_hashes_with_exact_candidate_name_hits"]));
    out.push_str(&format!("- Case-variant candidate-name hash hits: {}\n\n", summary["track_hashes_with_case_variant_candidate_name_hits"]));

    out.push_str("## Candidate Name Counts\n\n");
    if let Some(map) = summary["candidate_name_counts_by_source"].as_object() {
        for (source, count) in map {
            out.push_str(&format!("- {source}: {count}\n"));
        }
    }

    out.push_str("\n## Unresolved Hashes\n\n");
    if let Some(values) = report["unresolved_hashes"].as_array() {
        if values.is_empty() {
            out.push_str("None.\n");
        } else {
            for value in values.iter().take(200) {
                out.push_str(&format!("- {}\n", value.as_str().unwrap_or("?")));
            }
            if values.len() > 200 {
                out.push_str(&format!("- ... {} more\n", values.len() - 200));
            }
        }
    }

    out.push_str("\n## Clips With Unresolved Hashes\n\n");
    if let Some(clips) = report["clips"].as_array() {
        for clip in clips {
            let unresolved_count = clip["hashes"]
                .as_array()
                .map(|hashes| {
                    hashes
                        .iter()
                        .filter(|h| {
                            h["exact_name_hash_matches"].as_array().map_or(true, Vec::is_empty)
                                && h["case_variant_name_hash_matches"].as_array().map_or(true, Vec::is_empty)
                        })
                        .count()
                })
                .unwrap_or(0);
            if unresolved_count == 0 {
                continue;
            }
            out.push_str(&format!(
                "- `{}` — unresolved hashes: {}, source names: {}\n",
                clip["name"].as_str().unwrap_or("?"),
                unresolved_count,
                clip["source_node_name_count"],
            ));
        }
    }

    out
}

fn extract_attr_values(text: &str, attr: &str) -> Vec<String> {
    let needle = format!("{attr}=\"");
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(&needle) {
        let start = pos + needle.len();
        let after = &rest[start..];
        if let Some(end) = after.find('"') {
            values.push(after[..end].to_string());
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    values
}

fn read_glb_json(data: &[u8]) -> Option<Value> {
    if data.len() < 20 || &data[0..4] != b"glTF" {
        return None;
    }
    let version = u32::from_le_bytes(data[4..8].try_into().ok()?);
    if version != 2 {
        return None;
    }
    let mut offset = 12usize;
    while offset.checked_add(8)? <= data.len() {
        let len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let chunk_type = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?);
        offset += 8;
        let end = offset.checked_add(len)?;
        if end > data.len() {
            return None;
        }
        if chunk_type == 0x4E4F534A {
            return serde_json::from_slice(&data[offset..end]).ok();
        }
        offset = end;
    }
    None
}

fn sample_strings<'a>(iter: impl Iterator<Item = &'a str>, limit: usize) -> Vec<String> {
    iter.take(limit).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hash_key_accepts_hex_and_decimal() {
        assert_eq!(parse_hash_key("0x6B65934C"), Some(0x6B65934C));
        assert_eq!(parse_hash_key(" 255 "), Some(255));
        assert_eq!(parse_hash_key("nope"), None);
    }

    #[test]
    fn extract_attr_values_reads_quoted_attributes() {
        let xml = r#"<Joint Joint="hip" /><Joint Joint="spine_01" />"#;
        assert_eq!(extract_attr_values(xml, "Joint"), vec!["hip", "spine_01"]);
        assert!(extract_attr_values(xml, "missing").is_empty());
    }

    #[test]
    fn build_report_counts_resolved_and_unresolved() {
        let mut names = NameSources::default();
        names.add("cga", "shutter_15");
        // One clip: one hash resolvable from the cga, one not.
        let resolvable = format!("0x{:08X}", bone_name_hash("shutter_15"));
        let clip = json!({
            "name": "door_open",
            "bones": {
                resolvable: { "has_rotation": true, "source_node_name": "shutter_15" },
                "0x00000002": { "has_rotation": true },
            }
        });
        let stats = clips_from_value("door_open.json", &clip);
        let report = build_report(&names, vec![], 1, &stats);
        let summary = &report["summary"];
        assert_eq!(summary["clips"], 1);
        assert_eq!(summary["clips_with_any_source_node_name"], 1);
        assert_eq!(summary["unique_unresolved_hashes"], 1);
        assert_eq!(summary["track_hashes_with_exact_candidate_name_hits"], 1);
    }
}
