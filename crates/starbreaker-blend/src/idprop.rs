//! Allocation and block-writing helpers for Blender IDProperty trees.

use crate::{build_idprop_tree, write_block, IdPropValue, PtrAlloc, SDNA_IDX_IDPROPERTY};

/// Allocated IDProperty blocks ready to write after their owning ID block.
pub struct IdPropBlocks {
    pub root_ptr: u64,
    pub root: Vec<u8>,
    pub children: Vec<(u64, Vec<u8>)>,
    pub strings: Vec<(u64, Vec<u8>)>,
}

/// Allocate and build IDProperty blocks using Blender's required block order.
pub fn allocate_idprop_blocks(
    ptrs: &mut PtrAlloc,
    props: Vec<(String, IdPropValue)>,
) -> Option<IdPropBlocks> {
    if props.is_empty() {
        return None;
    }
    let root_ptr = ptrs.alloc();
    let child_ptrs = (0..props.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let string_ptrs = props
        .iter()
        .filter(|(_, value)| matches!(value, IdPropValue::String(_)))
        .map(|_| ptrs.alloc())
        .collect::<Vec<_>>();
    let (root, children, strings) = build_idprop_tree(root_ptr, &child_ptrs, &string_ptrs, &props);
    Some(IdPropBlocks {
        root_ptr,
        root,
        children,
        strings,
    })
}

/// Write allocated IDProperty blocks in Blender-compatible root/child/string order.
pub fn write_idprop_blocks(out: &mut Vec<u8>, props: &IdPropBlocks) {
    write_block(
        out,
        b"DATA",
        SDNA_IDX_IDPROPERTY,
        props.root_ptr,
        1,
        &props.root,
    );
    for (child_ptr, child_data) in &props.children {
        write_block(out, b"DATA", SDNA_IDX_IDPROPERTY, *child_ptr, 1, child_data);
        if let Some((string_ptr, string_data)) = props.strings.iter().find(|(ptr, _)| {
            u64::from_le_bytes(child_data[88..96].try_into().unwrap()) == *ptr
        }) {
            write_block(out, b"DATA", 0, *string_ptr, 1, string_data);
        }
    }
}
