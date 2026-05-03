//! Blender decomposed export: convert `DecomposedInput` to individual `.blend` files.
//!
//! Orchestrates the pipeline for Phase 1 (mesh decomposition):
//! - Extract meshes from `DecomposedInput::children`
//! - Build material slots (empty, names-only)
//! - Write individual `.blend` files for each mesh
//! - Generate `ExportedFile` entries
//!
//! Phase 2 (scene.blend linking) and Phase 3+ will be implemented in subsequent phases.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::pipeline::{DecomposedExport, ExportedFile, ExportedFileKind, ExportOptions};
use crate::types::{EntityPayload, Mesh};

/// Convert `DecomposedInput` into individual `.blend` files, one per mesh.
///
/// **Phase 1A: Module Scaffolding** (1 hour)
/// - ✅ Function signature defined
/// - ✅ Type imports established
/// - ✅ Module added to pipeline/mod.rs
///
/// **Phase 1B: Mesh Extraction** (2 hours)
/// - TODO: Iterate `DecomposedInput::children` (Vec<EntityPayload>)
/// - TODO: Extract mesh data: `payload.mesh` (vertices, faces, normals, UVs)
/// - TODO: Build file paths using GLB naming pattern
///
/// **Phase 1C: Material Slots** (2 hours)
/// - TODO: Create empty materials with GLB format names
/// - TODO: Add `mtl_source_path` custom property
/// - TODO: Create `StarBreaker_Decals` vertex group (Phase 4 placeholder)
///
/// **Phase 1D: Write .blend Files** (2 hours)
/// - TODO: Create Mesh object using starbreaker-blend functions
/// - TODO: Add material slots
/// - TODO: Add UV maps and vertex groups
/// - TODO: Write uncompressed .blend files
///
/// **Phase 1E: Generate ExportedFile Entries** (1 hour)
/// - TODO: Create ExportedFile for each .blend file
/// - TODO: Return DecomposedExport with all files
pub fn write_decomposed_export_blend(
    _p4k: &MappedP4k,
    _input: crate::decomposed::DecomposedInput,
    _opts: &ExportOptions,
    _progress: Option<&Progress>,
    _existing_asset_paths: Option<&HashSet<String>>,
) -> Result<DecomposedExport, Error> {
    // Phase 1A: Scaffolding complete. Phases 1B-1E to follow.
    todo!("Phase 1B: Mesh extraction from DecomposedInput")
}
