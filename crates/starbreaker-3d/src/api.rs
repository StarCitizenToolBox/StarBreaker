//! Thin public facade for application integrations.
//!
//! This module exposes stable helpers for consumers that want StarBreaker's
//! P4K/DataCore/3D pipeline without depending on crate-private pipeline modules.

use std::path::Path;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::pipeline::{self, ExportFormat, ExportKind, ExportOptions, MaterialMode};

/// Convert a DataCore geometry path to the P4K path format used by StarBreaker.
pub fn normalize_datacore_path_to_p4k(path: &str) -> String {
    pipeline::datacore_path_to_p4k(path)
}

/// Return true when the path extension is handled by StarBreaker's geometry loader.
pub fn is_supported_geometry_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "cdf" | "cga" | "cgf" | "chr" | "skin"
            )
        })
        .unwrap_or(false)
}

/// Read `Data/Game2.dcb` or `Data/Game.dcb` from an opened P4K archive.
pub fn read_datacore_from_p4k(p4k: &MappedP4k) -> Result<Vec<u8>, Error> {
    p4k.read_file("Data\\Game2.dcb")
        .or_else(|_| p4k.read_file("Data\\Game.dcb"))
        .map_err(Error::P4k)
}

/// Find an entity record by its short or fully-qualified EntityClassDefinition name.
pub fn find_entity_record<'a>(db: &'a Database<'a>, entity_name: &str) -> Option<&'a Record> {
    let needle = entity_name.to_ascii_lowercase();
    db.struct_id("EntityClassDefinition").and_then(|struct_id| {
        db.records_of_type(struct_id).find(|record| {
            let record_name = db.resolve_string2(record.name_offset);
            let short_name = record_name.rsplit('.').next().unwrap_or(record_name);
            record_name.eq_ignore_ascii_case(&needle) || short_name.eq_ignore_ascii_case(&needle)
        })
    })
}

/// Export a single P4K geometry path as GLB bytes.
pub fn export_geometry_glb(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: Option<&str>,
    opts: &ExportOptions,
) -> Result<Vec<u8>, Error> {
    let (
        mesh,
        materials,
        textures,
        nmc,
        palette,
        resolved_geometry_path,
        resolved_material_path,
        bones,
        _skeleton_source_path,
    ) = pipeline::export_entity_from_paths(
        p4k,
        geometry_path,
        material_path.unwrap_or_default(),
        opts,
    )?;

    let mut no_textures = |_: Option<&crate::mtl::MtlFile>,
                           _: Option<&crate::mtl::TintPalette>|
     -> Option<crate::types::MaterialTextures> { None };
    let mut no_interior = |_: &pipeline::InteriorCgfEntry|
     -> Option<(crate::Mesh, Option<crate::mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
        None
    };

    crate::gltf::write_glb(
        crate::gltf::GlbInput {
            root_mesh: Some(mesh),
            root_materials: materials,
            root_textures: textures,
            root_nmc: nmc,
            root_palette: palette,
            skeleton_bones: bones,
            children: Vec::new(),
            interiors: pipeline::LoadedInteriors::default(),
        },
        &mut crate::gltf::GlbLoaders {
            load_textures: &mut no_textures,
            load_interior_mesh: &mut no_interior,
        },
        &crate::gltf::GlbOptions {
            material_mode: opts.material_mode,
            preserve_textureless_decal_primitives: opts.include_nodraw,
            metadata: crate::gltf::GlbMetadata {
                entity_name: None,
                geometry_path: Some(resolved_geometry_path),
                material_path: Some(resolved_material_path),
                export_options: gltf_export_metadata(opts),
            },
            fallback_palette: None,
        },
    )
}

/// Export an EntityClassDefinition record and its loadout as GLB bytes.
pub fn export_entity_glb(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    opts: &ExportOptions,
) -> Result<Vec<u8>, Error> {
    let tree = loadout::resolve_loadout(db, record);
    let result = crate::assemble_glb_with_loadout(db, p4k, record, &tree, opts)?;
    Ok(result.glb)
}

fn gltf_export_metadata(opts: &ExportOptions) -> crate::gltf::ExportOptionsMetadata {
    crate::gltf::ExportOptionsMetadata {
        kind: export_kind_name(opts.kind).to_string(),
        material_mode: material_mode_name(opts.material_mode).to_string(),
        format: export_format_name(opts.format).to_string(),
        lod_level: opts.lod_level,
        texture_mip: opts.texture_mip,
        include_attachments: opts.include_attachments,
        include_interior: opts.include_interior,
    }
}

fn export_kind_name(kind: ExportKind) -> &'static str {
    match kind {
        ExportKind::Bundled => "Bundled",
        ExportKind::Decomposed => "Decomposed",
    }
}

fn export_format_name(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Glb => "Glb",
        ExportFormat::Stl => "Stl",
        ExportFormat::Blend => "Blend",
    }
}

fn material_mode_name(mode: MaterialMode) -> &'static str {
    match mode {
        MaterialMode::None => "None",
        MaterialMode::Colors => "Colors",
        MaterialMode::Textures => "Textures",
        MaterialMode::All => "All",
    }
}
