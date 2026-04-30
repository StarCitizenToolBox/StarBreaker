//! Interior mesh discovery, loading, and preloading orchestration.
//!
//! Discovers interior CGF geometry from DataCore (`load_interiors`), builds
//! interior containers from socpak payloads (`build_interiors_from_payloads`),
//! preloads unique CGF meshes in parallel (`preload_interior_meshes`) and their
//! associated texture sets (`preload_interior_textures`). Also contains
//! `tint_palette_hash` (hash for texture cache keys) and
//! `expand_loadout_into_placements` (loadout→interior placement expansion).
//! Public types: `LoadedInteriors`, `InteriorCgfEntry`, `InteriorContainerData`.

use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::mtl;
use crate::types::MaterialTextures;

use super::*;

pub(crate) struct LoadedInteriors {
    /// Unique CGF entries (deduplicated by path).
    pub unique_cgfs: Vec<InteriorCgfEntry>,
    /// Per-container data (one per socpak).
    pub containers: Vec<InteriorContainerData>,
}

impl Default for LoadedInteriors {
    fn default() -> Self {
        Self {
            unique_cgfs: Vec::new(),
            containers: Vec::new(),
        }
    }
}

/// Metadata for one unique interior CGF (no mesh data).
pub(crate) struct InteriorCgfEntry {
    pub cgf_path: String,
    pub material_path: Option<String>,
    pub name: String,
}

/// One interior container's placement data.
pub(crate) struct InteriorContainerData {
    pub name: String,
    /// 4×4 column-major transform positioning this container relative to the hull.
    pub container_transform: [[f32; 4]; 4],
    /// Each entry: (index into unique_cgfs, per-object local transform,
    /// optional per-placement tint palette override that takes precedence over
    /// the container's palette). Loadout-attached children resolve their own
    /// palette from the child entity's SGeometryResourceParams so each gadget
    /// tints independently of the parent socpak's tint palette.
    pub placements: Vec<(usize, [[f32; 4]; 4], Option<mtl::TintPalette>)>,
    pub lights: Vec<crate::types::LightInfo>,
    /// Tint palette resolved from the socpak's IncludedObjects tint_palette_paths.
    pub palette: Option<mtl::TintPalette>,
}

/// Discovery pass: parse socpaks to find unique CGF paths and placements.
/// No mesh data is loaded — that happens JIT during GLB packing.
pub(crate) fn load_interiors(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    opts: &ExportOptions,
) -> LoadedInteriors {
    use crate::socpak;

    let containers = socpak::query_object_containers(db, record);
    if containers.is_empty() {
        return LoadedInteriors::default();
    }

    log::info!("Discovering {} interior containers...", containers.len());

    let mut payloads = Vec::new();
    for container in &containers {
        let container_transform =
            socpak::build_container_transform(container.offset_position, container.offset_rotation);
        match socpak::load_interior_from_socpak(p4k, &container.file_name, container_transform) {
            Ok(p) => payloads.push(p),
            Err(e) => log::warn!("failed to load {}: {e}", container.file_name),
        }
    }

    build_interiors_from_payloads(db, p4k, &payloads, opts.include_lights)
}

/// Shared interior building: dedup CGFs, resolve GUIDs, collect placements and lights.
/// Used by both `load_interiors` (from DataCore) and `socpaks_to_glb` (from explicit paths).
pub(crate) fn build_interiors_from_payloads(
    db: &Database,
    p4k: &MappedP4k,
    payloads: &[crate::types::InteriorPayload],
    include_lights: bool,
) -> LoadedInteriors {
    use std::collections::HashMap;
    use starbreaker_common::CigGuid;
    use std::str::FromStr;

    let guid_geom_compiled = db.compile_rooted::<String>(
        "EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    ).ok();
    let guid_mtl_compiled = db.compile_rooted::<String>(
        "EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
    ).ok();

    let mut cgf_cache: HashMap<String, Option<usize>> = HashMap::new();
    let mut unique_cgfs = Vec::new();
    let mut container_data = Vec::new();
    // Cache of parent CGF NMC node tables for helper-bone resolution during
    // loadout expansion. Keyed by lowercase CGF path. Value of None means we
    // tried to load it and failed.
    let mut nmc_cache: HashMap<String, Option<crate::nmc::NodeMeshCombo>> = HashMap::new();
    // Built lazily — only entities that resolve via GUID trigger loadout walks.
    let mut entity_index: Option<starbreaker_datacore::loadout::EntityIndex> = None;

    for payload in payloads {
        log::debug!(
            "  {} → {} meshes, {} lights",
            payload.name,
            payload.meshes.len(),
            payload.lights.len()
        );

        let mut placements = Vec::new();

        for im in &payload.meshes {
            let (cgf_path, mtl_path) = if !im.cgf_path.is_empty() {
                (im.cgf_path.clone(), im.material_path.clone())
            } else if let Some(guid_str) = &im.entity_class_guid {
                match resolve_guid_geometry(
                    db,
                    guid_str,
                    guid_geom_compiled.as_ref(),
                    guid_mtl_compiled.as_ref(),
                ) {
                    Some((geom, mtl)) => (geom, Some(mtl).filter(|s| !s.is_empty())),
                    None => {
                        log::debug!("  GUID {guid_str} → no geometry found");
                        continue;
                    }
                }
            } else {
                continue;
            };

            let mesh_idx = *cgf_cache.entry(cgf_path.clone()).or_insert_with(|| {
                let idx = unique_cgfs.len();
                let name = cgf_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&cgf_path)
                    .strip_suffix(".cgf")
                    .unwrap_or(&cgf_path)
                    .to_string();
                unique_cgfs.push(InteriorCgfEntry {
                    cgf_path: cgf_path.clone(),
                    material_path: mtl_path.clone().or_else(|| im.material_path.clone()),
                    name,
                });
                Some(idx)
            });

            if let Some(idx) = mesh_idx {
                placements.push((idx, im.transform, None));

                // Expand entity loadout attachments. Many interior entities
                // (e.g. fire-extinguisher cabinets, kit lockers) carry their
                // visible body in a child loadout entry attached at a named
                // CryNode helper bone on the parent CGF, rather than on their
                // own SGeometryResourceParams.
                if let Some(guid_str) = &im.entity_class_guid {
                    if let Ok(guid) = CigGuid::from_str(guid_str) {
                        if let Some(parent_record) = db.record_by_id(&guid) {
                            let idx_ref = entity_index.get_or_insert_with(|| {
                                starbreaker_datacore::loadout::EntityIndex::new(db)
                            });
                            let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(
                                idx_ref,
                                parent_record,
                            );
                            if !tree.root.children.is_empty() {
                                expand_loadout_into_placements(
                                    db,
                                    p4k,
                                    &tree.root.children,
                                    mat4_from_array(&im.transform),
                                    &cgf_path,
                                    &mut nmc_cache,
                                    &mut cgf_cache,
                                    &mut unique_cgfs,
                                    &mut placements,
                                );
                            }
                        }
                    }
                }
            }
        }

        // Resolve tint palette from the socpak's IncludedObjects palette names.
        // These are DataCore TintPaletteTree record paths — extract the short name
        // (last path component) and look up the record.
        let palette = payload.tint_palette_names.first().and_then(|path| {
            let short_name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
            let tpt_si = db.struct_id("TintPaletteTree")?;
            let record = db.records_of_type(tpt_si).find(|r| {
                db.resolve_string2(r.name_offset).to_lowercase().ends_with(&short_name)
            })?;
            query_tint_from_record(
                db,
                record,
                Some(short_name),
            )
        });

        if let Some(ref p) = palette {
            log::debug!(
                "  {} palette: primary=[{:.2},{:.2},{:.2}] secondary=[{:.2},{:.2},{:.2}]",
                payload.name, p.primary[0], p.primary[1], p.primary[2],
                p.secondary[0], p.secondary[1], p.secondary[2],
            );
        }

        container_data.push(InteriorContainerData {
            name: payload.name.clone(),
            container_transform: payload.container_transform,
            placements,
            lights: if include_lights { payload.lights.clone() } else { Vec::new() },
            palette,
        });
    }

    log::info!(
        "  {} unique CGFs, {} containers",
        unique_cgfs.len(),
        container_data.len()
    );

    LoadedInteriors {
        unique_cgfs,
        containers: container_data,
    }
}

/// Resolve an EntityClassGUID to its geometry + material paths via DataCore.
pub(crate) fn resolve_guid_geometry(
    db: &Database,
    guid_str: &str,
    geom_compiled: Option<&starbreaker_datacore::query::compile::CompiledPath>,
    mtl_compiled: Option<&starbreaker_datacore::query::compile::CompiledPath>,
) -> Option<(String, String)> {
    use starbreaker_common::CigGuid;

    let guid = CigGuid::from_str(guid_str).ok()?;
    let record = db.record_by_id(&guid)?;
    let record_name = db.resolve_string2(record.name_offset);
    let struct_name = db.struct_name(record.struct_id());

    let geom_path = match geom_compiled
        .and_then(|compiled| db.query_single::<String>(compiled, record).ok().flatten())
        .or_else(|| {
            // Fallback: try compiling path for this specific struct type
            let compiled = db
                .compile_path::<String>(
                    record.struct_id(),
                    "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
                )
                .ok()?;
            db.query_single::<String>(&compiled, record).ok().flatten()
        }) {
        Some(p) => p,
        None => {
            log::debug!(
                "  GUID {guid_str} → {struct_name}.{record_name} has no geometry component"
            );
            return None;
        }
    };

    if geom_path.is_empty() {
        log::debug!("  GUID {guid_str} → {struct_name}.{record_name} has empty geometry path");
        return None;
    }

    let mtl_path = mtl_compiled
        .and_then(|compiled| db.query_single::<String>(compiled, record).ok().flatten())
        .or_else(|| {
            let compiled = db
                .compile_path::<String>(
                    record.struct_id(),
                    "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
                )
                .ok()?;
            db.query_single::<String>(&compiled, record).ok().flatten()
        })
        .unwrap_or_default();

    log::debug!("  GUID {guid_str} → {geom_path}");
    Some((geom_path, mtl_path))
}

/// Walk a loadout subtree, emitting additional `(cgf_idx, transform)` placements
/// for each child entity that has a resolvable geometry path.
///
/// The transform for each child is composed as
/// `parent_world × helper_local_on_parent_cgf × port_offset`.
/// If the parent NMC is missing or the helper bone is not found, the child is
/// still placed using the parent's world transform plus any port offset, so
/// missing geometry is never silently dropped.
pub(crate) fn expand_loadout_into_placements(
    db: &Database,
    p4k: &MappedP4k,
    children: &[starbreaker_datacore::loadout::LoadoutNode],
    parent_world: glam::Mat4,
    parent_cgf_path: &str,
    nmc_cache: &mut std::collections::HashMap<String, Option<crate::nmc::NodeMeshCombo>>,
    cgf_cache: &mut std::collections::HashMap<String, Option<usize>>,
    unique_cgfs: &mut Vec<InteriorCgfEntry>,
    placements: &mut Vec<(usize, [[f32; 4]; 4], Option<mtl::TintPalette>)>,
) {
    if children.is_empty() {
        return;
    }
    // Look up parent NMC once for all children at this level.
    let parent_key = parent_cgf_path.to_ascii_lowercase();
    if !nmc_cache.contains_key(&parent_key) {
        let nmc = load_nmc_for_cgf(p4k, parent_cgf_path);
        nmc_cache.insert(parent_key.clone(), nmc);
    }
    // Clone the NMC out of the cache so we can release the borrow before
    // recursing (children resolve a different parent NMC).
    let parent_nmc: Option<crate::nmc::NodeMeshCombo> =
        nmc_cache.get(&parent_key).and_then(|v| v.clone());

    for child in children {
        let Some(child_geom) = child.geometry_path.as_deref() else {
            // No geometry on this node — but its grandchildren may still have
            // some (e.g. an empty container item that holds tools). Recurse
            // using the parent's transform and CGF as the attachment frame.
            if !child.children.is_empty() {
                expand_loadout_into_placements(
                    db,
                    p4k,
                    &child.children,
                    parent_world,
                    parent_cgf_path,
                    nmc_cache,
                    cgf_cache,
                    unique_cgfs,
                    placements,
                );
            }
            continue;
        };

        let helper_xform = compose_helper_transform(
            parent_nmc.as_ref(),
            child.helper_bone_name.as_deref(),
            child.offset_position,
            child.offset_rotation,
        );
        let child_world = parent_world * helper_xform;

        // Each loadout child resolves its own tint palette from the child
        // entity's SGeometryResourceParams (falling back to a name-matched
        // TintPaletteTree record). Gadgets like fire-extinguisher cabinets
        // need their own red/black palette regardless of the parent socpak's
        // palette.
        let child_palette = query_tint_palette(db, &child.record);

        let geom_owned = child_geom.to_string();
        let mtl_owned = child.material_path.clone();
        let child_idx = *cgf_cache.entry(geom_owned.clone()).or_insert_with(|| {
            let idx = unique_cgfs.len();
            let name = geom_owned
                .rsplit('/')
                .next()
                .unwrap_or(&geom_owned)
                .rsplit_once('.')
                .map(|(stem, _)| stem.to_string())
                .unwrap_or_else(|| geom_owned.clone());
            unique_cgfs.push(InteriorCgfEntry {
                cgf_path: geom_owned.clone(),
                material_path: mtl_owned.clone(),
                name,
            });
            Some(idx)
        });
        if let Some(idx) = child_idx {
            placements.push((idx, mat4_to_array(child_world), child_palette));
        }

        if !child.children.is_empty() {
            expand_loadout_into_placements(
                db,
                p4k,
                &child.children,
                child_world,
                &geom_owned,
                nmc_cache,
                cgf_cache,
                unique_cgfs,
                placements,
            );
        }
    }
}

/// Load a .cgf/.cgfm mesh from P4k by interior path.
/// When `use_model_bbox` is true, dequantizes positions using the model bounding
/// box instead of the scaling bbox (needed for interior CGF placement).
pub(crate) fn export_cgf_from_path(
    p4k: &MappedP4k,
    cgf_path: &str,
    material_path: Option<&str>,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
    use_model_bbox: bool,
) -> Result<EntityPayload, Error> {
    // Strip "data/" prefix if present (CryXMLB paths sometimes include it)
    let clean_path = cgf_path.replace('\\', "/");
    let geometry_path = clean_path
        .strip_prefix("data/")
        .or_else(|| clean_path.strip_prefix("Data/"))
        .unwrap_or(&clean_path);
    let mtl_path = material_path.unwrap_or("");
    export_entity_from_paths_cached(p4k, geometry_path, mtl_path, opts, png_cache, use_model_bbox)
}

pub(crate) fn load_interior_mesh_asset(
    p4k: &MappedP4k,
    entry: &InteriorCgfEntry,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
) -> Option<InteriorMeshAsset> {
    match export_cgf_from_path(
        p4k,
        &entry.cgf_path,
        entry.material_path.as_deref(),
        opts,
        png_cache,
        false,
    ) {
        Ok((mesh, mtl, _tex, nmc, _palette, _, _, _bones, _skeleton_source_path)) => {
            let needs_bake = mesh
                .scaling_min
                .iter()
                .zip(&mesh.model_min)
                .chain(mesh.scaling_max.iter().zip(&mesh.model_max))
                .any(|(s, m)| (s - m).abs() > 0.01);
            let mesh = if needs_bake {
                bake_nmc_into_mesh(mesh, nmc.as_ref(), false)
            } else {
                mesh
            };
            Some((mesh, mtl, nmc))
        }
        Err(e) => {
            log::warn!("failed to load CGF {}: {e}", entry.cgf_path);
            None
        }
    }
}

pub(crate) fn preload_interior_meshes(
    interiors: &LoadedInteriors,
    p4k: &MappedP4k,
    opts: &ExportOptions,
) -> Vec<Option<InteriorMeshAsset>> {
    use rayon::prelude::*;

    interiors
        .unique_cgfs
        .par_iter()
        .map(|entry| {
            let mut png_cache = PngCache::new();
            load_interior_mesh_asset(p4k, entry, opts, &mut png_cache)
        })
        .collect()
}

pub(crate) fn tint_palette_hash(palette: Option<&mtl::TintPalette>) -> u64 {
    use std::hash::{Hash, Hasher};

    let Some(palette) = palette else {
        return 0;
    };

    let mut hasher = std::hash::DefaultHasher::new();
    for color in [palette.primary, palette.secondary, palette.tertiary, palette.glass] {
        color[0].to_bits().hash(&mut hasher);
        color[1].to_bits().hash(&mut hasher);
        color[2].to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

pub(crate) fn collect_interior_palettes(
    interiors: &LoadedInteriors,
    fallback_palette: Option<&mtl::TintPalette>,
) -> Vec<(u64, Option<mtl::TintPalette>)> {
    let mut seen = std::collections::HashSet::new();
    let mut palettes = Vec::new();

    for container in &interiors.containers {
        let palette = container.palette.as_ref().or(fallback_palette).cloned();
        let palette_hash = tint_palette_hash(palette.as_ref());
        if seen.insert(palette_hash) {
            palettes.push((palette_hash, palette));
        }

        // Per-placement palette overrides (e.g. loadout-attached gadgets that
        // carry their own tint palette via SGeometryResourceParams).
        for (_, _, placement_palette) in &container.placements {
            if let Some(pal) = placement_palette {
                let h = tint_palette_hash(Some(pal));
                if seen.insert(h) {
                    palettes.push((h, Some(pal.clone())));
                }
            }
        }
    }

    palettes
}

pub(crate) fn preload_interior_textures(
    interiors: &LoadedInteriors,
    preloaded_meshes: &[Option<InteriorMeshAsset>],
    fallback_palette: Option<&mtl::TintPalette>,
    p4k: &MappedP4k,
    opts: &ExportOptions,
) -> std::collections::HashMap<PreloadedTextureKey, MaterialTextures> {
    use rayon::prelude::*;

    if !opts.material_mode.include_textures() {
        return std::collections::HashMap::new();
    }

    let palettes = collect_interior_palettes(interiors, fallback_palette);
    if palettes.is_empty() {
        return std::collections::HashMap::new();
    }

    let mut unique_materials = std::collections::HashMap::<String, mtl::MtlFile>::new();
    for asset in preloaded_meshes {
        let Some((_, Some(materials), _)) = asset else {
            continue;
        };
        let Some(source_path) = materials.source_path.as_ref() else {
            continue;
        };
        unique_materials
            .entry(source_path.to_ascii_lowercase())
            .or_insert_with(|| materials.clone());
    }

    if unique_materials.is_empty() {
        return std::collections::HashMap::new();
    }

    let jobs: Vec<(String, mtl::MtlFile, u64, Option<mtl::TintPalette>)> = unique_materials
        .into_iter()
        .flat_map(|(material_source, materials)| {
            palettes.iter().map(move |(palette_hash, palette)| {
                (
                    material_source.clone(),
                    materials.clone(),
                    *palette_hash,
                    palette.clone(),
                )
            })
        })
        .collect();

    jobs
        .into_par_iter()
        .map(|(material_source, materials, palette_hash, palette)| {
            let mut png_cache = PngCache::new();
            let textures = load_material_textures(
                p4k,
                &materials,
                palette.as_ref(),
                opts.texture_mip,
                &mut png_cache,
                opts.material_mode.include_normals(),
                opts.material_mode.experimental(),
            );
            (
                PreloadedTextureKey {
                    material_source,
                    palette_hash,
                },
                textures,
            )
        })
        .collect()
}
