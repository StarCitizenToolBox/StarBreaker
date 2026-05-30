use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
    UiIrDocument, UiRegressionManifest, UiScreenSnapshot, UiSnapshotElement,
    compare_manifest_targets_with_loader, compile_ir_for_binding, snapshot_from_ui_ir,
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

fn compile_target_ir(
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
        helper_name: Some("manifest-live-ir-guard"),
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

    compile_ir_for_binding(&inputs).expect("target IR compile should succeed")
}

fn live_snapshot_manifest() -> UiRegressionManifest {
    let mut manifest: UiRegressionManifest = serde_json::from_str(include_str!(
        "fixtures/ui_ir/ui_snapshot_manifest.json"
    ))
    .expect("snapshot manifest fixture should parse");
    manifest
        .targets
        .retain(|target| target.id == "ui_target_a" || target.id == "ui_target_b");
    manifest
}

fn focused_movement_snapshot(snapshot: &UiScreenSnapshot) -> UiScreenSnapshot {
    let mut elements: Vec<UiSnapshotElement> = snapshot
        .elements
        .iter()
        .cloned()
        .map(|mut element| {
            // Preserve element sets/categories while normalizing style noise.
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

fn missing_font_metadata_nodes(document: &UiIrDocument) -> Vec<String> {
    document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .filter_map(|node| {
            let text = match node.text_payload.as_ref() {
                Some(starbreaker_ui::UiIrTextPayload::Resolved { text }) if !text.trim().is_empty() => text,
                _ => return None,
            };
            let style = node.text_style.as_ref()?;
            let font_record = style.font_record.as_deref().unwrap_or("");
            if font_record.is_empty() {
                return None;
            }

            let Some(resolved) = style.resolved_font_record.as_ref() else {
                return None;
            };
            let resolved_value = resolved.get("_RecordValue_").unwrap_or(resolved);
            let font_symbol = resolved_value
                .get("font")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let paint_file = resolved_value
                .get("paintFile")
                .and_then(|value| value.as_str())
                .unwrap_or("");

            if font_symbol.is_empty() || paint_file.is_empty() {
                Some(format!(
                    "id={} name={} text='{}' font_record='{}' font='{}' paintFile='{}'",
                    node.id,
                    node.name,
                    text,
                    font_record,
                    font_symbol,
                    paint_file,
                ))
            } else {
                None
            }
        })
        .collect()
}

fn load_live_and_baseline_target_irs() -> Option<(UiIrDocument, UiIrDocument, UiIrDocument, UiIrDocument)> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root should resolve from CARGO_MANIFEST_DIR");
    let canvas_root = workspace_root.join("ships/dcb_canvas/libs/foundry/records");
    if !canvas_root.is_dir() {
        eprintln!(
            "skipping live manifest IR guard (missing records root: {})",
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

    let med1_live = compile_target_ir(&fetcher, localization_map.clone(), med1_guid);
    let med2_live = compile_target_ir(&fetcher, localization_map, med2_guid);

    let med1_baseline: UiIrDocument = serde_json::from_str(include_str!(
        "fixtures/ui_ir/target_a-screen_16x9_a-ir.json"
    ))
    .expect("ui_target_a baseline should parse");
    let med2_baseline: UiIrDocument = serde_json::from_str(include_str!(
        "fixtures/ui_ir/target_b-mesh_end_screen_plane-ir.json"
    ))
    .expect("ui_target_b baseline should parse");

    Some((med1_live, med2_live, med1_baseline, med2_baseline))
}

#[test]
fn live_manifest_targets_have_no_visible_placeholder_text() {
    let Some((med1_live, med2_live, _, _)) = load_live_and_baseline_target_irs() else {
        return;
    };

    let med1_placeholders = visible_placeholder_nodes(&med1_live);
    let med2_placeholders = visible_placeholder_nodes(&med2_live);

    assert!(
        med1_placeholders.is_empty(),
        "ui_target_a gold-standard output contains visible placeholder text. This indicates broken UI generation, not baseline drift. Do not update baselines; investigate placeholder/default/localization handling first.\n{}",
        med1_placeholders.join("\n")
    );
    assert!(
        med2_placeholders.is_empty(),
        "ui_target_b gold-standard output contains visible placeholder text. This indicates broken UI generation, not baseline drift. Do not update baselines; investigate placeholder/default/localization handling first.\n{}",
        med2_placeholders.join("\n")
    );
}

#[test]
fn live_manifest_targets_match_gold_standard_snapshot_geometry() {
    let Some((med1_live, med2_live, med1_baseline, med2_baseline)) =
        load_live_and_baseline_target_irs()
    else {
        return;
    };

    if !visible_placeholder_nodes(&med1_live).is_empty() || !visible_placeholder_nodes(&med2_live).is_empty() {
        eprintln!(
            "skipping gold-standard geometry comparison because visible placeholder text already indicates broken target output"
        );
        return;
    }

    let manifest = live_snapshot_manifest();
    let snapshots = HashMap::from([
        (
            "ui_target_a.baseline".to_string(),
            focused_movement_snapshot(&snapshot_from_ui_ir(&med1_baseline)),
        ),
        (
            "ui_target_a.current".to_string(),
            focused_movement_snapshot(&snapshot_from_ui_ir(&med1_live)),
        ),
        (
            "ui_target_b.baseline".to_string(),
            focused_movement_snapshot(&snapshot_from_ui_ir(&med2_baseline)),
        ),
        (
            "ui_target_b.current".to_string(),
            focused_movement_snapshot(&snapshot_from_ui_ir(&med2_live)),
        ),
    ]);
    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot fixture for {path}"))
    })
    .expect("manifest runner should compare live snapshots against baselines");

    for result in results {
        assert!(
            result.comparison.passed,
            "{} gold-standard live IR drift. Do not update baselines unless the drift is intentional, source-backed, and explicitly approved.\n{}",
            result.id,
            result.comparison.failures.join("\n")
        );
    }
}

#[test]
fn live_manifest_targets_have_resolved_font_symbol_metadata() {
    let Some((med1_live, med2_live, _, _)) = load_live_and_baseline_target_irs() else {
        return;
    };

    let med1_missing = missing_font_metadata_nodes(&med1_live);
    let med2_missing = missing_font_metadata_nodes(&med2_live);

    assert!(
        med1_missing.is_empty(),
        "ui_target_a has active text nodes missing structural font metadata (font symbol/paintFile). This is a wrong-font risk and should be fixed in production data flow before baseline changes.\n{}",
        med1_missing.join("\n")
    );
    assert!(
        med2_missing.is_empty(),
        "ui_target_b has active text nodes missing structural font metadata (font symbol/paintFile). This is a wrong-font risk and should be fixed in production data flow before baseline changes.\n{}",
        med2_missing.join("\n")
    );
}
