//! Blender decomposed export: convert `DecomposedInput` to individual `.blend` files.
//!
//! Orchestrates the pipeline for Phase 1 (mesh decomposition):
//! - Extract meshes from `DecomposedInput::children`
//! - Build material slots (empty, names-only)
//! - Write individual `.blend` files for each mesh
//! - Generate `ExportedFile` entries
//!
//! Phase 2 (scene.blend linking) and Phase 3+ will be implemented in subsequent phases.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::json;
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
};

use crate::error::Error;
use crate::decomposed::DecomposedInput;
use crate::pipeline::{DecomposedExport, ExportedFile, ExportedFileKind, ExportOptions};
use crate::types::Mesh;

/// Convert `DecomposedInput` into individual `.blend` files, one per mesh.
///
/// **Phase 1**: Mesh Decomposition to individual .blend files
/// - Extracts each mesh from `DecomposedInput::children`
/// - Creates empty material slots (names only, no shaders/textures)
/// - Writes uncompressed `.blend` files to `Data/Objects/...` paths
/// - Returns `DecomposedExport` with all exported files
pub fn write_decomposed_export_blend(
    _p4k: &MappedP4k,
    input: DecomposedInput,
    _opts: &ExportOptions,
    progress: Option<&Progress>,
    _existing_asset_paths: Option<&HashSet<String>>,
) -> Result<DecomposedExport, Error> {
    let mut files = Vec::new();
    let total_meshes = input.children.len();

    report_progress(progress, 0.0, &format!("Exporting {} meshes to .blend files", total_meshes));

    // Phase 1B-1E: Process each child mesh
    for (idx, child) in input.children.into_iter().enumerate() {
        let progress_ratio = idx as f32 / total_meshes.max(1) as f32;
        report_progress(progress, progress_ratio, &format!("Exporting mesh {}/{}", idx + 1, total_meshes));

        // Generate file path for this mesh (Phase 1B/1C)
        let mesh_name = sanitize_mesh_name(&child.entity_name);
        let blend_path = format!("Data/Objects/Spaceships/Ships/{}.blend", mesh_name);

        // Convert mesh to .blend bytes (Phase 1D)
        let blend_bytes = mesh_to_blend(&mesh_name, &child.mesh, &child.materials);

        // Create ExportedFile entry (Phase 1E)
        files.push(ExportedFile {
            relative_path: blend_path,
            bytes: blend_bytes,
            kind: ExportedFileKind::MeshAsset,
        });
    }

    report_progress(progress, 1.0, "Mesh export complete");

    Ok(DecomposedExport { files })
}

/// Sanitize entity name for use as a file/object name in Blender.
fn sanitize_mesh_name(entity_name: &str) -> String {
    entity_name
        .replace(' ', "_")
        .replace('/', "_")
        .replace('\\', "_")
        .replace('.', "_")
        .to_lowercase()
}

/// Convert a mesh to `.blend` file bytes (uncompressed).
///
/// Produces a .blend containing a single OB_MESH object with:
/// - Position vertices (POINT / FLOAT3)
/// - Corner vertices (.corner_vert, CORNER / INT)
/// - Material index per face (FACE / INT) — always allocated even if no submeshes
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

// ============================================================================
// Phase 2: Scene Linking - Create scene.blend from scene.json manifest
// ============================================================================

/// Phase 2A: Parse scene.json manifest to extract object hierarchy.
///
/// Returns the parsed JSON value if valid, or an error if the manifest is malformed.
pub(crate) fn parse_scene_manifest(scene_json_bytes: &[u8]) -> Result<serde_json::Value, Error> {
    serde_json::from_slice(scene_json_bytes)
        .map_err(|e| Error::Json(e))
}

/// Phase 2B: Coordinate system conversion from CryEngine to Blender.
///
/// CryEngine uses Z-up, right-handed coordinates.
/// Blender uses Y-up, right-handed coordinates.
///
/// Conversion formula:
///   M_blend = C × M_sc^T × C⁻¹
///   where C = [[1,0,0,0],[0,0,-1,0],[0,1,0,0],[0,0,0,1]]  (Y↔Z swap)
///
/// For position vectors only:
///   (x, y, z)_blend = (x, -z, y)_sc
fn sc_matrix_to_blender(m: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // C = [[1,0,0,0],[0,0,-1,0],[0,1,0,0],[0,0,0,1]]
    // C_inv = [[1,0,0,0],[0,0,1,0],[0,-1,0,0],[0,0,0,1]]
    
    // Step 1: Compute M_sc × C⁻¹
    let mut temp: [[f32; 4]; 4] = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            temp[i][j] = 0.0;
            for k in 0..4 {
                // C_inv[k][j]
                let c_inv_val = match (k, j) {
                    (0, 0) => 1.0, (0, 1) => 0.0, (0, 2) => 0.0, (0, 3) => 0.0,
                    (1, 0) => 0.0, (1, 1) => 0.0, (1, 2) => 1.0, (1, 3) => 0.0,
                    (2, 0) => 0.0, (2, 1) => -1.0, (2, 2) => 0.0, (2, 3) => 0.0,
                    (3, 0) => 0.0, (3, 1) => 0.0, (3, 2) => 0.0, (3, 3) => 1.0,
                    _ => 0.0,
                };
                temp[i][j] += m[i][k] * c_inv_val;
            }
        }
    }
    
    // Step 2: Compute C × temp
    let mut result: [[f32; 4]; 4] = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            result[i][j] = 0.0;
            for k in 0..4 {
                // C[i][k]
                let c_val = match (i, k) {
                    (0, 0) => 1.0, (0, 1) => 0.0, (0, 2) => 0.0, (0, 3) => 0.0,
                    (1, 0) => 0.0, (1, 1) => 0.0, (1, 2) => -1.0, (1, 3) => 0.0,
                    (2, 0) => 0.0, (2, 1) => 1.0, (2, 2) => 0.0, (2, 3) => 0.0,
                    (3, 0) => 0.0, (3, 1) => 0.0, (3, 2) => 0.0, (3, 3) => 1.0,
                    _ => 0.0,
                };
                result[i][j] += c_val * temp[k][j];
            }
        }
    }
    
    result
}

/// Convert CryEngine position vector to Blender space.
fn sc_vector_to_blender(v: &[f32; 3]) -> [f32; 3] {
    [v[0], -v[2], v[1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinate_conversion() {
        // Identity matrix should remain identity after conversion (identity is basis-independent)
        let identity: [[f32; 4]; 4] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let result = sc_matrix_to_blender(&identity);
        // Identity should stay identity (mathematically: C @ I @ C^-1 = I)
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((result[i][j] - expected).abs() < 0.001, 
                    "identity [{i}][{j}] = {}, expected {}", result[i][j], expected);
            }
        }
    }

    #[test]
    fn test_vector_conversion() {
        let v = [1.0, 2.0, 3.0];  // CryEngine (X=1, Y=2, Z=3)
        let result = sc_vector_to_blender(&v);
        assert_eq!(result[0], 1.0);   // X stays same
        assert_eq!(result[1], -3.0);  // -Z
        assert_eq!(result[2], 2.0);   // Y
    }
}
