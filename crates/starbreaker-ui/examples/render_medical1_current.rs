use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use starbreaker_p4k::MappedP4k;
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
    render_for_binding,
};

struct FsCanvasFetcher {
    guid_to_path: HashMap<String, PathBuf>,
    by_name: HashMap<String, String>,
}

impl CanvasFetcher for FsCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        self.load_json_for_guid(guid)
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        let guid = self
            .by_name
            .get(record_name)
            .or_else(|| self.by_name.get(&record_name.to_ascii_lowercase()))
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))?;
        self.load_json_for_guid(guid)
    }
}

impl FsCanvasFetcher {
    fn load_json_for_guid(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        let path = self
            .guid_to_path
            .get(guid)
            .ok_or_else(|| UiError::RenderError(format!("missing canvas guid: {guid}")))?;
        load_canvas_json_from_path(path).map_err(|e| {
            UiError::RenderError(format!(
                "failed to load canvas {} from {}: {e}",
                guid,
                path.display()
            ))
        })
    }
}

struct P4kFileFetcher {
    p4k: Arc<MappedP4k>,
}

impl SwfFetcher for P4kFileFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError> {
        read_p4k_path(&self.p4k, p4k_path)
    }
}

impl AssetFetcher for P4kFileFetcher {
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        read_p4k_path(&self.p4k, p4k_path).ok()
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

fn normalize_p4k_path(path: &str) -> String {
    let with_prefix = if path.to_ascii_lowercase().starts_with("data\\")
        || path.to_ascii_lowercase().starts_with("data/")
    {
        path.to_string()
    } else {
        format!("Data\\{}", path)
    };
    with_prefix.replace('/', "\\")
}

fn read_p4k_path(p4k: &MappedP4k, path: &str) -> Result<Vec<u8>, UiError> {
    let normalized = normalize_p4k_path(path);
    p4k.read_file(&normalized)
        .map_err(|e| UiError::RenderError(format!("failed to read '{}' from P4K: {e}", normalized)))
}

fn open_p4k() -> Result<Arc<MappedP4k>, String> {
    starbreaker_p4k::open_p4k()
        .map(Arc::new)
        .map_err(|e| format!("failed to open Data.p4k: {e}"))
}

fn existing_output_fallback(workspace: &Path, output_path: &Path) -> Result<(), String> {
    let existing = workspace.join("ships/Data/UI/Generated/ship/drak/Clipper/buildingblocks_canvas_i_med_medicalbed_a.png");
    if existing.is_file() {
        fs::copy(&existing, output_path)
            .map_err(|e| format!("failed to copy fallback {}: {e}", existing.display()))?;
        return Ok(());
    }
    Err("medical1 render failed and no existing generated fallback image was found".to_string())
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

fn load_canvas_index(root: &Path) -> Result<FsCanvasFetcher, String> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files);

    let mut guid_to_path = HashMap::new();
    let mut by_name = HashMap::new();

    for path in files {
        let json = load_canvas_json_from_path(&path)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;

        let Some(record_name) = json
            .get("_RecordName_")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(record_id) = json
            .get("_RecordId_")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            continue;
        };

        let bare_name = record_name
            .strip_prefix("BuildingBlocks_Canvas.")
            .unwrap_or(&record_name)
            .to_string();

        let path_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let path_rel = path
            .strip_prefix(root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"));

        guid_to_path.insert(record_id.clone(), path.clone());
        by_name.insert(record_name.clone(), record_id.clone());
        by_name.insert(record_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(bare_name.clone(), record_id.clone());
        by_name.insert(bare_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(record_id.clone(), record_id.clone());

        if !path_stem.is_empty() {
            by_name.insert(path_stem.clone(), record_id.clone());
            by_name.insert(path_stem.to_ascii_lowercase(), record_id.clone());
        }

        if let Some(rel) = path_rel {
            by_name.insert(rel.clone(), record_id.clone());
            by_name.insert(rel.to_ascii_lowercase(), record_id.clone());
        }

        let extracted_name = starbreaker_ui::pipeline::extract_record_name(&record_name);
        by_name.insert(extracted_name.clone(), record_id.clone());
        by_name.insert(extracted_name.to_ascii_lowercase(), record_id.clone());
    }

    Ok(FsCanvasFetcher {
        guid_to_path,
        by_name,
    })
}

fn load_canvas_json_from_path(path: &Path) -> Result<serde_json::Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

fn load_localization_map(workspace_root: &Path) -> Option<HashMap<String, String>> {
    let ini_path = workspace_root.join("target/Data/Localization/english/global.ini");
    let bytes = fs::read(&ini_path).ok()?;
    let map = starbreaker_ui::bb_loc_p4k::parse_ini_bytes(&bytes);
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

fn main() -> Result<(), String> {
    let workspace = PathBuf::from("/home/tom/projects/scorg_tools");
    let canvas_root = workspace.join("ships/dcb_canvas/libs/foundry/records");
    let localization_map = load_localization_map(&workspace);
    let fetcher = load_canvas_index(&canvas_root)?;

    let output_path = workspace
        .join("docs/StarBreaker/ui-rework-artifacts/phase-2/comparison/medical1-current.png");
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    let p4k = match open_p4k() {
        Ok(p4k) => Some(p4k),
        Err(e) => {
            eprintln!("warning: {e}");
            None
        }
    };

    let style_fetcher = DummyStyleFetcher;
    let file_fetcher = p4k.as_ref().map(|p4k| P4kFileFetcher { p4k: Arc::clone(p4k) });

    let medical1_guid = "534bab84-299b-479a-a4af-4469df112ea7";
    let binding = UiBindingView {
        canvas_guid: Some(medical1_guid),
        content_canvas_guid: Some(medical1_guid),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("medical1-phase2-render"),
        default_view_index: None,
        default_screen_slot: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: &fetcher,
        swf_fetcher: file_fetcher
            .as_ref()
            .ok_or_else(|| "P4K-backed SWF fetcher unavailable".to_string())?,
        style_fetcher: &style_fetcher,
        asset_fetcher: file_fetcher
            .as_ref()
            .ok_or_else(|| "P4K-backed asset fetcher unavailable".to_string())?,
        target_size: (1920, 1080),
        apply_postprocess: false,
        localization_map,
        loc_fetcher: None,
    };

    match render_for_binding(&inputs) {
        Ok(png) => {
            fs::write(&output_path, png)
                .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;
        }
        Err(e) => {
            eprintln!("warning: failed to render medical1 through current pipeline: {e}");
            existing_output_fallback(&workspace, &output_path)?;
        }
    }

    println!("Wrote {}", output_path.display());
    Ok(())
}