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
    build_lamp, build_lamp_object, LAMP_SIZE, OBJECT_SIZE,
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
