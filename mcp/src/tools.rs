use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_router,
};
use starbreaker_datacore::database::Database;
use starbreaker_p4k::{MappedP4k, P4kArchive};

/// Lazily-loaded game data. Initialized on first tool call.
struct GameData {
    p4k_path: PathBuf,
    p4k: Arc<MappedP4k>,
    dcb_bytes: &'static [u8],
    db: Database<'static>,
}

const MAX_DATACORE_QUERY_RESULTS: usize = 32;
const MAX_DATACORE_QUERY_JSON_NODES: usize = 50_000;
const MAX_DATACORE_QUERY_JSON_DEPTH: usize = 16;
const MAX_DATACORE_QUERY_ARRAY_ITEMS: usize = 256;
const MAX_DATACORE_QUERY_OBJECT_FIELDS: usize = 256;
const MAX_DATACORE_QUERY_STRING_CHARS: usize = 2048;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kSetDataPathRequest {
    #[schemars(description = "Absolute path to the Data.p4k file to use for subsequent StarBreaker MCP queries.")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchEntitiesRequest {
    #[schemars(description = "Case-insensitive name substring to search for")]
    pub query: String,
    #[schemars(description = "Maximum number of results (default 20)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityLoadoutRequest {
    #[schemars(description = "Entity name (substring match, uses shortest match)")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DatacoreRecordRequest {
    #[schemars(description = "Record GUID or name substring")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DatacoreQueryRequest {
    #[schemars(description = "Record GUID or name substring")]
    pub id: String,
    #[schemars(description = "DataCore property path (e.g. 'Components[VehicleComponentParams].vehicleDefinition')")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kReadRequest {
    #[schemars(description = "File path within P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kListRequest {
    #[schemars(description = "Directory path within P4k (e.g. 'Data\\Objects\\Spaceships'). Empty string for root.")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchRecordsRequest {
    #[schemars(description = "Case-insensitive name substring to search for")]
    pub query: String,
    #[schemars(description = "Optional struct type filter (e.g. 'EntityClassDefinition', 'TintPalette')")]
    pub struct_type: Option<String>,
    #[schemars(description = "Maximum number of results (default 20)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ImagePreviewRequest {
    #[schemars(description = "File path within P4k (DDS, PNG, JPG, etc.). For DDS, .tif extension is auto-converted to .dds")]
    pub path: String,
    #[schemars(description = "Mip level for DDS textures (0=full res, default 0)")]
    pub mip: Option<u32>,
    #[schemars(description = "Cubemap face index (0-5) for cubemap DDS. Omit for 2D textures. (Not yet implemented)")]
    #[allow(dead_code)]
    pub face: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChunkListRequest {
    #[schemars(description = "File path within P4k (.cga, .cgf, .cgam, .cgfm, .skin, .skinm, .chr, .soc)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChunkReadRequest {
    #[schemars(description = "File path within P4k")]
    pub path: String,
    #[schemars(description = "Chunk index (from chunk_list output). If omitted, returns all chunks.")]
    pub index: Option<u32>,
    #[schemars(description = "Maximum bytes to show per chunk in hex dump (default 256)")]
    pub max_bytes: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kSearchRequest {
    #[schemars(description = "Case-insensitive substring to search for in P4k file paths")]
    pub query: String,
    #[schemars(description = "Maximum number of results (default 50)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SocpakInspectRequest {
    #[schemars(description = "Path to a .socpak file in P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SocpakReadEntryRequest {
    #[schemars(description = "Path to a .socpak file in P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
    #[schemars(description = "Inner entry path within the .socpak ZIP (case-insensitive). Use the names returned by socpak_inspect.")]
    pub entry: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MtlSummaryRequest {
    #[schemars(description = "Path to .mtl file in P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DbaDumpRequest {
    #[schemars(description = "Path to a .dba or .caf file in P4k (case-insensitive, Data\\ prefix optional). May also be an absolute filesystem path.")]
    pub path: String,
    #[schemars(description = "Optional path to a rig source used to resolve bone hashes to names. Accepts either a .chr / .skin / .skinm skeleton (CompiledBones chunk) OR a .cga / .cgam scene-graph file (NMC chunk — used by ships like the Scorpius whose main body has no CHR). Same path resolution as 'path'. Required for bone_filter to work.")]
    pub skeleton: Option<String>,
    #[schemars(description = "Filter CLIPS by case-insensitive substring match on the clip metadata name (e.g. 'wings_deploy'). Independent of bone_filter.")]
    pub filter: Option<String>,
    #[schemars(description = "Filter CHANNELS by case-insensitive substring match on the resolved bone name (e.g. 'Wing_Mechanism'). Requires `skeleton` to be set; channels with unresolved hashes are excluded when this is active.")]
    pub bone_filter: Option<String>,
    #[schemars(description = "If true, include every keyframe per channel; otherwise only first/last samples are included. Default false.")]
    pub all_keyframes: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MannequinDumpRequest {
    #[schemars(description = "Entity name (substring match, uses shortest match) used to resolve the entity's SAnimationControllerParams.AnimationDatabase and AnimationController paths.")]
    pub entity: String,
    #[schemars(description = "Optional case-insensitive substring filter against fragment group name, GUID, or animation name.")]
    pub filter: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendSdnaRequest {
    #[schemars(description = "Absolute filesystem path to a .blend file (can be zstd-compressed Blender 5.x or uncompressed).")]
    pub path: String,
    #[schemars(description = "Optional struct name to look up (e.g. 'Object', 'Mesh', 'Light'). If omitted returns all struct names.")]
    pub struct_name: Option<String>,
    #[schemars(description = "Maximum recursion depth for nested struct fields (default 1, 0 = top-level only).")]
    pub max_depth: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendBlockInspectRequest {
    #[schemars(description = "Absolute filesystem path to a .blend file.")]
    pub path: String,
    #[schemars(description = "SDNA struct type name to filter blocks by (e.g. 'Object', 'Lamp'). If omitted, returns an overview of all block types.")]
    pub sdna_type: Option<String>,
    #[schemars(description = "Maximum bytes of raw hex to show per block (default 256, 0 = no hex dump).")]
    pub max_bytes: Option<u32>,
    #[schemars(description = "Maximum number of blocks to show (default 10).")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendPythonDiffRequest {
    #[schemars(description = "Absolute filesystem path to the .blend file to modify.")]
    pub blend_path: String,
    #[schemars(description = "Python script to run BEFORE saving (sets up a baseline). Pass empty string to skip baseline creation.")]
    pub before_script: String,
    #[schemars(description = "Python script to run AFTER loading the baseline (applies the change under test). Required.")]
    pub after_script: String,
    #[schemars(description = "Optional SDNA struct name to restrict the diff to blocks of that type (e.g. 'NodeTexImage'). If omitted, all changed blocks are shown.")]
    pub sdna_filter: Option<String>,
    #[schemars(description = "Override path to the Blender binary. Falls back to BLENDER_BIN env var, then PATH.")]
    pub blender_bin: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendRunScriptRequest {
    #[schemars(description = "Absolute filesystem path to a .blend file to open with Blender.")]
    pub blend_path: String,
    #[schemars(description = "Python script body to execute in headless Blender after opening the file. \
        Wrap your output in print() calls. Output between __BLEND_SCRIPT_OUTPUT_START__ and \
        __BLEND_SCRIPT_OUTPUT_END__ sentinels is returned; if sentinels are absent the full \
        stdout+stderr is returned.")]
    pub script: String,
    #[schemars(description = "Override path to the Blender binary. Falls back to BLENDER_BIN env var, then PATH.")]
    pub blender_bin: Option<String>,
}


pub struct StarBreakerMcp {
    p4k_path: RwLock<Option<PathBuf>>,
    data: RwLock<Option<Arc<GameData>>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl StarBreakerMcp {
    pub fn new(p4k_path: Option<std::path::PathBuf>) -> Self {
        Self {
            p4k_path: RwLock::new(p4k_path),
            data: RwLock::new(None),
            tool_router: Self::tool_router(),
        }
    }

    fn load_game_data(path_override: Option<PathBuf>) -> anyhow::Result<GameData> {
        let start = std::time::Instant::now();
        let (p4k_path, p4k) = match path_override {
            Some(path) => {
                let p4k = starbreaker_p4k::MappedP4k::open(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to open P4k at {}: {e}", path.display()))?;
                (path, p4k)
            }
            None => {
                let (path, source) = starbreaker_p4k::find_p4k()
                    .map_err(|e| anyhow::anyhow!("Failed to auto-discover P4k: {e}"))?;
                let p4k = starbreaker_p4k::MappedP4k::open(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to open auto-discovered P4k at {}: {e}", path.display()))?;
                log::info!("P4K: {} ({source})", path.display());
                (path, p4k)
            }
        };
        let p4k = Arc::new(p4k);
        log::info!("P4k loaded from {} in {:.1}s", p4k_path.display(), start.elapsed().as_secs_f32());

        let dcb_bytes = p4k
            .read_file("Data\\Game2.dcb")
            .or_else(|_| p4k.read_file("Data\\Game.dcb"))
            .map_err(|e| anyhow::anyhow!("Failed to read DataCore from {}: {e}", p4k_path.display()))?;
        let dcb_bytes: &'static [u8] = Box::leak(dcb_bytes.into_boxed_slice());
        let db = Database::from_bytes(dcb_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse DataCore from {}: {e}", p4k_path.display()))?;
        log::info!(
            "DataCore: {} bytes, loaded in {:.1}s",
            dcb_bytes.len(),
            start.elapsed().as_secs_f32()
        );

        Ok(GameData {
            p4k_path,
            p4k,
            dcb_bytes,
            db,
        })
    }

    /// Lazily load P4k and DataCore on first access.
    fn data(&self) -> Arc<GameData> {
        if let Some(data) = self.data.read().expect("data lock poisoned").as_ref().cloned() {
            return data;
        }

        let mut data_guard = self.data.write().expect("data lock poisoned");
        if let Some(data) = data_guard.as_ref().cloned() {
            return data;
        }

        let p4k_path = self.p4k_path.read().expect("p4k path lock poisoned").clone();
        let data = Arc::new(
            Self::load_game_data(p4k_path)
                .unwrap_or_else(|e| panic!("{e}")),
        );
        *data_guard = Some(data.clone());
        data
    }

    fn switch_p4k_path(&self, path: PathBuf) -> anyhow::Result<Arc<GameData>> {
        let data = Arc::new(Self::load_game_data(Some(path.clone()))?);
        *self.p4k_path.write().expect("p4k path lock poisoned") = Some(path);
        *self.data.write().expect("data lock poisoned") = Some(data.clone());
        Ok(data)
    }

    /// Find an entity record by name substring (shortest match).
    fn find_entity<'a>(
        &self,
        db: &'a starbreaker_datacore::database::Database<'a>,
        search: &str,
    ) -> Option<&'a starbreaker_datacore::types::Record> {
        let search = search.to_lowercase();
        let entity_si = db.struct_id("EntityClassDefinition")?;
        let mut candidates: Vec<_> = db
            .records_of_type(entity_si)
            .filter(|r| {
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        candidates.first().copied()
    }

    /// Normalize a path for P4k lookup (ensure Data\ prefix, backslashes).
    fn normalize_p4k_path(path: &str) -> String {
        let p = if path.to_lowercase().starts_with("data\\") || path.to_lowercase().starts_with("data/") {
            path.replace('/', "\\")
        } else {
            format!("Data\\{}", path.replace('/', "\\"))
        };
        // Auto-convert .tif to .dds for texture lookups
        if p.to_lowercase().ends_with(".tif") {
            format!("{}.dds", &p[..p.len() - 4])
        } else {
            p
        }
    }

    fn split_socpak_entry_path(path: &str) -> Option<(&str, &str)> {
        let (outer, inner) = path.split_once("::")?;
        let outer = outer.trim();
        let inner = inner.trim();
        if outer.is_empty() || inner.is_empty() {
            return None;
        }
        Some((outer, inner))
    }

    fn read_p4k_file_direct(&self, path: &str) -> Result<Vec<u8>, String> {
        let p4k_path = Self::normalize_p4k_path(path);
        let data = self.data();
        data.p4k.read_file(&p4k_path)
            .or_else(|_| {
                data.p4k.entry_case_insensitive(&p4k_path)
                    .ok_or_else(|| format!("File not found in P4k: {p4k_path}"))
                    .and_then(|entry| data.p4k.read(entry).map_err(|e| format!("Error reading: {e}")))
            })
            .map_err(|e| format!("{e}"))
    }

    /// Read a file from P4k with case-insensitive fallback.
    fn read_p4k_file(&self, path: &str) -> Result<Vec<u8>, String> {
        if let Some((socpak_path, entry_path)) = Self::split_socpak_entry_path(path) {
            let socpak_bytes = self.read_p4k_file_direct(socpak_path)?;
            let archive = P4kArchive::from_bytes(&socpak_bytes)
                .map_err(|e| format!("Failed to parse socpak ZIP '{}': {e}", socpak_path))?;
            let requested = entry_path.replace('/', "\\").to_ascii_lowercase();
            let entry = archive
                .entries()
                .iter()
                .find(|entry| entry.name.replace('/', "\\").to_ascii_lowercase() == requested)
                .ok_or_else(|| {
                    format!(
                        "Entry not found in socpak '{}': {}",
                        Self::normalize_p4k_path(socpak_path),
                        entry_path
                    )
                })?;
            return archive
                .read(entry)
                .map_err(|e| format!("Failed to read '{}' from '{}': {e}", entry_path, socpak_path));
        }
        self.read_p4k_file_direct(path)
    }

    /// Read a file either from disk (if the path exists on disk) or
    /// from P4k. Used by debug tools that may receive either an
    /// extracted scratch file or a P4k-internal path.
    fn read_p4k_or_disk(&self, path: &str) -> Result<Vec<u8>, String> {
        let direct = std::path::Path::new(path);
        if direct.is_file() {
            return std::fs::read(direct).map_err(|e| format!("disk read failed: {e}"));
        }
        self.read_p4k_file(path)
    }

    fn decode_archive_entry_bytes(path: &str, data: &[u8]) -> String {
        let lower = path.to_lowercase();
        let is_cryxml_ext = lower.ends_with(".xml")
            || lower.ends_with(".mtl")
            || lower.ends_with(".chrparams")
            || lower.ends_with(".cdf")
            || lower.ends_with(".adb")
            || lower.ends_with(".comb")
            || lower.ends_with(".entxml");

        if is_cryxml_ext {
            if let Ok(xml) = starbreaker_cryxml::from_bytes(data) {
                return format!("{xml}");
            }
            if let Ok(text) = std::str::from_utf8(data) {
                return text.to_string();
            }
        }

        if let Ok(text) = std::str::from_utf8(data) {
            return text.to_string();
        }

        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        format!("[base64, {} bytes]\n{encoded}", data.len())
    }

    /// Find any record by GUID or name substring.
    fn find_record<'a>(
        &self,
        db: &'a starbreaker_datacore::database::Database<'a>,
        id: &str,
    ) -> Option<&'a starbreaker_datacore::types::Record> {
        if let Ok(guid) = id.parse::<starbreaker_common::CigGuid>() {
            return db.record_by_id(&guid);
        }
        let search = id.to_lowercase();
        let mut candidates: Vec<_> = db
            .records()
            .iter()
            .filter(|r| {
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        candidates.first().copied()
    }
}

#[tool_router]
impl StarBreakerMcp {
    #[tool(description = "Return the currently active Data.p4k path used by StarBreaker MCP tools.")]
    fn p4k_data_status(&self) -> String {
        let data = self.data();
        serde_json::to_string_pretty(&serde_json::json!({
            "p4k_path": data.p4k_path,
            "entries": data.p4k.entries().len(),
            "datacore_bytes": data.dcb_bytes.len(),
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Switch StarBreaker MCP to a different Data.p4k for subsequent tools. The new archive and DataCore are loaded immediately; use p4k_data_status to confirm.")]
    fn p4k_set_data_path(&self, Parameters(req): Parameters<P4kSetDataPathRequest>) -> String {
        let path = Path::new(&req.path);
        if !path.is_file() {
            return format!("Data.p4k not found or not a file: {}", path.display());
        }
        match self.switch_p4k_path(path.to_path_buf()) {
            Ok(data) => serde_json::to_string_pretty(&serde_json::json!({
                "p4k_path": data.p4k_path,
                "entries": data.p4k.entries().len(),
                "datacore_bytes": data.dcb_bytes.len(),
            }))
            .unwrap_or_else(|e| format!("JSON error: {e}")),
            Err(e) => format!("Failed to switch Data.p4k: {e}"),
        }
    }

    #[tool(description = "Search DataCore for entity records by name substring. Returns JSON array of matches sorted by name length (best match first).")]
    fn search_entities(&self, Parameters(req): Parameters<SearchEntitiesRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let limit = req.limit.unwrap_or(20) as usize;
        let search = req.query.to_lowercase();

        let entity_si = match db.struct_id("EntityClassDefinition") {
            Some(si) => si,
            None => return "[]".to_string(),
        };

        let mut results: Vec<_> = db
            .records_of_type(entity_si)
            .filter(|r| {
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        results.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        results.truncate(limit);

        let json: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let name = db.resolve_string2(r.name_offset);
                let struct_type = db.resolve_string2(db.struct_def(r.struct_index).name_offset);
                let path = db.resolve_string(r.file_name_offset);
                serde_json::json!({
                    "name": format!("{struct_type}.{name}"),
                    "id": format!("{}", r.id),
                    "struct_type": struct_type,
                    "path": path,
                })
            })
            .collect();

        serde_json::to_string_pretty(&json).unwrap_or_else(|_| "[]".to_string())
    }

    #[tool(description = "Dump the resolved loadout tree for an entity. This is PROCESSED output from StarBreaker's loadout resolver — it resolves entityClassName references and queries geometry paths. For raw DataCore data, use datacore_query with path 'Components[SEntityComponentDefaultLoadoutParams]' instead.")]
    fn entity_loadout(&self, Parameters(req): Parameters<EntityLoadoutRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let record = match self.find_entity(&db, &req.name) {
            Some(r) => r,
            None => return format!("No entity found matching '{}'", req.name),
        };

        let idx = starbreaker_datacore::loadout::EntityIndex::new(&db);
        let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);

        let mut out = String::new();
        format_loadout_node(&tree.root, 0, &mut out);
        out
    }

    #[tool(description = "Dump a full DataCore record as pretty-printed JSON. Accepts a GUID or a name substring (uses shortest match).")]
    fn datacore_record(&self, Parameters(req): Parameters<DatacoreRecordRequest>) -> String {
        let data = self.data();
        let db = &data.db;

        let record = match self.find_record(&db, &req.id) {
            Some(r) => r,
            None => return format!("No record found for '{}'", req.id),
        };

        match starbreaker_datacore::export::to_json(&db, record) {
            Ok(bytes) => String::from_utf8(bytes)
                .unwrap_or_else(|_| "Error: invalid UTF-8 in JSON output".to_string()),
            Err(e) => format!("Error materializing record: {e}"),
        }
    }

    #[tool(description = "Query a specific property path on a DataCore record. Returns JSON at that path using bounded no-reference materialization, so broad object/array queries fail fast instead of expanding the whole graph. Example paths: 'Components[VehicleComponentParams].vehicleDefinition', 'Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path'")]
    fn datacore_query(&self, Parameters(req): Parameters<DatacoreQueryRequest>) -> String {
        let data = self.data();
        let db = &data.db;

        let record = match self.find_record(&db, &req.id) {
            Some(r) => r,
            None => return format!("No record found for '{}'", req.id),
        };

        let compiled = match db.compile_path::<starbreaker_datacore::query::value::Value>(
            record.struct_id(),
            &req.path,
        ) {
            Ok(c) => c,
            Err(e) => return format!("Invalid path '{}': {e}", req.path),
        };

        match db.query_no_references(&compiled, record) {
            Ok(results) => {
                if results.len() > MAX_DATACORE_QUERY_RESULTS {
                    return format!(
                        "Query result too large: {} top-level values matched (limit {}). Narrow the path before retrying.",
                        results.len(),
                        MAX_DATACORE_QUERY_RESULTS
                    );
                }
                let mut budget = JsonBudget::new();
                let json_values: Result<Vec<serde_json::Value>, String> = results
                    .iter()
                    .map(|value| value_to_json_limited(value, 0, &mut budget))
                    .collect();
                let json_values = match json_values {
                    Ok(values) => values,
                    Err(err) => return err,
                };
                if json_values.len() == 1 {
                    serde_json::to_string_pretty(&json_values[0])
                        .unwrap_or_else(|e| format!("JSON error: {e}"))
                } else {
                    serde_json::to_string_pretty(&json_values)
                        .unwrap_or_else(|e| format!("JSON error: {e}"))
                }
            }
            Err(e) => format!("Query error: {e}"),
        }
    }

    #[tool(description = "Read a file from the P4k archive. CryXML files (.xml, .mtl, .chrparams, .cdf) are auto-decoded to XML. Text files returned as-is. Binary files as base64. Also supports reading files inside a .socpak archive via 'outer/path.socpak::inner/path.entxml'.")]
    fn p4k_read(&self, Parameters(req): Parameters<P4kReadRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };
        Self::decode_archive_entry_bytes(&req.path, &data)
    }

    #[tool(description = "List files and directories under a P4k path. Shows name, compressed/uncompressed size, compression method, and encryption state for each file.")]
    fn p4k_list(&self, Parameters(req): Parameters<P4kListRequest>) -> String {
        let path = if req.path.is_empty() {
            String::new()
        } else {
            Self::normalize_p4k_path(&req.path).trim_end_matches('\\').to_string()
        };

        let data = self.data();
        let entries = data.p4k.list_dir(&path);
        if entries.is_empty() {
            return format!("No entries found under '{path}'");
        }

        let mut out = String::new();
        use std::fmt::Write;
        for entry in &entries {
            match entry {
                starbreaker_p4k::DirEntry::Directory(name) => {
                    let _ = writeln!(out, "  {name}/");
                }
                starbreaker_p4k::DirEntry::File(e) => {
                    let method = match e.compression_method {
                        0 => "store",
                        8 => "deflate",
                        100 => "zstd",
                        _ => "unknown",
                    };
                    let enc = if e.is_encrypted { " [encrypted]" } else { "" };
                    let ratio = if e.uncompressed_size > 0 {
                        format!("{:.0}%", e.compressed_size as f64 / e.uncompressed_size as f64 * 100.0)
                    } else {
                        "-".to_string()
                    };
                    let name = e.name.rsplit('\\').next().unwrap_or(&e.name);
                    let _ = writeln!(
                        out,
                        "  {name}  {}/{} ({ratio}, {method}){enc}",
                        format_size(e.compressed_size),
                        format_size(e.uncompressed_size),
                    );
                }
            }
        }
        let _ = writeln!(out, "\n{} entries", entries.len());
        out
    }

    #[tool(description = "Search all DataCore records by name substring. Unlike search_entities which only searches EntityClassDefinition records, this searches ALL record types. Optionally filter by struct type.")]
    fn search_records(&self, Parameters(req): Parameters<SearchRecordsRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let limit = req.limit.unwrap_or(20) as usize;
        let search = req.query.to_lowercase();

        let type_filter = req.struct_type.as_deref().map(|s| s.to_lowercase());

        let mut results: Vec<_> = db
            .records()
            .iter()
            .filter(|r| {
                if let Some(ref tf) = type_filter {
                    let st = db.resolve_string2(db.struct_def(r.struct_index).name_offset).to_lowercase();
                    if !st.contains(tf.as_str()) {
                        return false;
                    }
                }
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        results.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        results.truncate(limit);

        let json: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let name = db.resolve_string2(r.name_offset);
                let struct_type = db.resolve_string2(db.struct_def(r.struct_index).name_offset);
                let path = db.resolve_string(r.file_name_offset);
                serde_json::json!({
                    "name": format!("{struct_type}.{name}"),
                    "id": format!("{}", r.id),
                    "struct_type": struct_type,
                    "path": path,
                })
            })
            .collect();

        serde_json::to_string_pretty(&json).unwrap_or_else(|_| "[]".to_string())
    }

    #[tool(description = "Preview an image from the P4k archive. Supports DDS (with mip selection), PNG, JPG, and other formats. Returns the image for visual inspection. For DDS files, .tif extension is auto-converted to .dds.")]
    fn image_preview(&self, Parameters(req): Parameters<ImagePreviewRequest>) -> rmcp::model::Content {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return content_text(e),
        };

        let lower = req.path.to_lowercase();
        let is_dds = lower.ends_with(".dds") || lower.ends_with(".tif");

        let png_buf = if is_dds {
            let p4k_path = Self::normalize_p4k_path(&req.path);
            let p4k_clone = self.data().p4k.clone();
            let sibling = P4kSiblingReader { p4k: p4k_clone, base_path: p4k_path };
            let dds = match starbreaker_dds::DdsFile::from_split(&data, &sibling) {
                Ok(d) => d,
                Err(e) => return content_text(format!("DDS decode error: {e}")),
            };

            if dds.mip_count() == 0 {
                return content_text("DDS has no mip data");
            }

            let mip = req.mip.unwrap_or(0).min(dds.mip_count() as u32 - 1) as usize;
            let (w, h) = dds.dimensions(mip);

            let rgba = match dds.decode_rgba(mip) {
                Ok(r) => r,
                Err(e) => return content_text(format!("DDS decode error: {e}")),
            };

            let mut png_buf = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
            if let Err(e) = image::ImageEncoder::write_image(encoder, &rgba, w, h, image::ExtendedColorType::Rgba8) {
                return content_text(format!("PNG encode error: {e}"));
            }

            log::info!("DDS: {}x{}, mip {}/{}, cubemap={}", w, h, mip, dds.mip_count(), dds.is_cubemap());
            png_buf
        } else {
            match image::load_from_memory(&data) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    let mut png_buf = Vec::new();
                    let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
                    if let Err(e) = image::ImageEncoder::write_image(encoder, &rgba, w, h, image::ExtendedColorType::Rgba8) {
                        return content_text(format!("PNG encode error: {e}"));
                    }
                    png_buf
                }
                Err(e) => return content_text(format!("Image decode error: {e}")),
            }
        };

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
        // Return as image content — Claude can see this directly
        content_image(b64, "image/png")
    }

    #[tool(description = "List all chunks in a CryEngine chunk file (IVO or CrCh format). Shows chunk type, name, version, offset, and size. For IVO files with a NodeMeshCombos chunk, also shows NMC node summary (names + parent indices).")]
    fn chunk_list(&self, Parameters(req): Parameters<ChunkListRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let chunk_file = match starbreaker_chunks::ChunkFile::from_bytes(&data) {
            Ok(cf) => cf,
            Err(e) => return format!("Chunk file parse error: {e}"),
        };

        let mut out = String::new();
        use std::fmt::Write;

        match &chunk_file {
            starbreaker_chunks::ChunkFile::Ivo(ivo) => {
                let _ = writeln!(out, "Format: IVO (#ivo), {} chunks\n", ivo.chunks().len());
                let _ = writeln!(out, "{:<4} {:<12} {:>8} {:>10} {:>10}", "Idx", "Type", "Version", "Offset", "Size");
                let _ = writeln!(out, "{}", "-".repeat(50));
                for (i, chunk) in ivo.chunks().iter().enumerate() {
                    let name = starbreaker_chunks::known_types::ivo::name(chunk.chunk_type)
                        .unwrap_or("Unknown");
                    let _ = writeln!(out, "{:<4} {:<12} {:>8} {:>#10x} {:>10}",
                        i, name, chunk.version, chunk.offset, chunk.size);
                }

                // NMC summary if present
                if let Some(nmc_chunk) = ivo.chunks().iter().find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::NODE_MESH_COMBOS) {
                    let nmc_data = ivo.chunk_data(nmc_chunk);
                    // Try parsing NMC — use the full file data since parse_nmc_full expects the whole file
                    if let Some((nodes, _mat_indices)) = starbreaker_3d::nmc::parse_nmc_full(&data) {
                        let _ = writeln!(out, "\nNMC Nodes ({}):", nodes.len());
                        for (i, node) in nodes.iter().enumerate() {
                            let parent = node.parent_index.map(|p| format!("{p}")).unwrap_or_else(|| "root".to_string());
                            let _ = writeln!(out, "  [{i}] {:<30} parent={:<5} type={}", node.name, parent, node.geometry_type);
                        }
                    } else {
                        let _ = writeln!(out, "\nNMC chunk present ({} bytes) but could not parse", nmc_data.len());
                    }
                }
            }
            starbreaker_chunks::ChunkFile::CrCh(crch) => {
                let _ = writeln!(out, "Format: CrCh, {} chunks\n", crch.chunks().len());
                let _ = writeln!(out, "{:<4} {:<18} {:>4} {:>8} {:>10} {:>10} {}", "Idx", "Type", "ID", "Version", "Offset", "Size", "Endian");
                let _ = writeln!(out, "{}", "-".repeat(70));
                for (i, chunk) in crch.chunks().iter().enumerate() {
                    let name = starbreaker_chunks::known_types::crch::name(chunk.chunk_type)
                        .unwrap_or("Unknown");
                    let endian = if chunk.big_endian { "BE" } else { "LE" };
                    let _ = writeln!(out, "{:<4} {:<18} {:>4} {:>8} {:>#10x} {:>10} {}",
                        i, name, chunk.id, chunk.version, chunk.offset, chunk.size, endian);
                }
            }
        }

        out
    }

    #[tool(description = "Read raw bytes from specific chunk(s) in a CryEngine chunk file. Returns hex dump. Use chunk_list first to find chunk indices.")]
    fn chunk_read(&self, Parameters(req): Parameters<ChunkReadRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let chunk_file = match starbreaker_chunks::ChunkFile::from_bytes(&data) {
            Ok(cf) => cf,
            Err(e) => return format!("Chunk file parse error: {e}"),
        };

        let max_bytes = req.max_bytes.unwrap_or(256) as usize;
        let mut out = String::new();
        use std::fmt::Write;

        match &chunk_file {
            starbreaker_chunks::ChunkFile::Ivo(ivo) => {
                let chunks: Vec<usize> = if let Some(idx) = req.index {
                    vec![idx as usize]
                } else {
                    (0..ivo.chunks().len()).collect()
                };
                for idx in chunks {
                    let Some(chunk) = ivo.chunks().get(idx) else {
                        let _ = writeln!(out, "Chunk index {idx} out of range (max {})", ivo.chunks().len() - 1);
                        continue;
                    };
                    let name = starbreaker_chunks::known_types::ivo::name(chunk.chunk_type).unwrap_or("Unknown");
                    let chunk_data = ivo.chunk_data(chunk);
                    let show = chunk_data.len().min(max_bytes);
                    let _ = writeln!(out, "--- Chunk [{idx}] {name} ({} bytes) ---", chunk_data.len());
                    format_hex(&chunk_data[..show], &mut out);
                    if show < chunk_data.len() {
                        let _ = writeln!(out, "  ... ({} more bytes)", chunk_data.len() - show);
                    }
                    let _ = writeln!(out);
                }
            }
            starbreaker_chunks::ChunkFile::CrCh(crch) => {
                let chunks: Vec<usize> = if let Some(idx) = req.index {
                    vec![idx as usize]
                } else {
                    (0..crch.chunks().len()).collect()
                };
                for idx in chunks {
                    let Some(chunk) = crch.chunks().get(idx) else {
                        let _ = writeln!(out, "Chunk index {idx} out of range (max {})", crch.chunks().len() - 1);
                        continue;
                    };
                    let name = starbreaker_chunks::known_types::crch::name(chunk.chunk_type).unwrap_or("Unknown");
                    let chunk_data = crch.chunk_data(chunk);
                    let show = chunk_data.len().min(max_bytes);
                    let _ = writeln!(out, "--- Chunk [{idx}] {name} id={} ({} bytes) ---", chunk.id, chunk_data.len());
                    format_hex(&chunk_data[..show], &mut out);
                    if show < chunk_data.len() {
                        let _ = writeln!(out, "  ... ({} more bytes)", chunk_data.len() - show);
                    }
                    let _ = writeln!(out);
                }
            }
        }

        out
    }

    #[tool(description = "Search P4k archive file paths by substring. Returns matching paths with file sizes. Useful for finding files when you don't know the exact directory.")]
    fn p4k_search(&self, Parameters(req): Parameters<P4kSearchRequest>) -> String {
        let limit = req.limit.unwrap_or(50) as usize;
        let query = req.query.to_lowercase();

        let data = self.data();
        let mut results: Vec<_> = data
            .p4k
            .entries()
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&query))
            .collect();

        results.sort_by_key(|e| e.name.len());
        results.truncate(limit);

        if results.is_empty() {
            return format!("No P4k files matching '{}'", req.query);
        }

        let mut out = String::new();
        use std::fmt::Write;
        for e in &results {
            let _ = writeln!(out, "{}  ({})", e.name, format_size(e.uncompressed_size));
        }
        let _ = writeln!(out, "\n{} results", results.len());
        out
    }

    #[tool(description = "Inspect a .socpak file from P4k. Returns nested entry names and per-.soc chunk summaries, including raw IncludedObjects object-type counts so authored object variants inside the container can be identified.")]
    fn socpak_inspect(&self, Parameters(req): Parameters<SocpakInspectRequest>) -> String {
        let bytes = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let archive = match P4kArchive::from_bytes(&bytes) {
            Ok(a) => a,
            Err(e) => return format!("Failed to parse socpak ZIP '{}': {e}", req.path),
        };

        let entries: Vec<serde_json::Value> = archive
            .entries()
            .iter()
            .map(|entry| serde_json::json!({ "name": entry.name }))
            .collect();

        let soc_files: Vec<serde_json::Value> = archive
            .entries()
            .iter()
            .filter(|entry| entry.name.to_ascii_lowercase().ends_with(".soc"))
            .map(|entry| {
                let name = entry.name.clone();
                let soc_bytes = match archive.read(entry) {
                    Ok(data) => data,
                    Err(e) => {
                        return serde_json::json!({
                            "name": name,
                            "error": format!("Failed to read inner .soc: {e}"),
                        });
                    }
                };

                let chunk_file = match starbreaker_chunks::ChunkFile::from_bytes(&soc_bytes) {
                    Ok(cf) => cf,
                    Err(e) => {
                        return serde_json::json!({
                            "name": name,
                            "error": format!("Chunk parse failed: {e}"),
                        });
                    }
                };

                match &chunk_file {
                    starbreaker_chunks::ChunkFile::CrCh(crch) => {
                        let chunks: Vec<serde_json::Value> = crch
                            .chunks()
                            .iter()
                            .map(|chunk| {
                                serde_json::json!({
                                    "type": format!("0x{:04x}", chunk.chunk_type),
                                    "name": starbreaker_chunks::known_types::crch::name(chunk.chunk_type).unwrap_or("Unknown"),
                                    "id": chunk.id,
                                    "version": chunk.version,
                                    "offset": chunk.offset,
                                    "size": chunk.size,
                                })
                            })
                            .collect();

                        let included_objects: Vec<serde_json::Value> = crch
                            .chunks()
                            .iter()
                            .filter(|chunk| chunk.chunk_type == starbreaker_chunks::known_types::crch::INCLUDED_OBJECTS)
                            .map(|chunk| inspect_included_objects_chunk(crch.chunk_data(chunk)))
                            .collect();

                        serde_json::json!({
                            "name": name,
                            "format": "CrCh",
                            "chunk_count": crch.chunks().len(),
                            "chunks": chunks,
                            "included_objects": included_objects,
                        })
                    }
                    starbreaker_chunks::ChunkFile::Ivo(ivo) => {
                        let chunks: Vec<serde_json::Value> = ivo
                            .chunks()
                            .iter()
                            .map(|chunk| {
                                serde_json::json!({
                                    "type": format!("0x{:08x}", chunk.chunk_type),
                                    "name": starbreaker_chunks::known_types::ivo::name(chunk.chunk_type).unwrap_or("Unknown"),
                                    "version": chunk.version,
                                    "offset": chunk.offset,
                                    "size": chunk.size,
                                })
                            })
                            .collect();

                        serde_json::json!({
                            "name": name,
                            "format": "IVO",
                            "chunk_count": ivo.chunks().len(),
                            "chunks": chunks,
                            "included_objects": [],
                        })
                    }
                }
            })
            .collect();

        serde_json::to_string_pretty(&serde_json::json!({
            "path": Self::normalize_p4k_path(&req.path),
            "entry_count": archive.entries().len(),
            "entries": entries,
            "soc_files": soc_files,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Read a file from inside a .socpak archive. CryXML sidecars (.xml, .entxml, .mtl, .cdf, etc.) are auto-decoded like p4k_read; text entries are returned as text and binary entries as base64.")]
    fn socpak_read_entry(&self, Parameters(req): Parameters<SocpakReadEntryRequest>) -> String {
        let bytes = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let archive = match P4kArchive::from_bytes(&bytes) {
            Ok(a) => a,
            Err(e) => return format!("Failed to parse socpak ZIP '{}': {e}", req.path),
        };

        let requested = req.entry.replace('/', "\\").to_ascii_lowercase();
        let Some(entry) = archive
            .entries()
            .iter()
            .find(|entry| entry.name.replace('/', "\\").to_ascii_lowercase() == requested)
        else {
            return format!(
                "Entry not found in socpak '{}': {}",
                req.path,
                req.entry
            );
        };

        let entry_bytes = match archive.read(entry) {
            Ok(data) => data,
            Err(e) => return format!("Failed to read '{}' from '{}': {e}", req.entry, req.path),
        };

        Self::decode_archive_entry_bytes(&entry.name, &entry_bytes)
    }

    #[tool(description = "Summarize a .mtl material file from P4k. Shows each sub-material's index, name, shader, key flags (DECAL, STENCIL, POM, opacity, alpha_test), and texture slots. Much more compact than reading the raw MTL XML.")]
    fn mtl_summary(&self, Parameters(req): Parameters<MtlSummaryRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let xml = match starbreaker_cryxml::from_bytes(&data) {
            Ok(x) => x,
            Err(e) => return format!("Failed to parse MTL as CryXML: {e}"),
        };

        let root = xml.root();

        // Collect sub-material nodes. If there's a <SubMaterials> container, iterate its
        // children; otherwise treat the root as a single material.
        let mat_nodes: Vec<_> = if let Some(sub_node) = xml.node_children(root)
            .find(|c| xml.node_tag(c) == "SubMaterials")
        {
            xml.node_children(sub_node)
                .filter(|c| xml.node_tag(c) == "Material")
                .collect()
        } else {
            vec![root]
        };

        let mut out = String::new();
        use std::fmt::Write;
        let _ = writeln!(out, "{} sub-materials in {}\n", mat_nodes.len(), req.path);
        let _ = writeln!(out, "{:>3}  {:<40} {:<15} {}", "Idx", "Name", "Shader", "Flags / Textures");
        let _ = writeln!(out, "{}", "-".repeat(100));

        for (i, mat_node) in mat_nodes.iter().enumerate() {
            let attrs: std::collections::HashMap<&str, &str> =
                xml.node_attributes(mat_node).collect();

            let name = attrs.get("Name").copied().unwrap_or("--");
            let shader = attrs.get("Shader").copied().unwrap_or("--");
            let mask = attrs.get("StringGenMask").copied().unwrap_or("");
            let opacity: f32 = attrs.get("Opacity").and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let alpha_test: f32 = attrs.get("AlphaTest").and_then(|v| v.parse().ok()).unwrap_or(0.0);

            // Collect flags
            let mut flags = Vec::new();
            if mask.contains("%DECAL") { flags.push("DECAL".to_string()); }
            if mask.contains("STENCIL_MAP") { flags.push("STENCIL".to_string()); }
            if mask.contains("%PARALLAX_OCCLUSION_MAPPING") { flags.push("POM".to_string()); }
            if mask.contains("%VERTCOLORS") { flags.push("VCOL".to_string()); }
            if mask.contains("%WEAR_LAYER") { flags.push("WEAR".to_string()); }
            if mask.contains("%BLENDLAYER") { flags.push("BLEND".to_string()); }
            if opacity < 1.0 { flags.push(format!("opacity={opacity}")); }
            if alpha_test > 0.0 { flags.push(format!("alpha_test={alpha_test}")); }

            // Collect texture slots
            let mut tex_slots = Vec::new();
            if let Some(tex_node) = xml.node_children(mat_node)
                .find(|c| xml.node_tag(c) == "Textures")
            {
                for tex in xml.node_children(tex_node) {
                    if xml.node_tag(tex) != "Texture" { continue; }
                    let tex_attrs: std::collections::HashMap<&str, &str> =
                        xml.node_attributes(tex).collect();
                    let slot = tex_attrs.get("Map").copied().unwrap_or("?");
                    let file = tex_attrs.get("File").copied().unwrap_or("?");
                    // Show just the filename, not full path
                    let short = file.rsplit(['/', '\\']).next().unwrap_or(file);
                    tex_slots.push(format!("{slot}={short}"));
                }
            }

            // Count layers
            let layer_count = xml.node_children(mat_node)
                .find(|c| xml.node_tag(c) == "MatLayers")
                .map(|l| xml.node_children(l).filter(|c| xml.node_tag(c) == "Layer").count())
                .unwrap_or(0);
            if layer_count > 0 {
                flags.push(format!("{layer_count} layers"));
            }

            // Check palette tint on first layer
            if let Some(layers_node) = xml.node_children(mat_node)
                .find(|c| xml.node_tag(c) == "MatLayers")
            {
                if let Some(first_layer) = xml.node_children(layers_node)
                    .find(|c| xml.node_tag(c) == "Layer")
                {
                    let layer_attrs: std::collections::HashMap<&str, &str> =
                        xml.node_attributes(first_layer).collect();
                    if let Some(pt) = layer_attrs.get("PaletteTint") {
                        let pt_val: u8 = pt.parse().unwrap_or(0);
                        if pt_val > 0 {
                            let ch = match pt_val { 1 => "A", 2 => "B", 3 => "C", _ => "?" };
                            flags.push(format!("palette={ch}"));
                        }
                    }
                }
            }

            let flag_str = if flags.is_empty() { "-".to_string() } else { flags.join(", ") };

            let _ = writeln!(out, "{i:3}  {name:<40} {shader:<15} {flag_str}");
            for tex in &tex_slots {
                let _ = writeln!(out, "       {tex}");
            }
        }

        out
    }

    #[tool(description = "Inspect a CryEngine animation database (.dba) or compressed animation file (.caf). Returns structured JSON: clip metadata (name, fps, frame_count, channel_count) and per-channel bone hashes plus first/last keyframe samples (or full keyframe arrays with all_keyframes=true). Provide `skeleton` to resolve bone hashes to names — accepts either a CHR skeleton (CompiledBones) or a CGA/CGAM scene graph (NMC nodes) for ships whose main body has no CHR; combine with `bone_filter` to drill down to a small set of channels (e.g. wings, landing gear). Use `filter` for clip-name filtering. Replaces the legacy `starbreaker dba dump` CLI.")]
    fn dba_dump(&self, Parameters(req): Parameters<DbaDumpRequest>) -> String {
        let bytes = match self.read_p4k_or_disk(&req.path) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let db = if req.path.to_ascii_lowercase().ends_with(".caf") {
            match starbreaker_3d::animation::parse_caf(&bytes) {
                Ok(db) => db,
                Err(e) => return format!("parse_caf failed for {}: {e}", req.path),
            }
        } else {
            match starbreaker_3d::animation::parse_dba(&bytes) {
                Ok(db) => db,
                Err(e) => return format!("parse_dba failed for {}: {e}", req.path),
            }
        };
        let mut hash_to_name: std::collections::HashMap<u32, String> =
            std::collections::HashMap::new();
        if let Some(skel_path) = req.skeleton.as_ref() {
            match self.read_p4k_or_disk(skel_path) {
                Ok(sb) => {
                    if let Some(names) = starbreaker_3d::skeleton::parse_rig_node_names(&sb) {
                        for name in &names {
                            hash_to_name.insert(
                                starbreaker_3d::animation::bone_name_hash(name),
                                name.clone(),
                            );
                        }
                    } else {
                        return format!(
                            "Skeleton '{skel_path}' has no CompiledBones (CHR) or NMC (CGA/CGAM) chunk; cannot resolve bone names"
                        );
                    }
                }
                Err(e) => return format!("Failed to read skeleton '{skel_path}': {e}"),
            }
        }
        let value = starbreaker_3d::animation::dump_database_to_json(
            &db,
            &hash_to_name,
            req.filter.as_deref(),
            req.bone_filter.as_deref(),
            req.all_keyframes.unwrap_or(false),
        );
        match serde_json::to_string_pretty(&value) {
            Ok(s) => s,
            Err(e) => format!("serialize failed: {e}"),
        }
    }

    #[tool(description = "Dump the Mannequin Animation Database (ADB) plus its companion ControllerDef for an entity. Returns structured JSON listing every Mannequin Fragment with group name, GUID, tags, FragTags, BlendOutDuration, OptionWeight, animations, scopes (resolved from the ControllerDef), and procedurals. Use to inspect fragment-scope metadata for animation troubleshooting (Phase 37). Note: the ADB has no per-bone blend-mode flag; use dba_dump's `rot_format_flags`/`pos_format_flags` for per-bone CAF metadata.")]
    fn mannequin_dump(&self, Parameters(req): Parameters<MannequinDumpRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let record = match self.find_entity(&db, &req.entity) {
            Some(r) => r,
            None => return format!("No entity matching '{}'", req.entity),
        };
        let source = match starbreaker_3d::query_animation_controller_source(&db, record) {
            Some(s) => s,
            None => return format!("Entity '{}' has no SAnimationControllerParams", req.entity),
        };
        let value = match starbreaker_3d::animation::dump_mannequin_adb_to_json(
            data.p4k.as_ref(),
            &source,
            req.filter.as_deref(),
        ) {
            Ok(v) => v,
            Err(e) => return format!("dump_mannequin_adb_to_json failed: {e}"),
        };
        match serde_json::to_string_pretty(&value) {
            Ok(s) => s,
            Err(e) => format!("serialize failed: {e}"),
        }
    }

    #[tool(description = "Parse the DNA1/SDNA block from a .blend file and show struct field layouts with absolute byte offsets. Accepts an absolute filesystem path to a .blend file (zstd-compressed Blender 5.x or uncompressed). Optionally filter to a single struct name; otherwise returns a list of all struct names and sizes.")]
    fn blend_sdna(&self, Parameters(req): Parameters<BlendSdnaRequest>) -> String {
        use crate::blend_debug::{decompress_blend, parse_blend_blocks, parse_sdna, format_struct_layout};

        let raw = match std::fs::read(&req.path) {
            Ok(b) => b,
            Err(e) => return format!("failed to read {}: {e}", req.path),
        };
        let data = match decompress_blend(&raw) {
            Ok(d) => d,
            Err(e) => return format!("decompress failed: {e}"),
        };
        let blocks = match parse_blend_blocks(&data) {
            Ok(r) => r,
            Err(e) => return format!("block parse failed: {e}"),
        };
        let dna1 = match blocks.iter().find(|b| &b.code == b"DNA1") {
            Some(b) => b,
            None => return "No DNA1 block found in file".to_string(),
        };
        let dna_data = &data[dna1.data_offset..dna1.data_offset + dna1.data_len];
        let sdna = match parse_sdna(dna_data, 8) {
            Ok(s) => s,
            Err(e) => return format!("SDNA parse failed: {e}"),
        };
        let max_depth = req.max_depth.unwrap_or(1) as usize;

        if let Some(name) = &req.struct_name {
            match sdna.find_struct(name) {
                Some((idx, s)) => {
                    let mut out = format!(
                        "Struct #{idx}: {} ({} bytes, {} fields)\n",
                        s.name, s.size, s.fields.len()
                    );
                    format_struct_layout(&sdna, s, 0, 0, max_depth, &mut out);
                    out
                }
                None => {
                    let available: Vec<&str> = sdna.structs.iter()
                        .filter(|s| s.name.to_lowercase().contains(&name.to_lowercase()))
                        .map(|s| s.name.as_str())
                        .collect();
                    if available.is_empty() {
                        format!("Struct '{name}' not found. There are {} structs total.", sdna.structs.len())
                    } else {
                        format!("Struct '{name}' not found exactly. Similar names: {}", available.join(", "))
                    }
                }
            }
        } else {
            let mut out = format!(
                "{} structs, ptr_size=8\n\n",
                sdna.structs.len()
            );
            for (i, s) in sdna.structs.iter().enumerate() {
                out.push_str(&format!("  [{i:4}] {} ({} bytes, {} fields)\n", s.name, s.size, s.fields.len()));
            }
            out
        }
    }

    #[tool(description = "Hex dump specific blocks from a .blend file filtered by SDNA struct type. Accepts an absolute filesystem path. If sdna_type is omitted, returns a summary table of all block types with counts and sizes.")]
    fn blend_block_inspect(&self, Parameters(req): Parameters<BlendBlockInspectRequest>) -> String {
        use crate::blend_debug::{decompress_blend, parse_blend_blocks, parse_sdna, hex_dump};

        let raw = match std::fs::read(&req.path) {
            Ok(b) => b,
            Err(e) => return format!("failed to read {}: {e}", req.path),
        };
        let data = match decompress_blend(&raw) {
            Ok(d) => d,
            Err(e) => return format!("decompress failed: {e}"),
        };
        let blocks = match parse_blend_blocks(&data) {
            Ok(r) => r,
            Err(e) => return format!("block parse failed: {e}"),
        };
        let dna1 = blocks.iter().find(|b| &b.code == b"DNA1");
        let sdna = dna1.and_then(|b| {
            let dna_data = &data[b.data_offset..b.data_offset + b.data_len];
            parse_sdna(dna_data, 8).ok()
        });

        let max_bytes = req.max_bytes.unwrap_or(256) as usize;
        let limit = req.limit.unwrap_or(10) as usize;

        if let Some(filter) = &req.sdna_type {
            // Find the SDNA struct index for this type
            let target_sdna_idx = sdna.as_ref().and_then(|s| s.find_struct(filter).map(|(i, _)| i));

            let matching: Vec<_> = blocks.iter()
                .filter(|b| {
                    if let Some(idx) = target_sdna_idx {
                        b.sdna_index as usize == idx
                    } else {
                        // Fallback: no SDNA, can't filter
                        false
                    }
                })
                .collect();

            if matching.is_empty() {
                let all_types: std::collections::BTreeMap<String, usize> = {
                    let mut map = std::collections::BTreeMap::new();
                    for b in &blocks {
                        if let Some(ref s) = sdna {
                            if (b.sdna_index as usize) < s.structs.len() {
                                *map.entry(s.structs[b.sdna_index as usize].name.clone()).or_default() += 1;
                            }
                        }
                    }
                    map
                };
                return format!(
                    "No blocks with SDNA type '{}' found.\nAvailable types: {}\n",
                    filter,
                    all_types.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            }

            let mut out = format!(
                "{} block(s) with SDNA type '{}' (showing up to {limit}):\n\n",
                matching.len(), filter
            );
            for b in matching.iter().take(limit) {
                out.push_str(&format!(
                    "Block code={} old_ptr={:#018x} sdna={} count={} data_len={}\n",
                    b.code_str(), b.old_ptr, b.sdna_index, b.count, b.data_len
                ));
                if max_bytes > 0 && b.data_len > 0 {
                    let end = (b.data_offset + max_bytes.min(b.data_len)).min(data.len());
                    out.push_str(&hex_dump(&data[b.data_offset..end], b.data_offset));
                }
                out.push('\n');
            }
            out
        } else {
            // Summary of all block types
            let mut type_counts: std::collections::BTreeMap<String, (usize, usize)> =
                std::collections::BTreeMap::new();
            for b in &blocks {
                let type_name = if let Some(ref s) = sdna {
                    if (b.sdna_index as usize) < s.structs.len() {
                        s.structs[b.sdna_index as usize].name.clone()
                    } else {
                        b.code_str()
                    }
                } else {
                    b.code_str()
                };
                let e = type_counts.entry(type_name).or_default();
                e.0 += 1;
                e.1 += b.data_len;
            }
            let mut out = format!(
                "{} total blocks in {}:\n\n",
                blocks.len(), req.path
            );
            out.push_str(&format!("{:<40} {:>8} {:>14}\n", "Type", "Count", "Total bytes"));
            out.push_str(&format!("{}\n", "-".repeat(65)));
            for (name, (count, total_bytes)) in &type_counts {
                out.push_str(&format!("{:<40} {:>8} {:>14}\n", name, count, total_bytes));
            }
            out
        }
    }

    #[tool(description = "Run headless Blender with a before/after Python script pair, save .blend files, decompress them, and show a binary diff of changed bytes with offsets and SDNA struct names. Useful for reverse-engineering Blender binary format changes.")]
    fn blend_python_diff(&self, Parameters(req): Parameters<BlendPythonDiffRequest>) -> String {
        use crate::blend_debug::{decompress_blend, parse_blend_blocks, parse_sdna, hex_dump, find_blender_bin};
        use std::process::Command;

        let blender = match find_blender_bin(req.blender_bin.as_deref()) {
            Ok(b) => b,
            Err(e) => return format!("Blender not found: {e}"),
        };

        let tmp = std::env::temp_dir().join(format!(
            "blend_python_diff_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        if let Err(e) = std::fs::create_dir_all(&tmp) {
            return format!("failed to create temp dir: {e}");
        }

        let before_blend = tmp.join("before.blend");
        let after_blend = tmp.join("after.blend");
        let before_py = tmp.join("before.py");
        let after_py = tmp.join("after.py");

        let before_data: Vec<u8>;
        let after_data: Vec<u8>;

        // Run "before" script
        if !req.before_script.is_empty() {
            let script = format!(
                "import bpy\nbpy.ops.wm.open_mainfile(filepath=r'{}')\n{}\nbpy.ops.wm.save_as_mainfile(filepath=r'{}')\n",
                req.blend_path,
                req.before_script,
                before_blend.display()
            );
            if let Err(e) = std::fs::write(&before_py, &script) {
                return format!("failed to write before script: {e}");
            }
            let status = Command::new(&blender)
                .args(["--background", "--python"])
                .arg(&before_py)
                .output();
            match status {
                Ok(out) if !out.status.success() => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!(
                        "Before Blender run failed:\nstdout: {}\nstderr: {}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("Failed to run Blender: {e}");
                }
                _ => {}
            }
            before_data = match std::fs::read(&before_blend) {
                Ok(b) => b,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("before.blend not written: {e}");
                }
            };
        } else {
            before_data = match std::fs::read(&req.blend_path) {
                Ok(b) => b,
                Err(e) => return format!("failed to read input blend: {e}"),
            };
        }

        // Run "after" script
        {
            let input_path = if !req.before_script.is_empty() {
                before_blend.to_string_lossy().to_string()
            } else {
                req.blend_path.clone()
            };
            let script = format!(
                "import bpy\nbpy.ops.wm.open_mainfile(filepath=r'{input_path}')\n{}\nbpy.ops.wm.save_as_mainfile(filepath=r'{}')\n",
                req.after_script,
                after_blend.display()
            );
            if let Err(e) = std::fs::write(&after_py, &script) {
                return format!("failed to write after script: {e}");
            }
            let status = Command::new(&blender)
                .args(["--background", "--python"])
                .arg(&after_py)
                .output();
            match status {
                Ok(out) if !out.status.success() => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!(
                        "After Blender run failed:\nstdout: {}\nstderr: {}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("Failed to run Blender: {e}");
                }
                _ => {}
            }
            after_data = match std::fs::read(&after_blend) {
                Ok(b) => b,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("after.blend not written: {e}");
                }
            };
        }

        // Decompress both
        let before_decomp = match decompress_blend(&before_data) {
            Ok(d) => d,
            Err(e) => { let _ = std::fs::remove_dir_all(&tmp); return format!("before decompress: {e}"); }
        };
        let after_decomp = match decompress_blend(&after_data) {
            Ok(d) => d,
            Err(e) => { let _ = std::fs::remove_dir_all(&tmp); return format!("after decompress: {e}"); }
        };

        // Parse both block layouts + SDNA for name resolution
        let after_blocks = parse_blend_blocks(&after_decomp).unwrap_or_default();
        let sdna = after_blocks.iter().find(|b| &b.code == b"DNA1").and_then(|b| {
            let dna_data = &after_decomp[b.data_offset..b.data_offset + b.data_len];
            parse_sdna(dna_data, 8).ok()
        });

        // Diff changed bytes across blocks
        let before_blocks = parse_blend_blocks(&before_decomp).unwrap_or_default();

        let mut out = String::new();
        out.push_str(&format!(
            "blend_python_diff: before={} bytes, after={} bytes\n\n",
            before_decomp.len(), after_decomp.len()
        ));

        let mut diff_count = 0;
        for (idx, b_block) in after_blocks.iter().enumerate() {
            if &b_block.code == b"DNA1" || &b_block.code == b"ENDB" { continue; }

            // Resolve SDNA name
            let type_name = sdna.as_ref().and_then(|s| {
                s.structs.get(b_block.sdna_index as usize).map(|st| st.name.as_str())
            }).unwrap_or("?");

            // Apply filter
            if let Some(ref filter) = req.sdna_filter {
                if !type_name.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }
            }

            // Find matching block in before by old_ptr or position
            let matching_before = before_blocks.iter().find(|bb| bb.old_ptr == b_block.old_ptr)
                .or_else(|| before_blocks.get(idx));

            let b_data = if b_block.data_offset + b_block.data_len <= after_decomp.len() {
                &after_decomp[b_block.data_offset..b_block.data_offset + b_block.data_len]
            } else {
                continue;
            };

            if let Some(a_block) = matching_before {
                if a_block.data_offset + a_block.data_len > before_decomp.len() { continue; }
                let a_data = &before_decomp[a_block.data_offset..a_block.data_offset + a_block.data_len];
                if a_data == b_data { continue; }

                diff_count += 1;
                out.push_str(&format!(
                    "CHANGED block #{idx}: code={} type={} ptr={:#018x} sdna={} count={}\n",
                    b_block.code_str(), type_name, b_block.old_ptr, b_block.sdna_index, b_block.count
                ));
                // Show changed byte ranges
                let min_len = a_data.len().min(b_data.len());
                let mut range_start = None;
                let mut changed_ranges: Vec<(usize, usize)> = Vec::new();
                for byte_idx in 0..min_len {
                    if a_data[byte_idx] != b_data[byte_idx] {
                        match range_start {
                            None => range_start = Some(byte_idx),
                            Some(_) => {}
                        }
                    } else if let Some(start) = range_start.take() {
                        changed_ranges.push((start, byte_idx));
                    }
                }
                if let Some(start) = range_start.take() {
                    changed_ranges.push((start, min_len));
                }

                for (start, end) in &changed_ranges {
                    let ctx_start = start.saturating_sub(8);
                    let ctx_end = (end + 8).min(min_len);
                    out.push_str(&format!("  Bytes [{start}..{end}] changed (showing context [{ctx_start}..{ctx_end}]):\n"));
                    out.push_str("  BEFORE:\n");
                    out.push_str(&hex_dump(&a_data[ctx_start..ctx_end], ctx_start));
                    out.push_str("  AFTER:\n");
                    out.push_str(&hex_dump(&b_data[ctx_start..ctx_end], ctx_start));
                }
            } else {
                diff_count += 1;
                out.push_str(&format!(
                    "NEW block #{idx}: code={} type={} ptr={:#018x}\n",
                    b_block.code_str(), type_name, b_block.old_ptr
                ));
                out.push_str(&hex_dump(&b_data[..b_data.len().min(256)], 0));
            }
        }

        if diff_count == 0 {
            out.push_str("No changed blocks detected.\n");
        } else {
            out.push_str(&format!("\nTotal changed/new blocks: {diff_count}\n"));
        }

        let _ = std::fs::remove_dir_all(&tmp);
        out
    }

    /// Run an arbitrary Python script against a `.blend` file in headless Blender.
    ///
    /// Opens `blend_path` with `blender --background --python <script>` and returns the
    /// script's printed output.  If the script wraps output with `__BLEND_SCRIPT_OUTPUT_START__`
    /// and `__BLEND_SCRIPT_OUTPUT_END__` sentinels, only the content between them is returned
    /// (filtering Blender startup noise). If sentinels are absent, full stdout+stderr is returned.
    /// `import bpy` is always available in Blender scripts — no need to import it explicitly.
    #[tool(description = "Run an arbitrary Python script against a .blend file in headless Blender. \
        Opens blend_path with `blender --background --python script` and returns printed output. \
        Wrap important output with print('__BLEND_SCRIPT_OUTPUT_START__') / print('__BLEND_SCRIPT_OUTPUT_END__') \
        sentinels to filter out Blender startup noise. If sentinels are absent, full stdout+stderr is returned. \
        `import bpy` is always available — no need to import it explicitly in your script.")]
    fn blend_run_script(&self, Parameters(req): Parameters<BlendRunScriptRequest>) -> String {
        use crate::blend_debug::find_blender_bin;
        use std::io::Write as _;

        let blender_bin = match find_blender_bin(req.blender_bin.as_deref()) {
            Ok(b) => b,
            Err(e) => return format!("blender binary not found: {e}"),
        };

        let blend_path = std::path::Path::new(&req.blend_path);
        if !blend_path.exists() {
            return format!("blend file not found: {}", req.blend_path);
        }

        // Temp dir for the script file
        let tmp = std::path::PathBuf::from(format!(
            "/tmp/blend_run_script_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        if let Err(e) = std::fs::create_dir_all(&tmp) {
            return format!("failed to create temp dir: {e}");
        }

        let script_path = tmp.join("script.py");
        if let Err(e) = std::fs::File::create(&script_path)
            .and_then(|mut f| f.write_all(req.script.as_bytes()))
        {
            let _ = std::fs::remove_dir_all(&tmp);
            return format!("failed to write script: {e}");
        }

        let output = std::process::Command::new(&blender_bin)
            .args([
                "--background",
                &req.blend_path,
                "--python",
                script_path.to_str().unwrap_or(""),
            ])
            .output();

        let _ = std::fs::remove_dir_all(&tmp);

        let output = match output {
            Ok(o) => o,
            Err(e) => return format!("failed to run blender: {e}"),
        };

        let full = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        // Extract between sentinels if present
        const START: &str = "__BLEND_SCRIPT_OUTPUT_START__";
        const END: &str = "__BLEND_SCRIPT_OUTPUT_END__";
        if let (Some(start_pos), Some(end_pos)) = (full.find(START), full.rfind(END)) {
            let content_start = start_pos + START.len();
            if content_start < end_pos {
                return full[content_start..end_pos].trim().to_string();
            }
        }

        // Fallback: return everything (script likely errored)
        if !output.status.success() {
            format!("blender exited with status {}\n\n{full}", output.status)
        } else {
            full
        }
    }

}
#[rmcp::tool_handler]
impl ServerHandler for StarBreakerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Star Citizen game data server. Query DataCore records, entity loadouts, and P4k archive files.",
        )
    }
}

fn format_loadout_node(
    node: &starbreaker_datacore::loadout::LoadoutNode,
    depth: usize,
    out: &mut String,
) {
    use std::fmt::Write;
    let indent = "  ".repeat(depth);
    let geom = node.geometry_path.as_deref().unwrap_or("-");
    let _ = writeln!(
        out,
        "{indent}{} [{}] geom={geom}",
        node.entity_name, node.item_port_name
    );
    for child in &node.children {
        format_loadout_node(child, depth + 1, out);
    }
}

struct JsonBudget {
    remaining_nodes: usize,
}

impl JsonBudget {
    fn new() -> Self {
        Self {
            remaining_nodes: MAX_DATACORE_QUERY_JSON_NODES,
        }
    }

    fn consume(&mut self, detail: impl Into<String>) -> Result<(), String> {
        if self.remaining_nodes == 0 {
            return Err(format!(
                "Query result too large: exceeded JSON node limit {} while serializing {}. Narrow the path before retrying.",
                MAX_DATACORE_QUERY_JSON_NODES,
                detail.into()
            ));
        }
        self.remaining_nodes -= 1;
        Ok(())
    }
}

/// Convert a DataCore `Value` to a bounded `serde_json::Value`.
fn value_to_json_limited(
    v: &starbreaker_datacore::query::value::Value,
    depth: usize,
    budget: &mut JsonBudget,
) -> Result<serde_json::Value, String> {
    use starbreaker_datacore::query::value::Value;
    if depth > MAX_DATACORE_QUERY_JSON_DEPTH {
        return Err(format!(
            "Query result too deep: exceeded nesting limit {}. Narrow the path before retrying.",
            MAX_DATACORE_QUERY_JSON_DEPTH
        ));
    }
    budget.consume(format!("depth {}", depth))?;
    match v {
        Value::Null => Ok(serde_json::Value::Null),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Int8(n) => Ok(serde_json::json!(*n)),
        Value::Int16(n) => Ok(serde_json::json!(*n)),
        Value::Int32(n) => Ok(serde_json::json!(*n)),
        Value::Int64(n) => Ok(serde_json::json!(*n)),
        Value::UInt8(n) => Ok(serde_json::json!(*n)),
        Value::UInt16(n) => Ok(serde_json::json!(*n)),
        Value::UInt32(n) => Ok(serde_json::json!(*n)),
        Value::UInt64(n) => Ok(serde_json::json!(*n)),
        Value::Float(n) => Ok(serde_json::json!(*n)),
        Value::Double(n) => Ok(serde_json::json!(*n)),
        Value::String(s) => Ok(serde_json::Value::String(truncate_string(s))),
        Value::Guid(g) => Ok(serde_json::Value::String(format!("{g}"))),
        Value::Enum(s) => Ok(serde_json::Value::String(truncate_string(s))),
        Value::Locale(s) => Ok(serde_json::Value::String(truncate_string(s))),
        Value::Array(items) => {
            if items.len() > MAX_DATACORE_QUERY_ARRAY_ITEMS {
                return Err(format!(
                    "Query result too large: array has {} items (limit {}). Narrow the path before retrying.",
                    items.len(),
                    MAX_DATACORE_QUERY_ARRAY_ITEMS
                ));
            }
            let values = items
                .iter()
                .map(|item| value_to_json_limited(item, depth + 1, budget))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::Value::Array(values))
        }
        Value::Object {
            type_name, fields, record_id,
        } => {
            if fields.len() > MAX_DATACORE_QUERY_OBJECT_FIELDS {
                return Err(format!(
                    "Query result too large: object '{}' has {} fields (limit {}). Narrow the path before retrying.",
                    type_name,
                    fields.len(),
                    MAX_DATACORE_QUERY_OBJECT_FIELDS
                ));
            }
            let mut map = serde_json::Map::new();
            map.insert("__type".to_string(), serde_json::Value::String(type_name.to_string()));
            if let Some(rid) = record_id {
                map.insert("__id".to_string(), serde_json::Value::String(format!("{rid}")));
            }
            for (key, val) in fields {
                map.insert(
                    key.to_string(),
                    value_to_json_limited(val, depth + 1, budget)?,
                );
            }
            Ok(serde_json::Value::Object(map))
        }
    }
}

fn truncate_string(value: &str) -> String {
    if value.chars().count() <= MAX_DATACORE_QUERY_STRING_CHARS {
        return value.to_string();
    }
    value
        .chars()
        .take(MAX_DATACORE_QUERY_STRING_CHARS)
        .collect::<String>()
        + "…"
}

/// Create a Content::Text item.
fn content_text(text: impl Into<String>) -> rmcp::model::Content {
    rmcp::model::Content::new(rmcp::model::RawContent::text(text), None)
}

/// Create a Content::Image item.
fn content_image(data: impl Into<String>, mime_type: impl Into<String>) -> rmcp::model::Content {
    rmcp::model::Content::new(rmcp::model::RawContent::image(data, mime_type), None)
}

/// P4k-backed sibling reader for split DDS mip files.
struct P4kSiblingReader {
    p4k: Arc<MappedP4k>,
    base_path: String,
}

impl starbreaker_dds::ReadSibling for P4kSiblingReader {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let path = format!("{}{suffix}", self.base_path);
        self.p4k.read_file(&path).ok()
    }
}

/// Format bytes as a hex dump with ASCII sidebar.
fn format_hex(data: &[u8], out: &mut String) {
    use std::fmt::Write;
    for (i, chunk) in data.chunks(16).enumerate() {
        let _ = write!(out, "  {:04x}: ", i * 16);
        for (j, byte) in chunk.iter().enumerate() {
            let _ = write!(out, "{:02x} ", byte);
            if j == 7 { let _ = write!(out, " "); }
        }
        // Pad if short line
        for _ in chunk.len()..16 {
            let _ = write!(out, "   ");
        }
        if chunk.len() <= 8 { let _ = write!(out, " "); }
        let _ = write!(out, " |");
        for byte in chunk {
            let c = if byte.is_ascii_graphic() || *byte == b' ' { *byte as char } else { '.' };
            let _ = write!(out, "{c}");
        }
        let _ = writeln!(out, "|");
    }
}

/// Format a byte size as human-readable.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 { return format!("{bytes} B"); }
    if bytes < 1024 * 1024 { return format!("{:.1} KB", bytes as f64 / 1024.0); }
    if bytes < 1024 * 1024 * 1024 { return format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)); }
    format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

fn inspect_included_objects_chunk(data: &[u8]) -> serde_json::Value {
    match starbreaker_3d::IncludedObjects::from_bytes(data) {
        Ok(io) => {
            let raw_counts = match raw_included_object_type_counts(data) {
                Ok(counts) => counts
                    .into_iter()
                    .map(|(obj_type, count)| {
                        serde_json::json!({
                            "type": format!("0x{obj_type:08x}"),
                            "count": count,
                        })
                    })
                    .collect::<Vec<_>>(),
                Err(e) => {
                    return serde_json::json!({
                        "error": format!("Raw IncludedObjects scan failed: {e}"),
                    });
                }
            };

            serde_json::json!({
                "cgf_count": io.cgf_paths.len(),
                "material_count": io.material_paths.len(),
                "palette_count": io.tint_palette_paths.len(),
                "parsed_type1_object_count": io.objects.len(),
                "raw_object_type_counts": raw_counts,
                "cgf_sample": io.cgf_paths.iter().take(20).cloned().collect::<Vec<_>>(),
                "palette_paths": io.tint_palette_paths,
            })
        }
        Err(e) => serde_json::json!({
            "error": format!("IncludedObjects parse failed: {e}"),
        }),
    }
}

fn raw_included_object_type_counts(
    data: &[u8],
) -> Result<std::collections::BTreeMap<u32, usize>, String> {
    let mut off = 4usize;
    skip_included_objects_header(data, &mut off)?;

    let len_objects_bytes = read_u32_le(data, &mut off)? as usize;
    let objects_end = off
        .checked_add(len_objects_bytes)
        .ok_or_else(|| "IncludedObjects object section overflow".to_string())?;
    if objects_end > data.len() {
        return Err(format!(
            "IncludedObjects object section truncated: end={objects_end}, len={}",
            data.len()
        ));
    }

    let mut counts = std::collections::BTreeMap::new();
    while off + 4 <= objects_end {
        let obj_type = read_u32_at_le(data, off)?;
        *counts.entry(obj_type).or_insert(0) += 1;

        match obj_type {
            0x0000_0001 => {
                let base_size = 168usize;
                if off + base_size > objects_end {
                    return Err(format!("Type1 object truncated at offset {off}"));
                }
                let unknown3 = read_u64_at_le(data, off + 160)?;
                let actual_size = if unknown3 == 0 { 184usize } else { 168usize };
                let mut end = off + actual_size;
                if end > objects_end {
                    return Err(format!(
                        "Type1 object overruns object section at offset {off} (size {actual_size})"
                    ));
                }
                while end + 4 <= objects_end {
                    let next_type = read_u32_at_le(data, end)?;
                    if next_type == 0 {
                        end += 4;
                    } else {
                        break;
                    }
                }
                off = end;
            }
            0x0000_0007 => off += 152,
            0x0000_0010 => off += 136,
            _ => off += 4,
        }

        if off > objects_end {
            return Err(format!(
                "Object scan advanced past end of section: off={off}, end={objects_end}"
            ));
        }
    }

    Ok(counts)
}

fn skip_included_objects_header(data: &[u8], off: &mut usize) -> Result<(), String> {
    let num_cgfs = read_u32_le(data, off)? as usize;
    advance(data, off, num_cgfs * 256, "CGF paths")?;

    let num_materials = read_u16_le(data, off)? as usize;
    let num_palettes = read_u16_le(data, off)? as usize;
    advance(data, off, num_materials * 256, "material paths")?;
    advance(data, off, num_palettes * 256, "palette paths")?;
    advance(data, off, 28, "unknown header bytes")?;
    Ok(())
}

fn advance(data: &[u8], off: &mut usize, len: usize, label: &str) -> Result<(), String> {
    let end = off
        .checked_add(len)
        .ok_or_else(|| format!("Offset overflow while skipping {label}"))?;
    if end > data.len() {
        return Err(format!(
            "IncludedObjects truncated while skipping {label}: need end={end}, len={}",
            data.len()
        ));
    }
    *off = end;
    Ok(())
}

fn read_u16_le(data: &[u8], off: &mut usize) -> Result<u16, String> {
    let value = read_u16_at_le(data, *off)?;
    *off += 2;
    Ok(value)
}

fn read_u32_le(data: &[u8], off: &mut usize) -> Result<u32, String> {
    let value = read_u32_at_le(data, *off)?;
    *off += 4;
    Ok(value)
}

fn read_u16_at_le(data: &[u8], idx: usize) -> Result<u16, String> {
    let bytes: [u8; 2] = data
        .get(idx..idx + 2)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Truncated u16 at offset {idx}"))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_at_le(data: &[u8], idx: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = data
        .get(idx..idx + 4)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Truncated u32 at offset {idx}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64_at_le(data: &[u8], idx: usize) -> Result<u64, String> {
    let bytes: [u8; 8] = data
        .get(idx..idx + 8)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Truncated u64 at offset {idx}"))?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::{
        inspect_included_objects_chunk, raw_included_object_type_counts, value_to_json_limited,
        JsonBudget, StarBreakerMcp,
    };
    use starbreaker_datacore::query::value::Value;

    fn build_included_objects_chunk(object_payloads: &[Vec<u8>]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0u8; 4]); // padding
        buf.extend_from_slice(&1u32.to_le_bytes()); // one cgf

        let mut path = b"test.cgf".to_vec();
        path.resize(256, 0);
        buf.extend_from_slice(&path);

        buf.extend_from_slice(&0u16.to_le_bytes()); // materials
        buf.extend_from_slice(&0u16.to_le_bytes()); // palettes
        buf.extend_from_slice(&[0u8; 28]); // unknown bytes

        let objects: Vec<u8> = object_payloads.iter().flat_map(|payload| payload.clone()).collect();
        buf.extend_from_slice(&(objects.len() as u32).to_le_bytes());
        buf.extend_from_slice(&objects);
        buf
    }

    fn type1_object(unknown3: u64) -> Vec<u8> {
        let mut obj = Vec::new();
        obj.extend_from_slice(&1u32.to_le_bytes());
        obj.extend_from_slice(&[0u8; 48]); // vector1 + vector2
        obj.extend_from_slice(&[0u8; 8]); // unknown1
        obj.extend_from_slice(&0u16.to_le_bytes()); // cgf id
        obj.extend_from_slice(&0u16.to_le_bytes()); // unknown2
        for &val in &[
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0f64,
        ] {
            obj.extend_from_slice(&val.to_le_bytes());
        }
        obj.extend_from_slice(&unknown3.to_le_bytes());
        if unknown3 == 0 {
            obj.extend_from_slice(&[0u8; 16]);
        }
        obj
    }

    fn fixed_type_object(obj_type: u32, total_size: usize) -> Vec<u8> {
        let mut obj = Vec::with_capacity(total_size);
        obj.extend_from_slice(&obj_type.to_le_bytes());
        obj.resize(total_size, 0);
        obj
    }

    #[test]
    fn raw_included_object_type_counts_reports_mixed_types() {
        let data = build_included_objects_chunk(&[
            type1_object(42),
            fixed_type_object(0x0000_0007, 152),
            fixed_type_object(0x0000_0010, 136),
        ]);

        let counts = raw_included_object_type_counts(&data).expect("raw scan should succeed");
        assert_eq!(counts.get(&0x0000_0001), Some(&1));
        assert_eq!(counts.get(&0x0000_0007), Some(&1));
        assert_eq!(counts.get(&0x0000_0010), Some(&1));
    }

    #[test]
    fn inspect_included_objects_chunk_exposes_skipped_types() {
        let data = build_included_objects_chunk(&[
            type1_object(0),
            fixed_type_object(0x0000_0007, 152),
            fixed_type_object(0x0000_0010, 136),
        ]);

        let value = inspect_included_objects_chunk(&data);
        let parsed_count = value
            .get("parsed_type1_object_count")
            .and_then(|v| v.as_u64());
        assert_eq!(parsed_count, Some(1));

        let raw_counts = value
            .get("raw_object_type_counts")
            .and_then(|v| v.as_array())
            .expect("raw counts array");
        assert!(raw_counts.iter().any(|entry| {
            entry.get("type").and_then(|v| v.as_str()) == Some("0x00000007")
                && entry.get("count").and_then(|v| v.as_u64()) == Some(1)
        }));
        assert!(raw_counts.iter().any(|entry| {
            entry.get("type").and_then(|v| v.as_str()) == Some("0x00000010")
                && entry.get("count").and_then(|v| v.as_u64()) == Some(1)
        }));
    }

    #[test]
    fn decode_archive_entry_bytes_returns_utf8_for_entxml_fallback() {
        let decoded = StarBreakerMcp::decode_archive_entry_bytes(
            "top_deck\\entdata\\123.entxml",
            b"<Entity Name=\"Door\" />",
        );
        assert_eq!(decoded, "<Entity Name=\"Door\" />");
    }

    #[test]
    fn split_socpak_entry_path_parses_outer_and_inner_paths() {
        let (outer, inner) = StarBreakerMcp::split_socpak_entry_path(
            "ObjectContainers/Ships/DRAK/ironclad/top_deck.socpak::top_deck/entdata/1425539375.entxml",
        )
        .expect("nested socpak path should parse");
        assert_eq!(outer, "ObjectContainers/Ships/DRAK/ironclad/top_deck.socpak");
        assert_eq!(inner, "top_deck/entdata/1425539375.entxml");
    }

    #[test]
    fn split_socpak_entry_path_rejects_empty_segments() {
        assert!(StarBreakerMcp::split_socpak_entry_path("foo.socpak::").is_none());
        assert!(StarBreakerMcp::split_socpak_entry_path("::inner.entxml").is_none());
        assert!(StarBreakerMcp::split_socpak_entry_path("plain/path.xml").is_none());
    }

    #[test]
    fn value_to_json_limited_rejects_large_arrays() {
        let value = Value::Array(
            (0..(super::MAX_DATACORE_QUERY_ARRAY_ITEMS + 1))
                .map(|_| Value::Int32(1))
                .collect(),
        );

        let err = value_to_json_limited(&value, 0, &mut JsonBudget::new())
            .expect_err("large array should be rejected");

        assert!(err.contains("array has"));
    }

    #[test]
    fn value_to_json_limited_truncates_long_strings() {
        let input = "a".repeat(super::MAX_DATACORE_QUERY_STRING_CHARS + 10);
        let leaked: &'static str = Box::leak(input.into_boxed_str());

        let json = value_to_json_limited(&Value::String(leaked), 0, &mut JsonBudget::new())
            .expect("string should serialize");

        let output = json.as_str().expect("serialized string");
        assert_eq!(
            output.chars().count(),
            super::MAX_DATACORE_QUERY_STRING_CHARS + 1
        );
        assert!(output.ends_with('…'));
    }
}
