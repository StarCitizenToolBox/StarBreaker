use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
    UiIrDocument, UiScreenSnapshot, UiSnapshotElement, UiSnapshotTolerance, compare_snapshots,
    compile_ir_for_binding,
    snapshot_from_ui_ir,
};

struct FsCanvasFetcher {
    guid_to_path: HashMap<String, PathBuf>,
    by_name: HashMap<String, String>,
}

impl CanvasFetcher for FsCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        let path = self
            .guid_to_path
            .get(guid)
            .ok_or_else(|| UiError::RenderError(format!("missing canvas guid: {guid}")))?;
        load_canvas_json_from_path(path)
            .map_err(|err| UiError::RenderError(format!("failed loading canvas {guid}: {err}")))
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        let guid = self
            .by_name
            .get(record_name)
            .or_else(|| self.by_name.get(&record_name.to_ascii_lowercase()))
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))?;
        self.fetch_canvas_json(guid)
    }
}

struct DummySwfFetcher;

impl SwfFetcher for DummySwfFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("SWF fetch not required".to_string()))
    }
}

struct DummyStyleFetcher;

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Ok(starbreaker_ui::StyleLoader::for_manufacturer("drak").drake_amber_fallback())
    }
}

struct DummyAssetFetcher;

impl AssetFetcher for DummyAssetFetcher {
    fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
        None
    }
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

fn load_canvas_json_from_path(path: &Path) -> Result<serde_json::Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn load_canvas_index(root: &Path) -> Result<FsCanvasFetcher, String> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files);

    let mut guid_to_path = HashMap::new();
    let mut by_name = HashMap::new();

    for path in files {
        let json = load_canvas_json_from_path(&path)?;
        let Some(record_name) = json
            .get("_RecordName_")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(record_id) = json
            .get("_RecordId_")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
        else {
            continue;
        };

        let bare_name = record_name
            .strip_prefix("BuildingBlocks_Canvas.")
            .unwrap_or(&record_name)
            .to_string();

        guid_to_path.insert(record_id.clone(), path.clone());
        by_name.insert(record_name.clone(), record_id.clone());
        by_name.insert(record_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(bare_name.clone(), record_id.clone());
        by_name.insert(bare_name.to_ascii_lowercase(), record_id.clone());
    }

    Ok(FsCanvasFetcher { guid_to_path, by_name })
}

fn compile_medical_ir(
    fetcher: &FsCanvasFetcher,
    localization_map: Option<HashMap<String, String>>,
    canvas_guid: &str,
) -> UiIrDocument {
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some(canvas_guid),
        content_canvas_guid: Some(canvas_guid),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("medical-live-ir-guard"),
        default_view_index: None,
        default_screen_slot: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: fetcher,
        swf_fetcher: &swf_fetcher,
        style_fetcher: &style_fetcher,
        asset_fetcher: &asset_fetcher,
        target_size: (1920, 1080),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map,
        loc_fetcher: None,
    };

    compile_ir_for_binding(&inputs).expect("medical IR compile should succeed")
}

fn focused_movement_snapshot(snapshot: &UiScreenSnapshot) -> UiScreenSnapshot {
    let monitored_texts = ["PATIENT NAME", "No patient in bed", "MEDGELS", "200/200"];
    let mut elements: Vec<UiSnapshotElement> = snapshot
        .elements
        .iter()
        .filter(|element| {
            element
                .text_payload
                .as_deref()
                .is_some_and(|text| monitored_texts.contains(&text))
                || element
                    .node_type
                    .eq_ignore_ascii_case("component_general_button_secondary")
        })
        .cloned()
        .map(|mut element| {
            // Compare only movement-critical fields so source/style refreshes do
            // not mask layout regressions for these tracked UI anchors.
            element.alpha = 1.0;
            element.blend_mode = None;
            element.asset_identity = None;
            element.alignment = None;
            element.vertical_alignment = None;
            element.overflow_mode = None;
            element.background_rgba = None;
            element.stroke_rgba = None;
            element.text_rgba = None;
            element.icon_tint_rgba = None;
            element.stroke_extent = None;
            element.text_font_identity = None;
            element.line_spacing = None;
            element
        })
        .collect();
    elements.sort_by(|a, b| a.identity.cmp(&b.identity));

    UiScreenSnapshot {
        schema_version: snapshot.schema_version,
        canvas_guid: snapshot.canvas_guid.clone(),
        canvas_name: snapshot.canvas_name.clone(),
        target_width: snapshot.target_width,
        target_height: snapshot.target_height,
        elements,
    }
}

fn visible_placeholder_nodes(document: &UiIrDocument) -> Vec<String> {
    document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .filter_map(|node| {
            let payload = node.text_payload.as_ref()?;
            match payload {
                starbreaker_ui::UiIrTextPayload::Resolved { text }
                    if text.contains("PLACEHOLDER") =>
                {
                    Some(format!("id={} name={} resolved={text}", node.id, node.name))
                }
                starbreaker_ui::UiIrTextPayload::UnresolvedKey { key }
                    if key.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER") =>
                {
                    Some(format!("id={} name={} unresolved={key}", node.id, node.name))
                }
                _ => None,
            }
        })
        .collect()
}

fn load_live_and_baseline_medical_irs() -> Option<(UiIrDocument, UiIrDocument, UiIrDocument, UiIrDocument)> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root should resolve from CARGO_MANIFEST_DIR");
    let canvas_root = workspace_root.join("ships/dcb_canvas/libs/foundry/records");
    if !canvas_root.is_dir() {
        eprintln!(
            "skipping live medical IR guard (missing records root: {})",
            canvas_root.display()
        );
        return None;
    }
    let fetcher = load_canvas_index(&canvas_root).expect("canvas index should load");

    let localization_path = workspace_root.join("target/Data/Localization/english/global.ini");
    let localization_map = fs::read(&localization_path)
        .ok()
        .map(|bytes| starbreaker_ui::bb_loc_p4k::parse_ini_bytes(&bytes));

    let med1_guid = "534bab84-299b-479a-a4af-4469df112ea7";
    let med2_guid = "e9ad809d-ebcf-43a3-bb20-120f64556aef";

    let med1_live = compile_medical_ir(&fetcher, localization_map.clone(), med1_guid);
    let med2_live = compile_medical_ir(&fetcher, localization_map, med2_guid);

    let med1_baseline: UiIrDocument = serde_json::from_str(include_str!(
        "fixtures/medical_ir/medical1-screen_16x9_a-ir.json"
    ))
    .expect("medical1 baseline should parse");
    let med2_baseline: UiIrDocument = serde_json::from_str(include_str!(
        "fixtures/medical_ir/medical2-mesh_end_screen_plane-ir.json"
    ))
    .expect("medical2 baseline should parse");

    Some((med1_live, med2_live, med1_baseline, med2_baseline))
}

#[test]
fn live_medical_ir_gold_standard_has_no_visible_placeholder_text() {
    let Some((med1_live, med2_live, _, _)) = load_live_and_baseline_medical_irs() else {
        return;
    };

    let med1_placeholders = visible_placeholder_nodes(&med1_live);
    let med2_placeholders = visible_placeholder_nodes(&med2_live);

    assert!(
        med1_placeholders.is_empty(),
        "medical1 gold-standard output contains visible placeholder text. This indicates broken UI generation, not baseline drift. Do not update baselines; investigate placeholder/default/localization handling first.\n{}",
        med1_placeholders.join("\n")
    );
    assert!(
        med2_placeholders.is_empty(),
        "medical2 gold-standard output contains visible placeholder text. This indicates broken UI generation, not baseline drift. Do not update baselines; investigate placeholder/default/localization handling first.\n{}",
        med2_placeholders.join("\n")
    );
}

#[test]
fn live_medical_ir_matches_gold_standard_snapshot_geometry() {
    let Some((med1_live, med2_live, med1_baseline, med2_baseline)) =
        load_live_and_baseline_medical_irs()
    else {
        return;
    };

    if !visible_placeholder_nodes(&med1_live).is_empty() || !visible_placeholder_nodes(&med2_live).is_empty() {
        eprintln!(
            "skipping gold-standard geometry comparison because visible placeholder text already indicates broken medical output"
        );
        return;
    }

    let med1_cmp = compare_snapshots(
        &focused_movement_snapshot(&snapshot_from_ui_ir(&med1_baseline)),
        &focused_movement_snapshot(&snapshot_from_ui_ir(&med1_live)),
        UiSnapshotTolerance::default(),
    );
    let med2_cmp = compare_snapshots(
        &focused_movement_snapshot(&snapshot_from_ui_ir(&med2_baseline)),
        &focused_movement_snapshot(&snapshot_from_ui_ir(&med2_live)),
        UiSnapshotTolerance::default(),
    );

    assert!(
        med1_cmp.passed,
        "medical1 gold-standard live IR drift. Do not update baselines unless the drift is intentional, source-backed, and explicitly approved.\n{}",
        med1_cmp.failures.join("\n")
    );
    assert!(
        med2_cmp.passed,
        "medical2 gold-standard live IR drift. Do not update baselines unless the drift is intentional, source-backed, and explicitly approved.\n{}",
        med2_cmp.failures.join("\n")
    );
}
