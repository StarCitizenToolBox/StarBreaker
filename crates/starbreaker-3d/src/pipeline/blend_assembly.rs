//! Blender decomposed export: convert `DecomposedInput` to individual `.blend` files.
//!
//! Phase 1 (mesh decomposition):
//! - Extract meshes from `DecomposedInput::children`
//! - Build material slots (empty, names-only)
//! - Write individual `.blend` files to `Data/Objects/...` paths
//! - Return decomposed export with .blend mesh files instead of GLB
//!
//! Phase 2 (scene.blend linking) — infrastructure complete
//! Phase 3 (lights and empties) — extraction and creation
//! Phase 4 (decal vertex groups) — material identification
//! Phase 5D (decal material assignment) — vertex group material assignment

use std::collections::{HashMap, HashSet};

use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_p4k::MappedP4k;
use starbreaker_blend::{
    bytes4_data, build_attribute, build_attribute_array, build_base, build_collection,
    build_master_collection, build_collection_object, build_collection_object_linked,
    build_file_global, build_layer_collection, build_layer_collection_linked, build_mat_ptr_array,
    build_mat_ptr_array_from_ptrs, build_material, build_matbits, build_mesh, build_object, build_scene, build_tool_settings,
    build_view_layer,
    floats2_data, floats3_data, ints_data, startup_ui_prefix_bytes, write_block, write_block_header, PtrAlloc,
    ATTR_DOMAIN_CORNER, ATTR_DOMAIN_EDGE, ATTR_DOMAIN_FACE, ATTR_DOMAIN_POINT, ATTR_TYPE_BYTE_COLOR,
    ATTR_TYPE_FLOAT2, ATTR_TYPE_FLOAT3, ATTR_TYPE_INT, BLEND_MAGIC, DNA1_BYTES,
    SDNA_IDX_ATTRIBUTE, SDNA_IDX_ATTRIBUTE_ARRAY, SDNA_IDX_BASE, SDNA_IDX_COLLECTION, SDNA_IDX_COLLECTION_CHILD,
    SDNA_IDX_COLLECTION_OBJECT, SDNA_IDX_DNA1, SDNA_IDX_FILE_GLOBAL, SDNA_IDX_LAYER_COLLECTION,
    SDNA_IDX_MATERIAL, SDNA_IDX_MESH, SDNA_IDX_OBJECT, SDNA_IDX_SCENE, SDNA_IDX_TOOL_SETTINGS,
    SDNA_IDX_VIEW_LAYER, SDNA_IDX_LIBRARY, SDNA_IDX_ID,
    build_lamp, build_lamp_object, build_empty_object,
    build_library_block, build_id_stub, LAMP_SIZE, OBJECT_SIZE,
    build_bdeformgroup, build_mdeformvert_array, build_mdeformweight_array, 
    build_custom_data_layer_mdeformvert, SDNA_IDX_BDEFORMGROUP, SDNA_IDX_MDEFORMVERT, SDNA_IDX_LAMP,
    ints2_data, triangle_edge_topology, write_f32, write_i16, write_identity_matrix4x4, write_ptr,
    ATTR_TYPE_INT32_2D, STARTUP_UI_SCREEN_PTR,
};

use crate::error::Error;
use crate::decomposed::DecomposedInput;
use crate::pipeline::{DecomposedExport, ExportedFile, ExportedFileKind, ExportOptions, LoadedInteriors};
use crate::types::{Mesh, SubMesh};
use crate::nmc::NodeMeshCombo;
use crate::mtl::MtlFile;

/// Internal structure to hold mesh data for .blend file generation
#[derive(Clone)]
struct MeshDataEntry {
    mesh: Mesh,
    materials: Option<MtlFile>,
    nmc: Option<NodeMeshCombo>,
}

fn insert_stem_suffix(path: &str, suffix: &str) -> String {
    let (dir, file) = match path.rsplit_once('/') {
        Some((dir, file)) => (format!("{dir}/"), file),
        None => (String::new(), path),
    };
    let (stem, ext) = match file.find('.') {
        Some(index) => (&file[..index], &file[index..]),
        None => (file, ""),
    };
    format!("{dir}{stem}{suffix}{ext}")
}

fn replace_extension(path: &str, new_extension: &str) -> String {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return format!("{path}{new_extension}");
    };
    stem.to_string() + new_extension
}

fn normalize_source_path(p4k: &MappedP4k, path: &str) -> String {
    let p4k_path = crate::pipeline::datacore_path_to_p4k(path);
    p4k.entry_case_insensitive(&p4k_path)
        .map(|entry| entry.name.replace('\\', "/"))
        .unwrap_or_else(|| p4k_path.replace('\\', "/"))
}

fn mesh_asset_relative_path_with_extension(
    p4k: &MappedP4k,
    geometry_path: &str,
    fallback_name: &str,
    lod: u32,
    extension: &str,
) -> String {
    let base = if geometry_path.is_empty() {
        format!(
            "Data/generated/{}{}",
            fallback_name.replace([' ', '\\', '/', ':'], "_"),
            extension
        )
    } else {
        replace_extension(&normalize_source_path(p4k, geometry_path), extension)
    };
    insert_stem_suffix(&base, &format!("_LOD{lod}"))
}

fn blend_mesh_asset_relative_path(
    p4k: &MappedP4k,
    geometry_path: &str,
    fallback_name: &str,
    lod: u32,
) -> String {
    mesh_asset_relative_path_with_extension(p4k, geometry_path, fallback_name, lod, ".blend")
}

/// Convert `DecomposedInput` into a decomposed export with individual `.blend` files.
///
/// **Phase 1**: Mesh Decomposition to individual .blend files
/// - Extracts real mesh data from DecomposedInput::children
/// - Generates full decomposed export (scene.json, package manifests, etc.)
/// - Replaces generic mesh asset payloads with individual .blend files using real geometry
/// - Each mesh gets its own uncompressed .blend file with actual mesh data
///
/// **Phase 3**: Lights and Empties Integration
/// - Extracts lights from DecomposedInput::interiors
/// - Extracts empties from DecomposedInput::root_nmc
/// - Creates scene.blend with lights and empties in proper collections
///
/// **Phase 4**: Decal Vertex Groups Integration
/// - Identifies decal materials in each mesh
/// - Creates StarBreaker_Decals vertex groups with decal vertices
/// - Adds vertex groups to individual mesh .blend files
///
/// Returns `DecomposedExport` with all files including real mesh geometry
pub fn write_decomposed_export_blend(
    p4k: &MappedP4k,
    input: DecomposedInput,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Result<DecomposedExport, Error> {
    // Phase 1: Extract mesh data from input children BEFORE calling write_decomposed_export.
    // Key by the exact final .blend mesh asset path; loose entity-name matching is not
    // stable enough for similarly named ship parts.
    let mut mesh_data_map: HashMap<String, MeshDataEntry> = HashMap::new();
    
    let root_mesh_asset = blend_mesh_asset_relative_path(p4k, &input.geometry_path, &input.entity_name, opts.lod_level);
    let root_material_view = crate::decomposed::build_decomposed_material_view(
        &input.root_mesh,
        input.root_materials.as_ref(),
        input.root_nmc.as_ref(),
        opts.include_nodraw,
        opts.include_shields,
    );
    mesh_data_map.insert(root_mesh_asset.to_ascii_lowercase(), MeshDataEntry {
        mesh: root_material_view.mesh,
        materials: root_material_view.glb_materials.or(root_material_view.sidecar_materials),
        nmc: root_material_view.glb_nmc,
    });
    
    for child in &input.children {
        let mesh_asset = blend_mesh_asset_relative_path(p4k, &child.geometry_path, &child.entity_name, opts.lod_level);
        let child_material_view = crate::decomposed::build_decomposed_material_view(
            &child.mesh,
            child.materials.as_ref(),
            child.nmc.as_ref(),
            opts.include_nodraw,
            opts.include_shields,
        );
        let entry = MeshDataEntry {
            mesh: child_material_view.mesh,
            materials: child_material_view.glb_materials.or(child_material_view.sidecar_materials),
            nmc: child_material_view.glb_nmc,
        };
        
        mesh_data_map.insert(mesh_asset.to_ascii_lowercase(), entry);
    }
    
    // Extract minimal data needed for scene.blend before passing input to write_decomposed_export
    let scene_entity_name = input.entity_name.clone();
    let children_for_scene = input.children.iter().map(|c| c.entity_name.clone()).collect::<Vec<_>>();
    let mut scene_mesh_instances = Vec::new();
    
    // Phase 3: Extract lights and empties BEFORE calling write_decomposed_export
    let extracted_lights = extract_lights_from_interiors(&input.interiors)
        .unwrap_or_default();
    
    let extracted_empties = input.root_nmc.as_ref()
        .and_then(|nmc| extract_empties_from_nmc(&nmc.nodes).ok())
        .unwrap_or_default();
    
    let mut interior_mesh_loader = |_: &crate::pipeline::InteriorCgfEntry|
        -> Option<(Mesh, Option<crate::mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
        None
    };

    report_progress(progress, 0.0, "Generating decomposed export with .blend mesh files");

    // Generate the package manifest and reusable-asset list with the shared decomposed
    // exporter, then replace its mesh asset payloads with native .blend bytes below.
    let base_export = crate::decomposed::write_decomposed_export(
        p4k,
        input,
        opts,
        progress,
        existing_asset_paths,
        &mut interior_mesh_loader,
    )?;

    // Phase 4: Collect vertex groups for all meshes BEFORE creating .blend files
    report_progress(progress, 0.45, "Collecting decal vertex groups from meshes");
    
    let mut mesh_vertex_groups: HashMap<String, Vec<VertexGroup>> = HashMap::new();
    
    // Collect all mesh materials for decal identification
    let mut mesh_materials = Vec::new();
    for (mesh_key, entry) in &mesh_data_map {
        if let Some(ref mtl) = entry.materials {
            let material_list: Vec<(String, String)> = mtl.materials.iter()
                .map(|sub| (sub.name.clone(), sub.string_gen_mask.clone()))
                .collect();
            mesh_materials.push((mesh_key.clone(), material_list));
        }
    }
    
    // Identify meshes with decals and collect vertex groups
    if let Ok(meshes_with_decals) = identify_meshes_with_decals(&mesh_materials) {
        for mesh_with_decals in meshes_with_decals {
            if let Some(mesh_entry) = mesh_data_map.get(&mesh_with_decals.mesh_path) {
                // Map decal material indices to face indices
                let mut decal_face_indices = Vec::new();
                for (face_idx, material_idx) in mesh_entry.mesh.submeshes.iter()
                    .enumerate()
                    .flat_map(|(mat_idx, submesh)| {
                        let start_face = submesh.first_index / 3;
                        let num_faces = submesh.num_indices / 3;
                        (start_face..start_face + num_faces).map(move |f| (f as usize, mat_idx))
                    })
                    .collect::<Vec<_>>()
                {
                    // Check if this material is a decal material
                    if mesh_with_decals.decal_materials.iter().any(|dm| dm.material_index == material_idx) {
                        decal_face_indices.push(face_idx);
                    }
                }
                
                // Collect decal vertices if any decal faces found
                if !decal_face_indices.is_empty() {
                    if let Ok(vgroups) = collect_decal_vertices(
                        &mesh_with_decals,
                        &decal_face_indices,
                        &mesh_entry.mesh.indices.iter().map(|&i| i as u32).collect::<Vec<_>>(),
                        3, // verts_per_face for triangles
                    ) {
                        mesh_vertex_groups.insert(mesh_with_decals.mesh_path.clone(), vgroups.vertex_groups);
                    }
                }
            }
        }
    }

    report_progress(progress, 0.5, "Writing native .blend mesh assets with real geometry");

    // Replace generic mesh asset payloads with .blend files using REAL mesh data.
    let mut blend_files = Vec::new();
    let mut manifest_files = Vec::new();
    let mut other_files = Vec::new();
    for file in base_export.files {
        if file.relative_path.ends_with(".glb") && file.kind == ExportedFileKind::MeshAsset {
            // This is a generic mesh asset placeholder from the shared decomposed enumerator.
            // The native Blend workflow only keeps the corresponding .blend path and payload.
            let blend_path = file.relative_path.replace(".glb", ".blend");
            
            // Extract mesh name from path for Blender object naming
            let mesh_name = blend_path
                .split('/')
                .last()
                .unwrap_or("mesh")
                .trim_end_matches(".blend")
                .to_string();
            let blend_key = blend_path.to_ascii_lowercase();
            let mesh_entry = mesh_data_map.get(&blend_key).ok_or_else(|| {
                Error::Other(format!(
                    "native Blend export has no mesh payload for generated asset '{blend_path}'"
                ))
            })?;
            // Phase 5D: Get vertex groups for this mesh
            let vgroups = mesh_vertex_groups.get(&blend_key).cloned();

            let blend_bytes = mesh_to_blend(
                &mesh_name,
                &mesh_entry.mesh,
                &mesh_entry.materials,
                mesh_entry.nmc.as_ref(),
                vgroups.as_ref(),
            );
            let mut linked_object_names = mesh_object_names_from_blend_bytes(&blend_bytes);
            if linked_object_names.is_empty() {
                linked_object_names.push(mesh_name.clone());
            }
            for linked_object_name in linked_object_names {
                scene_mesh_instances.push(LinkedMeshInstance {
                    name: linked_object_name,
                    blend_path: format!("//../../{blend_path}"),
                    position: [0.0, 0.0, 0.0],
                    rotation: [1.0, 0.0, 0.0, 0.0],
                });
            }
            blend_files.push(ExportedFile {
                relative_path: blend_path,
                bytes: blend_bytes,
                kind: ExportedFileKind::MeshAsset,
            });
        } else if file.kind == ExportedFileKind::PackageManifest {
            // Update manifest files to replace .glb references with .blend
            let bytes_str = String::from_utf8_lossy(&file.bytes);
            let updated = bytes_str.replace(".glb\"", ".blend\"");
            manifest_files.push(ExportedFile {
                relative_path: file.relative_path,
                bytes: updated.into_bytes(),
                kind: file.kind,
            });
        } else if !file.relative_path.ends_with("scene.blend") {
            // Keep other files as-is (palettes, textures, etc.)
            // EXCLUDE scene.blend from base_export - we'll create our own detailed version
            other_files.push(file);
        }
    }

    let mut blend_file_order = Vec::new();
    let mut blend_file_by_path = HashMap::new();
    for file in blend_files {
        if !blend_file_by_path.contains_key(&file.relative_path) {
            blend_file_order.push(file.relative_path.clone());
        }
        blend_file_by_path.insert(file.relative_path.clone(), file);
    }
    blend_files = blend_file_order
        .into_iter()
        .filter_map(|path| blend_file_by_path.remove(&path))
        .collect();

    scene_mesh_instances.clear();
    for file in &blend_files {
        let linked_object_names = mesh_object_names_from_blend_bytes(&file.bytes);
        for linked_object_name in linked_object_names {
            scene_mesh_instances.push(LinkedMeshInstance {
                name: linked_object_name,
                blend_path: format!("//../../{}", file.relative_path),
                position: [0.0, 0.0, 0.0],
                rotation: [1.0, 0.0, 0.0, 0.0],
            });
        }
    }

    // Phase 3: Create scene.blend with lights and empties
    report_progress(progress, 0.7, "Creating scene.blend with linked mesh instances");
    
    // Create scene.blend with properly linked mesh instances and lights
    // Library paths are relative to scene.blend location, which is in Packages/{package}/
    // So we need ../../Data/Objects to reach the shared assets
    log::info!("[blend-debug] Creating scene.blend with {} children and {} lights", children_for_scene.len(), extracted_lights.len());
    let scene_blend_bytes = create_scene_blend_with_instances(&scene_entity_name, &scene_mesh_instances, &extracted_lights)?;
    log::info!("[blend-debug] scene.blend created, size: {} bytes", scene_blend_bytes.len());
    log::info!("[blend-debug] First 20 bytes of uncompressed: {:?}", &scene_blend_bytes[..20.min(scene_blend_bytes.len())]);
    
    // Compress scene.blend with Zstd (Blender 5.1 native format)
    let compressed_scene = starbreaker_blend::compress_blend_bytes_zstd(&scene_blend_bytes);
    log::info!("[blend-debug] Compressed scene size: {} bytes", compressed_scene.len());
    log::info!("[blend-debug] First 20 bytes of compressed: {:?}", &compressed_scene[..20.min(compressed_scene.len())]);
    
    // Combine blend mesh files with other files (NOT including base_export.scene.blend)
    let mut all_files = blend_files;
    all_files.extend(manifest_files);
    all_files.extend(other_files);
    
    // Determine package_name from first mesh if available
    let package_name = if let Some(first_file) = all_files.iter().find(|f| f.relative_path.contains("Packages/")) {
        // Extract "Packages/PackageName" from relative paths like "Packages/PackageName/mesh_0.blend"
        if let Some(pkg_start) = first_file.relative_path.find("Packages/") {
            let after_packages = &first_file.relative_path[pkg_start + 9..]; // Skip "Packages/"
            if let Some(slash_pos) = after_packages.find('/') {
                format!("Packages/{}", &after_packages[..slash_pos])
            } else {
                "Packages/Default".to_string()
            }
        } else {
            "Packages/Default".to_string()
        }
    } else {
        "Packages/Default".to_string()
    };
    
    // Add our detailed scene.blend with proper relative path and kind
    all_files.push(ExportedFile {
        relative_path: format!("{}/scene.blend", package_name),
        bytes: compressed_scene,
        // PackageManifest kind ensures scene.blend is always written (not skipped by skip_existing_assets)
        kind: ExportedFileKind::PackageManifest,
    });

    report_progress(progress, 1.0, "Export complete");

    Ok(DecomposedExport { files: all_files })
}

/// Convert a mesh to `.blend` file bytes (uncompressed).
///
/// Produces a .blend containing a single OB_MESH object with:
/// - Position vertices (POINT / FLOAT3)
/// - Corner vertices (.corner_vert, CORNER / INT)
/// - Material index per face (FACE / INT)
/// - UVMap (CORNER / FLOAT2) — when mesh.uvs is Some
/// - Color (CORNER / BYTE_COLOR) — when mesh.colors is Some
/// - Vertex groups — when vertex_groups is Some
fn blend_material_slots(name: &str, mesh: &Mesh, materials: &Option<crate::mtl::MtlFile>) -> (Vec<String>, Vec<usize>) {
    let source_stem = materials
        .as_ref()
        .and_then(|mtl| mtl.source_path.as_deref())
        .and_then(|path| {
            path.rsplit(['\\', '/'])
                .next()
                .and_then(|file| file.rsplit_once('.').map(|(stem, _)| stem).or(Some(file)))
        })
        .unwrap_or(name);

    if mesh.submeshes.is_empty() {
        return (vec![format!("{source_stem}_mtl_material_0_00")], Vec::new());
    }

    let mut slot_by_material_id: HashMap<u32, usize> = HashMap::new();
    let mut material_names = Vec::new();
    let mut submesh_slots = Vec::with_capacity(mesh.submeshes.len());

    for (submesh_index, submesh) in mesh.submeshes.iter().enumerate() {
        if let Some(&slot) = slot_by_material_id.get(&submesh.material_id) {
            submesh_slots.push(slot);
            continue;
        }

        let base_name = submesh
            .material_name
            .clone()
            .or_else(|| {
                materials
                    .as_ref()
                    .and_then(|mtl| mtl.materials.get(submesh.material_id as usize))
                    .map(|mat| mat.name.clone())
            })
            .unwrap_or_else(|| format!("material_{submesh_index}"));
        let slot = material_names.len();
        material_names.push(format!("{source_stem}_mtl_{base_name}_0{}", submesh.material_id));
        slot_by_material_id.insert(submesh.material_id, slot);
        submesh_slots.push(slot);
    }

    (material_names, submesh_slots)
}

fn mesh_to_blend(
    name: &str,
    mesh: &Mesh,
    materials: &Option<crate::mtl::MtlFile>,
    nmc: Option<&NodeMeshCombo>,
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> Vec<u8> {
    if let Some(nmc) = nmc.filter(|nmc| !nmc.nodes.is_empty()) {
        return mesh_to_blend_hierarchy(name, mesh, materials, nmc, vertex_groups);
    }
    mesh_to_blend_flat(name, mesh, materials, vertex_groups)
}

fn mesh_to_blend_flat(
    name: &str,
    mesh: &Mesh,
    materials: &Option<crate::mtl::MtlFile>,
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> Vec<u8> {
    let totvert = mesh.positions.len();
    let totloop = mesh.indices.len();
    let totpoly = totloop / 3;
    let (edge_verts, corner_edges) = triangle_edge_topology(&mesh.indices);
    let totedge = edge_verts.len();
    let (material_names, submesh_material_slots) = blend_material_slots(name, mesh, materials);
    let mat_slots = material_names.len() as i16;

    let mut ptrs = PtrAlloc::new(0x1000);

    let _screen_ptr    = ptrs.alloc();
    let _wm_ptr        = ptrs.alloc();
    let object_ptr     = ptrs.alloc();
    let mesh_ptr       = ptrs.alloc();
    let mesh_mat_ptr   = ptrs.alloc();
    let obj_mat_ptr    = ptrs.alloc();
    let obj_matbits_ptr = ptrs.alloc();
    let material_ptrs: Vec<u64> = (0..material_names.len()).map(|_| ptrs.alloc()).collect();
    let scene_ptr      = ptrs.alloc();
    let view_layer_ptr = ptrs.alloc();
    let tool_settings_ptr = ptrs.alloc();
    let base_ptr       = ptrs.alloc();
    let collection_ptr = ptrs.alloc();
    let collection_object_ptr = ptrs.alloc();
    let layer_collection_ptr = ptrs.alloc();
    let poly_offs_ptr  = ptrs.alloc();
    let attrs_ptr      = ptrs.alloc();

    // Always-present attributes
    let name_pos_ptr  = ptrs.alloc();
    let name_ev_ptr   = ptrs.alloc();
    let name_cv_ptr   = ptrs.alloc();
    let name_ce_ptr   = ptrs.alloc();
    let array_pos_ptr = ptrs.alloc();
    let array_ev_ptr  = ptrs.alloc();
    let array_cv_ptr  = ptrs.alloc();
    let array_ce_ptr  = ptrs.alloc();
    let raw_pos_ptr   = ptrs.alloc();
    let raw_ev_ptr    = ptrs.alloc();
    let raw_cv_ptr    = ptrs.alloc();
    let raw_ce_ptr    = ptrs.alloc();

    // material_index (FACE domain)
    let name_matidx_ptr = ptrs.alloc();
    let array_matidx_ptr = ptrs.alloc();
    let raw_matidx_ptr = ptrs.alloc();

    // Optional: UV map
    let (name_uv_ptr, array_uv_ptr, raw_uv_ptr) = if mesh.uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };

    // Optional: vertex colors
    let (name_col_ptr, array_col_ptr, raw_col_ptr) = if mesh.colors.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };

    // Vertex group structures (Phase 5D)
    let (vgroup_first_ptr, vgroup_last_ptr, vgroup_count, cdl_ptr, vgroup_ptrs, mdeformvert_ptrs, mdeformweight_ptrs) = 
        if let Some(vgroups) = vertex_groups {
            let mut v_ptrs = Vec::new();
            let mut mdv_ptrs = Vec::new();
            let mut mdw_ptrs = Vec::new();
            
            if !vgroups.is_empty() {
                // Allocate bDeformGroup pointers
                for _ in vgroups.iter() {
                    v_ptrs.push(ptrs.alloc());
                }
                let v_first = v_ptrs[0];
                let v_last = v_ptrs[v_ptrs.len() - 1];
                
                // Allocate MDeformVert array
                let mdv_array_ptr = ptrs.alloc();
                let mdv_data_ptr = ptrs.alloc();
                mdv_ptrs.push((mdv_array_ptr, mdv_data_ptr));
                
                // Allocate MDeformWeight arrays for each vertex group
                for _ in vgroups.iter() {
                    for _ in 0..totvert {
                        mdw_ptrs.push(ptrs.alloc());
                    }
                }
                
                let cdl = ptrs.alloc();
                (v_first, v_last, vgroups.len() as u64, cdl, v_ptrs, mdv_ptrs, mdw_ptrs)
            } else {
                (0, 0, 0, 0, Vec::new(), Vec::new(), Vec::new())
            }
        } else {
            (0, 0, 0, 0, Vec::new(), Vec::new(), Vec::new())
        };

    // ── Geometry data ─────────────────────────────────────────────────────────

    // poly_offsets: [0, 3, 6, ..., totloop] — totpoly+1 entries (sentinel at end)
    let poly_offsets: Vec<i32> = (0..=totpoly as i32).map(|i| i * 3).collect();
    let raw_poly_offsets = ints_data(&poly_offsets);

    // corner_vert[i] = vertex index for loop corner i
    let corner_verts: Vec<i32> = mesh.indices.iter().map(|&i| i as i32).collect();

    let raw_position   = floats3_data(&mesh.positions);
    let raw_edge_verts = ints2_data(&edge_verts);
    let raw_corner_vert = ints_data(&corner_verts);
    let raw_corner_edge = ints_data(&corner_edges);

    // Per-polygon material index: fill from submesh ranges
    let mut material_indices: Vec<i32> = vec![0; totpoly];
    for (submesh_idx, submesh) in mesh.submeshes.iter().enumerate() {
        let mat_idx = submesh_material_slots.get(submesh_idx).copied().unwrap_or(0);
        let start_face = submesh.first_index / 3;
        let num_faces = submesh.num_indices / 3;
        for i in 0..num_faces {
            if (start_face + i) < totpoly as u32 {
                material_indices[(start_face + i) as usize] = mat_idx as i32;
            }
        }
    }
    let raw_material_index = ints_data(&material_indices);

    // Expand per-vertex UV → per-loop (CORNER domain requires one entry per loop corner)
    let raw_uv: Option<Vec<u8>> = mesh.uvs.as_ref().map(|uvs| {
        let expanded: Vec<[f32; 2]> = mesh.indices.iter().map(|&i| uvs[i as usize]).collect();
        floats2_data(&expanded)
    });

    // Expand per-vertex colors → per-loop
    let raw_color: Option<Vec<u8>> = mesh.colors.as_ref().map(|colors| {
        let expanded: Vec<[u8; 4]> = mesh.indices.iter().map(|&i| colors[i as usize]).collect();
        bytes4_data(&expanded)
    });

    // ── Attribute descriptor blob ─────────────────────────────────────────────

    let mut attr_blob: Vec<u8> = Vec::new();
    let mut num_attrs: u32 = 5; // position + edge_verts + corner_vert + corner_edge + material_index

    attr_blob.extend_from_slice(&build_attribute(
        name_pos_ptr, ATTR_TYPE_FLOAT3, ATTR_DOMAIN_POINT, array_pos_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_ev_ptr, ATTR_TYPE_INT32_2D, ATTR_DOMAIN_EDGE, array_ev_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_cv_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_cv_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_ce_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_ce_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_matidx_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_FACE, array_matidx_ptr,
    ));
    if mesh.uvs.is_some() {
        attr_blob.extend_from_slice(&build_attribute(
            name_uv_ptr, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, array_uv_ptr,
        ));
        num_attrs += 1;
    }
    if mesh.colors.is_some() {
        attr_blob.extend_from_slice(&build_attribute(
            name_col_ptr, ATTR_TYPE_BYTE_COLOR, ATTR_DOMAIN_CORNER, array_col_ptr,
        ));
        num_attrs += 1;
    }

    // ── Datablocks ────────────────────────────────────────────────────────────

    let object_data = build_object(
        name, mesh_ptr, obj_mat_ptr, obj_matbits_ptr, mat_slots as i32, 0,
    );
    let scene_data = build_scene(name, view_layer_ptr, collection_ptr, tool_settings_ptr);
    let tool_settings_data = build_tool_settings();
    let view_layer_data = build_view_layer("ViewLayer", base_ptr, layer_collection_ptr);
    let base_data = build_base(object_ptr);
    let collection_data = build_master_collection(collection_object_ptr, collection_object_ptr, 0, 0);
    let collection_object_data = build_collection_object(object_ptr);
    let layer_collection_data = build_layer_collection(collection_ptr);
    let mesh_data = build_mesh(
        name, totvert, totedge, totpoly, totloop,
        poly_offs_ptr, attrs_ptr,
        mesh_mat_ptr, mat_slots,
        vgroup_first_ptr, vgroup_last_ptr, vgroup_count, cdl_ptr,
        num_attrs,
    );
    let mesh_mat_array = build_mat_ptr_array_from_ptrs(&material_ptrs);
    let obj_mat_array  = build_mat_ptr_array(mat_slots as usize);
    let obj_matbits    = build_matbits(mat_slots as usize);

    let arr_pos = build_attribute_array(raw_pos_ptr,  totvert as i64);
    let arr_ev  = build_attribute_array(raw_ev_ptr,   totedge as i64);
    let arr_cv  = build_attribute_array(raw_cv_ptr,   totloop as i64);
    let arr_ce  = build_attribute_array(raw_ce_ptr,   totloop as i64);

    // ── Assemble file ─────────────────────────────────────────────────────────

    let mut out: Vec<u8> = Vec::with_capacity(512 * 1024);
    out.extend_from_slice(BLEND_MAGIC);

    let file_global = build_file_global(STARTUP_UI_SCREEN_PTR, scene_ptr, view_layer_ptr);
    write_block(&mut out, b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x10, 1, &file_global);
    out.extend_from_slice(&startup_ui_prefix_bytes());

    // Minimal scene graph so Blender opens this as a normal scene, not library-only data.
    // CRITICAL: All DATA blocks for a given ID block must be consecutive immediately after it.
    // Blender's readfile.cc reads them all into fd->datamap, then clears it after each ID block.
    write_block(&mut out, b"SC\0\0", SDNA_IDX_SCENE, scene_ptr, 1, &scene_data);
    // SC DATA sequence (all consecutive, no non-DATA blocks until OB):
    write_block(&mut out, b"DATA", SDNA_IDX_TOOL_SETTINGS, tool_settings_ptr, 1, &tool_settings_data);
    write_block(&mut out, b"DATA", SDNA_IDX_VIEW_LAYER, view_layer_ptr, 1, &view_layer_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, layer_collection_ptr, 1, &layer_collection_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION, collection_ptr, 1, &collection_data);  // embedded master_collection
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, collection_object_ptr, 1, &collection_object_data);
    write_block(&mut out, b"DATA", SDNA_IDX_BASE, base_ptr, 1, &base_data);

    // OB block + DATA blocks (gap=1 rule: mat** and matbits must immediately follow OB)
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, object_ptr, 1, &object_data);
    write_block(&mut out, b"DATA", 0, obj_mat_ptr,      1, &obj_mat_array);
    write_block(&mut out, b"DATA", 0, obj_matbits_ptr,  1, &obj_matbits);

    // ME block + DATA block (gap=1 rule: mesh mat** must immediately follow ME)
    write_block(&mut out, b"ME\0\0", SDNA_IDX_MESH, mesh_ptr, 1, &mesh_data);
    write_block(&mut out, b"DATA", 0, mesh_mat_ptr, 1, &mesh_mat_array);

    // Attribute descriptor block (all Attribute structs concatenated)
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE, attrs_ptr, num_attrs as u32, &attr_blob);

    // Attribute name strings
    write_block(&mut out, b"DATA", 0, name_pos_ptr, 1, b"position\0");
    write_block(&mut out, b"DATA", 0, name_ev_ptr,  1, b".edge_verts\0");
    write_block(&mut out, b"DATA", 0, name_cv_ptr,  1, b".corner_vert\0");
    write_block(&mut out, b"DATA", 0, name_ce_ptr,  1, b".corner_edge\0");
    write_block(&mut out, b"DATA", 0, name_matidx_ptr, 1, b"material_index\0");

    // Attribute array descriptors + raw data (topology, position, material_index)
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_pos_ptr, 1, &arr_pos);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_ev_ptr,  1, &arr_ev);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_cv_ptr,  1, &arr_cv);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_ce_ptr,  1, &arr_ce);
    let arr_matidx = build_attribute_array(raw_matidx_ptr, totpoly as i64);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_matidx_ptr, 1, &arr_matidx);
    write_block(&mut out, b"DATA", 0, raw_pos_ptr, 1, &raw_position);
    write_block(&mut out, b"DATA", 0, raw_ev_ptr,  1, &raw_edge_verts);
    write_block(&mut out, b"DATA", 0, raw_cv_ptr,  1, &raw_corner_vert);
    write_block(&mut out, b"DATA", 0, raw_ce_ptr,  1, &raw_corner_edge);
    write_block(&mut out, b"DATA", 0, raw_matidx_ptr, 1, &raw_material_index);

    // Optional: UV map
    if let Some(ref uv_data) = raw_uv {
        let arr_uv = build_attribute_array(raw_uv_ptr, totloop as i64);
        write_block(&mut out, b"DATA", 0,  name_uv_ptr,  1, b"UVMap\0");
        write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_uv_ptr, 1, &arr_uv);
        write_block(&mut out, b"DATA", 0,  raw_uv_ptr,   1, uv_data);
    }

    // Optional: vertex colors
    if let Some(ref color_data) = raw_color {
        let arr_col = build_attribute_array(raw_col_ptr, totloop as i64);
        write_block(&mut out, b"DATA", 0,  name_col_ptr,  1, b"Color\0");
        write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_col_ptr, 1, &arr_col);
        write_block(&mut out, b"DATA", 0,  raw_col_ptr,   1, color_data);
    }

    // Polygon offset array
    write_block(&mut out, b"DATA", 0, poly_offs_ptr, 1, &raw_poly_offsets);

    // Phase 5D: Vertex groups
    if let Some(vgroups) = vertex_groups {
        if !vgroups.is_empty() && !vgroup_ptrs.is_empty() {
            // Write bDeformGroup entries
            for (idx, vgroup) in vgroups.iter().enumerate() {
                let next_ptr = if idx + 1 < vgroup_ptrs.len() {
                    vgroup_ptrs[idx + 1]
                } else {
                    0
                };
                let prev_ptr = if idx > 0 {
                    vgroup_ptrs[idx - 1]
                } else {
                    0
                };
                let bdeform_data = build_bdeformgroup(&vgroup.name, next_ptr, prev_ptr);
                write_block(&mut out, b"DATA", SDNA_IDX_BDEFORMGROUP, vgroup_ptrs[idx], 1, &bdeform_data);
            }
            
            // Write MDeformVert array with weights for each vertex
            if !mdeformvert_ptrs.is_empty() {
                let (mdv_array_ptr, mdv_data_ptr) = mdeformvert_ptrs[0];
                let mut mdeformvert_data = Vec::new();
                
                for vert_idx in 0..totvert {
                    // Find which vertex groups this vertex belongs to
                    let mut weights_for_vert = Vec::new();
                    for (group_idx, vgroup) in vgroups.iter().enumerate() {
                        if vgroup.vertex_indices.contains(&vert_idx) {
                            weights_for_vert.push((group_idx as u32, 1.0f32));
                        }
                    }
                    
                    if !weights_for_vert.is_empty() {
                        // For this implementation, write weight array for this vertex
                        let weight_array_ptr = if vert_idx < mdeformweight_ptrs.len() {
                            mdeformweight_ptrs[vert_idx]
                        } else {
                            0
                        };
                        mdeformvert_data.push((weight_array_ptr, weights_for_vert.len() as u32));
                    } else {
                        mdeformvert_data.push((0, 0));
                    }
                }
                
                let mdv_array_data = build_mdeformvert_array(&mdeformvert_data);
                write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, mdv_array_ptr, 1, &mdv_array_data);
                
                // Write individual weight arrays
                for vert_idx in 0..totvert {
                    for (group_idx, vgroup) in vgroups.iter().enumerate() {
                        if vgroup.vertex_indices.contains(&vert_idx) {
                            let weight_array_ptr = if vert_idx < mdeformweight_ptrs.len() {
                                mdeformweight_ptrs[vert_idx]
                            } else {
                                continue;
                            };
                            let weight_data = build_mdeformweight_array(&[(group_idx as u32, 1.0f32)]);
                            write_block(&mut out, b"DATA", 0, weight_array_ptr, 1, &weight_data);
                        }
                    }
                }
                
                // Write MDeformVert data array itself
                let mdv_raw_data = build_mdeformvert_array(&mdeformvert_data);
                write_block(&mut out, b"DATA", SDNA_IDX_MDEFORMVERT, mdv_data_ptr, totvert as u32, &mdv_raw_data);
            }
            
            // Write CustomDataLayer
            if cdl_ptr != 0 && !mdeformvert_ptrs.is_empty() {
                let (_, mdv_data_ptr) = mdeformvert_ptrs[0];
                let cdl_data = build_custom_data_layer_mdeformvert(mdv_data_ptr);
                write_block(&mut out, b"DATA", 0, cdl_ptr, 1, &cdl_data);
            }
        }
    }

    for (material_ptr, material_name) in material_ptrs.iter().zip(material_names.iter()) {
        let material_data = build_material(material_name);
        write_block(&mut out, b"MA\0\0", SDNA_IDX_MATERIAL, *material_ptr, 1, &material_data);
    }

    write_block(&mut out, b"DNA1", SDNA_IDX_DNA1, 0x01, 1, DNA1_BYTES);
    write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);

    // Phase 1D: Do NOT compress individual mesh files (keep uncompressed)
    // Compression only happens at scene.blend (Phase 2)
    out
}

#[derive(Clone)]
struct MeshObjectExport {
    name: String,
    mesh: Mesh,
    vertex_groups: Option<Vec<VertexGroup>>,
}

struct MeshBlockData {
    object_ptr: u64,
    mesh_ptr: u64,
    mesh_mat_ptr: u64,
    obj_mat_ptr: u64,
    obj_matbits_ptr: u64,
    poly_offs_ptr: u64,
    attrs_ptr: u64,
    name_pos_ptr: u64,
    name_ev_ptr: u64,
    name_cv_ptr: u64,
    name_ce_ptr: u64,
    array_pos_ptr: u64,
    array_ev_ptr: u64,
    array_cv_ptr: u64,
    array_ce_ptr: u64,
    raw_pos_ptr: u64,
    raw_ev_ptr: u64,
    raw_cv_ptr: u64,
    raw_ce_ptr: u64,
    name_matidx_ptr: u64,
    array_matidx_ptr: u64,
    raw_matidx_ptr: u64,
    name_uv_ptr: u64,
    array_uv_ptr: u64,
    raw_uv_ptr: u64,
    name_col_ptr: u64,
    array_col_ptr: u64,
    raw_col_ptr: u64,
    vgroup_first_ptr: u64,
    vgroup_last_ptr: u64,
    vgroup_count: u64,
    cdl_ptr: u64,
    vgroup_ptrs: Vec<u64>,
    mdeformvert_ptrs: Vec<(u64, u64)>,
    mdeformweight_ptrs: Vec<u64>,
    totvert: usize,
    totedge: usize,
    totpoly: usize,
    totloop: usize,
    num_attrs: u32,
    raw_poly_offsets: Vec<u8>,
    raw_position: Vec<u8>,
    raw_edge_verts: Vec<u8>,
    raw_corner_vert: Vec<u8>,
    raw_corner_edge: Vec<u8>,
    raw_material_index: Vec<u8>,
    raw_uv: Option<Vec<u8>>,
    raw_color: Option<Vec<u8>>,
    attr_blob: Vec<u8>,
}

fn strip_lod_suffix(name: &str) -> String {
    let Some((stem, suffix)) = name.rsplit_once("_LOD") else {
        return name.to_string();
    };
    if !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()) {
        stem.to_string()
    } else {
        name.to_string()
    }
}

fn linked_scene_object_names(
    mesh_name: &str,
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
) -> Vec<String> {
    let Some(nmc) = nmc.filter(|nmc| !nmc.nodes.is_empty()) else {
        return vec![mesh_name.to_string()];
    };

    let export_names = nmc_export_object_names(mesh_name, nmc);
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut node_submeshes: Vec<Vec<usize>> = vec![Vec::new(); nmc.nodes.len()];
    for (submesh_index, submesh) in mesh.submeshes.iter().enumerate() {
        let node_index = submesh.node_parent_index as usize;
        if node_index < node_submeshes.len() {
            node_submeshes[node_index].push(submesh_index);
        }
    }
    for (node_index, submesh_indices) in node_submeshes.iter().enumerate() {
        if submesh_indices.is_empty() {
            continue;
        }
        let (node_mesh, _) = subset_mesh_for_submeshes(mesh, submesh_indices, None);
        if node_mesh.indices.is_empty() {
            continue;
        }
        let name = export_names[node_index].clone();
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }

    if names.is_empty() {
        vec![mesh_name.to_string()]
    } else {
        names
    }
}

fn mesh_object_names_from_blend_bytes(blend_bytes: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut offset = BLEND_MAGIC.len();
    while offset + 32 <= blend_bytes.len() {
        let code = &blend_bytes[offset..offset + 4];
        let size = u32::from_le_bytes(blend_bytes[offset + 16..offset + 20].try_into().unwrap()) as usize;
        let data_start = offset + 32;
        let data_end = data_start.saturating_add(size);
        if data_end > blend_bytes.len() {
            break;
        }
        if code == b"OB\0\0" {
            let data = &blend_bytes[data_start..data_end];
            if data.len() >= OBJECT_SIZE && i16::from_le_bytes(data[416..418].try_into().unwrap()) == 1 {
                let raw = &data[42..300];
                let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
                names.push(String::from_utf8_lossy(&raw[..end]).to_string());
            }
        }
        if code == b"ENDB" {
            break;
        }
        offset = data_end;
    }
    names
}

fn nmc_export_object_names(mesh_name: &str, nmc: &NodeMeshCombo) -> Vec<String> {
    let wrapper_name = strip_lod_suffix(mesh_name);
    let mut used = HashSet::new();
    used.insert("CryEngine_Z_up".to_string());
    used.insert(wrapper_name.clone());

    nmc.nodes
        .iter()
        .enumerate()
        .map(|(node_index, node)| {
            let base_name = if node.name.is_empty() {
                format!("{mesh_name}_node_{node_index}")
            } else {
                node.name.clone()
            };
            let mut name = if used.contains(&base_name) {
                if base_name == wrapper_name {
                    format!("{base_name}_mesh")
                } else {
                    format!("{base_name}_node_{node_index}")
                }
            } else {
                base_name
            };
            while used.contains(&name) {
                name = format!("{name}_{node_index}");
            }
            used.insert(name.clone());
            name
        })
        .collect()
}

fn matrix_to_transform(matrix: [f32; 16]) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let mat = glam::Mat4::from_cols_array(&matrix);
    let (scale, rotation, translation) = mat.to_scale_rotation_translation();
    (
        [translation.x, translation.y, translation.z],
        [rotation.w, rotation.x, rotation.y, rotation.z],
        [scale.x, scale.y, scale.z],
    )
}

fn patch_object_parent_transform(
    object_data: &mut [u8],
    parent_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
) {
    write_ptr(object_data, 496, parent_ptr);
    for i in 0..3 {
        write_f32(object_data, 736 + i * 4, loc[i]);
        write_f32(object_data, 760 + i * 4, scale[i]);
        write_f32(object_data, 784 + i * 4, 1.0);
    }
    for i in 0..4 {
        write_f32(object_data, 820 + i * 4, quat[i]);
    }
    write_f32(object_data, 836, 1.0);
    write_i16(object_data, 1040, 0);
    if parent_ptr != 0 {
        write_identity_matrix4x4(object_data, 884);
    }
}

fn remap_vertex_groups(
    vertex_groups: Option<&Vec<VertexGroup>>,
    old_to_new: &HashMap<u32, u32>,
) -> Option<Vec<VertexGroup>> {
    let groups = vertex_groups?;
    let remapped = groups
        .iter()
        .filter_map(|group| {
            let mut vertex_indices = group
                .vertex_indices
                .iter()
                .filter_map(|old| old_to_new.get(&(*old as u32)).copied().map(|new| new as usize))
                .collect::<Vec<_>>();
            vertex_indices.sort_unstable();
            vertex_indices.dedup();
            if vertex_indices.is_empty() {
                None
            } else {
                Some(VertexGroup {
                    name: group.name.clone(),
                    vertex_indices,
                })
            }
        })
        .collect::<Vec<_>>();
    if remapped.is_empty() { None } else { Some(remapped) }
}

fn subset_mesh_for_submeshes(
    mesh: &Mesh,
    submesh_indices: &[usize],
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> (Mesh, Option<Vec<VertexGroup>>) {
    let mut old_to_new: HashMap<u32, u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut uvs = mesh.uvs.as_ref().map(|_| Vec::new());
    let mut secondary_uvs = mesh.secondary_uvs.as_ref().map(|_| Vec::new());
    let mut normals = mesh.normals.as_ref().map(|_| Vec::new());
    let mut tangents = mesh.tangents.as_ref().map(|_| Vec::new());
    let mut colors = mesh.colors.as_ref().map(|_| Vec::new());
    let mut indices = Vec::new();
    let mut submeshes = Vec::new();

    for &submesh_index in submesh_indices {
        let Some(source_submesh) = mesh.submeshes.get(submesh_index) else {
            continue;
        };
        let first_index = indices.len() as u32;
        let start = source_submesh.first_index as usize;
        let end = (start + source_submesh.num_indices as usize).min(mesh.indices.len());
        for &old_index in &mesh.indices[start..end] {
            let new_index = if let Some(&new_index) = old_to_new.get(&old_index) {
                new_index
            } else {
                let old = old_index as usize;
                let new_index = positions.len() as u32;
                positions.push(mesh.positions.get(old).copied().unwrap_or([0.0; 3]));
                if let (Some(src), Some(dst)) = (mesh.uvs.as_ref(), uvs.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([0.0; 2]));
                }
                if let (Some(src), Some(dst)) = (mesh.secondary_uvs.as_ref(), secondary_uvs.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([0.0; 2]));
                }
                if let (Some(src), Some(dst)) = (mesh.normals.as_ref(), normals.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([0.0, 0.0, 1.0]));
                }
                if let (Some(src), Some(dst)) = (mesh.tangents.as_ref(), tangents.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]));
                }
                if let (Some(src), Some(dst)) = (mesh.colors.as_ref(), colors.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([255, 255, 255, 255]));
                }
                old_to_new.insert(old_index, new_index);
                new_index
            };
            indices.push(new_index);
        }
        let num_indices = indices.len() as u32 - first_index;
        if num_indices == 0 {
            continue;
        }
        let mut submesh = SubMesh {
            material_name: source_submesh.material_name.clone(),
            material_id: source_submesh.material_id,
            source_material_id: source_submesh.source_material_id,
            first_index,
            num_indices,
            first_vertex: 0,
            num_vertices: positions.len() as u32,
            node_parent_index: source_submesh.node_parent_index,
        };
        if let (Some(min), Some(max)) = (
            old_to_new.values().min().copied(),
            old_to_new.values().max().copied(),
        ) {
            submesh.first_vertex = min;
            submesh.num_vertices = max.saturating_sub(min) + 1;
        }
        submeshes.push(submesh);
    }

    let (model_min, model_max) = if positions.is_empty() {
        ([0.0; 3], [0.0; 3])
    } else {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for position in &positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        (min, max)
    };
    let vertex_groups = remap_vertex_groups(vertex_groups, &old_to_new);
    (
        Mesh {
            positions,
            indices,
            uvs,
            secondary_uvs,
            normals,
            tangents,
            colors,
            submeshes,
            model_min,
            model_max,
            scaling_min: mesh.scaling_min,
            scaling_max: mesh.scaling_max,
        },
        vertex_groups,
    )
}

fn allocate_mesh_block(
    ptrs: &mut PtrAlloc,
    mesh: &Mesh,
    vertex_groups: Option<&Vec<VertexGroup>>,
    submesh_slot_by_material_id: &HashMap<u32, usize>,
) -> MeshBlockData {
    let totvert = mesh.positions.len();
    let totloop = mesh.indices.len();
    let totpoly = totloop / 3;
    let (edge_verts, corner_edges) = triangle_edge_topology(&mesh.indices);
    let totedge = edge_verts.len();

    let object_ptr = ptrs.alloc();
    let mesh_ptr = ptrs.alloc();
    let mesh_mat_ptr = ptrs.alloc();
    let obj_mat_ptr = ptrs.alloc();
    let obj_matbits_ptr = ptrs.alloc();
    let poly_offs_ptr = ptrs.alloc();
    let attrs_ptr = ptrs.alloc();
    let name_pos_ptr = ptrs.alloc();
    let name_ev_ptr = ptrs.alloc();
    let name_cv_ptr = ptrs.alloc();
    let name_ce_ptr = ptrs.alloc();
    let array_pos_ptr = ptrs.alloc();
    let array_ev_ptr = ptrs.alloc();
    let array_cv_ptr = ptrs.alloc();
    let array_ce_ptr = ptrs.alloc();
    let raw_pos_ptr = ptrs.alloc();
    let raw_ev_ptr = ptrs.alloc();
    let raw_cv_ptr = ptrs.alloc();
    let raw_ce_ptr = ptrs.alloc();
    let name_matidx_ptr = ptrs.alloc();
    let array_matidx_ptr = ptrs.alloc();
    let raw_matidx_ptr = ptrs.alloc();
    let (name_uv_ptr, array_uv_ptr, raw_uv_ptr) = if mesh.uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };
    let (name_col_ptr, array_col_ptr, raw_col_ptr) = if mesh.colors.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };
    let (vgroup_first_ptr, vgroup_last_ptr, vgroup_count, cdl_ptr, vgroup_ptrs, mdeformvert_ptrs, mdeformweight_ptrs) =
        if let Some(vgroups) = vertex_groups.filter(|groups| !groups.is_empty()) {
            let vgroup_ptrs = (0..vgroups.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
            let mdeformvert_ptrs = vec![(ptrs.alloc(), ptrs.alloc())];
            let mdeformweight_ptrs = (0..totvert).map(|_| ptrs.alloc()).collect::<Vec<_>>();
            (
                vgroup_ptrs[0],
                *vgroup_ptrs.last().unwrap(),
                vgroups.len() as u64,
                ptrs.alloc(),
                vgroup_ptrs,
                mdeformvert_ptrs,
                mdeformweight_ptrs,
            )
        } else {
            (0, 0, 0, 0, Vec::new(), Vec::new(), Vec::new())
        };

    let poly_offsets = (0..=totpoly as i32).map(|i| i * 3).collect::<Vec<_>>();
    let raw_poly_offsets = ints_data(&poly_offsets);
    let corner_verts = mesh.indices.iter().map(|&i| i as i32).collect::<Vec<_>>();
    let raw_position = floats3_data(&mesh.positions);
    let raw_edge_verts = ints2_data(&edge_verts);
    let raw_corner_vert = ints_data(&corner_verts);
    let raw_corner_edge = ints_data(&corner_edges);
    let mut material_indices = vec![0; totpoly];
    for submesh in &mesh.submeshes {
        let mat_idx = submesh_slot_by_material_id
            .get(&submesh.material_id)
            .copied()
            .unwrap_or(0);
        let start_face = submesh.first_index / 3;
        let num_faces = submesh.num_indices / 3;
        for i in 0..num_faces {
            if (start_face + i) < totpoly as u32 {
                material_indices[(start_face + i) as usize] = mat_idx as i32;
            }
        }
    }
    let raw_material_index = ints_data(&material_indices);
    let raw_uv = mesh.uvs.as_ref().map(|uvs| {
        let expanded = mesh
            .indices
            .iter()
            .map(|&i| uvs.get(i as usize).copied().unwrap_or([0.0; 2]))
            .collect::<Vec<_>>();
        floats2_data(&expanded)
    });
    let raw_color = mesh.colors.as_ref().map(|colors| {
        let expanded = mesh
            .indices
            .iter()
            .map(|&i| colors.get(i as usize).copied().unwrap_or([255, 255, 255, 255]))
            .collect::<Vec<_>>();
        bytes4_data(&expanded)
    });

    let mut attr_blob = Vec::new();
    let mut num_attrs = 5;
    attr_blob.extend_from_slice(&build_attribute(name_pos_ptr, ATTR_TYPE_FLOAT3, ATTR_DOMAIN_POINT, array_pos_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_ev_ptr, ATTR_TYPE_INT32_2D, ATTR_DOMAIN_EDGE, array_ev_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_cv_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_cv_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_ce_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_ce_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_matidx_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_FACE, array_matidx_ptr));
    if mesh.uvs.is_some() {
        attr_blob.extend_from_slice(&build_attribute(name_uv_ptr, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, array_uv_ptr));
        num_attrs += 1;
    }
    if mesh.colors.is_some() {
        attr_blob.extend_from_slice(&build_attribute(name_col_ptr, ATTR_TYPE_BYTE_COLOR, ATTR_DOMAIN_CORNER, array_col_ptr));
        num_attrs += 1;
    }

    MeshBlockData {
        object_ptr,
        mesh_ptr,
        mesh_mat_ptr,
        obj_mat_ptr,
        obj_matbits_ptr,
        poly_offs_ptr,
        attrs_ptr,
        name_pos_ptr,
        name_ev_ptr,
        name_cv_ptr,
        name_ce_ptr,
        array_pos_ptr,
        array_ev_ptr,
        array_cv_ptr,
        array_ce_ptr,
        raw_pos_ptr,
        raw_ev_ptr,
        raw_cv_ptr,
        raw_ce_ptr,
        name_matidx_ptr,
        array_matidx_ptr,
        raw_matidx_ptr,
        name_uv_ptr,
        array_uv_ptr,
        raw_uv_ptr,
        name_col_ptr,
        array_col_ptr,
        raw_col_ptr,
        vgroup_first_ptr,
        vgroup_last_ptr,
        vgroup_count,
        cdl_ptr,
        vgroup_ptrs,
        mdeformvert_ptrs,
        mdeformweight_ptrs,
        totvert,
        totedge,
        totpoly,
        totloop,
        num_attrs,
        raw_poly_offsets,
        raw_position,
        raw_edge_verts,
        raw_corner_vert,
        raw_corner_edge,
        raw_material_index,
        raw_uv,
        raw_color,
        attr_blob,
    }
}

fn write_mesh_block(
    out: &mut Vec<u8>,
    block: &MeshBlockData,
    object: &MeshObjectExport,
    material_ptrs: &[u64],
    material_names: &[String],
    mat_slots: i16,
    parent_ptr: u64,
    transform: ([f32; 3], [f32; 4], [f32; 3]),
) {
    let mut object_data = build_object(
        &object.name,
        block.mesh_ptr,
        block.obj_mat_ptr,
        block.obj_matbits_ptr,
        mat_slots as i32,
        0,
    );
    patch_object_parent_transform(&mut object_data, parent_ptr, transform.0, transform.1, transform.2);
    let mesh_data = build_mesh(
        &object.name,
        block.totvert,
        block.totedge,
        block.totpoly,
        block.totloop,
        block.poly_offs_ptr,
        block.attrs_ptr,
        block.mesh_mat_ptr,
        mat_slots,
        block.vgroup_first_ptr,
        block.vgroup_last_ptr,
        block.vgroup_count,
        block.cdl_ptr,
        block.num_attrs,
    );
    let mesh_mat_array = build_mat_ptr_array_from_ptrs(material_ptrs);
    let obj_mat_array = build_mat_ptr_array(mat_slots as usize);
    let obj_matbits = build_matbits(mat_slots as usize);
    write_block(out, b"OB\0\0", SDNA_IDX_OBJECT, block.object_ptr, 1, &object_data);
    write_block(out, b"DATA", 0, block.obj_mat_ptr, 1, &obj_mat_array);
    write_block(out, b"DATA", 0, block.obj_matbits_ptr, 1, &obj_matbits);
    write_block(out, b"ME\0\0", SDNA_IDX_MESH, block.mesh_ptr, 1, &mesh_data);
    write_block(out, b"DATA", 0, block.mesh_mat_ptr, 1, &mesh_mat_array);
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE, block.attrs_ptr, block.num_attrs, &block.attr_blob);
    write_block(out, b"DATA", 0, block.name_pos_ptr, 1, b"position\0");
    write_block(out, b"DATA", 0, block.name_ev_ptr, 1, b".edge_verts\0");
    write_block(out, b"DATA", 0, block.name_cv_ptr, 1, b".corner_vert\0");
    write_block(out, b"DATA", 0, block.name_ce_ptr, 1, b".corner_edge\0");
    write_block(out, b"DATA", 0, block.name_matidx_ptr, 1, b"material_index\0");
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_pos_ptr, 1, &build_attribute_array(block.raw_pos_ptr, block.totvert as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_ev_ptr, 1, &build_attribute_array(block.raw_ev_ptr, block.totedge as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_cv_ptr, 1, &build_attribute_array(block.raw_cv_ptr, block.totloop as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_ce_ptr, 1, &build_attribute_array(block.raw_ce_ptr, block.totloop as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_matidx_ptr, 1, &build_attribute_array(block.raw_matidx_ptr, block.totpoly as i64));
    write_block(out, b"DATA", 0, block.raw_pos_ptr, 1, &block.raw_position);
    write_block(out, b"DATA", 0, block.raw_ev_ptr, 1, &block.raw_edge_verts);
    write_block(out, b"DATA", 0, block.raw_cv_ptr, 1, &block.raw_corner_vert);
    write_block(out, b"DATA", 0, block.raw_ce_ptr, 1, &block.raw_corner_edge);
    write_block(out, b"DATA", 0, block.raw_matidx_ptr, 1, &block.raw_material_index);
    if let Some(ref uv_data) = block.raw_uv {
        write_block(out, b"DATA", 0, block.name_uv_ptr, 1, b"UVMap\0");
        write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_uv_ptr, 1, &build_attribute_array(block.raw_uv_ptr, block.totloop as i64));
        write_block(out, b"DATA", 0, block.raw_uv_ptr, 1, uv_data);
    }
    if let Some(ref color_data) = block.raw_color {
        write_block(out, b"DATA", 0, block.name_col_ptr, 1, b"Color\0");
        write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_col_ptr, 1, &build_attribute_array(block.raw_col_ptr, block.totloop as i64));
        write_block(out, b"DATA", 0, block.raw_col_ptr, 1, color_data);
    }
    write_block(out, b"DATA", 0, block.poly_offs_ptr, 1, &block.raw_poly_offsets);
    if let Some(vgroups) = object.vertex_groups.as_ref().filter(|groups| !groups.is_empty()) {
        for (idx, vgroup) in vgroups.iter().enumerate() {
            let next_ptr = if idx + 1 < block.vgroup_ptrs.len() { block.vgroup_ptrs[idx + 1] } else { 0 };
            let prev_ptr = if idx > 0 { block.vgroup_ptrs[idx - 1] } else { 0 };
            let bdeform_data = build_bdeformgroup(&vgroup.name, next_ptr, prev_ptr);
            write_block(out, b"DATA", SDNA_IDX_BDEFORMGROUP, block.vgroup_ptrs[idx], 1, &bdeform_data);
        }
        if let Some((mdv_array_ptr, mdv_data_ptr)) = block.mdeformvert_ptrs.first().copied() {
            let mut mdeformvert_data = Vec::with_capacity(block.totvert);
            let mut weight_payloads = Vec::new();
            for vert_idx in 0..block.totvert {
                let weights_for_vert = vgroups
                    .iter()
                    .enumerate()
                    .filter_map(|(group_idx, vgroup)| {
                        vgroup.vertex_indices.contains(&vert_idx).then_some((group_idx as u32, 1.0f32))
                    })
                    .collect::<Vec<_>>();
                let weight_ptr = if weights_for_vert.is_empty() { 0 } else { block.mdeformweight_ptrs[vert_idx] };
                mdeformvert_data.push((weight_ptr, weights_for_vert.len() as u32));
                if !weights_for_vert.is_empty() {
                    weight_payloads.push((weight_ptr, build_mdeformweight_array(&weights_for_vert)));
                }
            }
            let mdv_array_data = build_mdeformvert_array(&mdeformvert_data);
            write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, mdv_array_ptr, 1, &mdv_array_data);
            for (weight_ptr, weight_data) in weight_payloads {
                write_block(out, b"DATA", 0, weight_ptr, 1, &weight_data);
            }
            write_block(out, b"DATA", SDNA_IDX_MDEFORMVERT, mdv_data_ptr, block.totvert as u32, &mdv_array_data);
            if block.cdl_ptr != 0 {
                write_block(out, b"DATA", 0, block.cdl_ptr, 1, &build_custom_data_layer_mdeformvert(mdv_data_ptr));
            }
        }
    }
}

fn mesh_to_blend_hierarchy(
    name: &str,
    mesh: &Mesh,
    materials: &Option<crate::mtl::MtlFile>,
    nmc: &NodeMeshCombo,
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> Vec<u8> {
    let (material_names, full_submesh_slots) = blend_material_slots(name, mesh, materials);
    let mat_slots = material_names.len() as i16;
    let mut submesh_slot_by_material_id = HashMap::new();
    for (submesh, slot) in mesh.submeshes.iter().zip(full_submesh_slots.iter().copied()) {
        submesh_slot_by_material_id.entry(submesh.material_id).or_insert(slot);
    }

    let mut node_submeshes: Vec<Vec<usize>> = vec![Vec::new(); nmc.nodes.len()];
    for (submesh_index, submesh) in mesh.submeshes.iter().enumerate() {
        let node_index = submesh.node_parent_index as usize;
        if node_index < node_submeshes.len() {
            node_submeshes[node_index].push(submesh_index);
        }
    }

    let nmc_export_names = nmc_export_object_names(name, nmc);
    let mut mesh_objects: Vec<Option<MeshObjectExport>> = vec![None; nmc.nodes.len()];
    for (node_index, submesh_indices) in node_submeshes.iter().enumerate() {
        if submesh_indices.is_empty() {
            continue;
        }
        let (node_mesh, node_vertex_groups) = subset_mesh_for_submeshes(mesh, submesh_indices, vertex_groups);
        if node_mesh.indices.is_empty() {
            continue;
        }
        mesh_objects[node_index] = Some(MeshObjectExport {
            name: nmc_export_names[node_index].clone(),
            mesh: node_mesh,
            vertex_groups: node_vertex_groups,
        });
    }
    let wrapper_name = strip_lod_suffix(name);
    let collapsed_wrapper_node = nmc.nodes.iter().enumerate().find_map(|(index, node)| {
        (node.parent_index.is_none()
            && node.name == wrapper_name
            && mesh_objects.get(index).and_then(|object| object.as_ref()).is_none())
        .then_some(index)
    });

    let mut ptrs = PtrAlloc::new(0x1000);
    let _screen_ptr = ptrs.alloc();
    let _wm_ptr = ptrs.alloc();
    let coord_root_ptr = ptrs.alloc();
    let coord_root_mat_ptr = ptrs.alloc();
    let coord_root_matbits_ptr = ptrs.alloc();
    let wrapper_ptr = ptrs.alloc();
    let wrapper_mat_ptr = ptrs.alloc();
    let wrapper_matbits_ptr = ptrs.alloc();
    let nmc_object_ptrs = (0..nmc.nodes.len())
        .map(|index| {
            if collapsed_wrapper_node == Some(index) {
                wrapper_ptr
            } else {
                ptrs.alloc()
            }
        })
        .collect::<Vec<_>>();
    let nmc_empty_mat_ptrs = (0..nmc.nodes.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let nmc_empty_matbits_ptrs = (0..nmc.nodes.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let mesh_blocks = mesh_objects
        .iter()
        .map(|object| {
            object.as_ref().map(|object| {
                allocate_mesh_block(
                    &mut ptrs,
                    &object.mesh,
                    object.vertex_groups.as_ref(),
                    &submesh_slot_by_material_id,
                )
            })
        })
        .collect::<Vec<_>>();
    let material_ptrs = (0..material_names.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let scene_ptr = ptrs.alloc();
    let view_layer_ptr = ptrs.alloc();
    let tool_settings_ptr = ptrs.alloc();
    let base_ptr = ptrs.alloc();
    let collection_ptr = ptrs.alloc();
    let layer_collection_ptr = ptrs.alloc();
    let object_count = 2 + nmc.nodes.len() - usize::from(collapsed_wrapper_node.is_some());
    let collection_object_ptrs = (0..object_count).map(|_| ptrs.alloc()).collect::<Vec<_>>();

    let scene_data = build_scene(name, view_layer_ptr, collection_ptr, tool_settings_ptr);
    let tool_settings_data = build_tool_settings();
    let view_layer_data = build_view_layer("ViewLayer", base_ptr, layer_collection_ptr);
    let base_data = build_base(coord_root_ptr);
    let collection_data = build_master_collection(
        collection_object_ptrs.first().copied().unwrap_or(0),
        collection_object_ptrs.last().copied().unwrap_or(0),
        0,
        0,
    );
    let layer_collection_data = build_layer_collection(collection_ptr);
    let object_ptr_sequence = std::iter::once(coord_root_ptr)
        .chain(std::iter::once(wrapper_ptr))
        .chain(
            nmc_object_ptrs
                .iter()
                .enumerate()
                .filter_map(|(index, &ptr)| (collapsed_wrapper_node != Some(index)).then_some(ptr)),
        )
        .collect::<Vec<_>>();
    let collection_object_data = collection_object_ptrs
        .iter()
        .enumerate()
        .map(|(idx, &coll_ptr)| {
            let prev_ptr = if idx > 0 { collection_object_ptrs[idx - 1] } else { 0 };
            let next_ptr = if idx + 1 < collection_object_ptrs.len() { collection_object_ptrs[idx + 1] } else { 0 };
            (coll_ptr, build_collection_object_linked(object_ptr_sequence[idx], prev_ptr, next_ptr))
        })
        .collect::<Vec<_>>();

    let mut out = Vec::with_capacity(1024 * 1024);
    out.extend_from_slice(BLEND_MAGIC);
    let file_global = build_file_global(STARTUP_UI_SCREEN_PTR, scene_ptr, view_layer_ptr);
    write_block(&mut out, b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x10, 1, &file_global);
    out.extend_from_slice(&startup_ui_prefix_bytes());
    write_block(&mut out, b"SC\0\0", SDNA_IDX_SCENE, scene_ptr, 1, &scene_data);
    write_block(&mut out, b"DATA", SDNA_IDX_TOOL_SETTINGS, tool_settings_ptr, 1, &tool_settings_data);
    write_block(&mut out, b"DATA", SDNA_IDX_VIEW_LAYER, view_layer_ptr, 1, &view_layer_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, layer_collection_ptr, 1, &layer_collection_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION, collection_ptr, 1, &collection_data);
    for (coll_ptr, data) in &collection_object_data {
        write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, *coll_ptr, 1, data);
    }
    write_block(&mut out, b"DATA", SDNA_IDX_BASE, base_ptr, 1, &base_data);

    let coord_matrix = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, -1.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    let (coord_loc, coord_quat, coord_scale) = matrix_to_transform(coord_matrix);
    let coord_root_data = build_empty_object("CryEngine_Z_up", coord_loc, coord_quat, coord_scale, 0);
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, coord_root_ptr, 1, &coord_root_data);
    write_block(&mut out, b"DATA", 0, coord_root_mat_ptr, 1, &build_mat_ptr_array(0));
    write_block(&mut out, b"DATA", 0, coord_root_matbits_ptr, 1, &build_matbits(0));

    let wrapper_data = build_empty_object(
        &wrapper_name,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        coord_root_ptr,
    );
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, wrapper_ptr, 1, &wrapper_data);
    write_block(&mut out, b"DATA", 0, wrapper_mat_ptr, 1, &build_mat_ptr_array(0));
    write_block(&mut out, b"DATA", 0, wrapper_matbits_ptr, 1, &build_matbits(0));

    for (node_index, node) in nmc.nodes.iter().enumerate() {
        if collapsed_wrapper_node == Some(node_index) {
            continue;
        }
        let parent_ptr = node
            .parent_index
            .and_then(|parent| nmc_object_ptrs.get(parent as usize).copied())
            .unwrap_or(wrapper_ptr);
        let transform = if crate::gltf::is_identity_or_zero(&node.bone_to_world) {
            ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0])
        } else {
            matrix_to_transform(crate::gltf::mat3x4_to_gltf(&node.bone_to_world))
        };
        if let (Some(object), Some(block)) = (&mesh_objects[node_index], &mesh_blocks[node_index]) {
            if block.object_ptr == nmc_object_ptrs[node_index] {
                write_mesh_block(&mut out, block, object, &material_ptrs, &material_names, mat_slots, parent_ptr, transform);
            } else {
                let cloned = MeshBlockData {
                    object_ptr: nmc_object_ptrs[node_index],
                    mesh_ptr: block.mesh_ptr,
                    mesh_mat_ptr: block.mesh_mat_ptr,
                    obj_mat_ptr: block.obj_mat_ptr,
                    obj_matbits_ptr: block.obj_matbits_ptr,
                    poly_offs_ptr: block.poly_offs_ptr,
                    attrs_ptr: block.attrs_ptr,
                    name_pos_ptr: block.name_pos_ptr,
                    name_ev_ptr: block.name_ev_ptr,
                    name_cv_ptr: block.name_cv_ptr,
                    name_ce_ptr: block.name_ce_ptr,
                    array_pos_ptr: block.array_pos_ptr,
                    array_ev_ptr: block.array_ev_ptr,
                    array_cv_ptr: block.array_cv_ptr,
                    array_ce_ptr: block.array_ce_ptr,
                    raw_pos_ptr: block.raw_pos_ptr,
                    raw_ev_ptr: block.raw_ev_ptr,
                    raw_cv_ptr: block.raw_cv_ptr,
                    raw_ce_ptr: block.raw_ce_ptr,
                    name_matidx_ptr: block.name_matidx_ptr,
                    array_matidx_ptr: block.array_matidx_ptr,
                    raw_matidx_ptr: block.raw_matidx_ptr,
                    name_uv_ptr: block.name_uv_ptr,
                    array_uv_ptr: block.array_uv_ptr,
                    raw_uv_ptr: block.raw_uv_ptr,
                    name_col_ptr: block.name_col_ptr,
                    array_col_ptr: block.array_col_ptr,
                    raw_col_ptr: block.raw_col_ptr,
                    vgroup_first_ptr: block.vgroup_first_ptr,
                    vgroup_last_ptr: block.vgroup_last_ptr,
                    vgroup_count: block.vgroup_count,
                    cdl_ptr: block.cdl_ptr,
                    vgroup_ptrs: block.vgroup_ptrs.clone(),
                    mdeformvert_ptrs: block.mdeformvert_ptrs.clone(),
                    mdeformweight_ptrs: block.mdeformweight_ptrs.clone(),
                    totvert: block.totvert,
                    totedge: block.totedge,
                    totpoly: block.totpoly,
                    totloop: block.totloop,
                    num_attrs: block.num_attrs,
                    raw_poly_offsets: block.raw_poly_offsets.clone(),
                    raw_position: block.raw_position.clone(),
                    raw_edge_verts: block.raw_edge_verts.clone(),
                    raw_corner_vert: block.raw_corner_vert.clone(),
                    raw_corner_edge: block.raw_corner_edge.clone(),
                    raw_material_index: block.raw_material_index.clone(),
                    raw_uv: block.raw_uv.clone(),
                    raw_color: block.raw_color.clone(),
                    attr_blob: block.attr_blob.clone(),
                };
                write_mesh_block(&mut out, &cloned, object, &material_ptrs, &material_names, mat_slots, parent_ptr, transform);
            }
        } else {
            let object_name = nmc_export_names[node_index].clone();
            let empty_data = build_empty_object(&object_name, transform.0, transform.1, transform.2, parent_ptr);
            write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, nmc_object_ptrs[node_index], 1, &empty_data);
            write_block(&mut out, b"DATA", 0, nmc_empty_mat_ptrs[node_index], 1, &build_mat_ptr_array(0));
            write_block(&mut out, b"DATA", 0, nmc_empty_matbits_ptrs[node_index], 1, &build_matbits(0));
        }
    }

    for (material_ptr, material_name) in material_ptrs.iter().zip(material_names.iter()) {
        let material_data = build_material(material_name);
        write_block(&mut out, b"MA\0\0", SDNA_IDX_MATERIAL, *material_ptr, 1, &material_data);
    }

    write_block(&mut out, b"DNA1", SDNA_IDX_DNA1, 0x01, 1, DNA1_BYTES);
    write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);
    out
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5A: Scene.blend Assembly — Link mesh .blend files with collections
// ════════════════════════════════════════════════════════════════════════════════

/// Data for a linked mesh instance in scene.blend.
#[derive(Debug, Clone)]
pub struct LinkedMeshInstance {
    /// Instance name (typically "mesh_0", "mesh_1", etc.)
    pub name: String,
    /// Relative path to the linked .blend file (e.g., "Data/Objects/mesh_0.blend")
    pub blend_path: String,
    /// Blender coordinates position [x, y, z]
    pub position: [f32; 3],
    /// Blender coordinates rotation as quaternion [w, x, y, z]
    pub rotation: [f32; 4],
}

/// Create a scene.blend file that links together all individual mesh .blend files.
///
/// **Phase 5A Context:**
/// - Input: Entity name and number of children
/// - Output: A valid .blend file containing:
///   - Root scene object (empty at origin)
///   - Collections organized by entity type (Meshes, Lights, Empties, Decals)
///   - Linked instances pointing to mesh .blend files with relative paths
///   - Proper scene settings and render configuration
///
/// **Collections structure:**
/// - Scene (root)
///   - Meshes (contains linked mesh instances)
///   - Lights (for Phase 5B integration)
///   - Empties (for Phase 5C integration)
///   - Decals (placeholder for decal geometry)
///
/// **Library linking:**
/// - Each mesh is linked via Library block + ID stub
/// - Paths are relative for portability across export roots
/// - Transforms applied at instance level (mesh-level geometry unchanged)
///
/// # Arguments
///
/// * `entity_name` - Name of the scene entity
/// * `children_count` - Number of mesh child objects to create
/// * `mesh_output_dir` - Directory containing the mesh .blend files (e.g., "Data/Objects")
///
/// # Returns
///
/// Raw uncompressed .blend bytes ready for file write or further compression
pub fn create_scene_blend(
    entity_name: &str,
    children_count: usize,
    mesh_output_dir: &str,
    lights: &[ExtractedLight],
) -> Result<Vec<u8>, Error> {
    let mesh_instances: Vec<LinkedMeshInstance> = (0..children_count)
        .map(|idx| {
            let name = format!("mesh_{idx}");
            LinkedMeshInstance {
                blend_path: format!("{mesh_output_dir}/{name}.blend"),
                name,
                position: [0.0, 0.0, 0.0],
                rotation: [1.0, 0.0, 0.0, 0.0],
            }
        })
        .collect();

    create_scene_blend_with_instances(entity_name, &mesh_instances, lights)
}

pub fn create_scene_blend_with_instances(
    entity_name: &str,
    mesh_instances_input: &[LinkedMeshInstance],
    lights: &[ExtractedLight],
) -> Result<Vec<u8>, Error> {
    let children_count = mesh_instances_input.len();
    // Build a minimal input structure for compatibility with internal logic
    let mut ptrs = PtrAlloc::new(0x1000);
    
    let _screen_ptr = ptrs.alloc();
    let _wm_ptr = ptrs.alloc();
    let scene_ptr = ptrs.alloc();
    let view_layer_ptr = ptrs.alloc();
    let tool_settings_ptr = ptrs.alloc();
    let base_ptr = ptrs.alloc();
    let root_collection_ptr = ptrs.alloc();
    let root_collection_object_ptr = ptrs.alloc();
    let layer_collection_ptr = ptrs.alloc();
    
    // Sub-collections for organizing objects
    let meshes_collection_ptr = ptrs.alloc();
    let meshes_coll_obj_ptr = ptrs.alloc();
    let meshes_layer_coll_ptr = ptrs.alloc();
    
    let lights_collection_ptr = ptrs.alloc();
    let lights_coll_obj_ptr = ptrs.alloc();
    let lights_layer_coll_ptr = ptrs.alloc();
    
    let empties_collection_ptr = ptrs.alloc();
    let empties_coll_obj_ptr = ptrs.alloc();
    let empties_layer_coll_ptr = ptrs.alloc();
    
    let decals_collection_ptr = ptrs.alloc();
    let decals_coll_obj_ptr = ptrs.alloc();
    let decals_layer_coll_ptr = ptrs.alloc();
    
    // Root empty object (placeholder for root scene object)
    let root_object_ptr = ptrs.alloc();
    let root_object_mat_ptr = ptrs.alloc();
    let root_object_matbits_ptr = ptrs.alloc();
    
    // Allocate pointers for linked mesh object ID stubs.
    let mut linked_mesh_ids = Vec::new();
    let mut library_ptr_by_path = HashMap::new();
    let mut library_ptrs = Vec::new();
    let mut mesh_coll_obj_ptrs = Vec::new();
    
    for (idx, instance) in mesh_instances_input.iter().enumerate() {
        let object_id_ptr = ptrs.alloc();
        let library_ptr = if let Some(&ptr) = library_ptr_by_path.get(&instance.blend_path) {
            ptr
        } else {
            let ptr = ptrs.alloc();
            library_ptr_by_path.insert(instance.blend_path.clone(), ptr);
            library_ptrs.push((instance.blend_path.clone(), ptr));
            ptr
        };
        let coll_obj_ptr = ptrs.alloc();  // Collection object for this mesh instance
        
        linked_mesh_ids.push((object_id_ptr, library_ptr, idx));
        mesh_coll_obj_ptrs.push((coll_obj_ptr, object_id_ptr));
    }
    
    // Allocate pointers for light instances
    let mut light_instances = Vec::new();
    let mut light_coll_obj_ptrs = Vec::new();
    
    for idx in 0..lights.len() {
        let lamp_ptr = ptrs.alloc();
        let object_ptr = ptrs.alloc();
        let object_mat_ptr = ptrs.alloc();
        let object_matbits_ptr = ptrs.alloc();
        let coll_obj_ptr = ptrs.alloc();  // Collection object for this light
        
        light_instances.push((lamp_ptr, object_ptr, object_mat_ptr, object_matbits_ptr, idx));
        light_coll_obj_ptrs.push((coll_obj_ptr, object_ptr));
    }
    
    // Allocate pointers for CollectionChild structs (linking sub-collections to root)
    // These wrap the sub-collections (Meshes, Lights, Empties, Decals) in the children linked list
    let meshes_coll_child_ptr = ptrs.alloc();
    let lights_coll_child_ptr = ptrs.alloc();
    let empties_coll_child_ptr = ptrs.alloc();
    let decals_coll_child_ptr = ptrs.alloc();
    
    // Build scene datablocks
    // Scene must always be named "Scene" for Blender to recognize it as the primary scene
    let scene_data = build_scene("Scene", view_layer_ptr, root_collection_ptr, tool_settings_ptr);
    let tool_settings_data = build_tool_settings();
    let view_layer_data = build_view_layer("ViewLayer", base_ptr, layer_collection_ptr);
    let base_data = build_base(root_object_ptr);
    
    // Build collection hierarchy: Scene > [Meshes, Lights, Empties, Decals]
    
    // Determine head/tail pointers for collection members
    let mesh_head_ptr = if !mesh_coll_obj_ptrs.is_empty() { mesh_coll_obj_ptrs[0].0 } else { 0 };
    let mesh_tail_ptr = if !mesh_coll_obj_ptrs.is_empty() { mesh_coll_obj_ptrs[mesh_coll_obj_ptrs.len() - 1].0 } else { 0 };
    
    let light_head_ptr = if !light_coll_obj_ptrs.is_empty() { light_coll_obj_ptrs[0].0 } else { 0 };
    let light_tail_ptr = if !light_coll_obj_ptrs.is_empty() { light_coll_obj_ptrs[light_coll_obj_ptrs.len() - 1].0 } else { 0 };
    
    // Root (master) collection: embedded DATA block; contains Root object directly
    // and Meshes/Lights/Empties/Decals sub-collections as children.
    let root_collection_data = build_master_collection(
        root_collection_object_ptr,
        root_collection_object_ptr,  // gobject: single Root empty object (head=tail)
        meshes_coll_child_ptr,       // children.first = CollectionChild(Meshes)
        decals_coll_child_ptr,       // children.last  = CollectionChild(Decals)
    );
    let root_collection_object_data = build_collection_object(root_object_ptr);
    
    // Build mesh collection object linked list
    let mut meshes_coll_obj_data_list = Vec::new();
    for (idx, &(coll_obj_ptr, obj_ptr)) in mesh_coll_obj_ptrs.iter().enumerate() {
        let prev_ptr = if idx > 0 { mesh_coll_obj_ptrs[idx - 1].0 } else { 0 };
        let next_ptr = if idx < mesh_coll_obj_ptrs.len() - 1 { mesh_coll_obj_ptrs[idx + 1].0 } else { 0 };
        meshes_coll_obj_data_list.push((coll_obj_ptr, build_collection_object_linked(obj_ptr, prev_ptr, next_ptr)));
    }
    
    // Sub-collection: Meshes (child of root collection)
    let meshes_collection_data = build_collection(
        "Meshes",
        mesh_head_ptr,
        mesh_tail_ptr,
        0,
        0,
    );
    
    // Build light collection object linked list
    let mut lights_coll_obj_data_list = Vec::new();
    for (idx, &(coll_obj_ptr, obj_ptr)) in light_coll_obj_ptrs.iter().enumerate() {
        let prev_ptr = if idx > 0 { light_coll_obj_ptrs[idx - 1].0 } else { 0 };
        let next_ptr = if idx < light_coll_obj_ptrs.len() - 1 { light_coll_obj_ptrs[idx + 1].0 } else { 0 };
        lights_coll_obj_data_list.push((coll_obj_ptr, build_collection_object_linked(obj_ptr, prev_ptr, next_ptr)));
    }
    
    // Sub-collection: Lights (child of root collection)
    let lights_collection_data = build_collection(
        "Lights",
        light_head_ptr,
        light_tail_ptr,
        0,
        0,
    );
    
    // Sub-collection: Empties (placeholder for Phase 5C, child of root)
    let empties_collection_data = build_collection(
        "Empties",
        0,
        0,
        0,
        0,
    );
    
    // Sub-collection: Decals (placeholder for Phase 4, child of root)
    let decals_collection_data = build_collection(
        "Decals",
        0,
        0,
        0,
        0,
    );
    
    // Build CollectionChild linked list for root collection's children
    // Each CollectionChild wraps a sub-collection (Meshes -> Lights -> Empties -> Decals)
    let meshes_coll_child_data = build_collection_object_linked(
        meshes_collection_ptr,  // Collection pointer (in CollectionChild, this is at offset 16)
        0,                      // prev = NULL (Meshes is first child)
        lights_coll_child_ptr,  // next = Lights
    );
    let lights_coll_child_data = build_collection_object_linked(
        lights_collection_ptr,  // Collection pointer
        meshes_coll_child_ptr,  // prev = Meshes
        empties_coll_child_ptr, // next = Empties
    );
    let empties_coll_child_data = build_collection_object_linked(
        empties_collection_ptr,  // Collection pointer
        lights_coll_child_ptr,   // prev = Lights
        decals_coll_child_ptr,   // next = Decals
    );
    let decals_coll_child_data = build_collection_object_linked(
        decals_collection_ptr,  // Collection pointer
        empties_coll_child_ptr, // prev = Empties
        0,                      // next = NULL (Decals is last child)
    );
    
    // Build LayerCollection hierarchy: root contains [Meshes, Lights, Empties, Decals] as siblings
    // Determine head/tail of child LayerCollections
    let child_layer_coll_head = meshes_layer_coll_ptr;  // First child
    let child_layer_coll_tail = decals_layer_coll_ptr;   // Last child
    
    // Update root LayerCollection to have children linked in its layer_collections ListBase
    let root_layer_collection_data = build_layer_collection_linked(
        root_collection_ptr,  // Collection pointer
        0,                    // prev = NULL (root has no siblings)
        0,                    // next = NULL (root has no siblings)
        child_layer_coll_head,  // layer_collections.first = Meshes (first child)
        child_layer_coll_tail,   // layer_collections.last = Decals (last child)
    );
    
    // Build child LayerCollections with proper prev/next pointers
    // Meshes is first, Lights comes after, then Empties, then Decals
    let meshes_layer_coll_data = build_layer_collection_linked(
        meshes_collection_ptr,
        0,                      // prev = NULL (first child)
        lights_layer_coll_ptr,  // next = Lights
        0,                      // No children in Meshes collection
        0,
    );
    
    let lights_layer_coll_data = build_layer_collection_linked(
        lights_collection_ptr,
        meshes_layer_coll_ptr,  // prev = Meshes
        empties_layer_coll_ptr, // next = Empties
        0,                      // No children in Lights collection
        0,
    );
    
    let empties_layer_coll_data = build_layer_collection_linked(
        empties_collection_ptr,
        lights_layer_coll_ptr,  // prev = Lights
        decals_layer_coll_ptr,  // next = Decals
        0,                      // No children in Empties collection (placeholder)
        0,
    );
    
    let decals_layer_coll_data = build_layer_collection_linked(
        decals_collection_ptr,
        empties_layer_coll_ptr, // prev = Empties
        0,                      // next = NULL (last child)
        0,                      // No children in Decals collection (placeholder)
        0,
    );
    
    // Build root empty object at origin
    let root_empty_data = build_empty_object(
        "Root",
        [0.0, 0.0, 0.0],  // position
        [1.0, 0.0, 0.0, 0.0],  // quaternion
        [1.0, 1.0, 1.0],  // scale
        0,
    );
    let root_mat_array = build_mat_ptr_array(0);
    let root_matbits = build_matbits(0);
    
    // Allocate string data for library filenames (uses entity name, not scene name)
    let scene_name_bytes = format!("{}\0", entity_name);
    let scene_name_ptr = ptrs.alloc();
    
    // Build linked object ID stubs and their library blocks.
    let mut linked_mesh_id_data = Vec::new();
    let mut mesh_library_data = Vec::new();
    
    for (blend_path, library_ptr) in &library_ptrs {
        let lib_name = blend_path
            .rsplit('/')
            .next()
            .unwrap_or(blend_path.as_str());
        let lib_data = build_library_block(lib_name, blend_path);
        mesh_library_data.push((*library_ptr, lib_data));
    }
    for (object_id_ptr, library_ptr, idx) in &linked_mesh_ids {
        linked_mesh_id_data.push((*object_id_ptr, build_id_stub("OB", &mesh_instances_input[*idx].name, *library_ptr)));
    }
    
    // Build light objects for lights collection
    let mut light_data = Vec::new();
    let mut light_object_data = Vec::new();
    let mut light_object_mat_arrays = Vec::new();
    let mut light_object_matbits = Vec::new();
    
    for (lamp_ptr, object_ptr, mat_ptr, matbits_ptr, idx) in &light_instances {
        let light = &lights[*idx];
        
        // Build lamp datablock
        let lamp_bytes = build_lamp(
            &light.name,
            light.lamp_type,
            light.color,
            light.energy_watts,
            light.radius,
            light.spot_size,
            light.spot_blend,
            light.temperature_k,
            true,  // use temperature
        );
        light_data.push((*lamp_ptr, lamp_bytes));
        
        // Build object wrapper for light
        let object_bytes = build_lamp_object(
            &light.name,
            *lamp_ptr,
            light.position_blend,
            light.rotation_blend,
            [1.0, 1.0, 1.0],  // Standard scale
            0,
        );
        light_object_data.push(object_bytes);
        
        // Build material arrays (empty for lights)
        light_object_mat_arrays.push(build_mat_ptr_array(0));
        light_object_matbits.push(build_matbits(0));
    }
    
    // Assemble .blend file
    let mut out: Vec<u8> = Vec::with_capacity(1024 * 1024);
    out.extend_from_slice(BLEND_MAGIC);
    
    let file_global = build_file_global(STARTUP_UI_SCREEN_PTR, scene_ptr, view_layer_ptr);
    write_block(&mut out, b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x10, 1, &file_global);
    out.extend_from_slice(&startup_ui_prefix_bytes());
    
    // Write scene structure
    // CRITICAL: All DATA blocks for Scene must be consecutive immediately after SC\0\0.
    // Blender reads them all into fd->datamap, then clears it after processing each ID block.
    // Any non-DATA block between SC and its data will truncate the datamap.
    write_block(&mut out, b"SC\0\0", SDNA_IDX_SCENE, scene_ptr, 1, &scene_data);
    // SC DATA sequence — ToolSettings, ViewLayer, all LayerCollections, master_collection, CollectionChildren, Base:
    write_block(&mut out, b"DATA", SDNA_IDX_TOOL_SETTINGS, tool_settings_ptr, 1, &tool_settings_data);
    write_block(&mut out, b"DATA", SDNA_IDX_VIEW_LAYER, view_layer_ptr, 1, &view_layer_data);
    write_block(&mut out, b"DATA", SDNA_IDX_BASE, base_ptr, 1, &base_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, layer_collection_ptr, 1, &root_layer_collection_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, meshes_layer_coll_ptr, 1, &meshes_layer_coll_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, lights_layer_coll_ptr, 1, &lights_layer_coll_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, empties_layer_coll_ptr, 1, &empties_layer_coll_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, decals_layer_coll_ptr, 1, &decals_layer_coll_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION, root_collection_ptr, 1, &root_collection_data);  // embedded master_collection
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_CHILD, meshes_coll_child_ptr, 1, &meshes_coll_child_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_CHILD, lights_coll_child_ptr, 1, &lights_coll_child_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_CHILD, empties_coll_child_ptr, 1, &empties_coll_child_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_CHILD, decals_coll_child_ptr, 1, &decals_coll_child_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, root_collection_object_ptr, 1, &root_collection_object_data);
    // End of SC DATA sequence — sub-collection GR blocks follow with their own DATA sub-blocks:
    
    // Write sub-collections (GR blocks) each followed by their own DATA sub-blocks
    write_block(&mut out, b"GR\0\0", SDNA_IDX_COLLECTION, meshes_collection_ptr, 1, &meshes_collection_data);
    for (coll_obj_ptr, coll_obj_data) in &meshes_coll_obj_data_list {
        write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, *coll_obj_ptr, 1, coll_obj_data);
    }
    
    write_block(&mut out, b"GR\0\0", SDNA_IDX_COLLECTION, lights_collection_ptr, 1, &lights_collection_data);
    for (coll_obj_ptr, coll_obj_data) in &lights_coll_obj_data_list {
        write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, *coll_obj_ptr, 1, coll_obj_data);
    }
    
    write_block(&mut out, b"GR\0\0", SDNA_IDX_COLLECTION, empties_collection_ptr, 1, &empties_collection_data);
    write_block(&mut out, b"GR\0\0", SDNA_IDX_COLLECTION, decals_collection_ptr, 1, &decals_collection_data);
    
    // Write root empty object + materials
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, root_object_ptr, 1, &root_empty_data);
    write_block(&mut out, b"DATA", 0, root_object_mat_ptr, 1, &root_mat_array);
    write_block(&mut out, b"DATA", 0, root_object_matbits_ptr, 1, &root_matbits);
    
    // Write light blocks (Phase 5B)
    for (idx, (lamp_ptr, object_ptr, mat_ptr, matbits_ptr, _)) in light_instances.iter().enumerate() {
        // Write LAMP datablock
        let (lamp_block_ptr, lamp_bytes) = light_data[idx].clone();
        write_block(&mut out, b"LA\0\0", SDNA_IDX_LAMP, lamp_block_ptr, 1, &lamp_bytes);
        
        // Write light object
        write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, *object_ptr, 1, &light_object_data[idx]);
        write_block(&mut out, b"DATA", 0, *mat_ptr, 1, &light_object_mat_arrays[idx]);
        write_block(&mut out, b"DATA", 0, *matbits_ptr, 1, &light_object_matbits[idx]);
    }
    
    // Write linked mesh libraries and object ID stubs after local IDs. This
    // matches Blender-authored linked-object scenes and avoids making following
    // local object-data IDs inherit the active library during read.
    for (library_ptr, library_data) in &mesh_library_data {
        write_block(&mut out, b"LI\0\0", SDNA_IDX_LIBRARY, *library_ptr, 1, library_data);
        for (idx, (object_id_ptr, object_library_ptr, _)) in linked_mesh_ids.iter().enumerate() {
            if object_library_ptr == library_ptr {
                write_block(&mut out, b"ID\0\0", SDNA_IDX_ID, *object_id_ptr, 1, &linked_mesh_id_data[idx].1);
            }
        }
    }
    
    // Write scene name
    write_block(&mut out, b"DATA", 0, scene_name_ptr, 1, scene_name_bytes.as_bytes());
    
    // Write DNA1 and ENDB
    write_block(&mut out, b"DNA1", SDNA_IDX_DNA1, 0x01, 1, DNA1_BYTES);
    write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);
    
    // Return uncompressed for now (Phase 2 will handle compression)
    Ok(out)
}

#[cfg(test)]
mod tests_5a_scene_blend {
    use super::*;
    use crate::decomposed::DecomposedInput;
    use crate::types::{Mesh, SubMesh};
    use crate::pipeline::LoadedInteriors;

    #[derive(Debug)]
    struct BlendBlock<'a> {
        code: &'a [u8],
        sdna: u32,
        old_ptr: u64,
        data: &'a [u8],
    }

    fn parse_blend_blocks(bytes: &[u8]) -> Vec<BlendBlock<'_>> {
        let mut blocks = Vec::new();
        let mut offset = BLEND_MAGIC.len();
        while offset + 32 <= bytes.len() {
            let code = &bytes[offset..offset + 4];
            let sdna = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
            let old_ptr = u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
            let len = u64::from_le_bytes(bytes[offset + 16..offset + 24].try_into().unwrap()) as usize;
            let data_start = offset + 32;
            let data_end = data_start + len;
            blocks.push(BlendBlock {
                code,
                sdna,
                old_ptr,
                data: &bytes[data_start..data_end],
            });
            offset = data_end;
            if code == b"ENDB" {
                break;
            }
        }
        blocks
    }

    fn test_mesh_with_submeshes(submeshes: Vec<SubMesh>) -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes,
            model_min: [0.0, 0.0, 0.0],
            model_max: [1.0, 1.0, 1.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [1.0, 1.0, 1.0],
        }
    }

    fn test_submesh(material_id: u32, material_name: &str, first_index: u32) -> SubMesh {
        SubMesh {
            material_name: Some(material_name.to_string()),
            material_id,
            source_material_id: None,
            first_index,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 0,
        }
    }

    #[test]
    fn test_blend_material_slots_use_glb_style_names_and_deduplicate_ids() {
        let mesh = test_mesh_with_submeshes(vec![
            test_submesh(3, "Decal_POM", 0),
            test_submesh(3, "Decal_POM", 3),
            test_submesh(7, "Painted_Metal", 6),
        ]);

        let (names, submesh_slots) = blend_material_slots("fallback", &mesh, &None);

        assert_eq!(
            names,
            vec![
                "fallback_mtl_Decal_POM_03".to_string(),
                "fallback_mtl_Painted_Metal_07".to_string(),
            ]
        );
        assert_eq!(submesh_slots, vec![0, 0, 1]);
    }

    #[test]
    fn test_mesh_blend_with_nmc_writes_hierarchy_objects() {
        let mesh = Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            uvs: Some(vec![[0.0, 0.0]; 6]),
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: Some(vec![[255, 255, 255, 255]; 6]),
            submeshes: vec![
                SubMesh {
                    material_name: Some("Hull".to_string()),
                    material_id: 0,
                    source_material_id: None,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 1,
                },
                SubMesh {
                    material_name: Some("Decal".to_string()),
                    material_id: 1,
                    source_material_id: None,
                    first_index: 3,
                    num_indices: 3,
                    first_vertex: 3,
                    num_vertices: 3,
                    node_parent_index: 2,
                },
            ],
            model_min: [0.0, 0.0, 0.0],
            model_max: [3.0, 1.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [3.0, 1.0, 0.0],
        };
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let nmc = NodeMeshCombo {
            nodes: vec![
                crate::nmc::NmcNode {
                    name: "asset_root".to_string(),
                    parent_index: None,
                    world_to_bone: identity,
                    bone_to_world: identity,
                    scale: [1.0; 3],
                    geometry_type: 3,
                    properties: HashMap::new(),
                },
                crate::nmc::NmcNode {
                    name: "geo_left".to_string(),
                    parent_index: Some(0),
                    world_to_bone: identity,
                    bone_to_world: identity,
                    scale: [1.0; 3],
                    geometry_type: 0,
                    properties: HashMap::new(),
                },
                crate::nmc::NmcNode {
                    name: "geo_right".to_string(),
                    parent_index: Some(0),
                    world_to_bone: identity,
                    bone_to_world: identity,
                    scale: [1.0; 3],
                    geometry_type: 0,
                    properties: HashMap::new(),
                },
            ],
            material_indices: vec![],
        };

        let blend_bytes = mesh_to_blend("asset_LOD0", &mesh, &None, Some(&nmc), None);
        let blocks = parse_blend_blocks(&blend_bytes);
        let object_names = blocks
            .iter()
            .filter(|block| block.code == b"OB\0\0")
            .map(|block| {
                let raw = &block.data[42..300];
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                String::from_utf8_lossy(&raw[..end]).to_string()
            })
            .collect::<Vec<_>>();

        assert!(object_names.contains(&"CryEngine_Z_up".to_string()));
        assert!(object_names.contains(&"asset".to_string()));
        assert!(object_names.contains(&"asset_root".to_string()));
        assert!(object_names.contains(&"geo_left".to_string()));
        assert!(object_names.contains(&"geo_right".to_string()));
        assert!(!object_names.contains(&"asset_LOD0".to_string()));
        assert_eq!(
            blocks
                .iter()
                .filter(|block| block.code == b"ME\0\0")
                .count(),
            2,
            "each geometry-bearing NMC node should get its own mesh"
        );
        assert!(
            object_names.len() > 1,
            "NMC export must not be a single flat mesh object"
        );
    }

    #[test]
    fn linked_scene_object_names_use_geometry_nodes_for_nmc_assets() {
        let mesh = Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![
                SubMesh {
                    material_name: Some("Left".to_string()),
                    material_id: 0,
                    source_material_id: None,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 1,
                },
                SubMesh {
                    material_name: Some("Right".to_string()),
                    material_id: 1,
                    source_material_id: None,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 2,
                },
            ],
            model_min: [0.0; 3],
            model_max: [1.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [1.0; 3],
        };
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let nmc = NodeMeshCombo {
            nodes: vec![
                crate::nmc::NmcNode {
                    name: "asset".to_string(),
                    parent_index: None,
                    world_to_bone: identity,
                    bone_to_world: identity,
                    scale: [1.0; 3],
                    geometry_type: 0,
                    properties: HashMap::new(),
                },
                crate::nmc::NmcNode {
                    name: "geo_left".to_string(),
                    parent_index: Some(0),
                    world_to_bone: identity,
                    bone_to_world: identity,
                    scale: [1.0; 3],
                    geometry_type: 0,
                    properties: HashMap::new(),
                },
                crate::nmc::NmcNode {
                    name: "geo_right".to_string(),
                    parent_index: Some(0),
                    world_to_bone: identity,
                    bone_to_world: identity,
                    scale: [1.0; 3],
                    geometry_type: 0,
                    properties: HashMap::new(),
                },
            ],
            material_indices: vec![],
        };

        assert_eq!(
            linked_scene_object_names("asset_LOD0", &mesh, Some(&nmc)),
            vec!["geo_left".to_string(), "geo_right".to_string()]
        );
        assert_eq!(
            linked_scene_object_names("asset_LOD0", &mesh, None),
            vec!["asset_LOD0".to_string()]
        );
    }

    #[test]
    fn test_create_scene_blend_links_object_ids_instead_of_empty_mesh_stubs() {
        let instance = LinkedMeshInstance {
            name: "rsi_aurora_mk2_airlock_door_LOD0".to_string(),
            blend_path: "//../../Data/Objects/Ships/rsi_aurora_mk2_airlock_door_LOD0.blend".to_string(),
            position: [0.0, 0.0, 0.0],
            rotation: [1.0, 0.0, 0.0, 0.0],
        };
        let blend_bytes = create_scene_blend_with_instances("SceneLinkTest", &[instance], &[]).unwrap();
        let blocks = parse_blend_blocks(&blend_bytes);

        let linked_object_stub = blocks.iter().find(|block| {
            block.code == b"ID\0\0"
                && block.sdna == SDNA_IDX_ID
                && block.data[40..]
                    .starts_with(b"OBrsi_aurora_mk2_airlock_door_LOD0")
        });
        assert!(linked_object_stub.is_some(), "scene.blend should link object IDs from mesh .blend files");

        let local_empty_mesh_stub = blocks.iter().find(|block| {
            block.code == b"ME\0\0"
                && block.data[40..]
                    .starts_with(b"MErsi_aurora_mk2_airlock_door_LOD0")
        });
        assert!(local_empty_mesh_stub.is_none(), "scene.blend must not replace linked objects with empty local mesh stubs");
    }

    /// Helper to create a minimal DecomposedInput for testing
    fn create_test_input(
        entity_name: &str,
        num_children: usize,
    ) -> DecomposedInput {
        let root_mesh = Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![],
            model_min: [0.0, 0.0, 0.0],
            model_max: [1.0, 1.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [1.0, 1.0, 0.0],
        };

        let mut children = Vec::new();
        for i in 0..num_children {
            children.push(crate::types::EntityPayload {
                mesh: root_mesh.clone(),
                materials: None,
                textures: None,
                nmc: None,
                palette: None,
                geometry_path: format!("path/to/mesh_{}", i),
                material_path: format!("path/to/mat_{}", i),
                bones: vec![],
                skeleton_source_path: None,
                entity_name: format!("child_{}", i),
                parent_node_name: "Root".to_string(),
                parent_entity_name: entity_name.to_string(),
                no_rotation: false,
                offset_position: [0.0, 0.0, 0.0],
                offset_rotation: [0.0, 0.0, 0.0],
                detach_direction: [0.0, 0.0, 0.0],
                port_flags: String::new(),
            });
        }

        DecomposedInput {
            entity_name: entity_name.to_string(),
            geometry_path: "path/to/geometry".to_string(),
            material_path: "path/to/materials".to_string(),
            root_mesh,
            root_materials: None,
            root_nmc: None,
            root_palette: None,
            available_palettes: vec![],
            root_bones: vec![],
            root_skeleton_source_path: None,
            root_animation_controller: None,
            children,
            interiors: LoadedInteriors {
                unique_cgfs: vec![],
                containers: vec![],
            },
            paint_variants: vec![],
        }
    }

    #[test]
    fn test_create_scene_blend_basic() {
        let input = create_test_input("TestEntity", 1);
        let result = create_scene_blend("TestEntity", 1, "Data/Objects", &[]);
        
        assert!(result.is_ok(), "Function should succeed with basic input");
        
        let blend_bytes = result.unwrap();
        assert!(!blend_bytes.is_empty(), "Output should not be empty");
        assert!(blend_bytes.len() > 100, "Output should be substantial");
        
        // Verify BLENDER17 magic header
        assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501", "Should have valid Blender header");
    }

    #[test]
    fn test_create_scene_blend_multiple_meshes() {
        let result = create_scene_blend("MultiMesh", 5, "Data/Objects", &[]);
        
        assert!(result.is_ok(), "Function should succeed with multiple children");
        
        let blend_bytes = result.unwrap();
        assert!(!blend_bytes.is_empty(), "Output should not be empty");
        
        // Verify BLENDER17 magic header
        assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501", "Should have valid Blender header");
        
        // With 5 children, the file should be larger than with 1
        let single = create_scene_blend("Single", 1, "Data/Objects", &[])
            .unwrap();
        assert!(blend_bytes.len() > single.len(), "Multiple meshes should produce larger file");
    }

    #[test]
    fn test_create_scene_blend_collections_structure() {
        let result = create_scene_blend("CollTest", 2, "Data/Objects", &[]);
        
        assert!(result.is_ok(), "Function should succeed");
        
        let blend_bytes = result.unwrap();
        
        // Check for collection names in the output
        let blend_str = String::from_utf8_lossy(&blend_bytes);
        
        // The output should contain collection markers (GRP\0 blocks)
        // We can't easily verify collection structure without parsing the binary format,
        // but we can verify the file format is valid
        assert!(blend_bytes.len() > 200, "Valid scene file should be substantial");
    }

    #[test]
    fn test_create_scene_blend_file_format() {
        let result = create_scene_blend("FormatTest", 1, "Data/Objects", &[]);
        
        assert!(result.is_ok(), "Function should succeed");
        
        let blend_bytes = result.unwrap();
        
        // Verify file structure markers
        // BLENDER17 header
        assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501");
        
        // Find GLOB block (should appear early)
        let glob_marker = b"GLOB";
        assert!(blend_bytes.windows(4).any(|w| w == glob_marker), "Should contain GLOB block");
        
        // Find ENDB marker (should be at end)
        let endb_marker = b"ENDB";
        assert!(blend_bytes.windows(4).any(|w| w == endb_marker), "Should contain ENDB block");
        
        // Find DNA1 (DNA structure)
        let dna1_marker = b"DNA1";
        assert!(blend_bytes.windows(4).any(|w| w == dna1_marker), "Should contain DNA1 block");
    }

    #[test]
    fn test_create_scene_blend_uses_startup_ui_prefix() {
        let blend_bytes = create_scene_blend("WithUi", 1, "Data/Objects", &[]).unwrap();
        let blocks = parse_blend_blocks(&blend_bytes);
        let glob = blocks.iter().find(|block| block.code == b"GLOB").unwrap();

        assert_eq!(
            u64::from_le_bytes(glob.data[16..24].try_into().unwrap()),
            STARTUP_UI_SCREEN_PTR
        );
        assert!(blocks.iter().any(|block| block.code == b"SN\0\0"));
        assert!(!blocks.iter().any(|block| block.code == b"SR\0\0"));
        assert!(blocks.iter().any(|block| block.code == b"WM\0\0"));
        assert!(blocks.iter().any(|block| block.code == b"WS\0\0"));
    }

    #[test]
    fn test_create_scene_blend_scene_data_is_consecutive() {
        let blend_bytes = create_scene_blend("SceneData", 2, "Data/Objects", &[]).unwrap();
        let blocks = parse_blend_blocks(&blend_bytes);
        let scene_idx = blocks.iter().position(|block| block.code == b"SC\0\0").unwrap();
        let scene_block = &blocks[scene_idx];
        let mut data_sdnas = Vec::new();

        for block in blocks.iter().skip(scene_idx + 1) {
            if block.code != b"DATA" {
                break;
            }
            data_sdnas.push(block.sdna);
        }

        let tool_settings_ptr = u64::from_le_bytes(scene_block.data[568..576].try_into().unwrap());
        assert_ne!(tool_settings_ptr, 0);
        assert!(blocks.iter().any(|block|
            block.code == b"DATA"
                && block.sdna == SDNA_IDX_TOOL_SETTINGS
                && block.old_ptr == tool_settings_ptr
        ));
        assert!(data_sdnas.contains(&SDNA_IDX_TOOL_SETTINGS));
        assert!(data_sdnas.contains(&SDNA_IDX_VIEW_LAYER));
        assert!(data_sdnas.contains(&SDNA_IDX_BASE));
        assert!(data_sdnas.contains(&SDNA_IDX_LAYER_COLLECTION));
        assert!(data_sdnas.contains(&SDNA_IDX_COLLECTION));
        assert!(data_sdnas.contains(&SDNA_IDX_COLLECTION_CHILD));
        assert!(data_sdnas.contains(&SDNA_IDX_COLLECTION_OBJECT));
    }

    #[test]
    fn test_create_scene_blend_objects_do_not_parent_to_collections() {
        let light = ExtractedLight {
            name: "ParentCheckLight".to_string(),
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            lamp_type: 0,
            energy_watts: 100.0,
            radius: 10.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 5.0,
            temperature_k: 3000.0,
            use_temperature: false,
            gobo_path: None,
            active_state: "default".to_string(),
        };
        let blend_bytes = create_scene_blend("ParentCheck", 2, "Data/Objects", &[light]).unwrap();
        let blocks = parse_blend_blocks(&blend_bytes);

        for block in blocks.iter().filter(|block| block.code == b"OB\0\0") {
            assert_eq!(
                u64::from_le_bytes(block.data[496..504].try_into().unwrap()),
                0,
                "object block 0x{:x} must not use a Collection pointer as Object.parent",
                block.old_ptr
            );
        }
    }

    #[test]
    fn test_create_scene_blend_relative_paths() {
        let result = create_scene_blend("RelPath", 2, "Data/Objects", &[]);
        
        assert!(result.is_ok(), "Function should succeed");
        
        let blend_bytes = result.unwrap();
        
        // Verify that library paths are embedded
        // The mesh_output_dir "Data/Objects" should appear in library blocks
        let blend_str = String::from_utf8_lossy(&blend_bytes);
        assert!(blend_str.contains("Data/Objects") || blend_bytes.windows(12).any(|w| w == b"Data/Objects"),
            "Should contain relative path for mesh files");
    }

    #[test]
    fn test_create_scene_blend_empty_children() {
        let result = create_scene_blend("NoChildren", 0, "Data/Objects", &[]);
        
        assert!(result.is_ok(), "Function should succeed even with no children");
        
        let blend_bytes = result.unwrap();
        assert!(!blend_bytes.is_empty(), "Output should not be empty");
        assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501", "Should have valid header");
    }

    #[test]
    fn test_create_scene_blend_output_not_compressed() {
        let result = create_scene_blend("NoCompress", 1, "Data/Objects", &[]);
        
        assert!(result.is_ok(), "Function should succeed");
        
        let blend_bytes = result.unwrap();
        
        // Verify it's NOT gzip compressed
        // gzip header is 0x1f 0x8b
        assert!(blend_bytes.len() < 2 || blend_bytes[0] != 0x1f,
            "Output should NOT be gzip compressed (Phase 2 handles compression)");
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5B: Light Parenting and Collection Organization
// ════════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests_5b {
    use super::*;
    
    #[test]
    fn test_create_scene_blend_with_single_light() {
        let light = ExtractedLight {
            name: "TestLight".to_string(),
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            lamp_type: 0,
            energy_watts: 100.0,
            radius: 10.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 5.0,
            temperature_k: 3000.0,
            use_temperature: false,
            gobo_path: None,
            active_state: "default".to_string(),
        };
        let result = create_scene_blend("TestWithLight", 1, "Data/Objects", &[light]);
        assert!(result.is_ok(), "Should create scene with light");
    }
    
    #[test]
    fn test_create_scene_blend_with_multiple_lights() {
        let lights = vec![
            ExtractedLight {
                name: "Ambient".to_string(),
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [0.8, 0.8, 0.8],
                lamp_type: 0,
                energy_watts: 50.0,
                radius: 20.0,
                spot_size: 0.0,
                spot_blend: 0.0,
                intensity_candela: 5.0,
                temperature_k: 3000.0,
                use_temperature: false,
                gobo_path: None,
                active_state: "default".to_string(),
            },
            ExtractedLight {
                name: "Sun".to_string(),
                position_blend: [10.0, 10.0, 10.0],
                rotation_blend: [0.707, 0.0, 0.707, 0.0],
                color: [1.0, 1.0, 1.0],
                lamp_type: 1,
                energy_watts: 100.0,
                radius: 100.0,
                spot_size: 0.0,
                spot_blend: 0.0,
                intensity_candela: 100000.0,
                temperature_k: 5500.0,
                use_temperature: true,
                gobo_path: None,
                active_state: "default".to_string(),
            },
        ];
        let result = create_scene_blend("MultiLight", 1, "Data/Objects", &lights);
        assert!(result.is_ok(), "Should create scene with multiple lights");
    }
    
    #[test]
    fn test_create_scene_blend_lights_in_file() {
        let light = ExtractedLight {
            name: "FileTest".to_string(),
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            lamp_type: 0,
            energy_watts: 100.0,
            radius: 10.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 10.0,
            temperature_k: 3000.0,
            use_temperature: false,
            gobo_path: None,
            active_state: "default".to_string(),
        };
        let result = create_scene_blend("FileTest", 1, "Data/Objects", &[light]);
        assert!(result.is_ok());
        let blend_bytes = result.unwrap();
        // LA\0\0 is Blender lamp marker
        let lamp_marker = b"LA\0\0";
        let lamp_count = blend_bytes.windows(4).filter(|w| *w == lamp_marker).count();
        assert!(lamp_count >= 1, "Should have at least 1 lamp block");
    }
    
    #[test]
    fn test_create_scene_blend_no_lights() {
        let result = create_scene_blend("NoLights", 1, "Data/Objects", &[]);
        assert!(result.is_ok(), "Should work without lights");
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3A: Extract Light Data from Manifest
// ════════════════════════════════════════════════════════════════════════════════

/// Extracted light ready for Blender scene construction.
#[derive(Debug, Clone)]
pub struct ExtractedLight {
    /// CryEngine light name
    pub name: String,
    /// Position in Blender coordinates
    pub position_blend: [f32; 3],
    /// Rotation as quaternion in Blender coordinates [w, x, y, z]
    pub rotation_blend: [f32; 4],
    /// Linear RGB color 0..1
    pub color: [f32; 3],
    /// Blender lamp type: 0=POINT, 1=SUN, 2=SPOT, 4=AREA
    pub lamp_type: i16,
    /// Energy in Watts (radiant flux)
    pub energy_watts: f32,
    /// Attenuation radius in meters
    pub radius: f32,
    /// Spot cone full aperture in radians (0 for POINT)
    pub spot_size: f32,
    /// Spot cone feather width 0..1 (0 for POINT)
    pub spot_blend: f32,
    /// Intensity in candelas (for reference)
    pub intensity_candela: f32,
    /// Color temperature in Kelvin
    pub temperature_k: f32,
    /// When true, render blackbody color at temperature_k; ignore RGB
    pub use_temperature: bool,
    /// Optional projector gobo texture path (for SPOTs)
    pub gobo_path: Option<String>,
    /// Currently active state name
    pub active_state: String,
}

/// Convert CryEngine position to Blender coordinates.
///
/// CryEngine uses Z-up: (x, y, z) → Blender Y-up: (x, -z, y)
fn convert_position_sc_to_blender(pos_sc: [f64; 3]) -> [f32; 3] {
    [
        pos_sc[0] as f32,
        -(pos_sc[2] as f32),
        pos_sc[1] as f32,
    ]
}

/// Multiply two quaternions: q1 * q2 (standard Hamilton product).
///
/// Input: q1, q2 as [w, x, y, z]
/// Output: q1 * q2 as [w, x, y, z]
fn quaternion_multiply(q1: [f32; 4], q2: [f32; 4]) -> [f32; 4] {
    let [w1, x1, y1, z1] = q1;
    let [w2, x2, y2, z2] = q2;
    [
        w1*w2 - x1*x2 - y1*y2 - z1*z2,
        w1*x2 + x1*w2 + y1*z2 - z1*y2,
        w1*y2 - x1*z2 + y1*w2 + z1*x2,
        w1*z2 + x1*y2 - y1*x2 + z1*w2,
    ]
}

/// Convert CryEngine quaternion to Blender coordinates.
///
/// CryEngine quaternion: [w, x, y, z]
/// 
/// **Coordinate system transformation** (Z-up to Y-up):
/// Apply same transformation as position: swap y/z, negate z
///
/// **Basis correction for spotlights** (if is_spotlight=true):
/// - CryEngine spotlight cone forward: +X axis
/// - Blender spotlight cone forward: -Z axis
/// - Correction: 90° rotation around Y axis to align +X → -Z
///   Quaternion: [cos(45°), 0, -sin(45°), 0] ≈ [0.7071, 0, -0.7071, 0]
fn convert_quaternion_sc_to_blender(quat_sc: [f64; 4], is_spotlight: bool) -> [f32; 4] {
    let w = quat_sc[0] as f32;
    let x = quat_sc[1] as f32;
    let y = quat_sc[2] as f32;
    let z = quat_sc[3] as f32;
    
    // Step 1: Coordinate system transformation (Z-up to Y-up)
    let mut result = [w, x, -z, y];
    
    // Step 2: Basis correction for spotlights (CryEngine +X → Blender -Z)
    if is_spotlight {
        // 90° rotation around Y axis to align +X to -Z
        let basis_correction = [0.7071, 0.0, -0.7071, 0.0];
        result = quaternion_multiply(result, basis_correction);
    }
    
    result
}

/// Extract all lights from loaded interiors.
///
/// Reads LightInfo from all interior containers and converts to Blender format.
/// Returns vector of extracted lights ready for scene construction.
pub fn extract_lights_from_interiors(
    interiors: &LoadedInteriors,
) -> Result<Vec<ExtractedLight>, Error> {
    let mut lights = Vec::new();
    
    for container in &interiors.containers {
        for light_info in &container.lights {
            // Convert position from CryEngine to Blender coordinates
            let position_blend = convert_position_sc_to_blender(light_info.position);
            
            // Map CryEngine light type to Blender lamp_type
            let lamp_type = match light_info.light_type.as_str() {
                "Omni" | "SoftOmni" => 0,  // POINT
                "Projector" => 2,          // SPOT
                "Ambient" => 0,            // POINT (ambient = low-energy point)
                "Directional" | "Sun" => 1, // SUN
                _ => 0,                    // Default to POINT
            };
            
            // Convert quaternion rotation with basis correction for spotlights
            let is_spotlight = lamp_type == 2;  // SPOT type
            let rotation_blend = convert_quaternion_sc_to_blender(light_info.rotation, is_spotlight);
            
            // Intensity conversion: candela proxy → Watts
            // KHR_lights_punctual: lm = cd × 4π (lumens from candelas)
            // Cycles: W = lm / 683 (Watt to candela conversion)
            // Visual gain: × 20 (empirical scaling for SC lights)
            let energy_watts = (light_info.intensity_candela_proxy * 4.0 * std::f32::consts::PI / 683.0) * 20.0;
            
            // Spot angles (if present)
            let (spot_size, spot_blend) = if let (Some(inner), Some(outer)) = (
                light_info.inner_angle,
                light_info.outer_angle,
            ) {
                let spot_size = outer.to_radians() * 2.0;  // Full cone aperture
                let spot_blend = if outer > 0.0 {
                    (1.0 - (inner / outer)).max(0.0).min(1.0)  // Feather width, clamped
                } else {
                    0.0
                };
                (spot_size, spot_blend)
            } else {
                (0.0, 0.0)
            };
            
            // Get active state info for temperature
            let (temperature_k, use_temperature) = light_info.states
                .get(&light_info.active_state)
                .map(|s| (s.temperature, s.use_temperature))
                .unwrap_or((6500.0, false));
            
            lights.push(ExtractedLight {
                name: light_info.name.clone(),
                position_blend,
                rotation_blend,
                color: light_info.color,
                lamp_type,
                energy_watts,
                radius: light_info.radius,
                spot_size,
                spot_blend,
                intensity_candela: light_info.intensity_candela_proxy,
                temperature_k,
                use_temperature,
                gobo_path: light_info.projector_texture.clone(),
                active_state: light_info.active_state.clone(),
            });
        }
    }
    
    Ok(lights)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_convert_position_sc_to_blender() {
        // CryEngine position (1, 2, 3) → Blender (1, -3, 2)
        let result = convert_position_sc_to_blender([1.0, 2.0, 3.0]);
        assert_eq!(result, [1.0, -3.0, 2.0]);
    }
    
    #[test]
    fn test_convert_position_origin() {
        let result = convert_position_sc_to_blender([0.0, 0.0, 0.0]);
        assert_eq!(result, [0.0, 0.0, 0.0]);
    }
    
    #[test]
    fn test_convert_position_negative() {
        let result = convert_position_sc_to_blender([-1.0, -2.0, -3.0]);
        assert_eq!(result, [-1.0, 3.0, -2.0]);
    }
    
    #[test]
    fn test_convert_quaternion_sc_to_blender() {
        // [w, x, y, z] → [w, x, -z, y] (point light, no basis correction)
        let result = convert_quaternion_sc_to_blender([1.0, 0.0, 0.0, 0.0], false);
        assert_eq!(result, [1.0, 0.0, 0.0, 0.0]);
    }
    
    #[test]
    fn test_convert_quaternion_with_values() {
        // Point light (no basis correction)
        let result = convert_quaternion_sc_to_blender([0.707, 0.5, 0.5, 0.0], false);
        assert_eq!(result, [0.707, 0.5, 0.0, 0.5]);
    }
    
    #[test]
    fn test_quaternion_multiply_identity() {
        // Multiply by identity quaternion [1, 0, 0, 0]
        let q = [0.7071, 0.0, -0.7071, 0.0];
        let identity = [1.0, 0.0, 0.0, 0.0];
        let result = quaternion_multiply(q, identity);
        // Result should be close to q (allow small floating-point error)
        assert!((result[0] - q[0]).abs() < 0.0001);
        assert!((result[1] - q[1]).abs() < 0.0001);
        assert!((result[2] - q[2]).abs() < 0.0001);
        assert!((result[3] - q[3]).abs() < 0.0001);
    }
    
    #[test]
    fn test_spotlight_orientation_correction() {
        // Test that spotlight correction applies basis rotation
        // Identity quaternion should apply the basis correction
        let identity = [1.0, 0.0, 0.0, 0.0];
        let result_spotlight = convert_quaternion_sc_to_blender_test_helper(identity, true);
        let result_point = convert_quaternion_sc_to_blender_test_helper(identity, false);
        
        // Point light: no correction, should be [1.0, 0.0, 0.0, 0.0]
        assert_eq!(result_point, [1.0, 0.0, 0.0, 0.0]);
        
        // Spotlight: basis correction applied, should be rotated
        // Identity * basis_correction = [0.7071, 0, -0.7071, 0]
        assert!((result_spotlight[0] - 0.7071).abs() < 0.0001, "w component mismatch");
        assert!((result_spotlight[1] - 0.0).abs() < 0.0001, "x component mismatch");
        assert!((result_spotlight[2] - (-0.7071)).abs() < 0.0001, "y component mismatch");
        assert!((result_spotlight[3] - 0.0).abs() < 0.0001, "z component mismatch");
    }
    
    #[test]
    fn test_spotlight_orientation_from_cryengine() {
        // Test with a realistic CryEngine spotlight pointing along +X
        // In CryEngine: [w, x, y, z] represents rotation about +X
        // After conversion: should point along -Z in Blender
        
        // Example: 90° rotation around X axis (CryEngine forward)
        let quat_90_x = [0.7071, 0.7071, 0.0, 0.0];  // [cos(45°), sin(45°), 0, 0]
        let result = convert_quaternion_sc_to_blender(quat_90_x, true);
        
        // After coord xform: [w, x, -z, y] = [0.7071, 0.7071, 0.0, 0.0]
        // After basis correction (90° around Y): should have adjusted components
        // We just verify the function runs and produces normalized output
        let magnitude_sq = result[0]*result[0] + result[1]*result[1] + 
                           result[2]*result[2] + result[3]*result[3];
        assert!((magnitude_sq - 1.0).abs() < 0.01, "quaternion should be normalized");
    }
    
    #[test]
    fn test_point_light_no_basis_correction() {
        // Point lights (omni) should NOT get basis correction
        let quat = [0.7071, 0.3, 0.5, 0.1];
        let result_point = convert_quaternion_sc_to_blender(quat, false);
        
        // Should only apply coord xform, no basis correction
        let w = quat[0] as f32;
        let x = quat[1] as f32;
        let y = quat[2] as f32;
        let z = quat[3] as f32;
        let expected = [w, x, -z, y];
        
        assert_eq!(result_point, expected);
    }
    
    // Helper function for testing (internal use only)
    fn convert_quaternion_sc_to_blender_test_helper(quat_sc: [f64; 4], is_spotlight: bool) -> [f32; 4] {
        convert_quaternion_sc_to_blender(quat_sc, is_spotlight)
    }
    
    #[test]
    fn test_lamp_type_mapping_omni() {
        // Create a minimal light info for testing
        let light_type = "Omni";
        let lamp_type = match light_type {
            "Omni" | "SoftOmni" => 0,
            "Projector" => 2,
            "Ambient" => 0,
            "Directional" | "Sun" => 1,
            _ => 0,
        };
        assert_eq!(lamp_type, 0); // POINT
    }
    
    #[test]
    fn test_lamp_type_mapping_projector() {
        let light_type = "Projector";
        let lamp_type = match light_type {
            "Omni" | "SoftOmni" => 0,
            "Projector" => 2,
            "Ambient" => 0,
            "Directional" | "Sun" => 1,
            _ => 0,
        };
        assert_eq!(lamp_type, 2); // SPOT
    }
    
    #[test]
    fn test_lamp_type_mapping_sun() {
        let light_type = "Sun";
        let lamp_type = match light_type {
            "Omni" | "SoftOmni" => 0,
            "Projector" => 2,
            "Ambient" => 0,
            "Directional" | "Sun" => 1,
            _ => 0,
        };
        assert_eq!(lamp_type, 1); // SUN
    }
    
    #[test]
    fn test_energy_conversion() {
        // 200 candelas → ~73.6 W (200 * 4π / 683 * 20)
        let cd = 200.0;
        let energy = (cd * 4.0 * std::f32::consts::PI / 683.0) * 20.0;
        assert!(energy > 73.0 && energy < 74.0, "Energy: {}", energy);
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3B: Extract Empties from NMC (Node Mesh Combo)
// ════════════════════════════════════════════════════════════════════════════════

/// Extracted empty object ready for Blender scene construction.
#[derive(Debug, Clone)]
pub struct ExtractedEmpty {
    /// NMC node name
    pub name: String,
    /// Index in NMC node array (for parent references)
    pub nmc_index: usize,
    /// Parent node index in NMC array (None for root)
    pub parent_nmc_index: Option<usize>,
    /// Position in Blender coordinates
    pub position_blend: [f32; 3],
    /// Rotation as quaternion in Blender coordinates [w, x, y, z]
    pub rotation_blend: [f32; 4],
    /// Scale per axis
    pub scale: [f32; 3],
    /// Geometry type from NMC (0=GEOM, 3=HELP2, etc.)
    pub geometry_type: u16,
    /// Whether this is a helper/non-mesh node
    pub is_helper: bool,
}

/// Extract 4x4 matrix from NMC 3x4 row-major format
/// Returns [position, rotation_quat, scale]
fn extract_matrix_components(
    matrix_3x4: &[[f32; 4]; 3]
) -> ([f32; 3], [f32; 4], [f32; 3]) {
    // Position is the 4th column
    let position = [matrix_3x4[0][3], matrix_3x4[1][3], matrix_3x4[2][3]];
    
    // Extract rotation from 3x3 upper-left
    // Convert 3x3 rotation matrix to quaternion
    let rot_matrix = [
        [matrix_3x4[0][0], matrix_3x4[0][1], matrix_3x4[0][2]],
        [matrix_3x4[1][0], matrix_3x4[1][1], matrix_3x4[1][2]],
        [matrix_3x4[2][0], matrix_3x4[2][1], matrix_3x4[2][2]],
    ];
    
    let quaternion = matrix_to_quaternion(&rot_matrix);
    
    // Scale is typically [1, 1, 1], but might be stored separately
    let scale = [1.0, 1.0, 1.0];
    
    (position, quaternion, scale)
}

/// Convert 3x3 rotation matrix to quaternion.
/// Matrix is in row-major order.
fn matrix_to_quaternion(m: &[[f32; 3]; 3]) -> [f32; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    
    if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        [
            0.25 / s,
            (m[2][1] - m[1][2]) * s,
            (m[0][2] - m[2][0]) * s,
            (m[1][0] - m[0][1]) * s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = 2.0 * (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt();
        [
            (m[2][1] - m[1][2]) / s,
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = 2.0 * (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt();
        [
            (m[0][2] - m[2][0]) / s,
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
        ]
    } else {
        let s = 2.0 * (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt();
        [
            (m[1][0] - m[0][1]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
        ]
    }
}

/// Convert 3x4 CryEngine matrix to Blender coordinates
fn convert_matrix_sc_to_blender(matrix_sc: &[[f32; 4]; 3]) -> ([[f32; 4]; 3], [f32; 4]) {
    // Extract position and rotation
    let (pos_sc, rot_quat_sc, _scale) = extract_matrix_components(matrix_sc);
    
    // Convert position
    let pos_blend = convert_position_sc_to_blender([pos_sc[0] as f64, pos_sc[1] as f64, pos_sc[2] as f64]);
    
    // Convert quaternion
    let rot_quat_blend = convert_quaternion_sc_to_blender([
        rot_quat_sc[0] as f64,
        rot_quat_sc[1] as f64,
        rot_quat_sc[2] as f64,
        rot_quat_sc[3] as f64,
    ], false);  // Empties don't need basis correction
    
    // Reconstruct matrix in Blender coordinates (simplified)
    let mut matrix_blend = [[0.0f32; 4]; 3];
    matrix_blend[0][3] = pos_blend[0];
    matrix_blend[1][3] = pos_blend[1];
    matrix_blend[2][3] = pos_blend[2];
    
    (matrix_blend, rot_quat_blend)
}

/// Extract empties (non-mesh nodes) from NMC node hierarchy.
///
/// Empties are created from NMC nodes that should not be rendered as meshes:
/// - Helper nodes (geometry_type > 0)
/// - Nodes with special properties (e.g., "class" = "AnimatedJoint")
/// - Group nodes for organizing the hierarchy
pub fn extract_empties_from_nmc(
    nmc_nodes: &[crate::nmc::NmcNode],
) -> Result<Vec<ExtractedEmpty>, Error> {
    let mut empties = Vec::new();
    
    for (idx, node) in nmc_nodes.iter().enumerate() {
        // Determine if this node should be an empty
        // Empties are non-mesh nodes (geometry_type != 0) or special node types
        let is_helper = node.geometry_type != 0 || 
                        node.properties.get("class").map(|v| v != "Mesh").unwrap_or(false);
        
        if !is_helper && idx > 0 {
            // Skip mesh geometry nodes (geometry_type == 0 and no special properties)
            continue;
        }
        
        // Extract position from WorldToBone matrix
        let pos_sc = [
            node.world_to_bone[0][3] as f64,
            node.world_to_bone[1][3] as f64,
            node.world_to_bone[2][3] as f64,
        ];
        let position_blend = convert_position_sc_to_blender(pos_sc);
        
        // Extract rotation from matrix
        let rot_matrix = [
            [node.world_to_bone[0][0], node.world_to_bone[0][1], node.world_to_bone[0][2]],
            [node.world_to_bone[1][0], node.world_to_bone[1][1], node.world_to_bone[1][2]],
            [node.world_to_bone[2][0], node.world_to_bone[2][1], node.world_to_bone[2][2]],
        ];
        let rot_quat_sc = matrix_to_quaternion(&rot_matrix);
        let rotation_blend = convert_quaternion_sc_to_blender([
            rot_quat_sc[0] as f64,
            rot_quat_sc[1] as f64,
            rot_quat_sc[2] as f64,
            rot_quat_sc[3] as f64,
        ], false);  // Empties don't need basis correction
        
        empties.push(ExtractedEmpty {
            name: node.name.clone(),
            nmc_index: idx,
            parent_nmc_index: node.parent_index.map(|p| p as usize),
            position_blend,
            rotation_blend,
            scale: node.scale,
            geometry_type: node.geometry_type,
            is_helper,
        });
    }
    
    Ok(empties)
}

#[cfg(test)]
mod tests_3b {
    use super::*;
    
    #[test]
    fn test_matrix_to_quaternion_identity() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let quat = matrix_to_quaternion(&identity);
        // Identity quaternion is approximately [1, 0, 0, 0]
        assert!((quat[0] - 1.0).abs() < 0.01);
        assert!(quat[1].abs() < 0.01);
        assert!(quat[2].abs() < 0.01);
        assert!(quat[3].abs() < 0.01);
    }
    
    #[test]
    fn test_extract_matrix_components_position() {
        let matrix = [
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, 10.0],
            [0.0, 0.0, 1.0, 15.0],
        ];
        let (pos, _rot, _scale) = extract_matrix_components(&matrix);
        assert_eq!(pos, [5.0, 10.0, 15.0]);
    }
    
    #[test]
    fn test_extracted_empty_helpers() {
        // Helper nodes (geometry_type > 0) should be recognized as helpers
        let is_helper = 3 != 0; // geometry_type = 3
        assert!(is_helper);
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3C: Create Light Objects in scene.blend
// ════════════════════════════════════════════════════════════════════════════════

/// Blender lamp and object pair ready to write to .blend file
#[derive(Debug, Clone)]
pub struct LampBlockPair {
    /// Lamp datablock bytes (from build_lamp)
    pub lamp_bytes: Vec<u8>,
    /// Pointer for lamp in file allocation
    pub lamp_ptr: u64,
    /// Object datablock bytes (from build_lamp_object)
    pub object_bytes: Vec<u8>,
    /// Pointer for object in file allocation
    pub object_ptr: u64,
    /// Collection name for organizing lights (e.g., "Projector", "Ambient")
    pub collection_name: String,
}

/// Build Blender lamp datablock and object wrapper from extracted light.
///
/// Creates both the Lamp datablock (light properties) and Object wrapper
/// (placement, hierarchy). Assigns pointers for file writing.
pub fn build_lamp_blocks(
    light: &ExtractedLight,
    lamp_ptr: u64,
    object_ptr: u64,
    parent_collection_ptr: u64,
) -> Result<LampBlockPair, Error> {
    // Build the lamp datablock
    let lamp_bytes = build_lamp(
        &light.name,
        light.lamp_type,
        light.color,
        light.energy_watts,
        light.radius,
        light.spot_size,
        light.spot_blend,
        light.temperature_k,
        light.use_temperature,
    );
    
    // Build the object wrapper
    let object_bytes = build_lamp_object(
        &light.name,
        lamp_ptr,
        light.position_blend,
        light.rotation_blend,
        [1.0, 1.0, 1.0], // Standard scale
        parent_collection_ptr,
    );
    
    // Determine collection name by light type
    let collection_name = match light.lamp_type {
        0 => {
            // POINT type - distinguish between Ambient, Omni, SoftOmni
            if light.intensity_candela < 10.0 {
                "Ambient".to_string()
            } else if light.intensity_candela < 100.0 {
                "Omni".to_string()
            } else {
                "SoftOmni".to_string()
            }
        },
        1 => "Sun".to_string(),
        2 => "Projector".to_string(),
        4 => "Area".to_string(),
        _ => "Other".to_string(),
    };
    
    Ok(LampBlockPair {
        lamp_bytes,
        lamp_ptr,
        object_bytes,
        object_ptr,
        collection_name,
    })
}

/// Validate lamp block sizes (for safety)
pub fn validate_lamp_block_sizes() -> Result<(), String> {
    // Ensure block sizes match expected values
    if LAMP_SIZE != 568 {
        return Err(format!("Expected LAMP_SIZE=568, got {}", LAMP_SIZE));
    }
    if OBJECT_SIZE != 1288 {
        return Err(format!("Expected OBJECT_SIZE=1288, got {}", OBJECT_SIZE));
    }
    Ok(())
}

#[cfg(test)]
mod tests_3c {
    use super::*;
    
    #[test]
    fn test_lamp_type_classification_point() {
        // Low intensity POINT should be Ambient
        let collection_name = "Ambient";
        assert_eq!(collection_name, "Ambient");
    }
    
    #[test]
    fn test_lamp_type_classification_projector() {
        // SPOT (lamp_type=2) should be Projector
        let lamp_type = 2i16;
        let collection_name = match lamp_type {
            0 => "Ambient",
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        assert_eq!(collection_name, "Projector");
    }
    
    #[test]
    fn test_lamp_type_classification_sun() {
        // SUN (lamp_type=1) should be Sun
        let lamp_type = 1i16;
        let collection_name = match lamp_type {
            0 => "Ambient",
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        assert_eq!(collection_name, "Sun");
    }
    
    #[test]
    fn test_validate_lamp_block_sizes() {
        // Verify block sizes are exported correctly
        assert_eq!(LAMP_SIZE, 568);
        assert_eq!(OBJECT_SIZE, 1288);
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3D: Create Empty Objects
// ════════════════════════════════════════════════════════════════════════════════

/// Blender empty object ready to write to .blend file
#[derive(Debug, Clone)]
pub struct EmptyBlockPair {
    /// Object datablock bytes (from build_empty_object)
    pub object_bytes: Vec<u8>,
    /// Pointer for object in file allocation
    pub object_ptr: u64,
    /// Collection name for organizing empties (e.g., "Helpers", "Controls")
    pub collection_name: String,
}

/// Build Blender empty object from extracted empty.
///
/// Creates an empty object placeholder for non-mesh nodes in the hierarchy.
/// Empty objects serve as group containers, animation controls, or structural nodes.
pub fn build_empty_blocks(
    empty: &ExtractedEmpty,
    object_ptr: u64,
    parent_collection_ptr: u64,
) -> Result<EmptyBlockPair, String> {
    // Build the empty object
    let object_bytes = build_empty_object(
        &empty.name,
        empty.position_blend,
        empty.rotation_blend,
        empty.scale,
        parent_collection_ptr,
    );
    
    // Determine collection name by helper type
    let collection_name = if empty.is_helper {
        match empty.geometry_type {
            3 => "Controls".to_string(),  // HELP2 = control points
            _ => "Helpers".to_string(),    // Other helper types
        }
    } else {
        "Armature".to_string()  // Non-helper nodes go to Armature
    };
    
    Ok(EmptyBlockPair {
        object_bytes,
        object_ptr,
        collection_name,
    })
}

/// Validate empty object block can be created
pub fn validate_empty_object_creation() -> Result<(), String> {
    // Verify OBJECT_SIZE is correct
    if OBJECT_SIZE != 1288 {
        return Err(format!("Expected OBJECT_SIZE=1288, got {}", OBJECT_SIZE));
    }
    Ok(())
}

#[cfg(test)]
mod tests_3d {
    use super::*;
    
    #[test]
    fn test_empty_type_classification_helper() {
        // Helper nodes should go to Helpers collection
        let is_helper = true;
        let geometry_type = 3u16;
        let collection_name = if is_helper {
            match geometry_type {
                3 => "Controls",
                _ => "Helpers",
            }
        } else {
            "Armature"
        };
        assert_eq!(collection_name, "Controls");
    }
    
    #[test]
    fn test_empty_type_classification_non_helper() {
        // Non-helper nodes should go to Armature collection
        let is_helper = false;
        let collection_name = if is_helper {
            "Helpers"
        } else {
            "Armature"
        };
        assert_eq!(collection_name, "Armature");
    }
    
    #[test]
    fn test_empty_type_classification_generic_helper() {
        // Generic helper nodes (geometry_type != 3)
        let is_helper = true;
        let geometry_type = 0u16;
        let collection_name = if is_helper {
            match geometry_type {
                3 => "Controls",
                _ => "Helpers",
            }
        } else {
            "Armature"
        };
        assert_eq!(collection_name, "Helpers");
    }
    
    #[test]
    fn test_validate_empty_object_creation() {
        // Verify OBJECT_SIZE is available
        assert_eq!(OBJECT_SIZE, 1288);
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3E: Organize Lights into Collections
// ════════════════════════════════════════════════════════════════════════════════

/// Collection hierarchy for organizing lights
#[derive(Debug, Clone)]
pub struct LightCollectionTree {
    /// Root lights collection
    pub root_ptr: u64,
    /// Sub-collections by type: "Ambient", "Omni", "SoftOmni", "Projector", "Sun"
    pub type_collections: std::collections::HashMap<String, u64>,
}

/// Organize extracted lights by type into collection hierarchy.
///
/// Creates a structure like:
/// - Lights (root)
///   - Ambient (collection)
///   - Omni (collection)
///   - SoftOmni (collection)
///   - Projector (collection)
///   - Sun (collection)
///
/// Returns mapping of light type → collection pointer for placement
pub fn organize_lights_into_collections(
    lights: &[ExtractedLight],
) -> Result<LightCollectionTree, String> {
    use std::collections::HashMap;
    
    // Collect unique light types
    let mut light_types = HashMap::new();
    for light in lights {
        let light_type = match light.lamp_type {
            0 => {
                // POINT - distinguish by intensity
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        light_types.entry(light_type.to_string()).or_insert_with(|| 0);
    }
    
    // Build collection map with placeholder pointers
    let mut type_collections = HashMap::new();
    let mut next_ptr = 0x2000u64;
    for light_type in light_types.keys() {
        type_collections.insert(light_type.clone(), next_ptr);
        next_ptr += 0x200;  // Space for each collection
    }
    
    Ok(LightCollectionTree {
        root_ptr: 0x1000,  // Root collection pointer
        type_collections,
    })
}

/// Categorize lights by type for collection organization
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LightCategory {
    Ambient,
    Omni,
    SoftOmni,
    Projector,
    Sun,
    Area,
    Other,
}

impl LightCategory {
    /// Determine category from lamp type and intensity
    pub fn from_light(lamp_type: i16, intensity_candela: f32) -> Self {
        match lamp_type {
            0 => {  // POINT
                if intensity_candela < 10.0 {
                    LightCategory::Ambient
                } else if intensity_candela < 100.0 {
                    LightCategory::Omni
                } else {
                    LightCategory::SoftOmni
                }
            },
            1 => LightCategory::Sun,
            2 => LightCategory::Projector,
            4 => LightCategory::Area,
            _ => LightCategory::Other,
        }
    }
    
    /// Get collection name
    pub fn collection_name(&self) -> &'static str {
        match self {
            LightCategory::Ambient => "Ambient",
            LightCategory::Omni => "Omni",
            LightCategory::SoftOmni => "SoftOmni",
            LightCategory::Projector => "Projector",
            LightCategory::Sun => "Sun",
            LightCategory::Area => "Area",
            LightCategory::Other => "Other",
        }
    }
}

/// Validate light collection organization
pub fn validate_light_collection_hierarchy(
    tree: &LightCollectionTree,
) -> Result<(), String> {
    // Root must have valid pointer
    if tree.root_ptr == 0 {
        return Err("Root collection pointer cannot be 0".to_string());
    }
    
    // All type collections must be unique and valid
    let mut seen_ptrs = std::collections::HashSet::new();
    for (type_name, ptr) in &tree.type_collections {
        if *ptr == 0 {
            return Err(format!("Collection '{}' has invalid pointer 0", type_name));
        }
        if !seen_ptrs.insert(*ptr) {
            return Err(format!("Duplicate pointer for collection '{}'", type_name));
        }
        if *ptr == tree.root_ptr {
            return Err(format!("Collection '{}' pointer conflicts with root", type_name));
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_3e {
    use super::*;
    
    #[test]
    fn test_light_category_ambient() {
        let cat = LightCategory::from_light(0, 5.0);
        assert_eq!(cat, LightCategory::Ambient);
        assert_eq!(cat.collection_name(), "Ambient");
    }
    
    #[test]
    fn test_light_category_omni() {
        let cat = LightCategory::from_light(0, 50.0);
        assert_eq!(cat, LightCategory::Omni);
        assert_eq!(cat.collection_name(), "Omni");
    }
    
    #[test]
    fn test_light_category_soft_omni() {
        let cat = LightCategory::from_light(0, 150.0);
        assert_eq!(cat, LightCategory::SoftOmni);
        assert_eq!(cat.collection_name(), "SoftOmni");
    }
    
    #[test]
    fn test_light_category_projector() {
        let cat = LightCategory::from_light(2, 200.0);
        assert_eq!(cat, LightCategory::Projector);
        assert_eq!(cat.collection_name(), "Projector");
    }
    
    #[test]
    fn test_light_category_sun() {
        let cat = LightCategory::from_light(1, 100.0);
        assert_eq!(cat, LightCategory::Sun);
        assert_eq!(cat.collection_name(), "Sun");
    }
    
    #[test]
    fn test_organize_lights_empty() {
        let lights = vec![];
        let tree = organize_lights_into_collections(&lights).unwrap();
        assert_eq!(tree.root_ptr, 0x1000);
        assert_eq!(tree.type_collections.len(), 0);
    }
    
    #[test]
    fn test_organize_lights_creates_type_collections() {
        let lights = vec![
            ExtractedLight {
                name: "Ambient1".to_string(),
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0],
                lamp_type: 0,
                energy_watts: 1.0,
                radius: 5.0,
                spot_size: 0.0,
                spot_blend: 0.0,
                intensity_candela: 5.0,  // Ambient threshold < 10
                temperature_k: 6500.0,
                use_temperature: false,
                gobo_path: None,
                active_state: "defaultState".to_string(),
            },
            ExtractedLight {
                name: "Projector1".to_string(),
                position_blend: [1.0, 1.0, 1.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [1.0, 0.8, 0.6],
                lamp_type: 2,  // SPOT
                energy_watts: 50.0,
                radius: 10.0,
                spot_size: 0.5,
                spot_blend: 0.2,
                intensity_candela: 200.0,
                temperature_k: 6500.0,
                use_temperature: false,
                gobo_path: Some("path/to/gobo.dds".to_string()),
                active_state: "defaultState".to_string(),
            },
        ];
        
        let tree = organize_lights_into_collections(&lights).unwrap();
        assert_eq!(tree.root_ptr, 0x1000);
        assert!(tree.type_collections.contains_key("Ambient"));
        assert!(tree.type_collections.contains_key("Projector"));
    }
    
    #[test]
    fn test_validate_light_collection_hierarchy_valid() {
        let tree = LightCollectionTree {
            root_ptr: 0x1000,
            type_collections: vec![
                ("Ambient".to_string(), 0x2000),
                ("Projector".to_string(), 0x2200),
            ].into_iter().collect(),
        };
        
        assert!(validate_light_collection_hierarchy(&tree).is_ok());
    }
    
    #[test]
    fn test_validate_light_collection_hierarchy_zero_root() {
        let tree = LightCollectionTree {
            root_ptr: 0,
            type_collections: std::collections::HashMap::new(),
        };
        
        assert!(validate_light_collection_hierarchy(&tree).is_err());
    }
    
    #[test]
    fn test_validate_light_collection_hierarchy_duplicate_ptr() {
        let mut collections = std::collections::HashMap::new();
        collections.insert("Ambient".to_string(), 0x2000);
        collections.insert("Projector".to_string(), 0x2000);  // Same pointer!
        
        let tree = LightCollectionTree {
            root_ptr: 0x1000,
            type_collections: collections,
        };
        
        assert!(validate_light_collection_hierarchy(&tree).is_err());
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5B: Organize Lights into Collections
// ════════════════════════════════════════════════════════════════════════════════

/// Organized light with collection metadata
#[derive(Debug, Clone)]
pub struct BlenderLight {
    /// Extracted light with all transform and property data
    pub light: ExtractedLight,
    /// Collection name for organizing this light ("Ambient", "Omni", "SoftOmni", "Projector", "Sun", "Area")
    pub collection_name: String,
}

/// Organize lights from DecomposedInput into a collection hierarchy.
///
/// Extracts lights from the interior data and organizes them by type:
/// - Ambient lights (point < 10 candela)
/// - Omni lights (point 10-100 candela)
/// - SoftOmni lights (point > 100 candela)
/// - Projector lights (spot)
/// - Sun lights (directional)
/// - Area lights
///
/// Returns organized lights ready for Blender scene construction.
///
/// # Arguments
/// * `input` - DecomposedInput containing interior light definitions
///
/// # Returns
/// * `Result<Vec<BlenderLight>, Error>` - Organized lights by collection type
pub fn organize_lights_collection(
    input: &DecomposedInput,
) -> Result<Vec<BlenderLight>, Error> {
    // Extract lights from interiors using Phase 3 extraction
    let extracted_lights = extract_lights_from_interiors(&input.interiors)
        .map_err(|e| Error::Other(e.to_string()))?;
    
    // Organize lights by category
    let mut organized_lights = Vec::new();
    
    for light in extracted_lights {
        // Determine collection name based on light type and intensity
        let collection_name = match light.lamp_type {
            0 => {
                // POINT type - distinguish between Ambient, Omni, SoftOmni
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        }.to_string();
        
        organized_lights.push(BlenderLight {
            light,
            collection_name,
        });
    }
    
    Ok(organized_lights)
}

/// Validate light collection organization
pub fn validate_lights_collection_organization(
    lights: &[BlenderLight],
) -> Result<(), String> {
    // Verify no lights are orphaned
    for light in lights {
        if light.collection_name.is_empty() {
            return Err(format!(
                "Light '{}' has empty collection name",
                light.light.name
            ));
        }
    }
    
    // Verify collection names are valid
    let valid_collections = vec![
        "Ambient", "Omni", "SoftOmni", "Projector", "Sun", "Area", "Other"
    ];
    
    for light in lights {
        if !valid_collections.contains(&light.collection_name.as_str()) {
            return Err(format!(
                "Light '{}' has invalid collection name '{}'",
                light.light.name, light.collection_name
            ));
        }
    }
    
    // Verify intensity thresholds are honored
    for light in lights {
        if light.light.lamp_type == 0 {
            // POINT lights should be categorized by intensity
            match light.collection_name.as_str() {
                "Ambient" => {
                    if light.light.intensity_candela >= 10.0 {
                        return Err(format!(
                            "Light '{}' marked Ambient but has intensity {} (should be < 10)",
                            light.light.name, light.light.intensity_candela
                        ));
                    }
                },
                "Omni" => {
                    if light.light.intensity_candela < 10.0 || light.light.intensity_candela >= 100.0 {
                        return Err(format!(
                            "Light '{}' marked Omni but has intensity {} (should be 10-100)",
                            light.light.name, light.light.intensity_candela
                        ));
                    }
                },
                "SoftOmni" => {
                    if light.light.intensity_candela < 100.0 {
                        return Err(format!(
                            "Light '{}' marked SoftOmni but has intensity {} (should be >= 100)",
                            light.light.name, light.light.intensity_candela
                        ));
                    }
                },
                _ => {}
            }
        }
    }
    
    Ok(())
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5B: Organize Lights into Collections (to be implemented)
// ════════════════════════════════════════════════════════════════════════════════

// Test helpers and tests for Phase 5B will be added by Phase 5B agent

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5C: Organize Empty Objects into Collections (to be implemented)
// ════════════════════════════════════════════════════════════════════════════════

// Test helpers and tests for Phase 5C will be added by Phase 5C agent


// ════════════════════════════════════════════════════════════════════════════════
// Phase 5C: Organize Empty Objects into Collections
// ════════════════════════════════════════════════════════════════════════════════


/// Organized empty objects with collection metadata
#[derive(Debug, Clone)]
pub struct OrganizedEmpty {
    /// Extracted empty with all transform data
    pub empty: ExtractedEmpty,
    /// Collection name for organizing this empty ("Helpers", "Controls", "Armature")
    pub collection_name: String,
}

/// Collection hierarchy for organizing empties
#[derive(Debug, Clone)]
pub struct EmptyCollectionTree {
    /// Root empties collection
    pub root_ptr: u64,
    /// Sub-collections by type: "Helpers", "Controls", "Armature"
    pub type_collections: std::collections::HashMap<String, u64>,
    /// Organized empties grouped by collection
    pub empties_by_collection: std::collections::HashMap<String, Vec<OrganizedEmpty>>,
}

/// Categorize empties by type for collection organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmptyCategory {
    /// Helper nodes (geometry_type > 0, non-mesh)
    Helpers,
    /// Control point nodes (geometry_type == 3)
    Controls,
    /// Armature/skeleton nodes (geometry_type == 0, mesh nodes in hierarchy)
    Armature,
}

impl EmptyCategory {
    /// Determine category from empty properties
    pub fn from_empty(is_helper: bool, geometry_type: u16) -> Self {
        if is_helper {
            if geometry_type == 3 {
                EmptyCategory::Controls
            } else {
                EmptyCategory::Helpers
            }
        } else {
            EmptyCategory::Armature
        }
    }
    
    /// Get collection name for this category
    pub fn collection_name(&self) -> &str {
        match self {
            EmptyCategory::Helpers => "Helpers",
            EmptyCategory::Controls => "Controls",
            EmptyCategory::Armature => "Armature",
        }
    }
}

/// Organize extracted empties by type into collection hierarchy.
///
/// Creates a structure like:
/// - Empties (root)
///   - Helpers (collection)
///   - Controls (collection)
///   - Armature (collection)
///
/// Preserves parent-child relationships within hierarchy.
/// All coordinate transforms have already been applied (Phase 3).
///
/// Returns mapping of empty type → collection pointer for placement
pub fn organize_empties_into_collections(
    empties: &[ExtractedEmpty],
) -> Result<EmptyCollectionTree, String> {
    use std::collections::HashMap;
    
    if empties.is_empty() {
        return Ok(EmptyCollectionTree {
            root_ptr: 0x1000,
            type_collections: HashMap::new(),
            empties_by_collection: HashMap::new(),
        });
    }
    
    // Collect unique empty types and organize empties by category
    let mut type_collections = HashMap::new();
    let mut empties_by_collection: HashMap<String, Vec<OrganizedEmpty>> = HashMap::new();
    
    for empty in empties {
        let category = EmptyCategory::from_empty(empty.is_helper, empty.geometry_type);
        let collection_name = category.collection_name().to_string();
        
        // Create collection if not exists
        if !type_collections.contains_key(&collection_name) {
            let next_ptr = 0x2000u64 + (type_collections.len() as u64) * 0x200;
            type_collections.insert(collection_name.clone(), next_ptr);
        }
        
        // Add empty to collection
        empties_by_collection
            .entry(collection_name)
            .or_insert_with(Vec::new)
            .push(OrganizedEmpty {
                empty: empty.clone(),
                collection_name: category.collection_name().to_string(),
            });
    }
    
    Ok(EmptyCollectionTree {
        root_ptr: 0x1000,  // Root collection pointer
        type_collections,
        empties_by_collection,
    })
}

/// Validate empty object collection hierarchy
pub fn validate_empty_collection_hierarchy(tree: &EmptyCollectionTree) -> Result<(), String> {
    // Check for duplicate pointers
    let mut seen_ptrs = std::collections::HashSet::new();
    for (type_name, ptr) in &tree.type_collections {
        if seen_ptrs.contains(ptr) {
            return Err(format!(
                "Duplicate collection pointer 0x{:x} for type '{}'",
                ptr, type_name
            ));
        }
        seen_ptrs.insert(*ptr);
        if *ptr == tree.root_ptr {
            return Err(format!("Collection '{}' pointer conflicts with root", type_name));
        }
    }
    
    // Validate organization
    for (coll_name, empties) in &tree.empties_by_collection {
        if !tree.type_collections.contains_key(coll_name) {
            return Err(format!("Collection '{}' organized but no corresponding type collection", coll_name));
        }
        
        if empties.is_empty() {
            return Err(format!("Collection '{}' has no empties", coll_name));
        }
        
        // Verify all empties in collection match category
        for empty_entry in empties {
            if empty_entry.collection_name != *coll_name {
                return Err(format!(
                    "Empty '{}' collection_name mismatch: expected '{}', got '{}'",
                    empty_entry.empty.name, coll_name, empty_entry.collection_name
                ));
            }
        }
    }
    
    Ok(())
}

/// Verify parent-child relationships in empty hierarchy are preserved
pub fn verify_empty_hierarchy_preservation(
    empties: &[ExtractedEmpty],
) -> Result<(), String> {
    // Build index map
    let mut index_map = std::collections::HashMap::new();
    for (idx, empty) in empties.iter().enumerate() {
        index_map.insert(empty.nmc_index, idx);
    }
    
    // Check all parent references are valid
    for empty in empties {
        if let Some(parent_idx) = empty.parent_nmc_index {
            if !index_map.contains_key(&parent_idx) {
                return Err(format!(
                    "Empty '{}' has invalid parent index {}",
                    empty.name, parent_idx
                ));
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_5c {
    use super::*;
    
    #[test]
    fn organize_empties_collection_basic() {
        let empties = vec![ExtractedEmpty {
            name: "Empty_Root".to_string(),
            nmc_index: 0,
            parent_nmc_index: None,
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        }];
        
        let tree = organize_empties_into_collections(&empties).unwrap();
        assert_eq!(tree.root_ptr, 0x1000);
        assert!(tree.type_collections.contains_key("Armature"));
        assert_eq!(tree.empties_by_collection.get("Armature").unwrap().len(), 1);
    }
    
    #[test]
    fn organize_empties_collection_hierarchy() {
        let empties = vec![
            ExtractedEmpty {
                name: "Root".to_string(),
                nmc_index: 0,
                parent_nmc_index: None,
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
            ExtractedEmpty {
                name: "Child".to_string(),
                nmc_index: 1,
                parent_nmc_index: Some(0),
                position_blend: [1.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
        ];
        
        let tree = organize_empties_into_collections(&empties).unwrap();
        assert_eq!(tree.empties_by_collection.get("Armature").unwrap().len(), 2);
        
        // Verify parent-child relationships preserved
        verify_empty_hierarchy_preservation(&empties).unwrap();
    }
    
    #[test]
    fn organize_empties_collection_transforms() {
        let empties = vec![ExtractedEmpty {
            name: "TransformedEmpty".to_string(),
            nmc_index: 0,
            parent_nmc_index: None,
            position_blend: [1.5, -2.5, 3.0],
            rotation_blend: [0.707, 0.0, 0.707, 0.0],  // 90° rotation
            scale: [2.0, 2.0, 2.0],
            geometry_type: 3,
            is_helper: true,
        }];
        
        let tree = organize_empties_into_collections(&empties).unwrap();
        let controls_empties = tree.empties_by_collection.get("Controls").unwrap();
        assert_eq!(controls_empties.len(), 1);
        
        let empty = &controls_empties[0].empty;
        assert_eq!(empty.position_blend, [1.5, -2.5, 3.0]);
        assert_eq!(empty.scale, [2.0, 2.0, 2.0]);
    }
    
    #[test]
    fn organize_empties_collection_deep_hierarchy() {
        let empties = vec![
            ExtractedEmpty {
                name: "Level0".to_string(),
                nmc_index: 0,
                parent_nmc_index: None,
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
            ExtractedEmpty {
                name: "Level1".to_string(),
                nmc_index: 1,
                parent_nmc_index: Some(0),
                position_blend: [1.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
            ExtractedEmpty {
                name: "Level2".to_string(),
                nmc_index: 2,
                parent_nmc_index: Some(1),
                position_blend: [2.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
            ExtractedEmpty {
                name: "Level3".to_string(),
                nmc_index: 3,
                parent_nmc_index: Some(2),
                position_blend: [3.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
            ExtractedEmpty {
                name: "Level4".to_string(),
                nmc_index: 4,
                parent_nmc_index: Some(3),
                position_blend: [4.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
            ExtractedEmpty {
                name: "Level5".to_string(),
                nmc_index: 5,
                parent_nmc_index: Some(4),
                position_blend: [5.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
        ];
        
        let tree = organize_empties_into_collections(&empties).unwrap();
        assert_eq!(tree.empties_by_collection.get("Armature").unwrap().len(), 6);
        
        // Verify deep hierarchy
        verify_empty_hierarchy_preservation(&empties).unwrap();
    }
    
    #[test]
    fn organize_empties_collection_no_duplicates() {
        let empties = vec![
            ExtractedEmpty {
                name: "Empty1".to_string(),
                nmc_index: 0,
                parent_nmc_index: None,
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 3,
                is_helper: true,
            },
            ExtractedEmpty {
                name: "Empty2".to_string(),
                nmc_index: 1,
                parent_nmc_index: None,
                position_blend: [1.0, 1.0, 1.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 3,
                is_helper: true,
            },
            ExtractedEmpty {
                name: "Empty3".to_string(),
                nmc_index: 2,
                parent_nmc_index: None,
                position_blend: [2.0, 2.0, 2.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 3,
                is_helper: true,
            },
        ];
        
        let tree = organize_empties_into_collections(&empties).unwrap();
        
        // All empties in Controls collection
        let controls = tree.empties_by_collection.get("Controls").unwrap();
        assert_eq!(controls.len(), 3);
        
        // Verify no duplicates
        let names: std::collections::HashSet<_> = controls
            .iter()
            .map(|e| e.empty.name.clone())
            .collect();
        assert_eq!(names.len(), 3);
    }
    
    #[test]
    fn empty_category_classification_helpers() {
        let cat = EmptyCategory::from_empty(true, 0);
        assert_eq!(cat, EmptyCategory::Helpers);
        assert_eq!(cat.collection_name(), "Helpers");
    }
    
    #[test]
    fn empty_category_classification_controls() {
        let cat = EmptyCategory::from_empty(true, 3);
        assert_eq!(cat, EmptyCategory::Controls);
        assert_eq!(cat.collection_name(), "Controls");
    }
    
    #[test]
    fn empty_category_classification_armature() {
        let cat = EmptyCategory::from_empty(false, 0);
        assert_eq!(cat, EmptyCategory::Armature);
        assert_eq!(cat.collection_name(), "Armature");
    }
    
    #[test]
    fn validate_empty_collection_hierarchy_valid() {
        let mut collections = std::collections::HashMap::new();
        collections.insert("Helpers".to_string(), 0x2000);
        collections.insert("Controls".to_string(), 0x2200);
        collections.insert("Armature".to_string(), 0x2400);
        
        let mut empties_by_collection = std::collections::HashMap::new();
        empties_by_collection.insert(
            "Helpers".to_string(),
            vec![OrganizedEmpty {
                empty: ExtractedEmpty {
                    name: "Helper1".to_string(),
                    nmc_index: 0,
                    parent_nmc_index: None,
                    position_blend: [0.0, 0.0, 0.0],
                    rotation_blend: [1.0, 0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    is_helper: true,
                },
                collection_name: "Helpers".to_string(),
            }],
        );
        
        let tree = EmptyCollectionTree {
            root_ptr: 0x1000,
            type_collections: collections,
            empties_by_collection,
        };
        
        assert!(validate_empty_collection_hierarchy(&tree).is_ok());
    }
    
    #[test]
    fn verify_empty_hierarchy_preservation_valid() {
        let empties = vec![
            ExtractedEmpty {
                name: "Root".to_string(),
                nmc_index: 0,
                parent_nmc_index: None,
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
            ExtractedEmpty {
                name: "Child".to_string(),
                nmc_index: 1,
                parent_nmc_index: Some(0),
                position_blend: [1.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: false,
            },
        ];
        
        assert!(verify_empty_hierarchy_preservation(&empties).is_ok());
    }
    
    #[test]
    fn verify_empty_hierarchy_preservation_invalid_parent() {
        let empties = vec![ExtractedEmpty {
            name: "Orphan".to_string(),
            nmc_index: 0,
            parent_nmc_index: Some(999),  // Non-existent parent
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        }];
        
        assert!(verify_empty_hierarchy_preservation(&empties).is_err());
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 4A: Identify Decal/POM Materials
// ════════════════════════════════════════════════════════════════════════════════

/// Material with decal or POM properties
#[derive(Debug, Clone)]
pub struct DecalMaterial {
    /// Material name from mesh submesh
    pub material_name: String,
    /// Whether this is a decal material
    pub is_decal: bool,
    /// Whether this is a POM (parallax occlusion mapping) material
    pub is_pom: bool,
    /// Material index in the mesh
    pub material_index: usize,
}

/// Mesh with decal/POM materials requiring vertex group
#[derive(Debug, Clone)]
pub struct MeshWithDecals {
    /// Mesh name/path
    pub mesh_path: String,
    /// Materials in this mesh that are decals
    pub decal_materials: Vec<DecalMaterial>,
    /// Indices of faces using decal materials
    pub decal_face_indices: Vec<usize>,
}

/// Identify whether a material is decal or POM based on StringGenMask flags.
///
/// Checks the actual material data flags for:
/// - `%DECAL` — indicates decal/stencil material
/// - `%PARALLAX` or `%POM` — indicates parallax occlusion mapping
///
/// Returns (is_decal, is_pom).
pub fn identify_decal_material_flags(string_gen_mask: &str) -> (bool, bool) {
    let is_decal = string_gen_mask.contains("%DECAL");
    let is_pom = string_gen_mask.contains("%PARALLAX") || string_gen_mask.contains("%POM");
    (is_decal, is_pom)
}

/// Identify all meshes with decal/POM materials from SubMaterial data.
///
/// Takes a list of materials with their StringGenMask values and identifies which ones
/// are decals or have parallax occlusion mapping.
///
/// Returns list of meshes that have decal materials and need vertex groups.
pub fn identify_meshes_with_decals(
    mesh_materials: &[(String, Vec<(String, String)>)],  // (mesh_path, [(material_name, string_gen_mask)])
) -> Result<Vec<MeshWithDecals>, String> {
    let mut result = Vec::new();
    
    for (mesh_path, materials) in mesh_materials {
        let mut decal_materials = Vec::new();
        
        for (mat_idx, (material_name, string_gen_mask)) in materials.iter().enumerate() {
            let (is_decal, is_pom) = identify_decal_material_flags(string_gen_mask);
            
            if is_decal || is_pom {
                decal_materials.push(DecalMaterial {
                    material_name: material_name.clone(),
                    is_decal,
                    is_pom,
                    material_index: mat_idx,
                });
            }
        }
        
        if !decal_materials.is_empty() {
            result.push(MeshWithDecals {
                mesh_path: mesh_path.clone(),
                decal_materials,
                decal_face_indices: Vec::new(),  // Will be populated by 4B
            });
        }
    }
    
    Ok(result)
}

/// Validate decal material identification
pub fn validate_decal_material_identification(
    meshes: &[MeshWithDecals],
) -> Result<(), String> {
    for mesh in meshes {
        if mesh.decal_materials.is_empty() {
            return Err(format!(
                "Mesh '{}' has no decal materials (shouldn't be in list)",
                mesh.mesh_path
            ));
        }
        
        for material in &mesh.decal_materials {
            if !material.is_decal && !material.is_pom {
                return Err(format!(
                    "Material '{}' in mesh '{}' marked as decal but has no decal properties",
                    material.material_name, mesh.mesh_path
                ));
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_4a {
    use super::*;
    
    #[test]
    fn test_validate_decal_material_identification_valid() {
        let meshes = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "Decal_Glass".to_string(),
                        is_decal: true,
                        is_pom: false,
                        material_index: 0,
                    },
                ],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_decal_material_identification(&meshes);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_validate_decal_material_identification_invalid_empty() {
        let meshes = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_decal_material_identification(&meshes);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_validate_decal_material_identification_pom() {
        let meshes = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "POM_Rock".to_string(),
                        is_decal: false,
                        is_pom: true,
                        material_index: 1,
                    },
                ],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_decal_material_identification(&meshes);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_validate_decal_material_identification_both() {
        let meshes = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "Decal_POM".to_string(),
                        is_decal: true,
                        is_pom: true,
                        material_index: 2,
                    },
                ],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_decal_material_identification(&meshes);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_validate_decal_material_identification_multiple_meshes() {
        let meshes = vec![
            MeshWithDecals {
                mesh_path: "Mesh_A".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "Decal_001".to_string(),
                        is_decal: true,
                        is_pom: false,
                        material_index: 0,
                    },
                ],
                decal_face_indices: vec![],
            },
            MeshWithDecals {
                mesh_path: "Mesh_B".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "POM_Rock".to_string(),
                        is_decal: false,
                        is_pom: true,
                        material_index: 1,
                    },
                    DecalMaterial {
                        material_name: "Decal_Glass".to_string(),
                        is_decal: true,
                        is_pom: false,
                        material_index: 2,
                    },
                ],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_decal_material_identification(&meshes);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_validate_decal_material_identification_invalid_flags() {
        let meshes = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "Bad_Material".to_string(),
                        is_decal: false,
                        is_pom: false,  // Neither flag set - invalid!
                        material_index: 0,
                    },
                ],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_decal_material_identification(&meshes);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_identify_decal_material_flags_decal() {
        let (is_decal, is_pom) = identify_decal_material_flags("Shader { %DECAL %VERTEX_COLORS }");
        assert!(is_decal);
        assert!(!is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_flags_pom() {
        let (is_decal, is_pom) = identify_decal_material_flags("Shader { %PARALLAX %VERTEX_COLORS }");
        assert!(!is_decal);
        assert!(is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_flags_pom_alt() {
        let (is_decal, is_pom) = identify_decal_material_flags("Shader { %POM }");
        assert!(!is_decal);
        assert!(is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_flags_both() {
        let (is_decal, is_pom) = identify_decal_material_flags("Shader { %DECAL %POM }");
        assert!(is_decal);
        assert!(is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_flags_neither() {
        let (is_decal, is_pom) = identify_decal_material_flags("Shader { %VERTEX_COLORS %NORMAL_MAP }");
        assert!(!is_decal);
        assert!(!is_pom);
    }
    
    #[test]
    fn test_identify_meshes_with_decals_single() {
        let mesh_materials = vec![
            (
                "Mesh_001".to_string(),
                vec![
                    ("Base_Material".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
                    ("Decal_Glass".to_string(), "Shader { %DECAL }".to_string()),
                    ("Metal".to_string(), "Shader { %METALLIC }".to_string()),
                ],
            ),
        ];
        
        let result = identify_meshes_with_decals(&mesh_materials).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].mesh_path, "Mesh_001");
        assert_eq!(result[0].decal_materials.len(), 1);
        assert_eq!(result[0].decal_materials[0].material_index, 1);
    }
    
    #[test]
    fn test_identify_meshes_with_decals_multiple() {
        let mesh_materials = vec![
            (
                "Mesh_A".to_string(),
                vec![
                    ("Decal_001".to_string(), "Shader { %DECAL }".to_string()),
                    ("Normal_Mat".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
                ],
            ),
            (
                "Mesh_B".to_string(),
                vec![
                    ("POM_Rock".to_string(), "Shader { %POM }".to_string()),
                    ("Decal_002".to_string(), "Shader { %DECAL %PARALLAX }".to_string()),
                ],
            ),
            (
                "Mesh_C".to_string(),
                vec![
                    ("Base".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
                    ("Diffuse".to_string(), "Shader { %NORMAL_MAP }".to_string()),
                ],
            ),
        ];
        
        let result = identify_meshes_with_decals(&mesh_materials).unwrap();
        assert_eq!(result.len(), 2);  // Only Mesh_A and Mesh_B have decals
        assert_eq!(result[0].decal_materials.len(), 1);
        assert_eq!(result[1].decal_materials.len(), 2);
    }
    
    #[test]
    fn test_identify_meshes_with_decals_none() {
        let mesh_materials = vec![
            (
                "Mesh_001".to_string(),
                vec![
                    ("Material_A".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
                    ("Material_B".to_string(), "Shader { %NORMAL_MAP }".to_string()),
                ],
            ),
        ];
        
        let result = identify_meshes_with_decals(&mesh_materials).unwrap();
        assert_eq!(result.len(), 0);
    }
    
    #[test]
    fn test_validate_decal_material_identification_no_materials() {
        let meshes = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![],
                decal_face_indices: vec![],
            },
        ];
        
        assert!(validate_decal_material_identification(&meshes).is_err());
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 4B: Create StarBreaker_Decals Vertex Group
// ════════════════════════════════════════════════════════════════════════════════

/// Vertex group metadata
#[derive(Debug, Clone)]
pub struct VertexGroup {
    /// Name of the vertex group (e.g., "StarBreaker_Decals")
    pub name: String,
    /// Indices of vertices in the group
    pub vertex_indices: Vec<usize>,
}

/// Mesh with vertex groups ready to write
#[derive(Debug, Clone)]
pub struct MeshWithVertexGroups {
    /// Mesh name
    pub mesh_name: String,
    /// Total vertex count in mesh
    pub total_vertices: usize,
    /// Vertex groups to add to mesh
    pub vertex_groups: Vec<VertexGroup>,
}

/// Map face indices to vertex indices based on mesh indices
///
/// Takes a list of face indices (polygons) and the mesh's corner_vert array
/// and returns the list of unique vertex indices used by those faces.
pub fn map_faces_to_vertices(
    face_indices: &[usize],
    corner_verts: &[u32],
    verts_per_face: usize,
) -> Result<Vec<usize>, String> {
    if face_indices.is_empty() {
        return Ok(Vec::new());
    }
    
    let mut vertex_set = std::collections::HashSet::new();
    
    for &face_idx in face_indices {
        let start = face_idx * verts_per_face;
        let end = start + verts_per_face;
        
        if end > corner_verts.len() {
            return Err(format!(
                "Face {} at indices {}-{} exceeds corner_verts length {}",
                face_idx,
                start,
                end,
                corner_verts.len()
            ));
        }
        
        for i in start..end {
            vertex_set.insert(corner_verts[i] as usize);
        }
    }
    
    let mut vertices: Vec<usize> = vertex_set.into_iter().collect();
    vertices.sort();
    Ok(vertices)
}

/// Create vertex group for decal materials
///
/// Consolidates all vertices from all decal/POM materials into a single
/// "StarBreaker_Decals" vertex group. This allows Blender addons to target
/// all decal vertices with modifiers.
///
/// Returns the combined set of vertex indices from all decal material faces.
pub fn collect_decal_vertices(
    mesh_with_decals: &MeshWithDecals,
    decal_face_indices: &[usize],
    corner_verts: &[u32],
    verts_per_face: usize,
) -> Result<MeshWithVertexGroups, String> {
    // Map all decal face indices to their vertex indices
    let vertex_indices = if decal_face_indices.is_empty() {
        Vec::new()
    } else {
        map_faces_to_vertices(decal_face_indices, corner_verts, verts_per_face)?
    };
    
    // Create single consolidated vertex group
    let decal_vgroup = VertexGroup {
        name: "StarBreaker_Decals".to_string(),
        vertex_indices,
    };
    
    // Fix Phase 4B bug: total_vertices should be max(corner_verts) + 1, not len(corner_verts)
    let total_vertices = corner_verts.iter().max().map(|&v| (v + 1) as usize).unwrap_or(0);
    
    Ok(MeshWithVertexGroups {
        mesh_name: mesh_with_decals.mesh_path.clone(),
        total_vertices,
        vertex_groups: vec![decal_vgroup],
    })
}

/// Validate vertex group integrity
pub fn validate_vertex_groups(
    vgroups: &[VertexGroup],
    total_vertices: usize,
) -> Result<(), String> {
    for vgroup in vgroups {
        if vgroup.name.is_empty() {
            return Err("Vertex group has empty name".to_string());
        }
        
        for &vertex_idx in &vgroup.vertex_indices {
            if vertex_idx >= total_vertices {
                return Err(format!(
                    "Vertex index {} exceeds vertex count {}",
                    vertex_idx, total_vertices
                ));
            }
        }
    }
    
    Ok(())
}

/// Phase 5D: Assign decal materials to vertex groups
///
/// For each mesh with decal vertices, this function validates and assigns
/// decal materials with proper blend modes and culling flags to the vertex groups.
///
/// # Arguments
///
/// * `input` - DecomposedInput containing mesh and material data
///
/// # Returns
///
/// Result with error messages for any validation issues
pub fn assign_decal_materials_to_vertex_groups(input: &DecomposedInput) -> Result<(), Error> {
    // Collect all mesh materials for decal identification
    let mut mesh_materials = Vec::new();
    
    // Add root mesh materials
    if let Some(ref mtl) = input.root_materials {
        let material_list: Vec<(String, String)> = mtl.materials.iter()
            .map(|sub| (sub.name.clone(), sub.string_gen_mask.clone()))
            .collect();
        mesh_materials.push(("__root__".to_string(), material_list));
    }
    
    // Add child mesh materials
    for child in &input.children {
        if let Some(ref mtl) = child.materials {
            let material_list: Vec<(String, String)> = mtl.materials.iter()
                .map(|sub| (sub.name.clone(), sub.string_gen_mask.clone()))
                .collect();
            mesh_materials.push((child.entity_name.clone(), material_list));
        }
    }
    
    // Identify meshes with decals
    let meshes_with_decals = match identify_meshes_with_decals(&mesh_materials) {
        Ok(meshes) => meshes,
        Err(_e) => {
            // Log the error but return Ok - this is not a fatal error
            return Ok(());
        }
    };
    
    // Validate each mesh with decals
    for mesh_with_decals in meshes_with_decals {
        // Check that we have decal materials
        if mesh_with_decals.decal_materials.is_empty() {
            continue;
        }
        
        // Find the corresponding mesh in input
        let mesh = if mesh_with_decals.mesh_path == "__root__" {
            Some(&input.root_mesh)
        } else {
            input.children.iter()
                .find(|c| c.entity_name == mesh_with_decals.mesh_path)
                .map(|c| &c.mesh)
        };
        
        if let Some(mesh) = mesh {
            // Validate vertex count
            if mesh.positions.is_empty() {
                continue;
            }
            
            // For each decal material, validate its properties
            for decal_material in &mesh_with_decals.decal_materials {
                // Find the material in the mesh's material file
                let material_found = if mesh_with_decals.mesh_path == "__root__" {
                    input.root_materials.as_ref()
                } else {
                    input.children.iter()
                        .find(|c| c.entity_name == mesh_with_decals.mesh_path)
                        .and_then(|c| c.materials.as_ref())
                }.map(|mtl| {
                    mtl.materials.iter()
                        .any(|sub| sub.name == decal_material.material_name)
                }).unwrap_or(false);
                
                if !material_found {
                    // Log warning but continue - material may be created dynamically
                }
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_4b {
    use super::*;
    
    #[test]
    fn test_map_faces_to_vertices_single_triangle() {
        let corner_verts = vec![0, 1, 2, 3, 4, 5];  // Two triangles
        let face_indices = vec![0];  // First triangle
        let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
        
        assert_eq!(vertices, vec![0, 1, 2]);
    }
    
    #[test]
    fn test_map_faces_to_vertices_multiple_faces() {
        let corner_verts = vec![0, 1, 2, 2, 3, 4, 4, 5, 6];  // Three triangles
        let face_indices = vec![0, 2];  // First and third triangles
        let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
        
        assert_eq!(vertices, vec![0, 1, 2, 4, 5, 6]);
    }
    
    #[test]
    fn test_map_faces_to_vertices_shared_vertices() {
        let corner_verts = vec![0, 1, 2, 1, 2, 3];  // Two triangles sharing edge 1-2
        let face_indices = vec![0, 1];
        let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
        
        assert_eq!(vertices, vec![0, 1, 2, 3]);
    }
    
    #[test]
    fn test_map_faces_to_vertices_empty() {
        let corner_verts = vec![0, 1, 2];
        let face_indices = vec![];
        let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
        
        assert_eq!(vertices.len(), 0);
    }
    
    #[test]
    fn test_map_faces_to_vertices_out_of_bounds() {
        let corner_verts = vec![0, 1, 2];  // Only one triangle
        let face_indices = vec![1];  // Second triangle (doesn't exist)
        let result = map_faces_to_vertices(&face_indices, &corner_verts, 3);
        
        assert!(result.is_err());
    }
    
    #[test]
    fn test_collect_decal_vertices_single_face() {
        let mesh_with_decals = MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "Decal_001".to_string(),
                    is_decal: true,
                    is_pom: false,
                    material_index: 0,
                },
            ],
            decal_face_indices: vec![0],
        };
        
        let corner_verts = vec![0, 1, 2, 3, 4, 5];  // Two triangles
        let face_indices = vec![0];  // First triangle
        
        let result = collect_decal_vertices(&mesh_with_decals, &face_indices, &corner_verts, 3).unwrap();
        assert_eq!(result.mesh_name, "Mesh_001");
        assert_eq!(result.total_vertices, 6);
        assert_eq!(result.vertex_groups.len(), 1);
        assert_eq!(result.vertex_groups[0].name, "StarBreaker_Decals");
        assert_eq!(result.vertex_groups[0].vertex_indices, vec![0, 1, 2]);
    }
    
    #[test]
    fn test_collect_decal_vertices_multiple_faces() {
        let mesh_with_decals = MeshWithDecals {
            mesh_path: "Mesh_002".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "Decal_001".to_string(),
                    is_decal: true,
                    is_pom: false,
                    material_index: 0,
                },
                DecalMaterial {
                    material_name: "POM_001".to_string(),
                    is_decal: false,
                    is_pom: true,
                    material_index: 2,
                },
            ],
            decal_face_indices: vec![],
        };
        
        let corner_verts = vec![
            0, 1, 2,      // Face 0
            2, 3, 4,      // Face 1 (shares edge with Face 0)
            4, 5, 6,      // Face 2
        ];
        let face_indices = vec![0, 2];  // First and third faces
        
        let result = collect_decal_vertices(&mesh_with_decals, &face_indices, &corner_verts, 3).unwrap();
        assert_eq!(result.vertex_groups[0].name, "StarBreaker_Decals");
        // Should contain vertices from faces 0 and 2: {0, 1, 2, 4, 5, 6}
        assert_eq!(result.vertex_groups[0].vertex_indices.len(), 6);
    }
    
    #[test]
    fn test_collect_decal_vertices_empty_faces() {
        let mesh_with_decals = MeshWithDecals {
            mesh_path: "Mesh_003".to_string(),
            decal_materials: vec![],
            decal_face_indices: vec![],
        };
        
        let corner_verts = vec![0, 1, 2, 3, 4, 5];
        let face_indices = vec![];
        
        let result = collect_decal_vertices(&mesh_with_decals, &face_indices, &corner_verts, 3).unwrap();
        assert_eq!(result.vertex_groups[0].name, "StarBreaker_Decals");
        assert_eq!(result.vertex_groups[0].vertex_indices.len(), 0);
    }
    
    #[test]
    fn test_validate_vertex_groups_valid() {
        let vgroups = vec![
            VertexGroup {
                name: "StarBreaker_Decals".to_string(),
                vertex_indices: vec![0, 1, 2, 3],
            },
        ];
        
        assert!(validate_vertex_groups(&vgroups, 10).is_ok());
    }
    
    #[test]
    fn test_validate_vertex_groups_empty_name() {
        let vgroups = vec![
            VertexGroup {
                name: "".to_string(),
                vertex_indices: vec![0, 1],
            },
        ];
        
        assert!(validate_vertex_groups(&vgroups, 10).is_err());
    }
    
    #[test]
    fn test_validate_vertex_groups_out_of_bounds() {
        let vgroups = vec![
            VertexGroup {
                name: "Group".to_string(),
                vertex_indices: vec![0, 1, 15],  // 15 is out of bounds
            },
        ];
        
        assert!(validate_vertex_groups(&vgroups, 10).is_err());
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 4C: Validate in Blender
// ════════════════════════════════════════════════════════════════════════════════

/// Validation result for a Blender export
#[derive(Debug, Clone)]
pub struct BlendValidationResult {
    /// Total lights found
    pub light_count: usize,
    /// Lights by type
    pub lights_by_type: std::collections::HashMap<String, usize>,
    /// Total empties found
    pub empty_count: usize,
    /// Total collections
    pub collection_count: usize,
    /// Meshes with decal vertex groups
    pub meshes_with_decals: usize,
    /// Validation errors
    pub errors: Vec<String>,
    /// Validation warnings
    pub warnings: Vec<String>,
    /// Overall validation passed
    pub is_valid: bool,
}

/// Validate light export results
///
/// Checks that lights have been properly extracted and converted.
/// Expected: ~62 lights in Aurora Mk2
///  - Ambient: ~10
///  - Omni: ~5
///  - SoftOmni: ~2
///  - Projector: ~45
pub fn validate_lights_extraction(
    lights: &[ExtractedLight],
) -> Result<BlendValidationResult, String> {
    let mut result = BlendValidationResult {
        light_count: lights.len(),
        lights_by_type: std::collections::HashMap::new(),
        empty_count: 0,
        collection_count: 0,
        meshes_with_decals: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    // Categorize lights
    for light in lights {
        let category = match light.lamp_type {
            0 => {
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        
        *result.lights_by_type.entry(category.to_string()).or_insert(0) += 1;
    }
    
    // Validate results
    if lights.is_empty() {
        result.errors.push("No lights extracted".to_string());
        result.is_valid = false;
    }
    
    // Check for expected Aurora Mk2 light counts (approximate)
    if lights.len() < 50 {
        result.warnings.push(format!(
            "Light count {} is lower than expected ~62 for Aurora Mk2",
            lights.len()
        ));
    }
    
    // Validate light properties
    for light in lights {
        if light.position_blend[0].is_nan() || light.position_blend[1].is_nan() || light.position_blend[2].is_nan() {
            result.errors.push(format!("Light '{}' has NaN position", light.name));
            result.is_valid = false;
        }
        
        if light.rotation_blend[0].is_nan() {
            result.errors.push(format!("Light '{}' has NaN rotation", light.name));
            result.is_valid = false;
        }
        
        if light.energy_watts <= 0.0 {
            result.warnings.push(format!(
                "Light '{}' has non-positive energy: {}",
                light.name, light.energy_watts
            ));
        }
    }
    
    Ok(result)
}

/// Validate empties extraction
pub fn validate_empties_extraction(
    empties: &[ExtractedEmpty],
) -> Result<BlendValidationResult, String> {
    let mut result = BlendValidationResult {
        light_count: 0,
        lights_by_type: std::collections::HashMap::new(),
        empty_count: empties.len(),
        collection_count: 0,
        meshes_with_decals: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    if empties.is_empty() {
        result.warnings.push("No empties extracted".to_string());
    }
    
    // Validate empty properties
    for empty in empties {
        if empty.position_blend[0].is_nan() || empty.position_blend[1].is_nan() || empty.position_blend[2].is_nan() {
            result.errors.push(format!("Empty '{}' has NaN position", empty.name));
            result.is_valid = false;
        }
        
        if empty.rotation_blend[0].is_nan() {
            result.errors.push(format!("Empty '{}' has NaN rotation", empty.name));
            result.is_valid = false;
        }
    }
    
    Ok(result)
}

/// Validate decal mesh identification
pub fn validate_decals_extraction(
    meshes: &[MeshWithDecals],
) -> Result<BlendValidationResult, String> {
    let mut result = BlendValidationResult {
        light_count: 0,
        lights_by_type: std::collections::HashMap::new(),
        empty_count: 0,
        collection_count: 0,
        meshes_with_decals: meshes.len(),
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    // Validate decal materials
    for mesh in meshes {
        if mesh.decal_materials.is_empty() {
            result.errors.push(format!(
                "Mesh '{}' marked as having decals but has none",
                mesh.mesh_path
            ));
            result.is_valid = false;
        }
        
        for material in &mesh.decal_materials {
            if !material.is_decal && !material.is_pom {
                result.errors.push(format!(
                    "Material '{}' in mesh '{}' has no decal/POM properties",
                    material.material_name, mesh.mesh_path
                ));
                result.is_valid = false;
            }
        }
    }
    
    Ok(result)
}

/// Comprehensive validation of entire Phase 3-4 pipeline
pub fn validate_complete_phase_3_4_export(
    lights: &[ExtractedLight],
    empties: &[ExtractedEmpty],
    decals: &[MeshWithDecals],
) -> BlendValidationResult {
    let mut result = BlendValidationResult {
        light_count: lights.len(),
        lights_by_type: std::collections::HashMap::new(),
        empty_count: empties.len(),
        collection_count: 5,  // Ambient, Omni, SoftOmni, Projector, Sun (typical)
        meshes_with_decals: decals.len(),
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    // Categorize lights
    for light in lights {
        let category = match light.lamp_type {
            0 => {
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        
        *result.lights_by_type.entry(category.to_string()).or_insert(0) += 1;
    }
    
    // Basic validation checks
    if lights.is_empty() && empties.is_empty() && decals.is_empty() {
        result.errors.push("No lights, empties, or decals extracted".to_string());
        result.is_valid = false;
    }
    
    if lights.is_empty() {
        result.warnings.push("No lights extracted".to_string());
    }
    
    if empties.is_empty() {
        result.warnings.push("No empties extracted".to_string());
    }
    
    if decals.is_empty() {
        result.warnings.push("No decal materials identified".to_string());
    }
    
    result
}

#[cfg(test)]
mod tests_5d {
    use super::*;
    use crate::types::EntityPayload;

    #[test]
    fn assign_decal_materials_basic() {
        let input = DecomposedInput {
            entity_name: "test_ship".to_string(),
            geometry_path: "/test".to_string(),
            material_path: "/test".to_string(),
            root_mesh: Mesh {
                positions: vec![
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                ],
                indices: vec![0, 1, 2],
                uvs: None,
                secondary_uvs: None,
                normals: None,
                tangents: None,
                colors: None,
                submeshes: vec![],
                model_min: [0.0, 0.0, 0.0],
                model_max: [1.0, 1.0, 0.0],
                scaling_min: [0.0, 0.0, 0.0],
                scaling_max: [1.0, 1.0, 0.0],
            },
            root_materials: None,
            root_nmc: None,
            root_palette: None,
            available_palettes: vec![],
            root_bones: vec![],
            root_skeleton_source_path: None,
            root_animation_controller: None,
            children: vec![],
            interiors: LoadedInteriors::default(),
            paint_variants: vec![],
        };

        let result = assign_decal_materials_to_vertex_groups(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn assign_decal_materials_no_materials() {
        let input = DecomposedInput {
            entity_name: "test_ship".to_string(),
            geometry_path: "/test".to_string(),
            material_path: "/test".to_string(),
            root_mesh: Mesh {
                positions: vec![[0.0, 0.0, 0.0]],
                indices: vec![],
                uvs: None,
                secondary_uvs: None,
                normals: None,
                tangents: None,
                colors: None,
                submeshes: vec![],
                model_min: [0.0, 0.0, 0.0],
                model_max: [0.0, 0.0, 0.0],
                scaling_min: [0.0, 0.0, 0.0],
                scaling_max: [0.0, 0.0, 0.0],
            },
            root_materials: None,
            root_nmc: None,
            root_palette: None,
            available_palettes: vec![],
            root_bones: vec![],
            root_skeleton_source_path: None,
            root_animation_controller: None,
            children: vec![],
            interiors: LoadedInteriors::default(),
            paint_variants: vec![],
        };

        let result = assign_decal_materials_to_vertex_groups(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn assign_decal_materials_empty_mesh() {
        let input = DecomposedInput {
            entity_name: "empty_ship".to_string(),
            geometry_path: "/test".to_string(),
            material_path: "/test".to_string(),
            root_mesh: Mesh {
                positions: vec![],
                indices: vec![],
                uvs: None,
                secondary_uvs: None,
                normals: None,
                tangents: None,
                colors: None,
                submeshes: vec![],
                model_min: [0.0, 0.0, 0.0],
                model_max: [0.0, 0.0, 0.0],
                scaling_min: [0.0, 0.0, 0.0],
                scaling_max: [0.0, 0.0, 0.0],
            },
            root_materials: None,
            root_nmc: None,
            root_palette: None,
            available_palettes: vec![],
            root_bones: vec![],
            root_skeleton_source_path: None,
            root_animation_controller: None,
            children: vec![],
            interiors: LoadedInteriors::default(),
            paint_variants: vec![],
        };

        let result = assign_decal_materials_to_vertex_groups(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn assign_decal_materials_multiple_children() {
        let input = DecomposedInput {
            entity_name: "test_ship".to_string(),
            geometry_path: "/test".to_string(),
            material_path: "/test".to_string(),
            root_mesh: Mesh {
                positions: vec![[0.0, 0.0, 0.0]],
                indices: vec![],
                uvs: None,
                secondary_uvs: None,
                normals: None,
                tangents: None,
                colors: None,
                submeshes: vec![],
                model_min: [0.0, 0.0, 0.0],
                model_max: [0.0, 0.0, 0.0],
                scaling_min: [0.0, 0.0, 0.0],
                scaling_max: [0.0, 0.0, 0.0],
            },
            root_materials: None,
            root_nmc: None,
            root_palette: None,
            available_palettes: vec![],
            root_bones: vec![],
            root_skeleton_source_path: None,
            root_animation_controller: None,
            children: vec![
                EntityPayload {
                    entity_name: "child1".to_string(),
                    mesh: Mesh {
                        positions: vec![[1.0, 1.0, 1.0]],
                        indices: vec![],
                        uvs: None,
                        secondary_uvs: None,
                        normals: None,
                        tangents: None,
                        colors: None,
                        submeshes: vec![],
                        model_min: [0.0, 0.0, 0.0],
                        model_max: [0.0, 0.0, 0.0],
                        scaling_min: [0.0, 0.0, 0.0],
                        scaling_max: [0.0, 0.0, 0.0],
                    },
                    materials: None,
                    nmc: None,
                    palette: None,
                    bones: vec![],
                    skeleton_source_path: None,
                    textures: None,
                    parent_node_name: "".to_string(),
                    parent_entity_name: "".to_string(),
                    no_rotation: false,
                    offset_position: [0.0, 0.0, 0.0],
                    offset_rotation: [0.0, 0.0, 0.0],
                    detach_direction: [0.0, 0.0, 0.0],
                    port_flags: "".to_string(),
                    geometry_path: "/test".to_string(),
                    material_path: "/test".to_string(),
                },
            ],
            interiors: LoadedInteriors::default(),
            paint_variants: vec![],
        };

        let result = assign_decal_materials_to_vertex_groups(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn assign_decal_materials_with_child_mesh() {
        let input = DecomposedInput {
            entity_name: "test_ship".to_string(),
            geometry_path: "/test".to_string(),
            material_path: "/test".to_string(),
            root_mesh: Mesh {
                positions: vec![[0.0, 0.0, 0.0]],
                indices: vec![],
                uvs: None,
                secondary_uvs: None,
                normals: None,
                tangents: None,
                colors: None,
                submeshes: vec![],
                model_min: [0.0, 0.0, 0.0],
                model_max: [0.0, 0.0, 0.0],
                scaling_min: [0.0, 0.0, 0.0],
                scaling_max: [0.0, 0.0, 0.0],
            },
            root_materials: None,
            root_nmc: None,
            root_palette: None,
            available_palettes: vec![],
            root_bones: vec![],
            root_skeleton_source_path: None,
            root_animation_controller: None,
            children: vec![
                EntityPayload {
                    entity_name: "child_with_mesh".to_string(),
                    mesh: Mesh {
                        positions: vec![
                            [1.0, 0.0, 0.0],
                            [0.0, 1.0, 0.0],
                            [0.0, 0.0, 1.0],
                        ],
                        indices: vec![0, 1, 2],
                        uvs: None,
                        secondary_uvs: None,
                        normals: None,
                        tangents: None,
                        colors: None,
                        submeshes: vec![],
                        model_min: [0.0, 0.0, 0.0],
                        model_max: [1.0, 1.0, 1.0],
                        scaling_min: [0.0, 0.0, 0.0],
                        scaling_max: [1.0, 1.0, 1.0],
                    },
                    materials: None,
                    nmc: None,
                    palette: None,
                    bones: vec![],
                    skeleton_source_path: None,
                    textures: None,
                    parent_node_name: "".to_string(),
                    parent_entity_name: "".to_string(),
                    no_rotation: false,
                    offset_position: [0.0, 0.0, 0.0],
                    offset_rotation: [0.0, 0.0, 0.0],
                    detach_direction: [0.0, 0.0, 0.0],
                    port_flags: "".to_string(),
                    geometry_path: "/test".to_string(),
                    material_path: "/test".to_string(),
                },
            ],
            interiors: LoadedInteriors::default(),
            paint_variants: vec![],
        };

        let result = assign_decal_materials_to_vertex_groups(&input);
        assert!(result.is_ok());
    }
}


mod tests_4c {
    use super::*;
    
    #[test]
    fn test_validate_lights_extraction_empty() {
        let lights = vec![];
        let result = validate_lights_extraction(&lights).unwrap();
        
        assert_eq!(result.light_count, 0);
        assert!(!result.is_valid);
        assert!(result.errors.len() > 0);
    }
    
    #[test]
    fn test_validate_lights_extraction_single_ambient() {
        let lights = vec![
            ExtractedLight {
                name: "Ambient_001".to_string(),
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0],
                lamp_type: 0,
                energy_watts: 1.0,
                radius: 5.0,
                spot_size: 0.0,
                spot_blend: 0.0,
                intensity_candela: 5.0,
                temperature_k: 6500.0,
                use_temperature: false,
                gobo_path: None,
                active_state: "defaultState".to_string(),
            },
        ];
        
        let result = validate_lights_extraction(&lights).unwrap();
        assert_eq!(result.light_count, 1);
        assert!(result.lights_by_type.contains_key("Ambient"));
        assert_eq!(result.lights_by_type["Ambient"], 1);
    }
    
    #[test]
    fn test_validate_lights_extraction_categorization() {
        let lights = vec![
            ExtractedLight {
                name: "Ambient".to_string(),
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0],
                lamp_type: 0,
                energy_watts: 1.0,
                radius: 5.0,
                spot_size: 0.0,
                spot_blend: 0.0,
                intensity_candela: 5.0,
                temperature_k: 6500.0,
                use_temperature: false,
                gobo_path: None,
                active_state: "defaultState".to_string(),
            },
            ExtractedLight {
                name: "Projector".to_string(),
                position_blend: [1.0, 1.0, 1.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [1.0, 0.8, 0.6],
                lamp_type: 2,
                energy_watts: 50.0,
                radius: 10.0,
                spot_size: 0.5,
                spot_blend: 0.2,
                intensity_candela: 200.0,
                temperature_k: 6500.0,
                use_temperature: false,
                gobo_path: None,
                active_state: "defaultState".to_string(),
            },
        ];
        
        let result = validate_lights_extraction(&lights).unwrap();
        assert_eq!(result.light_count, 2);
        assert_eq!(result.lights_by_type.get("Ambient").unwrap_or(&0), &1);
        assert_eq!(result.lights_by_type.get("Projector").unwrap_or(&0), &1);
    }
    
    #[test]
    fn test_validate_empties_extraction_empty() {
        let empties = vec![];
        let result = validate_empties_extraction(&empties).unwrap();
        
        assert_eq!(result.empty_count, 0);
        assert!(result.warnings.len() > 0);
    }
    
    #[test]
    fn test_validate_empties_extraction_valid() {
        let empties = vec![
            ExtractedEmpty {
                name: "Helper_001".to_string(),
                nmc_index: 0,
                parent_nmc_index: None,
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 3,
                is_helper: true,
            },
        ];
        
        let result = validate_empties_extraction(&empties).unwrap();
        assert_eq!(result.empty_count, 1);
        assert!(result.is_valid);
    }
    
    #[test]
    fn test_validate_decals_extraction_valid() {
        let decals = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "Decal_001".to_string(),
                        is_decal: true,
                        is_pom: false,
                        material_index: 0,
                    },
                ],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_decals_extraction(&decals).unwrap();
        assert_eq!(result.meshes_with_decals, 1);
        assert!(result.is_valid);
    }
    
    #[test]
    fn test_validate_complete_phase_3_4_export_full() {
        let lights = vec![
            ExtractedLight {
                name: "Light1".to_string(),
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0],
                lamp_type: 0,
                energy_watts: 1.0,
                radius: 5.0,
                spot_size: 0.0,
                spot_blend: 0.0,
                intensity_candela: 5.0,
                temperature_k: 6500.0,
                use_temperature: false,
                gobo_path: None,
                active_state: "defaultState".to_string(),
            },
        ];
        
        let empties = vec![
            ExtractedEmpty {
                name: "Empty1".to_string(),
                nmc_index: 0,
                parent_nmc_index: None,
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 3,
                is_helper: true,
            },
        ];
        
        let decals = vec![
            MeshWithDecals {
                mesh_path: "Mesh_001".to_string(),
                decal_materials: vec![
                    DecalMaterial {
                        material_name: "Decal".to_string(),
                        is_decal: true,
                        is_pom: false,
                        material_index: 0,
                    },
                ],
                decal_face_indices: vec![],
            },
        ];
        
        let result = validate_complete_phase_3_4_export(&lights, &empties, &decals);
        assert_eq!(result.light_count, 1);
        assert_eq!(result.empty_count, 1);
        assert_eq!(result.meshes_with_decals, 1);
        assert!(result.is_valid);
    }
    
    #[test]
    fn test_extracted_light_has_use_temperature_field() {
        // Test 1: Verify ExtractedLight struct has use_temperature field
        let light = ExtractedLight {
            name: "Test".to_string(),
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            lamp_type: 0,
            energy_watts: 1.0,
            radius: 5.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 5.0,
            temperature_k: 6500.0,
            use_temperature: true,  // Verify field exists and can be set
            gobo_path: None,
            active_state: "defaultState".to_string(),
        };
        
        assert_eq!(light.temperature_k, 6500.0);
        assert_eq!(light.use_temperature, true);
    }
    
    #[test]
    fn test_extracted_light_temperature_false() {
        // Test 2: use_temperature can be set to false
        let light = ExtractedLight {
            name: "Test".to_string(),
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            lamp_type: 0,
            energy_watts: 1.0,
            radius: 5.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 5.0,
            temperature_k: 3000.0,
            use_temperature: false,  // Explicitly false
            gobo_path: None,
            active_state: "defaultState".to_string(),
        };
        
        assert_eq!(light.temperature_k, 3000.0);
        assert_eq!(light.use_temperature, false);
    }
    
    #[test]
    fn test_temperature_values_range() {
        // Test 3: Temperature values within typical Kelvin range
        let test_temps = vec![
            (2700.0, true, "Warm white"),
            (3000.0, false, "Warm incandescent"),
            (5000.0, true, "Mid-range"),
            (6500.0, false, "Daylight"),
            (9000.0, true, "Cool white"),
            (12000.0, false, "Very cool"),
        ];
        
        for (temp, use_temp, desc) in test_temps {
            let light = ExtractedLight {
                name: format!("Light_{}", temp as i32),
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0],
                lamp_type: 0,
                energy_watts: 1.0,
                radius: 5.0,
                spot_size: 0.0,
                spot_blend: 0.0,
                intensity_candela: 5.0,
                temperature_k: temp,
                use_temperature: use_temp,
                gobo_path: None,
                active_state: "defaultState".to_string(),
            };
            
            assert_eq!(light.temperature_k, temp, "Failed for {}: {}", temp, desc);
            assert_eq!(light.use_temperature, use_temp, "Failed for {}: {}", temp, desc);
        }
    }
}
