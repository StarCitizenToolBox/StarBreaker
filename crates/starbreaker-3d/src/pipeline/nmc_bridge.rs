use starbreaker_p4k::MappedP4k;

use super::{datacore_path_to_p4k, P4kSiblingReader};


/// Compute world-space transforms for each NMC node by walking the parent chain.
pub(crate) fn compute_nmc_world_transforms(nmc: &crate::nmc::NodeMeshCombo) -> Vec<glam::Mat4> {
    let local: Vec<glam::Mat4> = nmc
        .nodes
        .iter()
        .map(|n| {
            let m = &n.bone_to_world;
            glam::Mat4::from_cols_array(&[
                m[0][0], m[1][0], m[2][0], 0.0,
                m[0][1], m[1][1], m[2][1], 0.0,
                m[0][2], m[1][2], m[2][2], 0.0,
                m[0][3], m[1][3], m[2][3], 1.0,
            ])
        })
        .collect();

    let n = nmc.nodes.len();
    let mut world = vec![None; n];

    fn resolve(
        i: usize,
        nodes: &[crate::nmc::NmcNode],
        local: &[glam::Mat4],
        world: &mut [Option<glam::Mat4>],
    ) -> glam::Mat4 {
        if let Some(w) = world[i] {
            return w;
        }
        let w = match nodes[i].parent_index {
            Some(pi) if (pi as usize) < nodes.len() && (pi as usize) != i => {
                resolve(pi as usize, nodes, local, world) * local[i]
            }
            _ => local[i],
        };
        world[i] = Some(w);
        w
    }

    for i in 0..n {
        resolve(i, &nmc.nodes, &local, &mut world);
    }

    world.into_iter().flatten().collect()
}


/// Convert a column-major 4x4 array into a glam::Mat4.
pub(crate) fn mat4_from_array(m: &[[f32; 4]; 4]) -> glam::Mat4 {
    glam::Mat4::from_cols_array_2d(m)
}


/// Convert a glam::Mat4 back to the column-major array form used in placements.
pub(crate) fn mat4_to_array(m: glam::Mat4) -> [[f32; 4]; 4] {
    m.to_cols_array_2d()
}


pub(crate) fn mat3x4_from_mat4(m: glam::Mat4) -> [[f32; 4]; 3] {
    let cols = m.to_cols_array_2d();
    [
        [cols[0][0], cols[1][0], cols[2][0], cols[3][0]],
        [cols[0][1], cols[1][1], cols[2][1], cols[3][1]],
        [cols[0][2], cols[1][2], cols[2][2], cols[3][2]],
    ]
}


pub(crate) fn bone_world_transform(bone: &crate::skeleton::Bone) -> glam::Mat4 {
    let rotation = glam::Quat::from_xyzw(
        bone.world_rotation[1],
        bone.world_rotation[2],
        bone.world_rotation[3],
        bone.world_rotation[0],
    );
    glam::Mat4::from_rotation_translation(rotation, glam::Vec3::from(bone.world_position))
}


pub(crate) fn synthesize_nmc_from_bones(
    mesh: &crate::types::Mesh,
    bones: &[crate::skeleton::Bone],
) -> Option<crate::nmc::NodeMeshCombo> {
    if bones.is_empty() || mesh.submeshes.is_empty() {
        return None;
    }

    let mut referenced_node_indices = std::collections::BTreeSet::new();
    for submesh in &mesh.submeshes {
        let index = submesh.node_parent_index as usize;
        if index >= bones.len() {
            return None;
        }
        referenced_node_indices.insert(index);
    }

    if referenced_node_indices.len() <= 1 {
        return None;
    }

    let world_transforms = bones.iter().map(bone_world_transform).collect::<Vec<_>>();
    let root_index = bones
        .iter()
        .enumerate()
        .find(|(index, bone)| bone.parent_index.is_none() || bone.parent_index == Some(*index as u16))
        .map(|(index, _)| index)
        .unwrap_or(0);
    let root_inv = world_transforms[root_index].inverse();
    let nodes = bones
        .iter()
        .enumerate()
        .map(|(index, bone)| {
            let parent_index = bone
                .parent_index
                .filter(|parent| (*parent as usize) < bones.len() && *parent as usize != index);
            let relative = if let Some(parent) = parent_index {
                let _ = parent;
                glam::Mat4::from_rotation_translation(
                    glam::Quat::from_xyzw(
                        bone.local_rotation[1],
                        bone.local_rotation[2],
                        bone.local_rotation[3],
                        bone.local_rotation[0],
                    ),
                    glam::Vec3::from_array(bone.local_position),
                )
            } else {
                root_inv * world_transforms[index]
            };
            crate::nmc::NmcNode {
                name: bone.name.clone(),
                parent_index,
                world_to_bone: mat3x4_from_mat4(relative.inverse()),
                bone_to_world: mat3x4_from_mat4(relative),
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                properties: std::collections::HashMap::new(),
            }
        })
        .collect();

    Some(crate::nmc::NodeMeshCombo {
        nodes,
        material_indices: vec![0; bones.len()],
    })
}


/// Load NMC node table for a CGF/CGA file. The metadata is bundled with the
/// .cgf itself in Ivo-format files; for split files (.cgf + .cgfm) the table
/// lives in the .cgfm sidecar.
pub(crate) fn load_nmc_for_cgf(p4k: &MappedP4k, cgf_path: &str) -> Option<crate::nmc::NodeMeshCombo> {
    let try_path = |path: &str| -> Option<crate::nmc::NodeMeshCombo> {
        let p4k_path = datacore_path_to_p4k(path);
        let bytes = p4k.entry_case_insensitive(&p4k_path).and_then(|e| p4k.read(e).ok())?;
        let (nodes, material_indices) = crate::nmc::parse_nmc_full(&bytes)?;
        Some(crate::nmc::NodeMeshCombo { nodes, material_indices })
    };
    if let Some(nmc) = try_path(cgf_path) {
        return Some(nmc);
    }
    let lower = cgf_path.to_lowercase();
    if lower.ends_with(".cgf") || lower.ends_with(".cga") {
        let sidecar = format!("{cgf_path}m");
        if let Some(nmc) = try_path(&sidecar) {
            return Some(nmc);
        }
    }
    None
}


/// Compose a child-attachment transform from a parent's NMC + named helper bone
/// + per-port `Offset` (Position + Euler-degree Rotation, CryEngine X,Y,Z order).
///
/// Returns identity if the parent NMC is unavailable, or if the helper bone
/// cannot be located. Callers should still emit the placement so the geometry
/// is not silently dropped.
pub(crate) fn compose_helper_transform(
    parent_nmc: Option<&crate::nmc::NodeMeshCombo>,
    helper_name: Option<&str>,
    offset_pos: [f32; 3],
    offset_rot_deg: [f32; 3],
) -> glam::Mat4 {
    let helper_local = if let (Some(nmc), Some(name)) = (parent_nmc, helper_name) {
        let world = compute_nmc_world_transforms(nmc);
        let lower_name = name.to_ascii_lowercase();
        match nmc
            .nodes
            .iter()
            .position(|n| n.name.eq_ignore_ascii_case(&lower_name))
        {
            Some(i) if i < world.len() => world[i],
            _ => {
                log::debug!(
                    "  loadout helper bone '{name}' not found in parent NMC ({} nodes)",
                    nmc.nodes.len()
                );
                glam::Mat4::IDENTITY
            }
        }
    } else {
        glam::Mat4::IDENTITY
    };

    let offset = if offset_pos == [0.0; 3] && offset_rot_deg == [0.0; 3] {
        glam::Mat4::IDENTITY
    } else {
        let r = glam::Mat4::from_euler(
            glam::EulerRot::XYZ,
            offset_rot_deg[0].to_radians(),
            offset_rot_deg[1].to_radians(),
            offset_rot_deg[2].to_radians(),
        );
        let t = glam::Mat4::from_translation(glam::Vec3::from(offset_pos));
        t * r
    };

    helper_local * offset
}


/// Bake NMC node transforms into mesh vertex positions.
///
/// When `use_root_inv` is true, transforms are made root-relative by factoring
/// out the root node's world transform (used for instanced geometry where scaling
/// bbox = model bbox). When false, absolute world transforms are used (for
/// interior CGFs where scaling bbox ≠ model bbox).
pub(crate) fn bake_nmc_into_mesh(
    mut mesh: crate::types::Mesh,
    nmc: Option<&crate::nmc::NodeMeshCombo>,
    use_root_inv: bool,
) -> crate::types::Mesh {
    let nmc = match nmc {
        Some(n) if !n.nodes.is_empty() => n,
        _ => return mesh,
    };

    let world_transforms = compute_nmc_world_transforms(nmc);

    let root_inv = if use_root_inv {
        let root_idx = nmc.nodes.iter().position(|n| n.parent_index.is_none());
        root_idx
            .map(|i| world_transforms[i].inverse())
            .unwrap_or(glam::Mat4::IDENTITY)
    } else {
        glam::Mat4::IDENTITY
    };

    let mut vert_node: Vec<Option<usize>> = vec![None; mesh.positions.len()];
    for sub in &mesh.submeshes {
        let node_idx = sub.node_parent_index as usize;
        if node_idx >= world_transforms.len() {
            continue;
        }
        let start = sub.first_index as usize;
        let end = (start + sub.num_indices as usize).min(mesh.indices.len());
        for &idx in &mesh.indices[start..end] {
            let vi = idx as usize;
            if vi < vert_node.len() && vert_node[vi].is_none() {
                vert_node[vi] = Some(node_idx);
            }
        }
    }

    for (vi, node_opt) in vert_node.iter().enumerate() {
        let Some(node_idx) = node_opt else { continue };
        let xform = root_inv * world_transforms[*node_idx];
        if xform == glam::Mat4::IDENTITY {
            continue;
        }
        let v = xform.transform_point3(glam::Vec3::from(mesh.positions[vi]));
        mesh.positions[vi] = v.into();
        if let Some(ref mut normals) = mesh.normals {
            if vi < normals.len() {
                let normal_mat = xform.inverse().transpose();
                let n = normal_mat
                    .transform_vector3(glam::Vec3::from(normals[vi]))
                    .normalize();
                normals[vi] = n.into();
            }
        }
    }

    mesh
}

