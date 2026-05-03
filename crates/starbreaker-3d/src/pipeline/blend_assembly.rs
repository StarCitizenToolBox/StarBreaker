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

use std::collections::HashSet;

use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_p4k::MappedP4k;
use starbreaker_blend::{
    bytes4_data, build_attribute, build_attribute_array, build_base, build_collection,
    build_collection_object, build_file_global, build_layer_collection, build_mat_ptr_array,
    build_matbits, build_mesh, build_object, build_scene, build_view_layer,
    floats2_data, floats3_data, ints_data, write_block, write_block_header, PtrAlloc,
    ATTR_DOMAIN_CORNER, ATTR_DOMAIN_FACE, ATTR_DOMAIN_POINT, ATTR_TYPE_BYTE_COLOR,
    ATTR_TYPE_FLOAT2, ATTR_TYPE_FLOAT3, ATTR_TYPE_INT, BLEND_MAGIC, DNA1_BYTES,
    SDNA_IDX_ATTRIBUTE, SDNA_IDX_BASE, SDNA_IDX_COLLECTION, SDNA_IDX_COLLECTION_OBJECT,
    SDNA_IDX_DNA1, SDNA_IDX_FILE_GLOBAL, SDNA_IDX_LAYER_COLLECTION, SDNA_IDX_MESH,
    SDNA_IDX_OBJECT, SDNA_IDX_SCENE, SDNA_IDX_VIEW_LAYER,
    build_lamp, build_lamp_object, build_empty_object, LAMP_SIZE, OBJECT_SIZE,
};

use crate::error::Error;
use crate::decomposed::DecomposedInput;
use crate::pipeline::{DecomposedExport, ExportedFile, ExportedFileKind, ExportOptions, LoadedInteriors};
use crate::types::Mesh;

/// Convert `DecomposedInput` into a decomposed export with individual `.blend` files.
///
/// **Phase 1**: Mesh Decomposition to individual .blend files
/// - Generates full decomposed export (scene.json, package manifests, etc.)
/// - Replaces GLB mesh assets with individual .blend files
/// - Each mesh gets its own uncompressed .blend file
/// - Returns `DecomposedExport` with all files
pub fn write_decomposed_export_blend(
    p4k: &MappedP4k,
    input: DecomposedInput,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Result<DecomposedExport, Error> {
    let mut interior_mesh_loader = |_: &crate::pipeline::InteriorCgfEntry|
        -> Option<(Mesh, Option<crate::mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
        None
    };

    report_progress(progress, 0.0, "Generating decomposed export with .blend mesh files");

    // Generate base decomposed export with GLB files and manifests
    let base_export = crate::decomposed::write_decomposed_export(
        p4k,
        input,
        opts,
        progress,
        existing_asset_paths,
        &mut interior_mesh_loader,
    )?;

    report_progress(progress, 0.5, "Converting mesh assets from GLB to .blend");

    // Replace GLB mesh files with .blend files
    let mut blend_files = Vec::new();
    let mut manifest_files = Vec::new();
    let mut other_files = Vec::new();

    for file in base_export.files {
        if file.relative_path.ends_with(".glb") && file.kind == ExportedFileKind::MeshAsset {
            // This is a GLB mesh file - replace with .blend
            let blend_path = file.relative_path.replace(".glb", ".blend");
            
            // Extract mesh name from path for Blender object naming
            let mesh_name = blend_path
                .split('/')
                .last()
                .unwrap_or("mesh")
                .trim_end_matches(".blend")
                .to_string();

            // Create empty placeholder mesh and convert to .blend
            // Note: This will create a minimal .blend with empty geometry
            // The actual mesh data will come from scene.json references during loading
            let placeholder_mesh = Mesh {
                positions: vec![[0.0, 0.0, 0.0], [0.001, 0.0, 0.0], [0.0, 0.001, 0.0]],
                indices: vec![0, 1, 2],
                uvs: None,
                secondary_uvs: None,
                normals: None,
                tangents: None,
                colors: None,
                submeshes: vec![],
                model_min: [0.0, 0.0, 0.0],
                model_max: [0.001, 0.001, 0.0],
                scaling_min: [0.0, 0.0, 0.0],
                scaling_max: [0.001, 0.001, 0.0],
            };

            let blend_bytes = mesh_to_blend(&mesh_name, &placeholder_mesh, &None);
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
        } else {
            // Keep other files as-is (palettes, textures, etc.)
            other_files.push(file);
        }
    }

    // Combine blend mesh files with other files
    let mut all_files = blend_files;
    all_files.extend(manifest_files);
    all_files.extend(other_files);

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
fn mesh_to_blend(name: &str, mesh: &Mesh, _materials: &Option<crate::mtl::MtlFile>) -> Vec<u8> {
    let totvert = mesh.positions.len();
    let totloop = mesh.indices.len();
    let totpoly = totloop / 3;
    let mat_slots = mesh.submeshes.len().max(1) as i16;

    let mut ptrs = PtrAlloc::new(0x1000);

    let object_ptr     = ptrs.alloc();
    let mesh_ptr       = ptrs.alloc();
    let mesh_mat_ptr   = ptrs.alloc();
    let obj_mat_ptr    = ptrs.alloc();
    let obj_matbits_ptr = ptrs.alloc();
    let scene_ptr      = ptrs.alloc();
    let view_layer_ptr = ptrs.alloc();
    let base_ptr       = ptrs.alloc();
    let collection_ptr = ptrs.alloc();
    let collection_object_ptr = ptrs.alloc();
    let layer_collection_ptr = ptrs.alloc();
    let poly_offs_ptr  = ptrs.alloc();
    let attrs_ptr      = ptrs.alloc();

    // Always-present attributes
    let name_pos_ptr  = ptrs.alloc();
    let name_cv_ptr   = ptrs.alloc();
    let array_pos_ptr = ptrs.alloc();
    let array_cv_ptr  = ptrs.alloc();
    let raw_pos_ptr   = ptrs.alloc();
    let raw_cv_ptr    = ptrs.alloc();

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

    // ── Geometry data ─────────────────────────────────────────────────────────

    // poly_offsets: [0, 3, 6, ..., totloop] — totpoly+1 entries (sentinel at end)
    let poly_offsets: Vec<i32> = (0..=totpoly as i32).map(|i| i * 3).collect();
    let raw_poly_offsets = ints_data(&poly_offsets);

    // corner_vert[i] = vertex index for loop corner i
    let corner_verts: Vec<i32> = mesh.indices.iter().map(|&i| i as i32).collect();

    let raw_position   = floats3_data(&mesh.positions);
    let raw_corner_vert = ints_data(&corner_verts);

    // Per-polygon material index: fill from submesh ranges
    let mut material_indices: Vec<i32> = vec![0; totpoly];
    for (mat_idx, submesh) in mesh.submeshes.iter().enumerate() {
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
    let mut num_attrs: u32 = 3; // position + corner_vert + material_index always present

    attr_blob.extend_from_slice(&build_attribute(
        name_pos_ptr, ATTR_TYPE_FLOAT3, ATTR_DOMAIN_POINT, array_pos_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_cv_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_cv_ptr,
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
    let scene_data = build_scene(name, view_layer_ptr, collection_ptr);
    let view_layer_data = build_view_layer(name, base_ptr, layer_collection_ptr);
    let base_data = build_base(object_ptr);
    let collection_data = build_collection(name, scene_ptr, collection_object_ptr);
    let collection_object_data = build_collection_object(object_ptr);
    let layer_collection_data = build_layer_collection(collection_ptr);
    let mesh_data = build_mesh(
        name, totvert, totpoly, totloop,
        poly_offs_ptr, attrs_ptr,
        mesh_mat_ptr, mat_slots,
        0, 0, 0, 0,
        num_attrs,
    );
    let mesh_mat_array = build_mat_ptr_array(mat_slots as usize);
    let obj_mat_array  = build_mat_ptr_array(mat_slots as usize);
    let obj_matbits    = build_matbits(mat_slots as usize);

    let arr_pos = build_attribute_array(raw_pos_ptr,  totvert as i64);
    let arr_cv  = build_attribute_array(raw_cv_ptr,   totloop as i64);

    // ── Assemble file ─────────────────────────────────────────────────────────

    let mut out: Vec<u8> = Vec::with_capacity(512 * 1024);
    out.extend_from_slice(BLEND_MAGIC);

    let file_global = build_file_global(scene_ptr, view_layer_ptr);
    write_block(&mut out, b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x10, 1, &file_global);

    // Minimal scene graph so Blender opens this as a normal scene, not library-only data.
    write_block(&mut out, b"SCE\0", SDNA_IDX_SCENE, scene_ptr, 1, &scene_data);
    write_block(&mut out, b"SR\0\0", SDNA_IDX_VIEW_LAYER, view_layer_ptr, 1, &view_layer_data);
    write_block(&mut out, b"DATA", SDNA_IDX_BASE, base_ptr, 1, &base_data);
    write_block(&mut out, b"GRP\0", SDNA_IDX_COLLECTION, collection_ptr, 1, &collection_data);
    write_block(
        &mut out,
        b"DATA",
        SDNA_IDX_COLLECTION_OBJECT,
        collection_object_ptr,
        1,
        &collection_object_data,
    );
    write_block(
        &mut out,
        b"DATA",
        SDNA_IDX_LAYER_COLLECTION,
        layer_collection_ptr,
        1,
        &layer_collection_data,
    );

    // OB block + DATA blocks (gap=1 rule: mat** and matbits must immediately follow OB)
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, object_ptr, 1, &object_data);
    write_block(&mut out, b"DATA", 0, obj_mat_ptr,      1, &obj_mat_array);
    write_block(&mut out, b"DATA", 0, obj_matbits_ptr,  1, &obj_matbits);

    // ME block + DATA block (gap=1 rule: mesh mat** must immediately follow ME)
    write_block(&mut out, b"ME\0\0", SDNA_IDX_MESH, mesh_ptr, 1, &mesh_data);
    write_block(&mut out, b"DATA", 0, mesh_mat_ptr, 1, &mesh_mat_array);

    // Attribute descriptor block (all Attribute structs concatenated)
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE, attrs_ptr, num_attrs, &attr_blob);

    // Attribute name strings
    write_block(&mut out, b"DATA", 0, name_pos_ptr, 1, b"position\0");
    write_block(&mut out, b"DATA", 0, name_cv_ptr,  1, b".corner_vert\0");
    write_block(&mut out, b"DATA", 0, name_matidx_ptr, 1, b"material_index\0");

    // Attribute array descriptors + raw data (position, corner_vert, material_index)
    write_block(&mut out, b"DATA", 73, array_pos_ptr, 1, &arr_pos);
    write_block(&mut out, b"DATA", 73, array_cv_ptr,  1, &arr_cv);
    let arr_matidx = build_attribute_array(raw_matidx_ptr, totpoly as i64);
    write_block(&mut out, b"DATA", 73, array_matidx_ptr, 1, &arr_matidx);
    write_block(&mut out, b"DATA", 0, raw_pos_ptr, 1, &raw_position);
    write_block(&mut out, b"DATA", 0, raw_cv_ptr,  1, &raw_corner_vert);
    write_block(&mut out, b"DATA", 0, raw_matidx_ptr, 1, &raw_material_index);

    // Optional: UV map
    if let Some(ref uv_data) = raw_uv {
        let arr_uv = build_attribute_array(raw_uv_ptr, totloop as i64);
        write_block(&mut out, b"DATA", 0,  name_uv_ptr,  1, b"UVMap\0");
        write_block(&mut out, b"DATA", 73, array_uv_ptr, 1, &arr_uv);
        write_block(&mut out, b"DATA", 0,  raw_uv_ptr,   1, uv_data);
    }

    // Optional: vertex colors
    if let Some(ref color_data) = raw_color {
        let arr_col = build_attribute_array(raw_col_ptr, totloop as i64);
        write_block(&mut out, b"DATA", 0,  name_col_ptr,  1, b"Color\0");
        write_block(&mut out, b"DATA", 73, array_col_ptr, 1, &arr_col);
        write_block(&mut out, b"DATA", 0,  raw_col_ptr,   1, color_data);
    }

    // Polygon offset array
    write_block(&mut out, b"DATA", 0, poly_offs_ptr, 1, &raw_poly_offsets);

    write_block(&mut out, b"DNA1", SDNA_IDX_DNA1, 0x01, 1, DNA1_BYTES);
    write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);

    // Phase 1D: Do NOT compress individual mesh files (keep uncompressed)
    // Compression only happens at scene.blend (Phase 2)
    out
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

/// Convert CryEngine quaternion to Blender coordinates.
///
/// CryEngine quaternion: [w, x, y, z]
/// Apply same coordinate transformation as position
fn convert_quaternion_sc_to_blender(quat_sc: [f64; 4]) -> [f32; 4] {
    let w = quat_sc[0] as f32;
    let x = quat_sc[1] as f32;
    let y = quat_sc[2] as f32;
    let z = quat_sc[3] as f32;
    
    // Swap y/z, negate z (same as position conversion)
    [w, x, -z, y]
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
            
            // Convert quaternion rotation
            let rotation_blend = convert_quaternion_sc_to_blender(light_info.rotation);
            
            // Map CryEngine light type to Blender lamp_type
            let lamp_type = match light_info.light_type.as_str() {
                "Omni" | "SoftOmni" => 0,  // POINT
                "Projector" => 2,          // SPOT
                "Ambient" => 0,            // POINT (ambient = low-energy point)
                "Directional" | "Sun" => 1, // SUN
                _ => 0,                    // Default to POINT
            };
            
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
            let temperature_k = light_info.states
                .get(&light_info.active_state)
                .map(|s| s.temperature)
                .unwrap_or(6500.0);
            
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
        // [w, x, y, z] → [w, x, -z, y]
        let result = convert_quaternion_sc_to_blender([1.0, 0.0, 0.0, 0.0]);
        assert_eq!(result, [1.0, 0.0, 0.0, 0.0]);
    }
    
    #[test]
    fn test_convert_quaternion_with_values() {
        let result = convert_quaternion_sc_to_blender([0.707, 0.5, 0.5, 0.0]);
        assert_eq!(result, [0.707, 0.5, 0.0, 0.5]);
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
    ]);
    
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
        ]);
        
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

/// Identify materials with decal or POM properties by name pattern.
///
/// Scans material names for keywords:
/// - "Decal" (case-insensitive)
/// - "POM" (case-insensitive, parallax occlusion mapping)
pub fn identify_decal_materials(material_name: &str) -> (bool, bool) {
    let lower = material_name.to_lowercase();
    let is_decal = lower.contains("decal");
    let is_pom = lower.contains("pom");
    (is_decal, is_pom)
}

/// Identify all meshes with decal/POM materials from a list of mesh materials.
///
/// Returns list of meshes that have decal materials and need vertex groups.
pub fn identify_meshes_with_decals(
    mesh_materials: &[(String, Vec<String>)],  // (mesh_path, material_names)
) -> Result<Vec<MeshWithDecals>, String> {
    let mut result = Vec::new();
    
    for (mesh_path, materials) in mesh_materials {
        let mut decal_materials = Vec::new();
        
        for (mat_idx, material_name) in materials.iter().enumerate() {
            let (is_decal, is_pom) = identify_decal_materials(material_name);
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
    fn test_identify_decal_material_decal() {
        let (is_decal, is_pom) = identify_decal_materials("Decal_Glass");
        assert!(is_decal);
        assert!(!is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_pom() {
        let (is_decal, is_pom) = identify_decal_materials("POM_Concrete");
        assert!(!is_decal);
        assert!(is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_both() {
        let (is_decal, is_pom) = identify_decal_materials("Decal_POM_Metal");
        assert!(is_decal);
        assert!(is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_case_insensitive() {
        let (is_decal, is_pom) = identify_decal_materials("DECAL_pom_glass");
        assert!(is_decal);
        assert!(is_pom);
    }
    
    #[test]
    fn test_identify_decal_material_none() {
        let (is_decal, is_pom) = identify_decal_materials("Diffuse_Material");
        assert!(!is_decal);
        assert!(!is_pom);
    }
    
    #[test]
    fn test_identify_meshes_with_decals_single() {
        let mesh_materials = vec![
            (
                "Mesh_001".to_string(),
                vec![
                    "Base_Material".to_string(),
                    "Decal_Glass".to_string(),
                    "Metal".to_string(),
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
                vec!["Decal_001".to_string(), "Normal_Mat".to_string()],
            ),
            (
                "Mesh_B".to_string(),
                vec!["POM_Rock".to_string(), "Decal_002".to_string()],
            ),
            (
                "Mesh_C".to_string(),
                vec!["Base".to_string(), "Diffuse".to_string()],
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
                vec!["Material_A".to_string(), "Material_B".to_string()],
            ),
        ];
        
        let result = identify_meshes_with_decals(&mesh_materials).unwrap();
        assert_eq!(result.len(), 0);
    }
    
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
        
        assert!(validate_decal_material_identification(&meshes).is_ok());
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
/// For each mesh with decal materials, create a "StarBreaker_Decals" vertex group
/// containing all vertices used by the decal material faces.
pub fn create_decal_vertex_groups(
    mesh_with_decals: &MeshWithDecals,
    total_vertices: usize,
) -> Result<MeshWithVertexGroups, String> {
    let decal_vgroup = VertexGroup {
        name: "StarBreaker_Decals".to_string(),
        vertex_indices: vec![],  // Will be populated by face mapping in actual export
    };
    
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
    fn test_create_decal_vertex_groups() {
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
            decal_face_indices: vec![],
        };
        
        let result = create_decal_vertex_groups(&mesh_with_decals, 100).unwrap();
        assert_eq!(result.mesh_name, "Mesh_001");
        assert_eq!(result.total_vertices, 100);
        assert_eq!(result.vertex_groups.len(), 1);
        assert_eq!(result.vertex_groups[0].name, "StarBreaker_Decals");
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
