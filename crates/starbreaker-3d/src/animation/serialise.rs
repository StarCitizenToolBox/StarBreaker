//! JSON serialisation helpers for animation data.
//!
//! Converts parsed [`AnimationDatabase`] / [`AnimationClip`] structures into
//! the `serde_json::Value` format consumed by the Blender addon.

use std::collections::HashMap;

use super::pose::cry_xyzw_to_blender_wxyz;
use super::{AnimationClip, AnimationDatabase};

pub fn clip_to_json(clip: &AnimationClip) -> serde_json::Value {
    let mut bones = serde_json::json!({});

    for channel in &clip.channels {
        let has_rotation = !channel.rotations.is_empty();
        let has_position = !channel.positions.is_empty();

        if !has_rotation && !has_position {
            continue; // Skip empty channels
        }

        let mut rotation_array = vec![];
        let mut rotation_time_array = vec![];
        for keyframe in &channel.rotations {
            let q = cry_xyzw_to_blender_wxyz(keyframe.value);
            rotation_array.push(serde_json::json!([q[0], q[1], q[2], q[3]]));
            rotation_time_array.push(serde_json::json!(keyframe.time));
        }

        let mut position_array = vec![];
        let mut position_time_array = vec![];
        for keyframe in &channel.positions {
            let p = keyframe.value;
            // CryEngine Y-up → Blender Z-up axis swap: (x, y, z) → (x, -z, y).
            // Must match the static-import convention used by the addon's
            // `_scene_position_to_blender` (runtime/importer/utils.py); both
            // sides need to put CryEngine X into Blender X so that animation
            // deltas land in the same frame as the bone's bind position.
            position_array.push(serde_json::json!([p[0], -p[2], p[1]]));
            position_time_array.push(serde_json::json!(keyframe.time));
        }

        let bone_key = format!("0x{:X}", channel.bone_hash);
        bones[bone_key] = serde_json::json!({
            "has_rotation": has_rotation,
            "has_position": has_position,
            "rotation": rotation_array,
            "rotation_time": rotation_time_array,
            "position": position_array,
            "position_time": position_time_array,
        });
    }

    // Calculate frame count from both rotation and position keyframes
    let mut max_frame = 0u32;
    for channel in &clip.channels {
        for keyframe in &channel.rotations {
            max_frame = max_frame.max(keyframe.time.ceil() as u32);
        }
        for keyframe in &channel.positions {
            max_frame = max_frame.max(keyframe.time.ceil() as u32);
        }
    }

    let mut value = serde_json::json!({
        "name": clip.name,
        "fps": clip.fps as u32,
        "frame_count": max_frame,
        "bones": bones,
    });
    // Phase 53: emit DBA-metadata `start_rotation` / `start_position` as
    // top-level clip fields so the addon can use them as the data-backed
    // anchor for `result = bind ⋅ (start⁻¹ ⋅ sample)` (see SC animation
    // formats whitepaper §14.6). Both fields are converted into the same
    // Blender Z-up convention used for sample keyframes:
    //   - rotation: CryEngine xyzw → Blender wxyz, axis-swapped via
    //     `cry_xyzw_to_blender_wxyz` (matches per-sample emission above).
    //   - position: CryEngine (x, y, z) → Blender (x, -z, y) (matches the
    //     per-sample swap used a few lines up). DBA only stores XY on
    //     disk; the Z is already filled in as 0.0 by
    //     `match_dba_metadata_to_blocks`, which is the documented
    //     empirical default. CAF clips leave both fields `None` and the
    //     addon falls back to first-sample anchoring.
    if let Some(start_rot_xyzw) = clip.start_rotation {
        let q = cry_xyzw_to_blender_wxyz(start_rot_xyzw);
        value["start_rotation"] = serde_json::json!([q[0], q[1], q[2], q[3]]);
    }
    if let Some(p) = clip.start_position {
        value["start_position"] = serde_json::json!([p[0], -p[2], p[1]]);
    }
    value
}

/// Convert a full database to a JSON array of animations.
pub fn database_to_animations_json(db: &AnimationDatabase) -> serde_json::Value {
    serde_json::Value::Array(db.clips.iter().map(clip_to_json).collect())
}

fn parse_bone_hash_key(key: &str) -> Option<u32> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).ok();
    }
    trimmed.parse::<u32>().ok()
}

pub(super) fn annotate_animation_json_source(
    value: &mut serde_json::Value,
    source_skeleton_path: &str,
    source_node_name_by_hash: &HashMap<u32, String>,
) {
    let Some(clips) = value.as_array_mut() else {
        return;
    };
    for clip in clips {
        let Some(clip_obj) = clip.as_object_mut() else {
            continue;
        };
        let Some(bones) = clip_obj.get_mut("bones").and_then(|bones| bones.as_object_mut()) else {
            continue;
        };
        for (bone_key, channel_value) in bones {
            let Some(channel_obj) = channel_value.as_object_mut() else {
                continue;
            };
            channel_obj.insert(
                "source_skeleton_path".to_string(),
                serde_json::Value::String(source_skeleton_path.to_string()),
            );
            if let Some(hash) = parse_bone_hash_key(bone_key)
                && let Some(name) = source_node_name_by_hash.get(&hash)
            {
                channel_obj.insert(
                    "source_node_name".to_string(),
                    serde_json::Value::String(name.clone()),
                );
            }
        }
    }
}

/// Per-bone animation blend mode, derived from the geometric
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoneBlendMode {
    /// Bind sits inside (or coincident with) the CAF sample AABB —
    /// the clip is interpreted as additive on top of bind. This is
    /// the default and matches the addon's anchor-relative
    /// composition path (`bind ⋅ (anchor⁻¹ ⋅ sample)`).
    Additive,
    /// Bind sits strictly outside the CAF sample AABB on at least one
    /// axis — the clip is interpreted as an override. The addon
    /// should use the sampled pose verbatim (`result = sample`)
    /// instead of composing it on top of bind.
    Override,
}

impl BoneBlendMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BoneBlendMode::Additive => "additive",
            BoneBlendMode::Override => "override",
        }
    }
}

/// Classify each bone's animation blend mode by testing whether the
/// CHR-bind local position sits inside the AABB of the bone's CAF
/// position samples across **all** clips. A bone with no position
/// samples or no bind entry is omitted from the result (the addon
/// defaults to additive).
///
/// Containment is strict on each axis: bind is "outside" if any
/// component is < min or > max with no epsilon. This is data-grounded
/// — the only inputs are the CHR-bind position and the CAF sample
/// stream — and uses no heuristics, name lookups, or absolute-unit
/// thresholds.
///
/// Both inputs are interpreted in raw CryEngine local-space (the
/// same convention as `BoneChannel.positions` and
/// `crate::skeleton::Bone::local_position`); no axis swap is
/// applied.
pub fn classify_bone_blend_modes(
    clips: &[AnimationClip],
    binds: &std::collections::HashMap<u32, [f32; 3]>,
) -> std::collections::HashMap<u32, BoneBlendMode> {
    // Build per-bone AABB from all CAF position samples.
    let mut bbox: std::collections::HashMap<u32, ([f32; 3], [f32; 3])> =
        std::collections::HashMap::new();
    for clip in clips {
        for ch in &clip.channels {
            if ch.positions.is_empty() {
                continue;
            }
            let entry = bbox.entry(ch.bone_hash).or_insert_with(|| {
                let p = ch.positions[0].value;
                (p, p)
            });
            for kf in &ch.positions {
                let p = kf.value;
                for axis in 0..3 {
                    if p[axis] < entry.0[axis] {
                        entry.0[axis] = p[axis];
                    }
                    if p[axis] > entry.1[axis] {
                        entry.1[axis] = p[axis];
                    }
                }
            }
        }
    }
    let mut out = std::collections::HashMap::new();
    for (hash, (min, max)) in &bbox {
        let Some(bind) = binds.get(hash) else {
            continue;
        };
        let outside = (0..3).any(|axis| bind[axis] < min[axis] || bind[axis] > max[axis]);
        out.insert(
            *hash,
            if outside { BoneBlendMode::Override } else { BoneBlendMode::Additive },
        );
    }
    out
}

/// Inject a `blend_mode` field into every clip's per-bone entry of
/// the JSON produced by [`database_to_animations_json`]. Bones
/// without an entry in `modes` are left untouched (the addon
/// defaults to additive).
pub fn annotate_animations_json_with_blend_modes(
    clips_json: &mut serde_json::Value,
    modes: &std::collections::HashMap<u32, BoneBlendMode>,
) {
    let Some(arr) = clips_json.as_array_mut() else {
        return;
    };
    for clip in arr.iter_mut() {
        let Some(bones) = clip.get_mut("bones").and_then(|v| v.as_object_mut()) else {
            continue;
        };
        for (key, value) in bones.iter_mut() {
            // bone_key is "0xHEX" — parse back to u32.
            let Some(stripped) = key.strip_prefix("0x").or(Some(key)) else { continue };
            let Ok(hash) = u32::from_str_radix(stripped, 16) else { continue };
            let Some(mode) = modes.get(&hash) else { continue };
            if let Some(obj) = value.as_object_mut() {
                obj.insert(
                    "blend_mode".to_string(),
                    serde_json::Value::String(mode.as_str().to_string()),
                );
            }
        }
    }
}

/// Structured dump of an animation database for diagnostic / debug
/// tooling. Returns a JSON value with one entry per clip listing
/// channel counts, frame counts, per-channel bone hashes (resolved to
/// names when `hash_to_name` provides them), and either first/last
/// keyframe samples or the full keyframe stream depending on
/// `all_keyframes`.
///
/// Used by the StarBreaker MCP `dba_dump` tool. Replaces the previous
/// `starbreaker dba dump` CLI subcommand (Phase 36).
pub fn dump_database_to_json(
    db: &AnimationDatabase,
    hash_to_name: &std::collections::HashMap<u32, String>,
    filter: Option<&str>,
    bone_filter: Option<&str>,
    all_keyframes: bool,
) -> serde_json::Value {
    let filter_lc = filter.map(|f| f.to_ascii_lowercase());
    let bone_filter_lc = bone_filter.map(|f| f.to_ascii_lowercase());
    let mut clips_out: Vec<serde_json::Value> = Vec::new();
    for (idx, clip) in db.clips.iter().enumerate() {
        if let Some(needle) = filter_lc.as_ref() {
            if !clip.name.to_ascii_lowercase().contains(needle) {
                continue;
            }
        }
        let frame_count = clip
            .channels
            .iter()
            .map(|ch| ch.rotations.len().max(ch.positions.len()))
            .max()
            .unwrap_or(0);
        let mut channels_out: Vec<serde_json::Value> = Vec::with_capacity(clip.channels.len());
        for ch in &clip.channels {
            let bone_name = hash_to_name.get(&ch.bone_hash).cloned();
            // Bone-name filter: when set, skip channels whose resolved name
            // doesn't contain the substring (case-insensitive). Channels with
            // unresolved hashes are skipped when a bone filter is active so the
            // output is unambiguous.
            if let Some(needle) = bone_filter_lc.as_ref() {
                let matches = bone_name
                    .as_ref()
                    .map(|n| n.to_ascii_lowercase().contains(needle))
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            let mut channel_value = serde_json::json!({
                "bone_hash": format!("0x{:08X}", ch.bone_hash),
                "bone_name": bone_name,
                "rotation_count": ch.rotations.len(),
                "position_count": ch.positions.len(),
                "rot_format_flags": format!("0x{:04X}", ch.rot_format_flags),
                "pos_format_flags": format!("0x{:04X}", ch.pos_format_flags),
            });
            if all_keyframes {
                channel_value["rotations"] = serde_json::Value::Array(
                    ch.rotations.iter().map(|kf| serde_json::json!({
                        "time": kf.time,
                        "value": kf.value,
                    })).collect(),
                );
                channel_value["positions"] = serde_json::Value::Array(
                    ch.positions.iter().map(|kf| serde_json::json!({
                        "time": kf.time,
                        "value": kf.value,
                    })).collect(),
                );
            } else {
                if let (Some(first), Some(last)) = (ch.rotations.first(), ch.rotations.last()) {
                    channel_value["rotation_first"] =
                        serde_json::json!({"time": first.time, "value": first.value});
                    channel_value["rotation_last"] =
                        serde_json::json!({"time": last.time, "value": last.value});
                }
                if let (Some(first), Some(last)) = (ch.positions.first(), ch.positions.last()) {
                    channel_value["position_first"] =
                        serde_json::json!({"time": first.time, "value": first.value});
                    channel_value["position_last"] =
                        serde_json::json!({"time": last.time, "value": last.value});
                }
            }
            channels_out.push(channel_value);
        }
        clips_out.push(serde_json::json!({
            "index": idx,
            "name": clip.name,
            "fps": clip.fps,
            "channel_count": clip.channels.len(),
            "frame_count": frame_count,
            "channels": channels_out,
        }));
    }
    serde_json::json!({
        "clip_count": db.clips.len(),
        "skeleton_bones_resolved": hash_to_name.len(),
        "clips": clips_out,
    })
}

/// Sanitize a clip name into a safe filename component.
///
/// Replaces characters outside `[A-Za-z0-9_.-]` with `_`. Used by the
/// decomposed exporter to derive per-clip animation sidecar filenames
/// under `Packages/<entity>/animations/<clip>.json`.
pub fn sanitize_clip_filename(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "clip".to_string();
    }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "clip".to_string()
    } else {
        out
    }
}

/// Split a fully-serialized animation clip into a lightweight index
/// record (preserves `name`, `fps`, `frame_count`, `fragments`, etc.,
/// and adds a `sidecar` reference) and a heavy sidecar body (the full
/// clip including the `bones` keyframe arrays).
///
/// The exporter writes the sidecar body to a separate JSON file under
/// `Packages/<entity>/animations/<clip>.json` so that the inline
/// `scene.json` only carries an index. The Blender importer then loads
/// the sidecar lazily when a clip is actually applied.
///
/// `sidecar_relative_path` is stored on the index record verbatim.
pub fn split_clip_for_sidecar(
    clip: &serde_json::Value,
    sidecar_relative_path: &str,
) -> (serde_json::Value, serde_json::Value) {
    let mut index = clip.clone();
    if let Some(map) = index.as_object_mut() {
        map.remove("bones");
        map.insert(
            "sidecar".to_string(),
            serde_json::Value::String(sidecar_relative_path.to_string()),
        );
    }
    (index, clip.clone())
}

