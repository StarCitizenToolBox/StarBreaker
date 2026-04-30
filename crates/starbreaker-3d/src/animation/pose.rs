//! Bone pose utilities: axis-swap, final-frame pose extraction, skeleton
//! block selection, and pose-application.

use std::collections::{HashMap, HashSet};

use crate::error::Error;

use super::{AnimationClip, AnimationDatabase, parse_dba};

/// Final-frame local TRS pose for a single bone.
#[derive(Debug, Clone, Copy)]
pub struct BonePose {
    /// Local rotation as quaternion in Blender Z-up `wxyz` order.
    pub rotation: [f32; 4],
    /// Local position in Blender Z-up.
    pub position: Option<[f32; 3]>,
}

/// Convert a quaternion produced by the SmallTree48BitQuat decoder (CryEngine
/// Y-up `xyzw` convention) into the Blender Z-up `wxyz` form used by our
/// pipeline.
///
/// Must match the position axis swap used by `clip_to_json` and the static
/// import's `_scene_position_to_blender`: `(cx, cy, cz) → (cx, -cz, cy)`.
/// Applying the same basis change to a quaternion's vector component gives
/// `(qx, qy, qz, qw) → (qw, qx, -qz, qy)` in Blender WXYZ form.
pub fn cry_xyzw_to_blender_wxyz(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    [w, x, -z, y]
}

/// Read a single named animation from a `.dba` and return final-frame local
/// TRS keyed by bone CRC32 hash.
///
/// `animation_name` is matched against the metadata strings stored in the
/// DBA chunk (case-insensitive substring). Prefer [`find_block_for_skeleton`]
/// for production use; this helper is kept for debugging.
pub fn read_dba_final_pose(
    dba_bytes: &[u8],
    animation_name: &str,
) -> Result<HashMap<u32, BonePose>, Error> {
    let db = parse_dba(dba_bytes)?;
    let needle = animation_name.to_ascii_lowercase();

    let clip = db
        .clips
        .iter()
        .find(|c| c.name.to_ascii_lowercase().contains(&needle))
        .ok_or_else(|| Error::Other(format!("Animation '{animation_name}' not found in DBA")))?;

    Ok(clip_final_pose(clip))
}

/// Build a final-frame `BonePose` map from a single animation clip.
///
/// Quaternions are converted to Blender Z-up `wxyz` via
/// [`cry_xyzw_to_blender_wxyz`] and positions get the same axis swap.
pub fn clip_final_pose(clip: &AnimationClip) -> HashMap<u32, BonePose> {
    let mut poses = HashMap::with_capacity(clip.channels.len());
    for ch in &clip.channels {
        let rotation = ch
            .rotations
            .last()
            .map(|kf| cry_xyzw_to_blender_wxyz(kf.value))
            .unwrap_or([1.0, 0.0, 0.0, 0.0]);
        let position = ch.positions.last().map(|kf| {
            let [x, y, z] = kf.value;
            [x, -z, y]
        });
        poses.insert(ch.bone_hash, BonePose { rotation, position });
    }
    poses
}

/// Pick the best matching animation clip in `db` for a given skeleton, by
/// bone-hash signature.
///
/// A clip is a *candidate* iff every one of its channel bone hashes is present
/// in the skeleton (clip bones ⊆ skeleton bones). The first candidate is
/// returned, with the option to break ties by selecting the clip with the
/// largest angular delta between first and last keyframe.
pub fn find_block_for_skeleton<'a>(
    db: &'a AnimationDatabase,
    skeleton_bone_hashes: &HashSet<u32>,
    prefer_longest_arc: bool,
) -> Option<&'a AnimationClip> {
    let candidates: Vec<&AnimationClip> = db
        .clips
        .iter()
        .filter(|c| {
            !c.channels.is_empty()
                && c.channels
                    .iter()
                    .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }
    if !prefer_longest_arc || candidates.len() == 1 {
        return Some(candidates[0]);
    }

    candidates
        .into_iter()
        .map(|c| (clip_arc_score(c), c))
        .fold(None, |acc, (s, c)| match acc {
            None => Some((s, c)),
            Some((bs, _)) if s > bs => Some((s, c)),
            other => other,
        })
        .map(|(_, c)| c)
}

/// Sum of angular deltas between first and last rotation key across all
/// channels (radians, sign-invariant). Higher = more motion.
fn clip_arc_score(clip: &AnimationClip) -> f32 {
    let mut total = 0.0f32;
    for ch in &clip.channels {
        if let (Some(first), Some(last)) = (ch.rotations.first(), ch.rotations.last()) {
            let a = first.value;
            let b = last.value;
            let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs();
            total += 1.0 - dot.clamp(0.0, 1.0);
        }
    }
    total
}

/// Quaternion multiplication on `wxyz` quaternions (Blender convention).
pub(crate) fn quat_mul_wxyz(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [aw, ax, ay, az] = a;
    let [bw, bx, by, bz] = b;
    [
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    ]
}

/// Rotate a 3-vector by a `wxyz` unit quaternion.
pub(crate) fn quat_rotate_vec_wxyz(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [w, x, y, z] = q;
    let [vx, vy, vz] = v;
    let tx = 2.0 * (y * vz - z * vy);
    let ty = 2.0 * (z * vx - x * vz);
    let tz = 2.0 * (x * vy - y * vx);
    [
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    ]
}

/// Bone-like accessor: minimal interface needed to bake a pose without a
/// circular dependency on `crate::skeleton`.
pub trait BoneTransforms {
    fn name(&self) -> &str;
    fn parent_index(&self) -> Option<usize>;
    fn local_rotation_wxyz(&self) -> [f32; 4];
    fn local_position(&self) -> [f32; 3];
    fn set_local_rotation_wxyz(&mut self, q: [f32; 4]);
    fn set_local_position(&mut self, p: [f32; 3]);
    fn set_world_rotation_wxyz(&mut self, q: [f32; 4]);
    fn set_world_position(&mut self, p: [f32; 3]);
}

/// Compute the CRC32 hash that DBA uses for a bone name.
///
/// CryEngine uses standard CRC32 (zlib polynomial) on the **case-preserved**
/// UTF-8 byte sequence of the bone name (no terminator).
pub fn bone_name_hash(name: &str) -> u32 {
    crc32fast::hash(name.as_bytes())
}

/// Apply a final-frame `pose` to a slice of bones, overwriting both their
/// local TRS and their cached world TRS. Returns the number of updated bones.
pub fn apply_pose_to_skeleton<B: BoneTransforms>(
    bones: &mut [B],
    pose: &HashMap<u32, BonePose>,
) -> usize {
    let mut updated = 0usize;
    for bone in bones.iter_mut() {
        let h = bone_name_hash(bone.name());
        if let Some(p) = pose.get(&h) {
            bone.set_local_rotation_wxyz(p.rotation);
            if let Some(pos) = p.position {
                bone.set_local_position(pos);
            }
            updated += 1;
        }
    }
    if updated == 0 {
        return 0;
    }

    let n = bones.len();
    let mut world_q: Vec<[f32; 4]> = vec![[1.0, 0.0, 0.0, 0.0]; n];
    let mut world_p: Vec<[f32; 3]> = vec![[0.0; 3]; n];
    let mut done: Vec<bool> = vec![false; n];

    let mut progress = true;
    while progress {
        progress = false;
        for i in 0..n {
            if done[i] {
                continue;
            }
            let parent = bones[i].parent_index();
            let (pq, pp) = match parent {
                None => ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                Some(pi) if pi == i => ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                Some(pi) if pi < n && done[pi] => (world_q[pi], world_p[pi]),
                Some(_) => continue,
            };
            let lq = bones[i].local_rotation_wxyz();
            let lp = bones[i].local_position();
            world_q[i] = quat_mul_wxyz(pq, lq);
            let rotated = quat_rotate_vec_wxyz(pq, lp);
            world_p[i] = [pp[0] + rotated[0], pp[1] + rotated[1], pp[2] + rotated[2]];
            done[i] = true;
            progress = true;
        }
    }

    for i in 0..n {
        if done[i] {
            bones[i].set_world_rotation_wxyz(world_q[i]);
            bones[i].set_world_position(world_p[i]);
        }
    }

    updated
}
