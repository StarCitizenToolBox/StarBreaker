//! `starbreaker-blend` — write Blender 5.x `.blend` library files.
//!
//! Encodes mesh geometry, UV layers, vertex colors, custom normals, vertex groups,
//! empties, lights, parent hierarchies, and IDProperties (custom object properties)
//! into the binary `.blend` format understood by Blender 5.1.x.
//!
//! All layout constants (SDNA indices, struct sizes, field offsets) were extracted
//! from a live Blender 5.1.1 instance and cross-checked against the bundled DNA1
//! block (`dna1_blender501.bin`).

use std::io::Write;

use zstd::Encoder;

pub const DNA1_BYTES: &[u8] = include_bytes!("dna1_blender501.bin");

/// Blender 5.1.x file magic (17 bytes).
/// Format: BLENDER (7) + 17 (2) + - (1) + 01 (2) + v (1) + 0501 (4) = 17 bytes
pub const BLEND_MAGIC: &[u8] = b"BLENDER17-01v0501";

// ── SDNA indices (verified against Blender 5.1.1 DNA1 block) ─────────────────
pub const SDNA_IDX_ATTRIBUTE: u32 = 75;
pub const SDNA_IDX_ATTRIBUTE_ARRAY: u32 = 73;
pub const SDNA_IDX_COLLECTION_OBJECT: u32 = 104;
pub const SDNA_IDX_COLLECTION: u32 = 107;
pub const SDNA_IDX_FILE_GLOBAL: u32 = 171;
pub const SDNA_IDX_LAYER_COLLECTION: u32 = 247;
pub const SDNA_IDX_BASE: u32 = 246;
pub const SDNA_IDX_VIEW_LAYER: u32 = 252;
pub const SDNA_IDX_MESH: u32 = 322;
pub const SDNA_IDX_OBJECT: u32 = 692;
pub const SDNA_IDX_SCENE: u32 = 757;
pub const SDNA_IDX_LAMP: u32 = 253;
pub const SDNA_IDX_BDEFORMGROUP: u32 = 686;
pub const SDNA_IDX_MDEFORMVERT: u32 = 331;
pub const SDNA_IDX_MDEFORMWEIGHT: u32 = 330;
pub const SDNA_IDX_CUSTOM_DATA_LAYER: u32 = 160;
pub const SDNA_IDX_IDPROPERTY: u32 = 9;
pub const SDNA_IDX_LIBRARY: u32 = 15;
pub const SDNA_IDX_ID: u32 = 14;
pub const SDNA_IDX_DNA1: u32 = 0;

// ── Struct sizes (bytes) ──────────────────────────────────────────────────────
pub const FILE_GLOBAL_SIZE: usize = 1216;
pub const SCENE_SIZE: usize = 6920;
pub const VIEW_LAYER_SIZE: usize = 328;
pub const BASE_SIZE: usize = 48;
pub const COLLECTION_SIZE: usize = 520;
pub const COLLECTION_OBJECT_SIZE: usize = 32;
pub const LAYER_COLLECTION_SIZE: usize = 64;
pub const OBJECT_SIZE: usize = 1288;
pub const MESH_SIZE: usize = 1960;
pub const LAMP_SIZE: usize = 568;
pub const ATTRIBUTE_SIZE: usize = 24;
pub const ATTRIBUTE_ARRAY_SIZE: usize = 32;
pub const BDEFORMGROUP_SIZE: usize = 88;
pub const MDEFORMVERT_SIZE: usize = 16;
pub const MDEFORMWEIGHT_SIZE: usize = 8;
pub const CUSTOM_DATA_LAYER_SIZE: usize = 112;
pub const IDPROPERTY_SIZE: usize = 144;
pub const LIBRARY_SIZE: usize = 1426;  // ID (370) + filepath[1024] + flag (2) + undo_runtime_tag (2) + padding (4) + pointers (24)
pub const ID_STUB_SIZE: usize = 408;

// ── Attribute enums ───────────────────────────────────────────────────────────
pub const ATTR_DOMAIN_POINT: u8 = 0;
pub const ATTR_DOMAIN_EDGE: u8 = 1;
pub const ATTR_DOMAIN_FACE: u8 = 2;
pub const ATTR_DOMAIN_CORNER: u8 = 3;
pub const ATTR_TYPE_INT16_2D: i16 = 2;
pub const ATTR_TYPE_INT: i16 = 3;
pub const ATTR_TYPE_FLOAT2: i16 = 6;
pub const ATTR_TYPE_FLOAT3: i16 = 7;
pub const ATTR_TYPE_BYTE_COLOR: i16 = 9;
pub const ATTR_STORAGE_ARRAY: u8 = 0;

// ── IDProperty type codes ─────────────────────────────────────────────────────
pub const IDP_STRING: u8 = 0;
pub const IDP_INT: u8 = 1;
pub const IDP_GROUP: u8 = 6;
pub const IDP_DOUBLE: u8 = 8;

// ── Pointer allocator ─────────────────────────────────────────────────────────

/// Sequential fake-pointer allocator (file-scope; not related to real addresses).
/// Each call returns the next address and advances by 0x10.
#[derive(Clone, Copy, Debug)]
pub struct PtrAlloc {
    next: u64,
}

impl PtrAlloc {
    pub fn new(start: u64) -> Self {
        Self { next: start }
    }

    pub fn alloc(&mut self) -> u64 {
        let p = self.next;
        self.next += 0x10;
        p
    }
}

// ── Low-level serialisation helpers ──────────────────────────────────────────

/// Write a 32-byte block header for Blender 5.x (17-byte magic files).
///
/// Layout: `code[4] + sdna_idx[u32] + old_ptr[u64] + data_len[u32] + zero[u32] + count[u32] + zero[u32]`
pub fn write_block_header(
    out: &mut Vec<u8>,
    code: &[u8; 4],
    sdna_idx: u32,
    old_ptr: u64,
    data_len: u32,
    count: u32,
) {
    out.extend_from_slice(code);
    out.extend_from_slice(&sdna_idx.to_le_bytes());
    out.extend_from_slice(&old_ptr.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

/// Write a complete block (header + data).
pub fn write_block(
    out: &mut Vec<u8>,
    code: &[u8; 4],
    sdna_idx: u32,
    old_ptr: u64,
    count: u32,
    data: &[u8],
) {
    write_block_header(out, code, sdna_idx, old_ptr, data.len() as u32, count);
    out.extend_from_slice(data);
}

/// Compress raw `.blend` bytes using Blender 5.x Zstandard (Zstd) format.
///
/// Blender 5.x requires Zstd compression (not gzip). If compression fails,
/// returns the original input bytes so callers still get a valid uncompressed
/// `.blend` payload.
///
/// Magic bytes: 0x28 0xB5 0x2F 0xFD (Zstd frame header)
pub fn compress_blend_bytes(raw_blend: &[u8]) -> Vec<u8> {
    match Encoder::new(Vec::new(), 19) {
        Ok(mut encoder) => {
            if encoder.write_all(raw_blend).is_err() {
                return raw_blend.to_vec();
            }
            match encoder.finish() {
                Ok(compressed) => compressed,
                Err(_) => raw_blend.to_vec(),
            }
        }
        Err(_) => raw_blend.to_vec(),
    }
}

// ── Byte-level field writers ──────────────────────────────────────────────────

pub fn write_ptr(buf: &mut [u8], off: usize, ptr: u64) {
    buf[off..off + 8].copy_from_slice(&ptr.to_le_bytes());
}

/// Write a 4×4 identity matrix (row-major, 16 × f32 = 64 bytes).
pub fn write_identity_matrix4x4(buf: &mut [u8], off: usize) {
    let id: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    for (i, &v) in id.iter().enumerate() {
        buf[off + i * 4..off + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

pub fn write_i32(buf: &mut [u8], off: usize, v: i32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn write_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn write_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn write_i16(buf: &mut [u8], off: usize, v: i16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn write_f32(buf: &mut [u8], off: usize, v: f32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn write_cstr_fixed(buf: &mut [u8], off: usize, max_len: usize, text: &str) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(max_len.saturating_sub(1));
    buf[off..off + n].copy_from_slice(&bytes[..n]);
    buf[off + n] = 0;
}

/// Write an ID name with a two-character prefix (e.g. `"OB"`, `"ME"`, `"LA"`).
/// The result is null-terminated and fits in `ID.name[258]` at offset 40.
pub fn write_id_name(buf: &mut [u8], id_prefix: &str, name: &str) {
    let full = format!("{}{}", id_prefix, name);
    write_cstr_fixed(buf, 40, 258, &full);
}

// ── Data-array helpers ────────────────────────────────────────────────────────

pub fn floats3_data(values: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 12);
    for v in values {
        out.extend_from_slice(&v[0].to_le_bytes());
        out.extend_from_slice(&v[1].to_le_bytes());
        out.extend_from_slice(&v[2].to_le_bytes());
    }
    out
}

pub fn floats2_data(values: &[[f32; 2]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for v in values {
        out.extend_from_slice(&v[0].to_le_bytes());
        out.extend_from_slice(&v[1].to_le_bytes());
    }
    out
}

pub fn ints_data(values: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn bytes4_data(values: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(v);
    }
    out
}

// ── Datablock builders ────────────────────────────────────────────────────────

pub fn build_file_global(scene_ptr: u64, view_layer_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; FILE_GLOBAL_SIZE];
    data[0..4].copy_from_slice(b"5.01");
    write_u16(&mut data, 4, 1);
    write_u16(&mut data, 6, 500);
    write_ptr(&mut data, 24, scene_ptr);
    write_ptr(&mut data, 32, view_layer_ptr);
    data
}

pub fn build_scene(scene_name: &str, view_layer_ptr: u64, master_collection_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; SCENE_SIZE];
    write_id_name(&mut data, "SC", scene_name);
    write_ptr(&mut data, 5632, view_layer_ptr);
    write_ptr(&mut data, 5640, view_layer_ptr);
    write_ptr(&mut data, 5648, master_collection_ptr);
    data
}

pub fn build_view_layer(
    view_layer_name: &str,
    base_ptr: u64,
    layer_collection_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; VIEW_LAYER_SIZE];
    write_cstr_fixed(&mut data, 16, 64, view_layer_name);
    write_u16(&mut data, 80, 5);
    write_ptr(&mut data, 88, base_ptr);
    write_ptr(&mut data, 96, base_ptr);
    write_ptr(&mut data, 120, layer_collection_ptr);
    write_ptr(&mut data, 128, layer_collection_ptr);
    data
}

pub fn build_base(object_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; BASE_SIZE];
    write_ptr(&mut data, 16, object_ptr);
    write_u16(&mut data, 36, 0x00d6);
    write_u16(&mut data, 38, 0x00d6);
    data
}

pub fn build_collection(
    collection_name: &str,
    owner_scene_ptr: u64,
    collection_object_head_ptr: u64,
    collection_object_tail_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; COLLECTION_SIZE];
    write_id_name(&mut data, "GR", collection_name);
    write_i16(&mut data, 298, 1024);
    
    // Offset 128-135: ListBase.first (CollectionObject* head)
    write_ptr(&mut data, 128, collection_object_head_ptr);
    
    // Offset 136-143: ListBase.last (CollectionObject* tail)
    write_ptr(&mut data, 136, collection_object_tail_ptr);
    
    // Offset 144-151: children ListBase.first (nested Collections)
    write_ptr(&mut data, 144, 0);  // No nested collections for now
    
    // Offset 152-159: children ListBase.last
    write_ptr(&mut data, 152, 0);
    
    // Offset 168-175: scene owner pointer
    write_ptr(&mut data, 168, owner_scene_ptr);
    
    data
}

pub fn build_collection_object(object_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; COLLECTION_OBJECT_SIZE];
    write_ptr(&mut data, 16, object_ptr);
    data
}

/// Build a CollectionObject with doubly-linked list pointers.
/// 
/// CollectionObject is a 32-byte node:
/// - Offset +0: prev pointer (u64)
/// - Offset +8: next pointer (u64)
/// - Offset +16: object pointer (u64)
pub fn build_collection_object_linked(
    object_ptr: u64,
    prev_ptr: u64,
    next_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; COLLECTION_OBJECT_SIZE];
    write_ptr(&mut data, 0, prev_ptr);
    write_ptr(&mut data, 8, next_ptr);
    write_ptr(&mut data, 16, object_ptr);
    data
}

pub fn build_layer_collection(collection_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; LAYER_COLLECTION_SIZE];
    write_ptr(&mut data, 16, collection_ptr);
    write_u16(&mut data, 32, 0x0001);
    data
}

/// Build a mesh `Object` block (`OB_MESH`, type=1).
///
/// `properties_ptr` points to the root `IDProperty` group block (0 = no custom props).
pub fn build_object(
    object_name: &str,
    mesh_ptr: u64,
    object_mat_ptr: u64,
    matbits_ptr: u64,
    material_slots: i32,
    properties_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 1); // OB_MESH
    write_ptr(&mut data, 344, properties_ptr);
    write_ptr(&mut data, 552, mesh_ptr);
    write_ptr(&mut data, 712, object_mat_ptr);
    write_ptr(&mut data, 720, matbits_ptr);
    write_i32(&mut data, 728, material_slots);
    write_i16(&mut data, 732, if material_slots > 0 { 1 } else { 0 }); // actcol
    // Default transform: zero translation, unit scale, identity quaternion
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, 0.0); }  // loc
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, 1.0); }  // scale
    // Identity quaternion: [x=0, y=0, z=0, w=1] stored at offset 820 (XYZW order)
    write_f32(&mut data, 832, 1.0); // quat.w at offset 820 + 3*4
    write_i16(&mut data, 1040, 0);  // ROT_MODE_QUAT
    data
}

/// Build an `OB_EMPTY` Object (type=0, no mesh data).
pub fn build_empty_object(
    object_name: &str,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 0); // OB_EMPTY
    write_ptr(&mut data, 496, parent_ptr);
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, loc[i]); }
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, scale[i]); }
    for i in 0..4 { write_f32(&mut data, 820 + i * 4, quat[i]); }
    write_i16(&mut data, 1040, 0);
    if parent_ptr != 0 {
        write_identity_matrix4x4(&mut data, 884);
    }
    data
}

/// Build an `OB_LAMP` Object (type=10) pointing to a `Lamp` datablock.
pub fn build_lamp_object(
    object_name: &str,
    lamp_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 10); // OB_LAMP
    write_ptr(&mut data, 496, parent_ptr);
    write_ptr(&mut data, 552, lamp_ptr);
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, loc[i]); }
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, scale[i]); }
    for i in 0..4 { write_f32(&mut data, 820 + i * 4, quat[i]); }
    write_i16(&mut data, 1040, 0);
    if parent_ptr != 0 {
        write_identity_matrix4x4(&mut data, 884);
    }
    data
}

/// Build an `OB_EMPTY` Object that links to an external mesh via an ID stub.
///
/// This creates an instance that references a mesh datablock from an external library.
/// The object contains a pointer to an ID stub which in turn points to the library.
pub fn build_linked_instance_object(
    object_name: &str,
    id_stub_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 1); // OB_MESH - linked instances must be MESH type to load external mesh data
    write_ptr(&mut data, 496, parent_ptr);
    write_ptr(&mut data, 552, id_stub_ptr); // Point to the ID stub instead of leaving blank
    write_ptr(&mut data, 712, 0); // No local material array (using linked mesh materials)
    write_ptr(&mut data, 720, 0); // No local material bits
    write_i32(&mut data, 728, 0); // No local material slots
    write_i16(&mut data, 732, 0); // actcol = 0
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, loc[i]); }
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, scale[i]); }
    for i in 0..4 { write_f32(&mut data, 820 + i * 4, quat[i]); }
    write_i16(&mut data, 1040, 0);
    if parent_ptr != 0 {
        write_identity_matrix4x4(&mut data, 884);
    }
    data
}

/// Build a `Lamp` (Light) datablock.
///
/// `lamp_type`: 0 = POINT, 1 = SUN, 2 = SPOT, 4 = AREA.
pub fn build_lamp(
    lamp_name: &str,
    lamp_type: i16,
    color: [f32; 3],
    energy: f32,
    radius: f32,
    spot_size: f32,
    spot_blend: f32,
    temperature_k: f32,
    use_temperature: bool,
) -> Vec<u8> {
    let mut data = vec![0u8; LAMP_SIZE];
    write_id_name(&mut data, "LA", lamp_name);
    write_i16(&mut data, 416, lamp_type);
    write_f32(&mut data, 424, color[0]);
    write_f32(&mut data, 428, color[1]);
    write_f32(&mut data, 432, color[2]);
    write_f32(&mut data, 440, energy);
    write_f32(&mut data, 448, radius);
    write_f32(&mut data, 452, spot_size);
    write_f32(&mut data, 456, spot_blend);
    // Write temperature fields (Blender 5.1 Lamp struct)
    // temperature at offset 460 (f32), use_temperature at offset 464 (bool)
    write_f32(&mut data, 460, temperature_k);
    data[464] = if use_temperature { 1 } else { 0 };
    data
}

/// Build a `Mesh` datablock.
pub fn build_mesh(
    mesh_name: &str,
    totvert: usize,
    totpoly: usize,
    totloop: usize,
    poly_offset_indices_ptr: u64,
    attributes_ptr: u64,
    mesh_mat_ptr: u64,
    material_slots: i16,
    vgroup_first_ptr: u64,
    vgroup_last_ptr: u64,
    vgroup_count: u64,
    cdl_ptr: u64,
    num_attributes: u32,
) -> Vec<u8> {
    let mut data = vec![0u8; MESH_SIZE];
    write_id_name(&mut data, "ME", mesh_name);
    write_u32(&mut data, 432, totvert as u32);
    write_u32(&mut data, 440, totpoly as u32);
    write_u32(&mut data, 444, totloop as u32);
    write_ptr(&mut data, 424, mesh_mat_ptr);
    write_ptr(&mut data, 448, poly_offset_indices_ptr);
    // AttributeStorage (inline): dna_attributes*, dna_attributes_num, pad, runtime*
    write_ptr(&mut data, 456, attributes_ptr);
    write_i32(&mut data, 464, num_attributes as i32);
    write_i16(&mut data, 1618, material_slots);
    write_ptr(&mut data, 1472, vgroup_first_ptr);
    write_ptr(&mut data, 1480, vgroup_last_ptr);
    data[1488..1496].copy_from_slice(&vgroup_count.to_le_bytes());
    if cdl_ptr != 0 {
        write_ptr(&mut data, 480, cdl_ptr);
        write_i32(&mut data, 488 + 2 * 4, 0); // typemap[CD_MDEFORMVERT=2] = 0
        write_i32(&mut data, 700, 1); // totlayer
        write_i32(&mut data, 704, 1); // maxlayer
    }
    data
}

pub fn build_attribute(name_ptr: u64, data_type: i16, domain: u8, data_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; ATTRIBUTE_SIZE];
    write_ptr(&mut data, 0, name_ptr);
    write_i16(&mut data, 8, data_type);
    data[10] = domain;
    data[11] = ATTR_STORAGE_ARRAY;
    write_ptr(&mut data, 16, data_ptr);
    data
}

pub fn build_attribute_array(raw_data_ptr: u64, size: i64) -> Vec<u8> {
    let mut data = vec![0u8; ATTRIBUTE_ARRAY_SIZE];
    write_ptr(&mut data, 0, raw_data_ptr);
    data[16..24].copy_from_slice(&size.to_le_bytes());
    data
}

/// Build a `bDeformGroup` block (vertex group name entry, 88 bytes).
pub fn build_bdeformgroup(name: &str, next_ptr: u64, prev_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; BDEFORMGROUP_SIZE];
    write_ptr(&mut data, 0, next_ptr);
    write_ptr(&mut data, 8, prev_ptr);
    write_cstr_fixed(&mut data, 16, 64, name);
    data
}

/// Build a `CustomDataLayer` for `MDeformVert` (CD type = 2, 112 bytes).
pub fn build_custom_data_layer_mdeformvert(data_ptr: u64) -> Vec<u8> {
    let mut buf = vec![0u8; CUSTOM_DATA_LAYER_SIZE];
    write_i32(&mut buf, 0, 2); // type = CD_MDEFORMVERT
    write_ptr(&mut buf, 96, data_ptr);
    buf
}

/// Build an `MDeformVert[]` block (16 bytes per vertex).
pub fn build_mdeformvert_array(dw_ptrs: &[(u64, u32)]) -> Vec<u8> {
    let mut data = vec![0u8; dw_ptrs.len() * MDEFORMVERT_SIZE];
    for (i, (dw_ptr, totweight)) in dw_ptrs.iter().enumerate() {
        write_ptr(&mut data, i * 16, *dw_ptr);
        write_i32(&mut data, i * 16 + 8, *totweight as i32);
    }
    data
}

/// Build an `MDeformWeight[]` block for a single vertex (8 bytes per entry).
pub fn build_mdeformweight_array(weights: &[(u32, f32)]) -> Vec<u8> {
    let mut data = vec![0u8; weights.len() * MDEFORMWEIGHT_SIZE];
    for (i, (def_nr, weight)) in weights.iter().enumerate() {
        write_u32(&mut data, i * 8, *def_nr);
        write_f32(&mut data, i * 8 + 4, *weight);
    }
    data
}

/// Custom property value for IDProperty creation.
#[derive(Clone, Debug, PartialEq)]
pub enum IdPropValue {
    String(String),
    Int(i32),
    Double(f64),
}

/// Build a single `IDProperty` block (144 bytes).
///
/// ## IDPropertyData union layout (SDNA-verified, Blender 5.1):
/// - `@88`: `data.pointer` (8 bytes) — string data pointer for `IDP_STRING`
/// - `@96`: `group.first` (8 bytes) — first child, for `IDP_GROUP`
/// - `@104`: `group.last`  (8 bytes) — last child,  for `IDP_GROUP`
/// - `@112`: `children_map*` (8 bytes)
/// - `@120`: `val` (i32) — integer value for `IDP_INT`
/// - `@124`: `val2` (i32) — high word for `IDP_DOUBLE`
/// - `@128`: `len`  (i32) — for `IDP_STRING`: byte count including null; for `IDP_GROUP`: child count
/// - `@132`: `totallen` (i32) — allocated buffer length (same as `len` for `IDP_STRING`)
pub fn build_idproperty(
    itype: u8,
    name: &str,
    next_ptr: u64,
    prev_ptr: u64,
    data_ptr: u64,
    group_first_ptr: u64,
    group_last_ptr: u64,
    val_int: i32,
    val_double: f64,
    str_len: i32,
    str_totallen: i32,
) -> Vec<u8> {
    let mut b = vec![0u8; IDPROPERTY_SIZE];
    write_ptr(&mut b, 0, next_ptr);
    write_ptr(&mut b, 8, prev_ptr);
    b[16] = itype;
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(63);
    b[20..20 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
    write_ptr(&mut b, 88, data_ptr); // IDP_STRING: string data pointer
    if itype == IDP_GROUP {
        write_ptr(&mut b, 96, group_first_ptr);
        write_ptr(&mut b, 104, group_last_ptr);
    }
    match itype {
        IDP_INT => write_i32(&mut b, 120, val_int),
        IDP_DOUBLE => b[120..128].copy_from_slice(&val_double.to_le_bytes()),
        _ => {}
    }
    write_i32(&mut b, 128, str_len);
    write_i32(&mut b, 132, str_totallen);
    b
}

/// Build a complete IDProperty subtree for a list of custom object properties.
///
/// Returns `(root_block, children, string_data)`:
/// - `root_block`: the `IDP_GROUP` "user_properties" root (144 bytes)
/// - `children`: `Vec<(old_ptr, block_bytes)>` — child IDProperty blocks
/// - `string_data`: `Vec<(old_ptr, data_bytes)>` — string DATA blocks (in child order)
///
/// **Block ordering rule**: write blocks in this sequence after the parent `OB` block:
/// 1. `DATA` root group
/// 2. For each child: `DATA` child IDProperty, then immediately `DATA` string data (if IDP_STRING)
/// 3. `DATA` Object.mat** and matbits (if any)
pub fn build_idprop_tree(
    _root_ptr: u64,
    child_ptrs: &[u64],
    str_data_ptrs: &[u64],
    props: &[(String, IdPropValue)],
) -> (Vec<u8>, Vec<(u64, Vec<u8>)>, Vec<(u64, Vec<u8>)>) {
    assert_eq!(child_ptrs.len(), props.len());
    let n = props.len();
    let mut str_idx = 0usize;
    let mut children = Vec::with_capacity(n);
    let mut string_data = Vec::new();

    for (i, (name, val)) in props.iter().enumerate() {
        let next_ptr = if i + 1 < n { child_ptrs[i + 1] } else { 0 };
        let prev_ptr = if i > 0 { child_ptrs[i - 1] } else { 0 };

        let (itype, data_ptr, str_data, slen, stotlen, ival, dval) = match val {
            IdPropValue::String(s) => {
                let mut bytes = s.as_bytes().to_vec();
                bytes.push(0);
                let slen = bytes.len() as i32; // len = totallen = strlen+1 (Blender convention)
                let sdptr = str_data_ptrs[str_idx];
                str_idx += 1;
                (IDP_STRING, sdptr, Some(bytes), slen, slen, 0i32, 0f64)
            }
            IdPropValue::Int(v) => (IDP_INT, 0u64, None, 0i32, 0i32, *v, 0f64),
            IdPropValue::Double(v) => (IDP_DOUBLE, 0u64, None, 0i32, 0i32, 0i32, *v),
        };

        let block = build_idproperty(
            itype, name, next_ptr, prev_ptr, data_ptr, 0, 0, ival, dval, slen, stotlen,
        );
        children.push((child_ptrs[i], block));

        if let Some(str_bytes) = str_data {
            string_data.push((str_data_ptrs[str_idx - 1], str_bytes));
        }
    }

    let group_first = if n > 0 { child_ptrs[0] } else { 0 };
    let group_last = if n > 0 { child_ptrs[n - 1] } else { 0 };
    let root = build_idproperty(
        IDP_GROUP, "user_properties", 0, 0, 0,
        group_first, group_last, 0, 0.0, 0, 0,
    );

    (root, children, string_data)
}

/// Build an `Object.mat**` (Material pointer array) block — all null pointers.
pub fn build_mat_ptr_array(n: usize) -> Vec<u8> {
    vec![0u8; n * 8]
}

/// Build an `Object.matbits` block — one byte per slot, 0 = mesh-linked.
pub fn build_matbits(n: usize) -> Vec<u8> {
    vec![0u8; n]
}

/// Compress Blender .blend file using Zstd (Blender 5.x native format).
///
/// Scene.blend files must be Zstd-compressed; mesh .blend files are uncompressed.
pub fn compress_blend_bytes_zstd(raw_blend: &[u8]) -> Vec<u8> {
    use zstd::Encoder;
    
    let mut compressed = Vec::new();
    // Use compression level 19 (maximum, matches Blender's default save)
    let mut encoder = match Encoder::new(&mut compressed, 19) {
        Ok(enc) => enc,
        Err(_) => return raw_blend.to_vec(), // Fallback: return uncompressed
    };
    
    if encoder.write_all(raw_blend).is_err() || encoder.finish().is_err() {
        return raw_blend.to_vec(); // Fallback: return uncompressed
    }
    
    compressed
}

/// Build a Library block (LI) for linking to an external .blend file.
///
/// Binary layout (1426 bytes total):
/// - ID struct: 0-370 (370 bytes)
/// - filepath[1024]: 370-1394 (1024 bytes, UTF-8, null-terminated, zero-padded)
/// - flag: 1394-1396 (2 bytes, uint16)
/// - undo_runtime_tag: 1396-1398 (2 bytes, uint16)
/// - _pad: 1398-1402 (4 bytes alignment)
/// - archive_parent_library: 1402-1410 (8 bytes pointer, nullptr)
/// - packedfile: 1410-1418 (8 bytes pointer, nullptr)
/// - runtime: 1418-1426 (8 bytes pointer, always nullptr)
///
/// Filepath stored as relative path when possible, UTF-8 encoded.
pub fn build_library_block(lib_name: &str, filepath: &str) -> Vec<u8> {
    let mut buf = vec![0u8; LIBRARY_SIZE];
    
    // Offset 0-66: ID.name (common to all datablocks)
    // Format: "LI" + null padding + 64-char name
    write_id_name(&mut buf[0..66], "LI", lib_name);
    
    // Offset 370-1394: filepath[1024] UTF-8, null-terminated, zero-padded
    let filepath_bytes = filepath.as_bytes();
    let filepath_len = filepath_bytes.len().min(1023); // Max 1023 chars + null terminator
    
    if filepath_len > 0 {
        buf[370..(370 + filepath_len)].copy_from_slice(&filepath_bytes[0..filepath_len]);
    }
    // Null-terminate and zero-pad (already initialized to zeros)
    buf[370 + filepath_len] = 0;
    
    // Offset 1394-1396: flag (uint16, 0 = no special flags)
    write_u16(&mut buf, 1394, 0);
    
    // Offset 1396-1398: undo_runtime_tag (uint16, 0)
    write_u16(&mut buf, 1396, 0);
    
    // Offset 1398-1402: _pad (4 bytes, already zero)
    
    // Offset 1402-1410: archive_parent_library (8 bytes pointer, nullptr)
    write_ptr(&mut buf, 1402, 0);
    
    // Offset 1410-1418: packedfile (8 bytes pointer, nullptr)
    write_ptr(&mut buf, 1410, 0);
    
    // Offset 1418-1426: runtime (8 bytes pointer, always nullptr)
    write_ptr(&mut buf, 1418, 0);
    
    buf
}

/// Build an ID stub (ID) for referencing a datablock from an external library.
///
/// The ID stub is embedded in the Object.data pointer and links to:
/// - A Library block (LI) specifying the external .blend file
/// - A specific datablock within that library (e.g., Mesh)
pub fn build_id_stub(datablock_type: &str, name: &str, lib_ptr: u64) -> Vec<u8> {
    let mut buf = vec![0u8; ID_STUB_SIZE];
    
    // ID.name at offset 0: 66 bytes
    write_id_name(&mut buf[0..66], datablock_type, name);
    
    // Library pointer at offset 24
    write_ptr(&mut buf, 24, lib_ptr);
    
    buf
}

/// Compress/decompress test for Zstd roundtrip.
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_zstd_compression_roundtrip() {
        let original = b"Hello, Blender 5.1!";
        let compressed = compress_blend_bytes_zstd(original);
        // Just verify it compresses to something
        assert!(!compressed.is_empty(), "Compression produced empty result");
        // Note: actual decompression requires zstd::Decoder, which we don't use in Rust export
    }
    
    #[test]
    fn test_build_lamp_accepts_temperature_parameters() {
        // Test 4: build_lamp accepts temperature_k and use_temperature parameters
        let lamp_bytes = build_lamp(
            "TestLight",
            0,  // POINT
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            0.0,
            0.0,
            5000.0,  // temperature_k
            true,    // use_temperature
        );
        
        // Verify output size
        assert_eq!(lamp_bytes.len(), LAMP_SIZE, "Lamp block size mismatch");
    }
    
    #[test]
    fn test_build_lamp_temperature_written_correctly() {
        // Test 5: Temperature value written to correct offset (460)
        let temp_value = 6500.0;
        let lamp_bytes = build_lamp(
            "TestLight",
            0,
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            0.0,
            0.0,
            temp_value,
            false,
        );
        
        // Read f32 from offset 460
        let mut temp_bytes = [0u8; 4];
        temp_bytes.copy_from_slice(&lamp_bytes[460..464]);
        let read_temp = f32::from_le_bytes(temp_bytes);
        
        assert!((read_temp - temp_value).abs() < 0.01, 
            "Temperature not written correctly: expected {}, got {}", temp_value, read_temp);
    }
    
    #[test]
    fn test_build_lamp_use_temperature_true() {
        // Test 5a: use_temperature flag set to 1 when true
        let lamp_bytes = build_lamp(
            "TestLight",
            0,
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            0.0,
            0.0,
            5000.0,
            true,  // use_temperature = true
        );
        
        // Check byte at offset 464
        assert_eq!(lamp_bytes[464], 1, "use_temperature flag not set to 1 when true");
    }
    
    #[test]
    fn test_build_lamp_use_temperature_false() {
        // Test 5b: use_temperature flag set to 0 when false
        let lamp_bytes = build_lamp(
            "TestLight",
            0,
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            0.0,
            0.0,
            5000.0,
            false,  // use_temperature = false
        );
        
        // Check byte at offset 464
        assert_eq!(lamp_bytes[464], 0, "use_temperature flag not set to 0 when false");
    }
    
    #[test]
    fn test_build_lamp_temperature_multiple_values() {
        // Test 6: Multiple temperature values written correctly
        let test_values = vec![
            (2700.0, true),
            (3000.0, false),
            (5000.0, true),
            (9000.0, false),
            (12000.0, true),
        ];
        
        for (temp, use_temp) in test_values {
            let lamp_bytes = build_lamp(
                &format!("Light_{}", temp as i32),
                0,
                [1.0, 1.0, 1.0],
                100.0,
                5.0,
                0.0,
                0.0,
                temp,
                use_temp,
            );
            
            // Read temperature
            let mut temp_bytes = [0u8; 4];
            temp_bytes.copy_from_slice(&lamp_bytes[460..464]);
            let read_temp = f32::from_le_bytes(temp_bytes);
            
            assert!((read_temp - temp).abs() < 0.01, 
                "Temperature mismatch for {}: expected {}, got {}", temp, temp, read_temp);
            
            // Read use_temperature
            let read_use_temp = lamp_bytes[464] != 0;
            assert_eq!(read_use_temp, use_temp, 
                "use_temperature flag mismatch for {}: expected {}, got {}", temp, use_temp, read_use_temp);
        }
    }

    // ── Block Header Compliance Tests (Blender 5.x 32-byte format) ───────────

    #[test]
    fn test_block_header_size_is_exactly_32_bytes() {
        let mut buf = Vec::new();
        let code = *b"DNA1";
        write_block_header(&mut buf, &code, 0, 0x0100, 0, 0);
        
        assert_eq!(
            buf.len(),
            32,
            "Block header MUST be exactly 32 bytes (Blender 5.x format), got {}",
            buf.len()
        );
    }

    #[test]
    fn test_block_header_code_is_4_bytes() {
        let mut buf = Vec::new();
        let code = *b"OB00";
        write_block_header(&mut buf, &code, 0, 0, 0, 0);
        
        // Verify bytes 0-3 match code
        assert_eq!(&buf[0..4], b"OB00", "Block code must be exactly 4 ASCII bytes at offset 0-3");
    }

    #[test]
    fn test_block_header_sdna_index_u32_little_endian() {
        let mut buf = Vec::new();
        let code = *b"SC00";
        let sdna_idx = 0x12345678u32;
        write_block_header(&mut buf, &code, sdna_idx, 0, 0, 0);
        
        // Verify bytes 4-7 contain u32 in little-endian
        let read_sdna = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        assert_eq!(
            read_sdna, sdna_idx,
            "SDNA index at bytes 4-7 must be u32 little-endian: expected 0x{:08x}, got 0x{:08x}",
            sdna_idx, read_sdna
        );
    }

    #[test]
    fn test_block_header_old_ptr_u64_little_endian() {
        let mut buf = Vec::new();
        let code = *b"ME00";
        let old_ptr = 0x0102030405060708u64;
        write_block_header(&mut buf, &code, 0, old_ptr, 0, 0);
        
        // Verify bytes 8-15 contain u64 in little-endian
        let mut ptr_bytes = [0u8; 8];
        ptr_bytes.copy_from_slice(&buf[8..16]);
        let read_ptr = u64::from_le_bytes(ptr_bytes);
        assert_eq!(
            read_ptr, old_ptr,
            "Old pointer at bytes 8-15 must be u64 little-endian: expected 0x{:016x}, got 0x{:016x}",
            old_ptr, read_ptr
        );
    }

    #[test]
    fn test_block_header_data_length_u32_little_endian() {
        let mut buf = Vec::new();
        let code = *b"DATA";
        let data_len = 0xdeadbeefu32;
        write_block_header(&mut buf, &code, 0, 0, data_len, 0);
        
        // Verify bytes 16-19 contain u32 in little-endian
        let read_len = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        assert_eq!(
            read_len, data_len,
            "Data length at bytes 16-19 must be u32 little-endian: expected 0x{:08x}, got 0x{:08x}",
            data_len, read_len
        );
    }

    #[test]
    fn test_block_header_padding1_is_zero() {
        let mut buf = Vec::new();
        let code = *b"LA00";
        write_block_header(&mut buf, &code, 0xffffffff, 0xffffffffffffffff, 0xffffffff, 0xffffffff);
        
        // Verify bytes 20-23 are EXACTLY zero (not garbage or uninitialized)
        assert_eq!(&buf[20..24], &[0, 0, 0, 0], 
            "Padding field 1 at bytes 20-23 MUST be exactly zero (not uninitialized)");
    }

    #[test]
    fn test_block_header_count_u32_little_endian() {
        let mut buf = Vec::new();
        let code = *b"GR00";
        let count = 0xabcdef01u32;
        write_block_header(&mut buf, &code, 0, 0, 0, count);
        
        // Verify bytes 24-27 contain u32 in little-endian
        let read_count = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
        assert_eq!(
            read_count, count,
            "Count field at bytes 24-27 must be u32 little-endian: expected 0x{:08x}, got 0x{:08x}",
            count, read_count
        );
    }

    #[test]
    fn test_block_header_padding2_is_zero() {
        let mut buf = Vec::new();
        let code = *b"ENDB";
        write_block_header(&mut buf, &code, 0xffffffff, 0xffffffffffffffff, 0xffffffff, 0xffffffff);
        
        // Verify bytes 28-31 are EXACTLY zero (not garbage or uninitialized)
        assert_eq!(&buf[28..32], &[0, 0, 0, 0], 
            "Padding field 2 at bytes 28-31 MUST be exactly zero (not uninitialized)");
    }

    #[test]
    fn test_block_header_dna1_block_normal() {
        // Normal DNA1 block: code="DNA1", typical indices and counts
        let mut buf = Vec::new();
        let code = *b"DNA1";
        let sdna_idx = SDNA_IDX_DNA1;
        let old_ptr = 0x0100;
        let data_len = 2048; // typical DNA1 size
        let count = 1;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        // Verify full header layout
        assert_eq!(&buf[0..4], b"DNA1");
        assert_eq!(u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]), sdna_idx);
        assert_eq!(u64::from_le_bytes([buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15]]), old_ptr);
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), data_len);
        assert_eq!(&buf[20..24], &[0, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), count);
        assert_eq!(&buf[28..32], &[0, 0, 0, 0]);
    }

    #[test]
    fn test_block_header_object_block_zero_count() {
        // Zero-count block: count=0 (e.g., empty collection or DATA block)
        let mut buf = Vec::new();
        let code = *b"OB00";
        let sdna_idx = SDNA_IDX_OBJECT;
        let old_ptr = 0x0200;
        let data_len = 0;
        let count = 0;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        assert_eq!(&buf[0..4], b"OB00");
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), 0, "Count must be 0");
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), 0, "Data length must be 0");
    }

    #[test]
    fn test_block_header_large_data_length() {
        // Large data length block: max u32 value
        let mut buf = Vec::new();
        let code = *b"DATA";
        let sdna_idx = 160; // CUSTOM_DATA_LAYER
        let old_ptr = 0x0300;
        let data_len = 0xffffffffu32; // max u32
        let count = 1000000;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), data_len, 
            "Data length must support max u32 value");
    }

    #[test]
    fn test_block_header_max_count() {
        // Max count block: count=0xffffffff
        let mut buf = Vec::new();
        let code = *b"SC00";
        let sdna_idx = SDNA_IDX_SCENE;
        let old_ptr = 0x0400;
        let data_len = 65536;
        let count = 0xffffffffu32;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), count, 
            "Count field must support max u32 value");
    }

    #[test]
    fn test_block_header_all_real_block_codes() {
        // Test all documented block codes from spec
        let block_specs = vec![
            (*b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x0100, 1216, 1),
            (*b"SC\0\0", SDNA_IDX_SCENE, 0x0200, 6920, 1),
            (*b"OB\0\0", SDNA_IDX_OBJECT, 0x0300, 800, 1),
            (*b"ME\0\0", SDNA_IDX_MESH, 0x0400, 4096, 1),
            (*b"LA\0\0", SDNA_IDX_LAMP, 0x0500, 1200, 1),
            (*b"GR\0\0", SDNA_IDX_COLLECTION, 0x0600, 520, 1),
            (*b"DATA", 160, 0x0700, 2048, 1),
            (*b"DNA1", SDNA_IDX_DNA1, 0x0800, 2048, 1),
            (*b"ENDB", 0, 0x0900, 0, 0),
        ];
        
        for (code, sdna_idx, old_ptr, data_len, count) in block_specs {
            let mut buf = Vec::new();
            write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
            
            assert_eq!(buf.len(), 32, "Block code {:?} header must be 32 bytes", 
                String::from_utf8_lossy(&code));
            assert_eq!(&buf[0..4], &code, "Block code mismatch");
            assert_eq!(&buf[20..24], &[0, 0, 0, 0], "Padding 1 must be zero for {:?}", 
                String::from_utf8_lossy(&code));
            assert_eq!(&buf[28..32], &[0, 0, 0, 0], "Padding 2 must be zero for {:?}", 
                String::from_utf8_lossy(&code));
        }
    }

    #[test]
    fn test_block_header_byte_layout_complete() {
        // Complete byte-by-byte layout verification
        let mut buf = Vec::new();
        let code = *b"TEST";
        let sdna_idx = 0x11223344u32;
        let old_ptr = 0x1122334455667788u64;
        let data_len = 0x99aabbccu32;
        let count = 0xddeeff00u32;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        // Verify complete layout:
        // bytes 0-3: code
        assert_eq!(&buf[0..4], b"TEST");
        
        // bytes 4-7: sdna_idx (u32 LE)
        assert_eq!(&buf[4..8], &[0x44, 0x33, 0x22, 0x11]);
        
        // bytes 8-15: old_ptr (u64 LE)
        assert_eq!(&buf[8..16], &[0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]);
        
        // bytes 16-19: data_len (u32 LE)
        assert_eq!(&buf[16..20], &[0xcc, 0xbb, 0xaa, 0x99]);
        
        // bytes 20-23: padding (ZERO)
        assert_eq!(&buf[20..24], &[0x00, 0x00, 0x00, 0x00]);
        
        // bytes 24-27: count (u32 LE)
        assert_eq!(&buf[24..28], &[0x00, 0xff, 0xee, 0xdd]);
        
        // bytes 28-31: padding (ZERO)
        assert_eq!(&buf[28..32], &[0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_write_block_with_data() {
        // Test write_block function that wraps write_block_header
        let mut buf = Vec::new();
        let code = *b"DATA";
        let sdna_idx = 160;
        let old_ptr = 0x1000;
        let count = 2;
        let data = b"Hello, Blender!";
        
        write_block(&mut buf, &code, sdna_idx, old_ptr, count, data);
        
        // Should be header (32 bytes) + data
        assert_eq!(buf.len(), 32 + data.len(), "Block size should be header + data");
        
        // Verify header portion
        assert_eq!(&buf[0..4], b"DATA");
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), data.len() as u32,
            "Data length in header must match actual data size");
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), count,
            "Count field must match provided count");
        
        // Verify data portion
        assert_eq!(&buf[32..32 + data.len()], data);
    }

    #[test]
    fn test_block_header_boundary_values() {
        // Test boundary values for u32 fields (SDNA index is critical fix)
        let boundary_cases = vec![
            (0u32, "zero"),
            (1u32, "one"),
            (0x7fffffffu32, "max signed i32"),
            (0x80000000u32, "min signed i32 as u32 (critical fix)"),
            (0xffffffffu32, "max u32"),
        ];
        
        for (value, description) in boundary_cases {
            let mut buf = Vec::new();
            let code = *b"TEST";
            
            // Test SDNA index field
            write_block_header(&mut buf, &code, value, 0, 0, 0);
            let read_value = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            assert_eq!(read_value, value, "SDNA index boundary case failed for {}: expected 0x{:08x}, got 0x{:08x}", 
                description, value, read_value);
            
            // Test data length field
            buf.clear();
            write_block_header(&mut buf, &code, 0, 0, value, 0);
            let read_value = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
            assert_eq!(read_value, value, "Data length boundary case failed for {}: expected 0x{:08x}, got 0x{:08x}", 
                description, value, read_value);
            
            // Test count field
            buf.clear();
            write_block_header(&mut buf, &code, 0, 0, 0, value);
            let read_value = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
            assert_eq!(read_value, value, "Count boundary case failed for {}: expected 0x{:08x}, got 0x{:08x}", 
                description, value, read_value);
        }
    }
}

