//! Unit tests for `starbreaker-blend`.
//!
//! These tests verify binary serialisation at the byte level and do NOT require
//! a running Blender instance.  Round-trip integration tests (loading in Blender)
//! live in Phase 67J.

use starbreaker_blend::*;

// ── Header / block serialisation ─────────────────────────────────────────────

#[test]
fn blend_magic_is_17_bytes() {
    assert_eq!(BLEND_MAGIC.len(), 17);
    assert_eq!(BLEND_MAGIC, b"BLENDER17-01v0501");
}

#[test]
fn write_block_header_is_32_bytes() {
    let mut out = Vec::new();
    write_block_header(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, 0x1000, 1288, 1);
    assert_eq!(out.len(), 32);
}

#[test]
fn write_block_header_layout() {
    let mut out = Vec::new();
    write_block_header(&mut out, b"ME\0\0", 42, 0xdeadbeef_cafebabe_u64, 100, 3);
    assert_eq!(&out[0..4], b"ME\0\0");                                    // code
    assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 42);    // sdna_idx
    assert_eq!(u64::from_le_bytes(out[8..16].try_into().unwrap()), 0xdeadbeef_cafebabe_u64); // old_ptr
    assert_eq!(u32::from_le_bytes(out[16..20].try_into().unwrap()), 100); // data_len
    assert_eq!(u32::from_le_bytes(out[20..24].try_into().unwrap()), 0);   // zero
    assert_eq!(u32::from_le_bytes(out[24..28].try_into().unwrap()), 3);   // count
    assert_eq!(u32::from_le_bytes(out[28..32].try_into().unwrap()), 0);   // zero2
}

#[test]
fn write_block_appends_data_after_header() {
    let payload = b"ABCD";
    let mut out = Vec::new();
    write_block(&mut out, b"DATA", 0, 0x2000, 1, payload);
    assert_eq!(out.len(), 32 + 4);
    assert_eq!(&out[32..], b"ABCD");
}

// ── PtrAlloc ──────────────────────────────────────────────────────────────────

#[test]
fn ptr_alloc_increments_by_0x10() {
    let mut pa = PtrAlloc::new(0x1000);
    assert_eq!(pa.alloc(), 0x1000);
    assert_eq!(pa.alloc(), 0x1010);
    assert_eq!(pa.alloc(), 0x1020);
}

// ── Struct size sanity checks ─────────────────────────────────────────────────

#[test]
fn object_size_is_1288() {
    let ob = build_object("TestOb", 0, 0, 0, 0, 0);
    assert_eq!(ob.len(), OBJECT_SIZE);
    assert_eq!(OBJECT_SIZE, 1288);
}

#[test]
fn mesh_size_is_1960() {
    let me = build_mesh("TestMesh", 4, 1, 4, 0, 0, 0, 0, 0, 0, 0, 0, 5);
    assert_eq!(me.len(), MESH_SIZE);
    assert_eq!(MESH_SIZE, 1960);
}

#[test]
fn lamp_size_is_568() {
    let la = build_lamp("TestLamp", 0, [1.0, 1.0, 1.0], 10.0, 0.1, 0.5, 0.1, 6500.0, false);
    assert_eq!(la.len(), LAMP_SIZE);
    assert_eq!(LAMP_SIZE, 568);
}

#[test]
fn empty_object_size_is_1288() {
    let ob = build_empty_object("Empty", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 0);
    assert_eq!(ob.len(), OBJECT_SIZE);
}

#[test]
fn idproperty_size_is_144() {
    let b = build_idproperty(IDP_INT, "x", 0, 0, 0, 0, 0, 42, 0.0, 0, 0);
    assert_eq!(b.len(), IDPROPERTY_SIZE);
    assert_eq!(IDPROPERTY_SIZE, 144);
}

// ── Object field layout ───────────────────────────────────────────────────────

#[test]
fn object_type_at_416() {
    // OB_MESH = 1
    let ob = build_object("TestOb", 0x2000, 0, 0, 0, 0);
    assert_eq!(i16::from_le_bytes(ob[416..418].try_into().unwrap()), 1);
}

#[test]
fn object_mesh_ptr_at_552() {
    let ob = build_object("TestOb", 0xdead_0000_u64, 0, 0, 0, 0);
    assert_eq!(u64::from_le_bytes(ob[552..560].try_into().unwrap()), 0xdead_0000_u64);
}

#[test]
fn object_id_properties_ptr_at_344() {
    let ob = build_object("TestOb", 0, 0, 0, 0, 0xbeef_1234_u64);
    assert_eq!(u64::from_le_bytes(ob[344..352].try_into().unwrap()), 0xbeef_1234_u64);
}

#[test]
fn object_mat_ptr_at_712() {
    let ob = build_object("TestOb", 0, 0xaaaa_u64, 0xbbbb_u64, 2, 0);
    assert_eq!(u64::from_le_bytes(ob[712..720].try_into().unwrap()), 0xaaaa_u64);
    assert_eq!(u64::from_le_bytes(ob[720..728].try_into().unwrap()), 0xbbbb_u64);
    assert_eq!(i32::from_le_bytes(ob[728..732].try_into().unwrap()), 2);
}

#[test]
fn object_parent_ptr_at_496() {
    let ob = build_empty_object("Child", [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0xcafe_u64);
    assert_eq!(u64::from_le_bytes(ob[496..504].try_into().unwrap()), 0xcafe_u64);
}

#[test]
fn empty_object_parentinv_is_identity_when_parented() {
    let ob = build_empty_object("Child", [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0x1000_u64);
    // parentinv @884: 4×4 f32 identity matrix
    let expected: [f32; 16] = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
    for (i, &v) in expected.iter().enumerate() {
        let got = f32::from_le_bytes(ob[884 + i*4..884 + i*4 + 4].try_into().unwrap());
        assert_eq!(got, v, "parentinv[{i}] mismatch");
    }
}

#[test]
fn empty_object_parentinv_is_zero_when_unparented() {
    let ob = build_empty_object("Root", [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0);
    // parentinv should be all zeros when parent_ptr == 0
    assert!(ob[884..948].iter().all(|&b| b == 0));
}

#[test]
fn lamp_object_type_at_416() {
    let ob = build_lamp_object("L", 0x3000, [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0);
    assert_eq!(i16::from_le_bytes(ob[416..418].try_into().unwrap()), 10);
}

// ── Mesh field layout ─────────────────────────────────────────────────────────

#[test]
fn mesh_totvert_at_432() {
    let me = build_mesh("M", 7, 2, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    assert_eq!(u32::from_le_bytes(me[432..436].try_into().unwrap()), 7);
}

#[test]
fn mesh_attributes_ptr_at_456() {
    let me = build_mesh("M", 4, 1, 4, 0, 0xcccc_u64, 0, 0, 0, 0, 0, 0, 2);
    assert_eq!(u64::from_le_bytes(me[456..464].try_into().unwrap()), 0xcccc_u64);
}

// ── Lamp field layout ─────────────────────────────────────────────────────────

#[test]
fn lamp_type_point_at_416() {
    let la = build_lamp("L", 0, [1.0, 0.5, 0.0], 100.0, 0.25, 0.0, 0.0, 6500.0, false);
    assert_eq!(i16::from_le_bytes(la[416..418].try_into().unwrap()), 0);
}

#[test]
fn lamp_energy_at_440() {
    let la = build_lamp("L", 0, [1.0, 1.0, 1.0], 77.5, 0.0, 0.0, 0.0, 6500.0, false);
    assert_eq!(f32::from_le_bytes(la[440..444].try_into().unwrap()), 77.5_f32);
}

#[test]
fn lamp_color_fields() {
    let la = build_lamp("L", 0, [0.1, 0.2, 0.3], 1.0, 0.0, 0.0, 0.0, 6500.0, false);
    assert!((f32::from_le_bytes(la[424..428].try_into().unwrap()) - 0.1_f32).abs() < 1e-6);
    assert!((f32::from_le_bytes(la[428..432].try_into().unwrap()) - 0.2_f32).abs() < 1e-6);
    assert!((f32::from_le_bytes(la[432..436].try_into().unwrap()) - 0.3_f32).abs() < 1e-6);
}

// ── IDProperty field layout ───────────────────────────────────────────────────

#[test]
fn idproperty_type_at_16() {
    let b = build_idproperty(IDP_STRING, "foo", 0, 0, 0, 0, 0, 0, 0.0, 5, 5);
    assert_eq!(b[16], IDP_STRING);
}

#[test]
fn idproperty_name_at_20() {
    let b = build_idproperty(IDP_INT, "MY_PROP", 0, 0, 0, 0, 0, 0, 0.0, 0, 0);
    let name = b[20..84].split(|&c| c == 0).next().unwrap();
    assert_eq!(name, b"MY_PROP");
}

#[test]
fn idproperty_int_val_at_120() {
    let b = build_idproperty(IDP_INT, "n", 0, 0, 0, 0, 0, -7, 0.0, 0, 0);
    assert_eq!(i32::from_le_bytes(b[120..124].try_into().unwrap()), -7);
}

#[test]
fn idproperty_string_data_ptr_at_88() {
    let b = build_idproperty(IDP_STRING, "s", 0, 0, 0xdead_u64, 0, 0, 0, 0.0, 4, 4);
    assert_eq!(u64::from_le_bytes(b[88..96].try_into().unwrap()), 0xdead_u64);
}

#[test]
fn idproperty_group_first_at_96_last_at_104() {
    let b = build_idproperty(IDP_GROUP, "user_properties", 0, 0, 0, 0x1111_u64, 0x2222_u64, 0, 0.0, 0, 0);
    assert_eq!(u64::from_le_bytes(b[96..104].try_into().unwrap()), 0x1111_u64);
    assert_eq!(u64::from_le_bytes(b[104..112].try_into().unwrap()), 0x2222_u64);
}

#[test]
fn idproperty_string_len_at_128() {
    let b = build_idproperty(IDP_STRING, "s", 0, 0, 0, 0, 0, 0, 0.0, 31, 31);
    assert_eq!(i32::from_le_bytes(b[128..132].try_into().unwrap()), 31);
    assert_eq!(i32::from_le_bytes(b[132..136].try_into().unwrap()), 31);
}

// ── build_idprop_tree ─────────────────────────────────────────────────────────

#[test]
fn idprop_tree_single_int() {
    let mut pa = PtrAlloc::new(0x1000);
    let root_ptr = pa.alloc();
    let child_ptr = pa.alloc();
    let props = vec![("N".to_string(), IdPropValue::Int(42))];
    let (root, children, strings) = build_idprop_tree(root_ptr, &[child_ptr], &[], &props);
    assert_eq!(root.len(), IDPROPERTY_SIZE);
    assert_eq!(root[16], IDP_GROUP);
    // group.first @96 = child_ptr
    assert_eq!(u64::from_le_bytes(root[96..104].try_into().unwrap()), child_ptr);
    assert_eq!(children.len(), 1);
    assert_eq!(strings.len(), 0);
    let (_, cblock) = &children[0];
    assert_eq!(i32::from_le_bytes(cblock[120..124].try_into().unwrap()), 42);
}

#[test]
fn idprop_tree_string_produces_data_block() {
    let mut pa = PtrAlloc::new(0x1000);
    let root_ptr = pa.alloc();
    let child_ptr = pa.alloc();
    let str_data_ptr = pa.alloc();
    let props = vec![("TEMPLATE".to_string(), IdPropValue::String("foo".to_string()))];
    let (_, children, strings) = build_idprop_tree(root_ptr, &[child_ptr], &[str_data_ptr], &props);
    assert_eq!(children.len(), 1);
    assert_eq!(strings.len(), 1);
    let (_, bytes) = &strings[0];
    assert_eq!(bytes, b"foo\0");
    // IDP_STRING: len = 4 (strlen+1)
    let (_, cblock) = &children[0];
    assert_eq!(i32::from_le_bytes(cblock[128..132].try_into().unwrap()), 4);
}

#[test]
fn idprop_tree_linked_list_chain() {
    let mut pa = PtrAlloc::new(0x1000);
    let root_ptr = pa.alloc();
    let ptrs: Vec<u64> = (0..3).map(|_| pa.alloc()).collect();
    let props = vec![
        ("A".to_string(), IdPropValue::Int(1)),
        ("B".to_string(), IdPropValue::Int(2)),
        ("C".to_string(), IdPropValue::Int(3)),
    ];
    let (_, children, _) = build_idprop_tree(root_ptr, &ptrs, &[], &props);
    // A.next = ptrs[1], A.prev = 0
    let (_, a) = &children[0];
    assert_eq!(u64::from_le_bytes(a[0..8].try_into().unwrap()), ptrs[1]);
    assert_eq!(u64::from_le_bytes(a[8..16].try_into().unwrap()), 0);
    // B.next = ptrs[2], B.prev = ptrs[0]
    let (_, b) = &children[1];
    assert_eq!(u64::from_le_bytes(b[0..8].try_into().unwrap()), ptrs[2]);
    assert_eq!(u64::from_le_bytes(b[8..16].try_into().unwrap()), ptrs[0]);
    // C.next = 0, C.prev = ptrs[1]
    let (_, c) = &children[2];
    assert_eq!(u64::from_le_bytes(c[0..8].try_into().unwrap()), 0);
    assert_eq!(u64::from_le_bytes(c[8..16].try_into().unwrap()), ptrs[1]);
}

// ── Vertex group helpers ───────────────────────────────────────────────────────

#[test]
fn bdeformgroup_name_at_16() {
    let b = build_bdeformgroup("GroupA", 0, 0);
    assert_eq!(b.len(), BDEFORMGROUP_SIZE);
    let name = b[16..80].split(|&c| c == 0).next().unwrap();
    assert_eq!(name, b"GroupA");
}

#[test]
fn mdeformweight_layout() {
    let b = build_mdeformweight_array(&[(2, 0.75)]);
    assert_eq!(b.len(), MDEFORMWEIGHT_SIZE);
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 2);
    assert!((f32::from_le_bytes(b[4..8].try_into().unwrap()) - 0.75_f32).abs() < 1e-6);
}

#[test]
fn mdeformvert_layout() {
    let b = build_mdeformvert_array(&[(0xabcd_u64, 2)]);
    assert_eq!(b.len(), MDEFORMVERT_SIZE);
    assert_eq!(u64::from_le_bytes(b[0..8].try_into().unwrap()), 0xabcd_u64);
    assert_eq!(i32::from_le_bytes(b[8..12].try_into().unwrap()), 2);
}

// ── Attribute helpers ─────────────────────────────────────────────────────────

#[test]
fn attribute_size_is_24() {
    let a = build_attribute(0x1000, ATTR_TYPE_FLOAT3, ATTR_DOMAIN_POINT, 0x2000);
    assert_eq!(a.len(), ATTRIBUTE_SIZE);
}

#[test]
fn attribute_array_size_is_32() {
    let a = build_attribute_array(0x3000, 4);
    assert_eq!(a.len(), ATTRIBUTE_ARRAY_SIZE);
}

#[test]
fn attribute_type_and_domain() {
    let a = build_attribute(0, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, 0);
    assert_eq!(i16::from_le_bytes(a[8..10].try_into().unwrap()), ATTR_TYPE_FLOAT2);
    assert_eq!(a[10], ATTR_DOMAIN_CORNER);
    assert_eq!(a[11], ATTR_STORAGE_ARRAY);
}

// ── Mat/matbits helpers ───────────────────────────────────────────────────────

#[test]
fn mat_ptr_array_is_all_zeroes() {
    let b = build_mat_ptr_array(3);
    assert_eq!(b.len(), 24);
    assert!(b.iter().all(|&x| x == 0));
}

#[test]
fn matbits_all_zeroes() {
    let b = build_matbits(2);
    assert_eq!(b, vec![0, 0]);
}

// ── Data serialisers ──────────────────────────────────────────────────────────

#[test]
fn floats3_data_correct() {
    let d = floats3_data(&[[1.0, 2.0, 3.0]]);
    assert_eq!(d.len(), 12);
    assert_eq!(f32::from_le_bytes(d[0..4].try_into().unwrap()), 1.0_f32);
    assert_eq!(f32::from_le_bytes(d[4..8].try_into().unwrap()), 2.0_f32);
    assert_eq!(f32::from_le_bytes(d[8..12].try_into().unwrap()), 3.0_f32);
}

#[test]
fn ints_data_correct() {
    let d = ints_data(&[0, 1, 2, 3]);
    assert_eq!(d.len(), 16);
    assert_eq!(i32::from_le_bytes(d[4..8].try_into().unwrap()), 1);
}

#[test]
fn bytes4_data_correct() {
    let d = bytes4_data(&[[10, 20, 30, 40]]);
    assert_eq!(d, vec![10, 20, 30, 40]);
}

// ── DNA1 sanity ───────────────────────────────────────────────────────────────

#[test]
fn dna1_starts_with_sdna() {
    assert_eq!(&DNA1_BYTES[0..4], b"SDNA");
}

// ── Collection Object Linking (Phase 5A) ─────────────────────────────────────

#[test]
fn collection_object_size_is_32_bytes() {
    let co = build_collection_object(0x1000);
    assert_eq!(co.len(), COLLECTION_OBJECT_SIZE);
    assert_eq!(COLLECTION_OBJECT_SIZE, 32);
}

#[test]
fn collection_object_layout_no_links() {
    let co = build_collection_object(0xdeadbeef);
    // Offset 16: object pointer
    assert_eq!(u64::from_le_bytes(co[16..24].try_into().unwrap()), 0xdeadbeef);
    // Offset 0-8: prev pointer should be zero (not set)
    assert_eq!(u64::from_le_bytes(co[0..8].try_into().unwrap()), 0);
    // Offset 8-16: next pointer should be zero (not set)
    assert_eq!(u64::from_le_bytes(co[8..16].try_into().unwrap()), 0);
}

#[test]
fn collection_object_linked_builds_doubly_linked_node() {
    let co = build_collection_object_linked(0xaaaabbbb, 0x1234, 0x5678);
    assert_eq!(co.len(), COLLECTION_OBJECT_SIZE);
    // Offset 0-8: prev pointer
    assert_eq!(u64::from_le_bytes(co[0..8].try_into().unwrap()), 0x1234);
    // Offset 8-16: next pointer
    assert_eq!(u64::from_le_bytes(co[8..16].try_into().unwrap()), 0x5678);
    // Offset 16-24: object pointer
    assert_eq!(u64::from_le_bytes(co[16..24].try_into().unwrap()), 0xaaaabbbb);
}

#[test]
fn collection_object_linked_single_element_chain() {
    let co = build_collection_object_linked(0x1000, 0, 0);  // No prev, no next
    // Prev should be null
    assert_eq!(u64::from_le_bytes(co[0..8].try_into().unwrap()), 0);
    // Next should be null
    assert_eq!(u64::from_le_bytes(co[8..16].try_into().unwrap()), 0);
    // Object pointer should be set
    assert_eq!(u64::from_le_bytes(co[16..24].try_into().unwrap()), 0x1000);
}

#[test]
fn collection_object_linked_multiple_elements() {
    let co1 = build_collection_object_linked(0x1111, 0, 0x2000);  // First element, no prev, next is second
    let co2 = build_collection_object_linked(0x2222, 0x1000, 0x3000);  // Middle element
    let co3 = build_collection_object_linked(0x3333, 0x2000, 0);  // Last element, next is null

    // Verify chain: co1 -> co2 -> co3
    assert_eq!(u64::from_le_bytes(co1[0..8].try_into().unwrap()), 0);  // co1 prev
    assert_eq!(u64::from_le_bytes(co1[8..16].try_into().unwrap()), 0x2000);  // co1 next
    
    assert_eq!(u64::from_le_bytes(co2[0..8].try_into().unwrap()), 0x1000);  // co2 prev
    assert_eq!(u64::from_le_bytes(co2[8..16].try_into().unwrap()), 0x3000);  // co2 next
    
    assert_eq!(u64::from_le_bytes(co3[0..8].try_into().unwrap()), 0x2000);  // co3 prev
    assert_eq!(u64::from_le_bytes(co3[8..16].try_into().unwrap()), 0);  // co3 next
}

#[test]
fn collection_size_is_544_bytes() {
    let c = build_collection("TestColl", 0, 0, 0);
    assert_eq!(c.len(), COLLECTION_SIZE);
    assert_eq!(COLLECTION_SIZE, 544);
}

#[test]
fn collection_gobject_offsets_416_424() {
    // Collection writes object pointers at correct offsets for ListBase.gobject:
    // Offset 416-423: ListBase.first (head of linked list) - AFTER 408-byte ID struct
    // Offset 424-431: ListBase.last (tail of linked list)
    let head_ptr = 0xdeadbeef_u64;
    let tail_ptr = 0xcafebabe_u64;
    let c = build_collection("TestColl", 0x1000, head_ptr, tail_ptr);
    
    // Verify head pointer at offset 416
    assert_eq!(u64::from_le_bytes(c[416..424].try_into().unwrap()), head_ptr);
    // Verify tail pointer at offset 424
    assert_eq!(u64::from_le_bytes(c[424..432].try_into().unwrap()), tail_ptr);
}
