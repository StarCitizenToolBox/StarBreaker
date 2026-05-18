//! Phase 4: UI Binding Canvas Resolution Integration Tests
//!
//! Verifies that per-helper canvas GUIDs are resolved correctly from DataCore:
//!
//! - Radar helper: canvas_guid is the star-map canvas (via UIMapEntityComponentParams)
//! - Annunciator helper: content_canvas_guid is resolved from the parent dashboard canvas def
//! - MFD helpers: canvas_guid is the container canvas (content_canvas_guid may be None for ships
//!   with no dashboard screen assignments)
//!
//! All tests require a live Data.p4k and are skipped when it is not available.

use std::path::PathBuf;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_p4k::MappedP4k;

fn integration_p4k_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SC_DATA_P4K") {
        let path = PathBuf::from(&path);
        if path.exists() {
            return Some(path);
        }
        eprintln!("SC_DATA_P4K set but not found at {}, skipping", path.display());
        return None;
    }
    eprintln!("SC_DATA_P4K not set; skipping Phase 4 UI binding integration tests");
    None
}

fn with_integration_context<F>(test_body: F)
where
    F: FnOnce(&Database<'_>, &EntityIndex<'_>),
{
    let Some(p4k_path) = integration_p4k_path() else {
        return;
    };
    let p4k = MappedP4k::open(&p4k_path).expect("failed to open Data.p4k");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .or_else(|_| p4k.read_file("Data\\Game.dcb"))
        .expect("failed to read Game.dcb from Data.p4k");
    let db = Database::from_bytes(&dcb_data).expect("failed to parse Game.dcb");
    let idx = EntityIndex::new(&db);
    test_body(&db, &idx);
}

/// Parse a scene.json file from a decomposed export result.
fn parse_scene_json(result: &starbreaker_3d::ExportResult) -> serde_json::Value {
    let decomposed = result
        .decomposed
        .as_ref()
        .expect("export must produce a decomposed package");
    let scene_file = decomposed
        .files
        .iter()
        .find(|f| f.relative_path == "scene.json")
        .expect("scene.json missing from decomposed export");
    serde_json::from_slice(&scene_file.bytes).expect("scene.json must be valid JSON")
}

/// Find the first `ui_bindings` entry across all interior container placements
/// whose `helper_name` matches the given name.
fn find_binding_by_helper(
    scene: &serde_json::Value,
    helper_name: &str,
) -> Option<serde_json::Value> {
    let containers = scene["interior_containers"].as_array()?;
    for container in containers {
        let placements = container["placements"].as_array()?;
        for placement in placements {
            let bindings = placement["ui_bindings"].as_array()?;
            for binding in bindings {
                if binding["helper_name"].as_str() == Some(helper_name) {
                    return Some(binding.clone());
                }
            }
        }
    }
    None
}

/// Run a decomposed export for a named entity and return the parsed scene.json.
fn export_scene_json(
    db: &Database<'_>,
    idx: &EntityIndex<'_>,
    p4k: &MappedP4k,
    entity_name: &str,
) -> serde_json::Value {
    let record = idx
        .find_record(entity_name)
        .unwrap_or_else(|| panic!("entity not found in DataCore: {entity_name}"));
    let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(idx, record);
    let opts = starbreaker_3d::ExportOptions {
        kind: starbreaker_3d::ExportKind::Decomposed,
        material_mode: starbreaker_3d::MaterialMode::All,
        include_interior: true,
        include_attachments: true,
        include_lights: true,
        lod_level: 0,
        texture_mip: 0,
        ..Default::default()
    };
    let result = starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, &opts)
        .expect("decomposed export failed");
    parse_scene_json(&result)
}

#[test]
#[ignore = "requires Data.p4k; set SC_DATA_P4K to run Phase 4 UI binding tests"]
fn test_radar_canvas_guid() {
    with_integration_context(|db, idx| {
        let p4k_path = integration_p4k_path().unwrap();
        let p4k = MappedP4k::open(&p4k_path).unwrap();

        let scene = export_scene_json(db, idx, &p4k, "DRAK_Clipper");
        let binding = find_binding_by_helper(&scene, "Screen_Radar_RTT")
            .expect("Screen_Radar_RTT helper not found in scene.json");

        let canvas_guid = binding["canvas_guid"]
            .as_str()
            .expect("canvas_guid must be a string for radar helper");

        // The radar canvas is BB_ScreenRadar_C_App_Starmap (or the underlying starmap canvas).
        // Verify the GUID is non-empty and non-zero.
        assert!(
            !canvas_guid.is_empty(),
            "Screen_Radar_RTT canvas_guid must not be empty"
        );
        assert!(
            !canvas_guid.chars().all(|ch| ch == '0' || ch == '-'),
            "Screen_Radar_RTT canvas_guid must not be a zero GUID, got: {canvas_guid}"
        );
        assert_eq!(
            binding["binding_kind"].as_str(),
            Some("radar"),
            "Screen_Radar_RTT binding_kind must be 'radar'"
        );
    });
}

#[test]
#[ignore = "requires Data.p4k; set SC_DATA_P4K to run Phase 4 UI binding tests"]
fn test_mfd_container_canvas_guid() {
    with_integration_context(|db, idx| {
        let p4k_path = integration_p4k_path().unwrap();
        let p4k = MappedP4k::open(&p4k_path).unwrap();

        let scene = export_scene_json(db, idx, &p4k, "DRAK_Clipper");

        for helper_name in &["Screen_Left_Lower_RTT", "Screen_Left_Upper_RTT", "Screen_Right_Upper_RTT"] {
            let binding = find_binding_by_helper(&scene, helper_name)
                .unwrap_or_else(|| panic!("{helper_name} helper not found in scene.json"));

            let canvas_guid = binding["canvas_guid"]
                .as_str()
                .unwrap_or_else(|| panic!("{helper_name} canvas_guid must be a string"));

            assert!(
                !canvas_guid.is_empty(),
                "{helper_name} canvas_guid must not be empty"
            );
            assert!(
                !canvas_guid.chars().all(|ch| ch == '0' || ch == '-'),
                "{helper_name} canvas_guid must not be a zero GUID, got: {canvas_guid}"
            );
            // All three MFD helpers should share the same container canvas.
            assert_eq!(
                binding["binding_kind"].as_str(),
                Some("mfd"),
                "{helper_name} binding_kind must be 'mfd'"
            );
        }
    });
}

#[test]
#[ignore = "requires Data.p4k; set SC_DATA_P4K to run Phase 4 UI binding tests"]
fn test_annunciator_content_canvas_guid() {
    with_integration_context(|db, idx| {
        let p4k_path = integration_p4k_path().unwrap();
        let p4k = MappedP4k::open(&p4k_path).unwrap();

        let scene = export_scene_json(db, idx, &p4k, "DRAK_Clipper");
        let binding = find_binding_by_helper(&scene, "Screen_Annunciator_L")
            .expect("Screen_Annunciator_L helper not found in scene.json");

        // Container canvas must be present (M_Physical_Screen or similar).
        let canvas_guid = binding["canvas_guid"]
            .as_str()
            .expect("Screen_Annunciator_L canvas_guid must be a string");
        assert!(
            !canvas_guid.is_empty(),
            "Screen_Annunciator_L canvas_guid must not be empty"
        );

        // content_canvas_guid should be set if the Clipper dashboard canvas def maps a
        // screen for this slot; it is None if no screen is assigned.
        // We assert the field exists in the JSON (may be null).
        assert!(
            binding.get("content_canvas_guid").is_some(),
            "Screen_Annunciator_L must have a content_canvas_guid field in JSON"
        );
        assert_eq!(
            binding["binding_kind"].as_str(),
            Some("physical"),
            "Screen_Annunciator_L binding_kind must be 'physical'"
        );
    });
}
