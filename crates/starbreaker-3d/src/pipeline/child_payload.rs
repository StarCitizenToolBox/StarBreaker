//! Child entity payload loading and caching helpers.
//!
//! Loads child mesh assets for loadout children and inline entity attachments.
//! `collect_child_payload_specs` gathers attachment specs from a loadout tree;
//! `load_child_payload_asset` exports a single child CGF; `load_child_payloads`
//! drives the full parallel load pipeline.
//! Also defines `LandingGearAsset` (cached landing gear geometry) and the path
//! normalisation helpers used for decomposed export deduplication.

use std::collections::{HashMap, HashSet};

use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::{CigGuid, Record};
use starbreaker_p4k::MappedP4k;

use crate::mtl;
use crate::nmc;
use crate::types::{MaterialTextures, UiBinding};

use super::*;

/// Try loading an entity's mesh data from its resolved geometry/material paths,
/// falling back to DataCore record lookup.
pub(crate) fn load_child_mesh(
    child: &crate::types::ResolvedNode,
    db: &Database,
    p4k: &MappedP4k,
    opts: &ExportOptions,
) -> Option<(
    crate::types::Mesh,
    Option<mtl::MtlFile>,
    Option<nmc::NodeMeshCombo>,
    Option<mtl::TintPalette>,
    Vec<crate::skeleton::Bone>,
    String,
    String,
    Option<String>,
)> {
    let result = if child.geometry_path.is_some() {
        let gp = child.geometry_path.as_deref().unwrap_or("");
        let mp = child.material_path.as_deref().unwrap_or("");
        export_entity_from_paths(p4k, gp, mp, opts)
            .map_err(|e| {
                log::warn!("  {} -> load from paths failed: {e}", child.entity_name);
                e
            })
            .or_else(|_| export_entity_payload(db, p4k, &child.record, opts))
    } else {
        export_entity_payload(db, p4k, &child.record, opts)
    };

    result
        .ok()
        .map(
            |(
                mesh,
                mtl,
                _tex,
                nmc,
                palette,
                geometry_path,
                material_path,
                bones,
                skeleton_source_path,
            )| {
                let resolved_palette = palette.or_else(|| query_tint_palette(db, &child.record));
                (
                    mesh,
                    mtl,
                    nmc,
                    resolved_palette,
                    bones,
                    geometry_path,
                    material_path,
                    skeleton_source_path,
                )
            },
        )
}

pub(crate) struct ChildPayloadSpec {
    pub(crate) child: crate::types::ResolvedNode,
    pub(crate) parent_entity_name: String,
    pub(crate) parent_node_name: String,
    pub(crate) no_rotation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ChildPayloadCacheKey {
    pub(crate) record_id: starbreaker_datacore::types::CigGuid,
    pub(crate) geometry_path: Option<String>,
    pub(crate) material_path: Option<String>,
}

#[derive(Clone)]
pub(crate) struct LoadedChildPayload {
    pub(crate) mesh: crate::types::Mesh,
    pub(crate) materials: Option<mtl::MtlFile>,
    pub(crate) textures: Option<MaterialTextures>,
    pub(crate) nmc: Option<nmc::NodeMeshCombo>,
    pub(crate) palette: Option<mtl::TintPalette>,
    pub(crate) bones: Vec<crate::skeleton::Bone>,
    pub(crate) geometry_path: String,
    pub(crate) material_path: String,
    pub(crate) skeleton_source_path: Option<String>,
}

/// Cached result of loading a landing-gear CGF. Multiple gear hardpoints
/// often share the same geometry (e.g. mirrored gears), so this cache
/// dedupes P4k reads and texture decoding by gear_path.
#[derive(Clone)]
pub(crate) struct LandingGearAsset {
    pub(crate) mesh: crate::types::Mesh,
    pub(crate) materials: Option<mtl::MtlFile>,
    pub(crate) textures: Option<MaterialTextures>,
    pub(crate) nmc: Option<nmc::NodeMeshCombo>,
    pub(crate) geometry_path: String,
    pub(crate) material_path: String,
    pub(crate) bones: Vec<crate::skeleton::Bone>,
    pub(crate) skeleton_source_path: Option<String>,
}

pub(crate) fn collect_child_payload_specs(
    children: &[crate::types::ResolvedNode],
    parent_entity_name: &str,
    override_attachment: Option<(&str, bool)>,
    out: &mut Vec<ChildPayloadSpec>,
) {
    for child in children {
        let (attach_name, no_rotation) = match override_attachment {
            Some((name, parent_no_rot)) => (name.to_string(), child.no_rotation || parent_no_rot),
            None => (child.attachment_name.clone(), child.no_rotation),
        };

        let child_creates_nodes = child.has_geometry || child.nmc.is_some();
        out.push(ChildPayloadSpec {
            child: child.clone_payload_source(),
            parent_entity_name: parent_entity_name.to_string(),
            parent_node_name: attach_name,
            no_rotation,
        });
        if child_creates_nodes {
            collect_child_payload_specs(&child.children, &child.entity_name, None, out);
        } else {
            collect_child_payload_specs(
                &child.children,
                parent_entity_name,
                Some((&child.attachment_name, child.no_rotation)),
                out,
            );
        }
    }
}

pub(crate) fn empty_child_mesh() -> crate::types::Mesh {
    crate::types::Mesh {
        positions: Vec::new(),
        indices: Vec::new(),
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: Vec::new(),
        model_min: [0.0; 3],
        model_max: [0.0; 3],
        scaling_min: [0.0; 3],
        scaling_max: [0.0; 3],
    }
}

pub(crate) fn normalize_decomposed_source_path(p4k: &MappedP4k, path: &str) -> String {
    let p4k_path = datacore_path_to_p4k(path);
    p4k.entry_case_insensitive(&p4k_path)
        .map(|entry| entry.name.replace('\\', "/"))
        .unwrap_or_else(|| p4k_path.replace('\\', "/"))
}

pub(crate) fn replace_extension(path: &str, new_extension: &str) -> String {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return format!("{path}{new_extension}");
    };
    stem.to_string() + new_extension
}

pub(crate) fn decomposed_mesh_asset_path(p4k: &MappedP4k, geometry_path: &str) -> Option<String> {
    if geometry_path.is_empty() {
        None
    } else {
        Some(replace_extension(&normalize_decomposed_source_path(p4k, geometry_path), ".glb"))
    }
}

pub(crate) fn build_child_payload_cache_key(child: &crate::types::ResolvedNode) -> ChildPayloadCacheKey {
    ChildPayloadCacheKey {
        record_id: child.record.id,
        geometry_path: child.geometry_path.clone(),
        material_path: child.material_path.clone(),
    }
}

pub(crate) fn load_child_payload_asset(
    child: &crate::types::ResolvedNode,
    db: &Database,
    p4k: &MappedP4k,
    mesh_opts: &ExportOptions,
    final_material_mode: MaterialMode,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Option<LoadedChildPayload> {
    if mesh_opts.kind == ExportKind::Decomposed {
        if let Some(geometry_path) = child.geometry_path.as_deref() {
            if let Some(mesh_asset_path) = decomposed_mesh_asset_path(p4k, geometry_path) {
                if existing_asset_paths
                    .is_some_and(|paths| paths.contains(&mesh_asset_path.to_ascii_lowercase()))
                {
                    let material_path = child.material_path.as_deref().unwrap_or("");
                    let (nmc, materials) = load_nmc_and_material(p4k, geometry_path, material_path);
                    let skeleton_source_path = resolve_geometry_files(p4k, geometry_path)
                        .ok()
                        .and_then(|resolved| {
                            skeleton_source_paths(resolved.skeleton_path.as_deref(), &resolved.parts[0].path)
                                .first()
                                .map(|path| (*path).to_string())
                        });
                    return Some(LoadedChildPayload {
                        mesh: empty_child_mesh(),
                        materials,
                        textures: None,
                        nmc,
                        palette: None,
                        bones: Vec::new(),
                        geometry_path: geometry_path.to_string(),
                        material_path: material_path.to_string(),
                        skeleton_source_path,
                    });
                }
            }
        }
    }

    let (mesh, mtl, nmc, palette, bones, geometry_path, material_path, skeleton_source_path) =
        load_child_mesh(child, db, p4k, mesh_opts)?;
    let textures = if final_material_mode.include_textures() {
        mtl.as_ref().map(|materials| {
            let mut png_cache = PngCache::new();
            load_material_textures(
                p4k,
                materials,
                palette.as_ref(),
                mesh_opts.texture_mip,
                &mut png_cache,
                final_material_mode.include_normals(),
                final_material_mode.experimental(),
            )
        })
    } else {
        None
    };

    Some(LoadedChildPayload {
        mesh,
        materials: mtl,
        textures,
        nmc,
        palette,
        bones,
        geometry_path,
        material_path,
        skeleton_source_path,
    })
}

pub(crate) fn load_child_payloads(
    specs: Vec<ChildPayloadSpec>,
    db: &Database,
    p4k: &MappedP4k,
    mesh_opts: &ExportOptions,
    final_material_mode: MaterialMode,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Vec<crate::types::EntityPayload> {
    use rayon::prelude::*;

    // Pre-resolve dashboard screen canvases for each unique parent entity so that
    // physical/mfd bindings with no inline content canvas can be assigned one.
    let dashboard_canvases_by_parent: HashMap<String, Vec<(u32, u32, String, Option<String>)>> = {
        let mut map: HashMap<String, Vec<(u32, u32, String, Option<String>)>> = HashMap::new();
        for spec in &specs {
            let parent_name = &spec.parent_entity_name;
            if map.contains_key(parent_name) {
                continue;
            }
            let canvases = find_entity_record_by_name(db, parent_name)
                .and_then(|parent_record| {
                    let guid = query_stringish_path(
                        db,
                        parent_record,
                        "Components[SCItemUIViewOwnerParams].dashboardCanvasConfig",
                    )?;
                    let parsed = parse_guid(&guid)?;
                    let dashboard_record = db.record_by_id(&parsed)?;
                    Some(collect_dashboard_screen_canvases(db, dashboard_record))
                })
                .unwrap_or_default();
            map.insert(parent_name.clone(), canvases);
        }
        map
    };
    // Per-parent counter tracking how many screen canvases have been assigned so far.
    let mut parent_canvas_used: HashMap<String, usize> = HashMap::new();

    let mut ui_bindings_by_parent = HashMap::<String, Vec<UiBinding>>::new();
    let mut direct_ui_bindings = Vec::with_capacity(specs.len());
    for spec in &specs {
        let binding = ui_binding_for_record(db, &spec.child.record).map(|mut binding| {
            binding.source_entity_name = spec.child.entity_name.clone();
            binding.helper_name = if spec.child.has_geometry {
                None
            } else {
                Some(spec.parent_node_name.clone())
            };
            // For physical/mfd bindings without an inline content canvas, attempt to
            // resolve the content canvas from the parent vehicle's dashboard canvas def.
            // Canvases are assigned positionally in the order specs are processed.
            if binding.content_canvas_guid.is_none()
                && matches!(binding.binding_kind.as_str(), "physical" | "mfd")
            {
                if let Some(canvases) = dashboard_canvases_by_parent.get(&spec.parent_entity_name) {
                    let used = parent_canvas_used
                        .entry(spec.parent_entity_name.clone())
                        .or_insert(0);
                    if let Some((view_idx, screen_slot, guid, name)) = canvases.get(*used) {
                        binding.content_canvas_guid = Some(guid.clone());
                        binding.content_canvas_record_name = name.clone();
                        binding.dashboard_view_index = Some(*view_idx);
                        binding.dashboard_screen_slot = Some(*screen_slot);
                        *used += 1;
                    }
                }
            }
            binding
        });
        if let Some(binding) = binding.clone() {
            ui_bindings_by_parent
                .entry(spec.parent_entity_name.clone())
                .or_default()
                .push(binding);
        }
        direct_ui_bindings.push(binding);
    }

    let mut unique_children = Vec::new();
    let mut unique_child_indices = std::collections::HashMap::new();
    let mut spec_asset_indices = Vec::with_capacity(specs.len());

    for spec in &specs {
        let child = &spec.child;
        if !child.has_geometry {
            spec_asset_indices.push(None);
            continue;
        }

        let cache_key = build_child_payload_cache_key(child);
        let unique_index = if let Some(&index) = unique_child_indices.get(&cache_key) {
            index
        } else {
            let index = unique_children.len();
            unique_children.push(child.clone_payload_source());
            unique_child_indices.insert(cache_key, index);
            index
        };
        spec_asset_indices.push(Some(unique_index));
    }

    let loaded_assets: Vec<Option<LoadedChildPayload>> = unique_children
        .into_par_iter()
        .map(|child| {
            load_child_payload_asset(
                &child,
                db,
                p4k,
                mesh_opts,
                final_material_mode,
                existing_asset_paths,
            )
        })
        .collect();

    specs
        .into_iter()
        .enumerate()
        .filter_map(|(spec_index, spec)| {
            let child = &spec.child;
            let entity_category = entity_record_category(db, &child.record);
            let attach_def_type = entity_record_attach_def_type(db, &child.record);
            let mut ui_bindings = ui_bindings_by_parent
                .get(&child.entity_name)
                .cloned()
                .unwrap_or_default();
            if let Some(binding) = direct_ui_bindings
                .get(spec_index)
                .and_then(|binding| binding.clone())
            {
                if !ui_bindings.iter().any(|existing| existing == &binding) {
                    ui_bindings.push(binding);
                }
            }
            if child.has_geometry {
                let asset_index = spec_asset_indices[spec_index]?;
                let loaded = loaded_assets.get(asset_index)?.as_ref()?;
                Some(crate::types::EntityPayload {
                    mesh: loaded.mesh.clone(),
                    materials: loaded.materials.clone(),
                    textures: loaded.textures.clone(),
                    nmc: loaded.nmc.clone(),
                    palette: loaded.palette.clone(),
                    geometry_path: loaded.geometry_path.clone(),
                    material_path: loaded.material_path.clone(),
                    bones: loaded.bones.clone(),
                    skeleton_source_path: loaded.skeleton_source_path.clone(),
                    entity_name: child.entity_name.clone(),
                    entity_category,
                    attach_def_type,
                    parent_node_name: spec.parent_node_name.clone(),
                    parent_entity_name: spec.parent_entity_name.clone(),
                    no_rotation: spec.no_rotation,
                    offset_position: child.offset_position,
                    offset_rotation: child.offset_rotation,
                    detach_direction: child.detach_direction,
                    port_flags: child.port_flags.clone(),
                    ui_bindings,
                })
            } else if child.nmc.is_some() {
                Some(crate::types::EntityPayload {
                    mesh: empty_child_mesh(),
                    materials: None,
                    textures: None,
                    nmc: child.nmc.clone(),
                    palette: None,
                    geometry_path: child.geometry_path.clone().unwrap_or_default(),
                    material_path: child.material_path.clone().unwrap_or_default(),
                    bones: Vec::new(),
                    skeleton_source_path: None,
                    entity_name: child.entity_name.clone(),
                    entity_category,
                    attach_def_type,
                    parent_node_name: spec.parent_node_name.clone(),
                    parent_entity_name: spec.parent_entity_name.clone(),
                    no_rotation: spec.no_rotation,
                    offset_position: child.offset_position,
                    offset_rotation: child.offset_rotation,
                    detach_direction: child.detach_direction,
                    port_flags: child.port_flags.clone(),
                    ui_bindings,
                })
            } else {
                None
            }
        })
        .collect()
}

fn entity_record_category(db: &Database, record: &starbreaker_datacore::types::Record) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), "Category")
        .ok()
        .and_then(|compiled| db.query_single::<String>(&compiled, record).ok().flatten())
        .filter(|category| !category.is_empty())
}

fn entity_record_attach_def_type(
    db: &Database,
    record: &starbreaker_datacore::types::Record,
) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), "Components[SAttachableComponentParams].AttachDef.Type")
        .ok()
        .and_then(|compiled| db.query_single::<String>(&compiled, record).ok().flatten())
        .filter(|attach_def_type| !attach_def_type.is_empty())
}

pub(crate) fn ui_binding_for_record(db: &Database, record: &Record) -> Option<UiBinding> {
    let owner_source_file =
        query_string_path(db, record, "Components[UIOwnerEntityComponentParams].element.sourceFile");
    let runtime_image_source = query_string_path(
        db,
        record,
        "Components[UIRenderToTextureEntityComponentParams].runtimeImageSource",
    );
    let (default_state_name, default_light_color, default_light_intensity_milli) =
        default_display_screen_state(record, db);
    if let Some(canvas_guid) = query_stringish_path(
        db,
        record,
        "Components[SCItemUIViewOwnerParams].dashboardCanvasConfig",
    ) {
        let (canvas_record_name, canvas_record_path) = resolve_record_metadata(db, &canvas_guid);
        let (canvas_widget_canvas_path, canvas_widget_url_postfix, canvas_widget_url_optional, canvas_variable_binding) =
            canvas_widget_context_for_guid(db, &canvas_guid);
        if canvas_widget_canvas_path.is_some()
            || canvas_widget_url_postfix.is_some()
            || canvas_widget_url_optional.is_some()
            || canvas_variable_binding.is_some()
        {
            return Some(UiBinding {
                binding_kind: "physical".to_string(),
                source_entity_name: String::new(),
                helper_name: None,
                default_view: None,
                default_state_name: default_state_name.clone(),
                default_light_color,
                default_light_intensity_milli,
                canvas_guid: Some(canvas_guid),
                canvas_record_name,
                canvas_record_path,
                canvas_widget_canvas_path,
                canvas_widget_url_postfix,
                canvas_widget_url_optional,
                canvas_variable_binding,
                content_canvas_guid: None,
                content_canvas_record_name: None,
                dashboard_view_index: None,
                dashboard_screen_slot: None,
                owner_source_file,
                runtime_image_source,
                generated_image_path: None,
                generated_context_manifest_path: None,
                generated_resolved_source_path: None,
                generated_backend: None,
                generated_provenance: None,
            });
        }
    }
    if let Some(canvas_guid) = query_stringish_path(
        db,
        record,
        "Components[UIMapEntityComponentParams].uiElementsCanvasGUID",
    )
    .or_else(|| {
        query_stringish_path(
            db,
            record,
            "Components[UIMapEntityComponentParams].starMapParams.uiElementsCanvasGUID",
        )
    }) {
        let (canvas_record_name, canvas_record_path) = resolve_record_metadata(db, &canvas_guid);
        let (canvas_widget_canvas_path, canvas_widget_url_postfix, canvas_widget_url_optional, canvas_variable_binding) =
            canvas_widget_context_for_guid(db, &canvas_guid);
        return Some(UiBinding {
            binding_kind: "radar".to_string(),
            source_entity_name: String::new(),
            helper_name: None,
            default_view: None,
            default_state_name: default_state_name.clone(),
            default_light_color,
            default_light_intensity_milli,
            canvas_guid: Some(canvas_guid),
            canvas_record_name,
            canvas_record_path,
            canvas_widget_canvas_path,
            canvas_widget_url_postfix,
            canvas_widget_url_optional,
            canvas_variable_binding,
            content_canvas_guid: None,
            content_canvas_record_name: None,
            dashboard_view_index: None,
            dashboard_screen_slot: None,
            owner_source_file,
            runtime_image_source,
            generated_image_path: None,
            generated_context_manifest_path: None,
            generated_resolved_source_path: None,
            generated_backend: None,
            generated_provenance: None,
        });
    }

    let layers = query_value_path(db, record, "Components[UIBuildingBlocksEntityComponentParams].layers")?;
    let first_layer = match layers {
        Value::Array(values) => values.into_iter().next()?,
        other => other,
    };
    let default_view = value_object(&first_layer, "defaultView")
        .and_then(|default_view| value_object_string(default_view, "name"));
    let canvas_guid = value_object(&first_layer, "defaultView")
        .and_then(|default_view| value_object(default_view, "component"))
        .and_then(|component| value_object_string(component, "canvas"))
        .or_else(|| {
            value_object(&first_layer, "element")
                .and_then(|element| value_object_string(element, "canvas"))
        })
        .or_else(|| value_object_string(&first_layer, "canvas"));
    let (canvas_record_name, canvas_record_path) = canvas_guid
        .as_deref()
        .map(|guid| resolve_record_metadata(db, guid))
        .unwrap_or((None, None));
    let (canvas_widget_canvas_path, canvas_widget_url_postfix, canvas_widget_url_optional, canvas_variable_binding) =
        canvas_guid
            .as_deref()
            .map(|guid| canvas_widget_context_for_guid(db, guid))
            .unwrap_or((None, None, None, None));
    let binding_kind = match default_view.as_deref() {
        Some("_mfd") => "mfd",
        Some("_physicalScreen") => "physical",
        _ => "physical",
    };
    Some(UiBinding {
        binding_kind: binding_kind.to_string(),
        source_entity_name: String::new(),
        helper_name: None,
        default_view,
        default_state_name,
        default_light_color,
        default_light_intensity_milli,
        canvas_guid,
        canvas_record_name,
        canvas_record_path,
        canvas_widget_canvas_path,
        canvas_widget_url_postfix,
        canvas_widget_url_optional,
        canvas_variable_binding,
        content_canvas_guid: None,
        content_canvas_record_name: None,
        dashboard_view_index: None,
        dashboard_screen_slot: None,
        owner_source_file,
        runtime_image_source,
        generated_image_path: None,
        generated_context_manifest_path: None,
        generated_resolved_source_path: None,
        generated_backend: None,
        generated_provenance: None,
    })
}

fn canvas_widget_context_for_guid(
    db: &Database,
    canvas_guid: &str,
) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let Some(guid) = parse_guid(canvas_guid) else {
        return (None, None, None, None);
    };
    let Some(record) = db.record_by_id(&guid) else {
        return (None, None, None, None);
    };
    if let Some(Value::Array(views)) = query_value_path(db, record, "views") {
        let mut fallback = (None, None, None, None);
        for view in views {
            let Some(screens) = value_object(&view, "screens") else {
                continue;
            };
            let screens: Vec<&Value<'_>> = match screens {
                Value::Array(values) => values.iter().collect(),
                other => vec![other],
            };
            for screen in screens {
                let Some(screen_guid) = value_stringish(screen) else {
                    continue;
                };
                if is_zero_guid(&screen_guid) {
                    continue;
                }
                let context = canvas_widget_context_for_guid(db, &screen_guid);
                if context.0.is_some()
                    || context.1.is_some()
                    || context.2.is_some()
                    || context.3.is_some()
                {
                    if context.3.is_some() || context.1.is_some() || context.2.is_some() {
                        return context;
                    }
                    if fallback.0.is_none() {
                        fallback = context;
                    }
                }
            }
        }
        if fallback.0.is_some() || fallback.1.is_some() || fallback.2.is_some() || fallback.3.is_some() {
            return fallback;
        }
    }
    let scene = query_value_path(db, record, "scene");
    let operations = query_value_path(db, record, "operations");

    let mut canvas_path = None;
    let mut url_postfix = None;
    let mut url_optional = None;
    if let Some(Value::Array(items)) = scene {
        for item in items {
            if value_object_string(&item, "_Type_").as_deref() != Some("BuildingBlocks_WidgetCanvas") {
                continue;
            }
            canvas_path = value_object_string(&item, "canvas")
                .filter(|value| !value.is_empty() && value != "null");
            url_postfix = value_object_string(&item, "urlPostfix")
                .filter(|value| !value.is_empty());
            url_optional = value_object_string(&item, "urlOptional")
                .filter(|value| !value.is_empty());
            if canvas_path.is_some() || url_postfix.is_some() || url_optional.is_some() {
                break;
            }
        }
    }

    let mut variable_binding = None;
    if let Some(Value::Array(items)) = operations {
        for item in items {
            if value_object_string(&item, "_Type_").as_deref()
                == Some("BuildingBlocks_BindingsIntegerVariable")
            {
                variable_binding = value_object_string(&item, "binding")
                    .filter(|value| !value.is_empty());
                if variable_binding.is_some() {
                    break;
                }
            }
        }
    }

    (canvas_path, url_postfix, url_optional, variable_binding)
}

/// Return the basename of a file path without its extension.
/// E.g. `"Libraries/UI/FlightController_Annunciator.dcb"` → `"FlightController_Annunciator"`
fn path_basename_no_ext(path: &str) -> &str {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if let Some(dot_pos) = base.rfind('.') { &base[..dot_pos] } else { base }
}

/// Find a `BuildingBlocks_Canvas` record by matching a path string from a dashboard
/// `screens[]` slot.  The `path` may be a bare GUID string, a filename like
/// `"flightcontroller_annunciator.json"`, or a full library path.  GUID lookup is
/// tried first; filename-basename matching is the fallback.
fn find_canvas_record_by_path_or_guid<'a>(db: &'a Database<'a>, path: &str) -> Option<&'a Record> {
    // Try as direct GUID reference first.
    if let Some(guid) = parse_guid(path) {
        if let Some(record) = db.record_by_id(&guid) {
            return Some(record);
        }
    }
    // Fall back: match on the basename (without extension) of the record's file path.
    let needle = path_basename_no_ext(path).to_lowercase();
    if needle.is_empty() {
        return None;
    }
    db.records_by_type_name("BuildingBlocks_Canvas").find(|record| {
        let file_path = db.resolve_string(record.file_name_offset);
        path_basename_no_ext(file_path).to_lowercase() == needle
    })
}

/// Find an `EntityClassDefinition` record by exact name.
fn find_entity_record_by_name<'a>(db: &'a Database<'a>, name: &str) -> Option<&'a Record> {
    let entity_struct = db.struct_id("EntityClassDefinition")?;
    db.records_of_type(entity_struct)
        .find(|record| db.resolve_string2(record.name_offset) == name)
}

/// Collect all non-null screen canvas entries from a `SCItemUIView_DashboardCanvasDef`
/// record.  Returns `(view_index, screen_slot, canvas_guid, canvas_record_name)` tuples
/// in `View[]` order, skipping null/zero-GUID entries.
pub(crate) fn collect_dashboard_screen_canvases(
    db: &Database,
    dashboard_record: &Record,
) -> Vec<(u32, u32, String, Option<String>)> {
    let views = match query_value_path(db, dashboard_record, "View") {
        Some(Value::Array(v)) => v,
        Some(other) => vec![other],
        None => return Vec::new(),
    };

    let mut result = Vec::new();
    for (view_idx, view) in views.iter().enumerate() {
        let screens = match value_object(view, "screens") {
            Some(Value::Array(s)) => s.iter().collect::<Vec<_>>(),
            Some(other) => vec![other],
            None => continue,
        };
        for (slot_idx, screen) in screens.iter().enumerate() {
            let path = match value_stringish(screen) {
                Some(p) if !p.is_empty() && p != "null" && !is_zero_guid(&p) => p,
                _ => continue,
            };
            let Some(canvas_record) = find_canvas_record_by_path_or_guid(db, &path) else {
                continue;
            };
            let guid = canvas_record.id.to_string();
            let name = {
                let n = db.resolve_string2(canvas_record.name_offset).to_string();
                if n.is_empty() { None } else { Some(n) }
            };
            result.push((view_idx as u32, slot_idx as u32, guid, name));
        }
    }
    result
}

fn value_stringish(value: &Value<'_>) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then_some((*text).to_string()),
        Value::Guid(guid) => Some(guid.to_string()),
        Value::Object { record_id: Some(guid), .. } => Some(guid.to_string()),
        _ => None,
    }
}

fn is_zero_guid(value: &str) -> bool {
    value.trim().trim_matches('{').trim_matches('}').chars().all(|ch| ch == '0' || ch == '-')
}

fn parse_guid(value: &str) -> Option<CigGuid> {
    let trimmed = value.trim().trim_matches('{').trim_matches('}');
    let mut bytes = [0u8; 16];
    let mut chars = trimmed.chars().filter(|ch| *ch != '-');
    for byte in &mut bytes {
        let hi = chars.next()?;
        let lo = chars.next()?;
        let hi = hi.to_digit(16)? as u8;
        let lo = lo.to_digit(16)? as u8;
        *byte = (hi << 4) | lo;
    }
    if chars.next().is_some() {
        return None;
    }
    Some(CigGuid::from_bytes(bytes))
}

fn default_display_screen_state(
    record: &Record,
    db: &Database,
) -> (Option<String>, Option<[u8; 4]>, Option<u16>) {
    let Some(states) = query_value_path(db, record, "Components[SCItemDisplayScreenComponentParams].screenStates")
    else {
        return (None, None, None);
    };
    let states = match states {
        Value::Array(values) => values,
        other => vec![other],
    };
    let selected = states
        .iter()
        .find(|state| {
            value_object_string(state, "statename").as_deref() == Some("Normal")
                && value_object(state, "stateLightParams")
                    .and_then(|params| value_object_bool(params, "lightOn"))
                    .unwrap_or(false)
        })
        .or_else(|| {
            states.iter().find(|state| {
                value_object_string(state, "statename")
                    .is_some_and(|name| !name.is_empty())
            })
        });
    let Some(selected) = selected else {
        return (None, None, None);
    };
    let default_state_name = value_object_string(selected, "statename");
    let default_light_color = value_object(selected, "stateLightParams")
        .and_then(|params| value_object(params, "color"))
        .and_then(value_rgba8);
    let default_light_intensity_milli = value_object(selected, "stateLightParams")
        .and_then(|params| value_object(params, "intensity"))
        .and_then(value_f64)
        .map(|value| (value.clamp(0.0, 65.535) * 1000.0).round() as u16);
    (
        default_state_name,
        default_light_color,
        default_light_intensity_milli,
    )
}

fn query_string_path(db: &Database, record: &Record, path: &str) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query_single::<String>(&compiled, record).ok().flatten())
        .filter(|value| !value.is_empty())
}

fn query_stringish_path(db: &Database, record: &Record, path: &str) -> Option<String> {
    query_value_path(db, record, path).and_then(|value| match value {
        Value::String(text) => (!text.is_empty()).then_some(text.to_string()),
        Value::Guid(guid) => Some(guid.to_string()),
        Value::Object { record_id: Some(guid), .. } => Some(guid.to_string()),
        _ => None,
    })
}

fn query_value_path<'a>(db: &'a Database<'a>, record: &'a Record, path: &str) -> Option<Value<'a>> {
    db.compile_path::<Value>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query_no_references(&compiled, record).ok())
        .and_then(|mut values| if values.is_empty() { None } else { Some(values.remove(0)) })
}

fn value_object<'a>(value: &'a Value<'a>, key: &str) -> Option<&'a Value<'a>> {
    match value {
        Value::Object { fields, .. } => fields.iter().find(|(name, _)| *name == key).map(|(_, value)| value),
        _ => None,
    }
}

fn value_object_string(value: &Value<'_>, key: &str) -> Option<String> {
    match value_object(value, key) {
        Some(Value::String(text)) if !text.is_empty() => Some((*text).to_string()),
        Some(Value::Guid(guid)) => Some(guid.to_string()),
        _ => None,
    }
}

fn value_object_bool(value: &Value<'_>, key: &str) -> Option<bool> {
    match value_object(value, key) {
        Some(Value::Bool(flag)) => Some(*flag),
        _ => None,
    }
}

fn value_rgba8(value: &Value<'_>) -> Option<[u8; 4]> {
    Some([
        value_object(value, "r").and_then(value_u8)?,
        value_object(value, "g").and_then(value_u8)?,
        value_object(value, "b").and_then(value_u8)?,
        value_object(value, "a").and_then(value_u8)?,
    ])
}

fn value_u8(value: &Value<'_>) -> Option<u8> {
    match value {
        Value::Int8(value) => Some(*value as u8),
        Value::UInt8(value) => Some(*value),
        Value::Int16(value) => u8::try_from(*value).ok(),
        Value::UInt16(value) => u8::try_from(*value).ok(),
        Value::Int32(value) => u8::try_from(*value).ok(),
        Value::UInt32(value) => u8::try_from(*value).ok(),
        _ => None,
    }
}

fn value_f64(value: &Value<'_>) -> Option<f64> {
    match value {
        Value::Float(value) => Some(f64::from(*value)),
        Value::Double(value) => Some(*value),
        Value::Int8(value) => Some(f64::from(*value)),
        Value::UInt8(value) => Some(f64::from(*value)),
        Value::Int16(value) => Some(f64::from(*value)),
        Value::UInt16(value) => Some(f64::from(*value)),
        Value::Int32(value) => Some(f64::from(*value)),
        Value::UInt32(value) => Some(f64::from(*value)),
        Value::Int64(value) => Some(*value as f64),
        Value::UInt64(value) => Some(*value as f64),
        _ => None,
    }
}

fn resolve_record_metadata(db: &Database, guid: &str) -> (Option<String>, Option<String>) {
    let Ok(record_guid) = guid.parse::<CigGuid>() else {
        return (None, None);
    };
    let Some(record) = db.record_by_id(&record_guid) else {
        return (None, None);
    };
    let name = Some(db.resolve_string2(record.name_offset).to_string()).filter(|value| !value.is_empty());
    let path = Some(db.resolve_string(record.file_name_offset).replace('\\', "/"))
        .filter(|value| !value.is_empty());
    (name, path)
}
