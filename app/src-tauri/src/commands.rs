use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use starbreaker_common::Progress;
use starbreaker_datacore::types::CigGuid;
use starbreaker_p4k::MappedP4k;

use crate::error::AppError;
use crate::state::AppState;

/// Minimum Blender version required to install the StarBreaker addon.
const MIN_BLENDER_MAJOR: u32 = 5;
const MIN_BLENDER_MINOR: u32 = 0;
const EXPORT_FINAL_WRITE_START: f32 = 0.80;

/// Discovery result returned to the frontend.
#[derive(Serialize)]
pub struct DiscoverResult {
    pub path: String,
    pub source: String,
}

/// A directory entry returned to the frontend.
#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum DirEntryDto {
    #[serde(rename = "file")]
    File {
        name: String,
        compressed_size: u64,
        uncompressed_size: u64,
    },
    #[serde(rename = "directory")]
    Directory { name: String },
}

/// Info returned after opening a P4k.
#[derive(Serialize)]
pub struct P4kInfo {
    pub entry_count: usize,
    pub total_bytes: u64,
}

/// Progress event payload.
#[derive(Clone, Serialize)]
pub struct LoadProgress {
    pub fraction: f32,
    pub message: String,
}

/// System theme palette returned to the frontend.
#[derive(Serialize)]
pub struct SystemPalette {
    pub scheme: String,
    pub background: String,
    pub foreground: String,
    pub accent: String,
    pub success: String,
    pub warning: String,
    pub danger: String,
}

/// Get the OS system theme (dark/light, accent color, palette).
#[tauri::command]
pub fn get_system_theme() -> SystemPalette {
    let st = system_theme::SystemTheme::new().ok();
    let theme = st.as_ref().map(|s| s.get_theme());
    let scheme = st
        .as_ref()
        .and_then(|s| s.get_scheme().ok())
        .unwrap_or(system_theme::ThemeScheme::Dark);

    if let Some(theme) = theme {
        let p = &theme.palette;
        let hex = |c: &system_theme::ThemeColor| {
            let r = (c.red * 255.0) as u8;
            let g = (c.green * 255.0) as u8;
            let b = (c.blue * 255.0) as u8;
            format!("#{r:02X}{g:02X}{b:02X}")
        };
        SystemPalette {
            scheme: format!("{scheme:?}"),
            background: hex(&p.background),
            foreground: hex(&p.foreground),
            accent: hex(&p.accent),
            success: hex(&p.success),
            warning: hex(&p.warning),
            danger: hex(&p.danger),
        }
    } else {
        // Fallback
        SystemPalette {
            scheme: "Dark".into(),
            background: "#1A1A1A".into(),
            foreground: "#E2E0E4".into(),
            accent: "#B07CFF".into(),
            success: "#5EC77A".into(),
            warning: "#E8B63A".into(),
            danger: "#E85454".into(),
        }
    }
}

/// Discover all Data.p4k installations across channels.
#[tauri::command]
pub fn discover_p4k() -> Vec<DiscoverResult> {
    starbreaker_common::discover::find_all_p4k()
        .into_iter()
        .map(|d| DiscoverResult {
            path: d.path.to_string_lossy().into_owned(),
            source: d.source,
        })
        .collect()
}

/// Open a P4k file and store it in managed state.
/// Also extracts Data\Game2.dcb and caches the bytes.
#[tauri::command]
pub async fn open_p4k(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<P4kInfo, AppError> {
    let path_clone = path.clone();
    let app_clone = app.clone();

    // Run the heavy open on a blocking thread with progress polling
    let (mapped, dcb_bytes, loc_map, record_index) = tokio::task::spawn_blocking(move || {
        let progress = std::sync::Arc::new(Progress::new());

        // Poll progress and emit events to the frontend
        let progress_poll = progress.clone();
        let poll_thread = std::thread::spawn(move || {
            loop {
                let (fraction, message) = progress_poll.get();
                let _ = app_clone.emit("load-progress", LoadProgress { fraction, message });
                if fraction >= 1.0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });

        let mapped = MappedP4k::open_with_progress(&path_clone, Some(&*progress));

        progress.report(1.0, "Done");
        let _ = poll_thread.join();

        // Extract DCB bytes and localization from the P4k
        let p4k = mapped?;
        let dcb_bytes = p4k.read_file("Data\\Game2.dcb")?;
        let loc_data = p4k
            .read_file("Data\\Localization\\english\\global.ini")
            .unwrap_or_default();
        let loc_map = crate::state::parse_localization(&loc_data);
        let record_index = crate::datacore_commands::build_record_index(&dcb_bytes);
        Ok::<_, AppError>((p4k, dcb_bytes, loc_map, record_index))
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))??;

    let entry_count = mapped.len();
    let total_bytes: u64 = mapped.entries().iter().map(|e| e.uncompressed_size).sum();
    let arc_p4k = Arc::new(mapped);

    *state.p4k.lock() = Some(arc_p4k);
    *state.dcb_bytes.lock() = Some(dcb_bytes);
    *state.localization.lock() = loc_map;
    *state.record_index.lock() = Some(record_index);

    Ok(P4kInfo {
        entry_count,
        total_bytes,
    })
}

/// List only subdirectory names under a path (fast — no file data serialized).
#[tauri::command]
pub fn list_subdirs(state: State<'_, AppState>, path: String) -> Result<Vec<String>, AppError> {
    let guard = state.p4k.lock();
    let p4k = guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?;

    Ok(p4k.list_subdirs(&path))
}

/// List directory contents from the loaded P4k.
#[tauri::command]
pub fn list_dir(state: State<'_, AppState>, path: String) -> Result<Vec<DirEntryDto>, AppError> {
    let guard = state.p4k.lock();
    let p4k = guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?;

    let entries = p4k.list_dir(&path);
    let dtos = entries
        .into_iter()
        .map(|e| match e {
            starbreaker_p4k::DirEntry::File(f) => DirEntryDto::File {
                name: f.name.rsplit('\\').next().unwrap_or(&f.name).to_string(),
                compressed_size: f.compressed_size,
                uncompressed_size: f.uncompressed_size,
            },
            starbreaker_p4k::DirEntry::Directory(name) => DirEntryDto::Directory { name },
        })
        .collect();

    Ok(dtos)
}

#[derive(Clone, Serialize)]
pub struct BlenderAddonStatusDto {
    pub state: String,
    pub current_version: String,
    pub installed_version: Option<String>,
    pub addons_path: Option<String>,
    pub blender_version: Option<String>,
    pub blender_running: bool,
    pub message: Option<String>,
    /// True when Blender installations were found but all are older than 5.0.
    pub incompatible_blender_found: bool,
}

#[derive(Clone)]
struct BlenderAddonTarget {
    blender_version: String,
    addons_path: PathBuf,
}

/// Result of scanning the system for Blender addon directories.
struct BlenderDiscovery {
    /// Targets with Blender ≥ 5.0, sorted by version descending.
    compatible: Vec<BlenderAddonTarget>,
    /// True when at least one Blender config directory was found but filtered out as < 5.0.
    has_incompatible: bool,
}

fn parse_addon_version_from_init(content: &str) -> Option<String> {
    // Prefer installer-facing VERSION first (e.g. VERSION = "0.2.2+addon.2").
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("VERSION") {
            continue;
        }
        let mut parts = trimmed.splitn(2, '=');
        let _ = parts.next();
        if let Some(raw) = parts.next() {
            let value = raw.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    // Fallback for legacy addon bundles that only expose bl_info["version"].
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("\"version\"") && trimmed.contains('(') && trimmed.contains(')') {
            let open = trimmed.find('(')?;
            let close = trimmed[open + 1..].find(')')? + open + 1;
            let tuple = &trimmed[open + 1..close];
            let parts: Vec<String> = tuple
                .split(',')
                .filter_map(|part| {
                    let digits: String = part.chars().filter(|ch| ch.is_ascii_digit()).collect();
                    if digits.is_empty() {
                        None
                    } else {
                        Some(digits)
                    }
                })
                .collect();
            if !parts.is_empty() {
                return Some(parts.join("."));
            }
        }
    }
    None
}

fn version_sort_key(version: &str) -> Vec<u32> {
    version
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u32>().ok())
        .collect()
}

fn version_meets_minimum(version: &str) -> bool {
    let parts = version_sort_key(version);
    match (parts.first(), parts.get(1)) {
        (Some(&major), Some(&minor)) => {
            major > MIN_BLENDER_MAJOR
                || (major == MIN_BLENDER_MAJOR && minor >= MIN_BLENDER_MINOR)
        }
        (Some(&major), None) => major > MIN_BLENDER_MAJOR,
        _ => false,
    }
}

/// Blender user profile folders use major.minor (e.g. 5.1), even when
/// `blender --version` reports a patch component like 5.1.1.
#[cfg(target_os = "windows")]
fn blender_profile_version_dir(version: &str) -> String {
    let parts = version_sort_key(version);
    match (parts.first(), parts.get(1)) {
        (Some(major), Some(minor)) => format!("{major}.{minor}"),
        (Some(major), None) => format!("{major}.0"),
        _ => version.trim().to_string(),
    }
}

/// Return candidate Blender executable paths for a given addons directory.
/// Walks up the tree looking for a binary, then checks common system locations.
fn find_blender_binary_candidates(addons_path: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Walk up (up to 6 levels) from addons_path looking for the binary
    let mut dir = addons_path.to_path_buf();
    for _ in 0..6 {
        if !dir.pop() {
            break;
        }
        #[cfg(target_os = "windows")]
        candidates.push(dir.join("blender.exe"));
        #[cfg(not(target_os = "windows"))]
        candidates.push(dir.join("blender"));
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/blender"));
        candidates.push(PathBuf::from("/usr/local/bin/blender"));
        candidates.push(PathBuf::from("/opt/blender/blender"));
        if let Ok(out) = Command::new("which").arg("blender").output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    candidates.push(PathBuf::from(s));
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/Blender.app/Contents/MacOS/Blender",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Blender.app/Contents/MacOS/blender",
        ));
        if let Ok(out) = Command::new("which").arg("blender").output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    candidates.push(PathBuf::from(s));
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for root in &[
            r"C:\Program Files\Blender Foundation",
            r"C:\Program Files (x86)\Blender Foundation",
        ] {
            if let Ok(entries) = fs::read_dir(root) {
                for entry in entries.flatten() {
                    candidates.push(entry.path().join("blender.exe"));
                }
            }
        }
    }

    candidates.into_iter().filter(|p| p.is_file()).collect()
}

/// Run `blender --version` and parse the version string from stdout.
/// Blender prints: "Blender 5.1.0 (hash abcdef built 2026-04-01 ...)"
fn probe_blender_version(binary: &Path) -> Option<String> {
    let out = Command::new(binary).arg("--version").output().ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("Blender ") {
            if let Some(ver) = line.split_whitespace().nth(1) {
                if ver.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                    return Some(ver.to_string());
                }
            }
        }
    }
    None
}

/// Determine the actual Blender version for a given addons directory.
/// Uses the parent directory name when it looks like a dotted version (e.g. "5.1"),
/// otherwise locates the Blender binary and probes it with `--version`.
fn detect_blender_version(addons_path: &Path, dir_name: &str) -> Option<String> {
    let is_dotted_version = !dir_name.is_empty()
        && !dir_name.starts_with('.')
        && !dir_name.ends_with('.')
        && dir_name.contains('.')
        && dir_name
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.');

    if is_dotted_version {
        return Some(dir_name.to_string());
    }

    // Fall back to probing the binary
    for binary in find_blender_binary_candidates(addons_path) {
        if let Some(ver) = probe_blender_version(&binary) {
            return Some(ver);
        }
    }
    None
}

fn discover_blender_addon_targets() -> BlenderDiscovery {
    let mut roots: Vec<PathBuf> = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            roots.push(PathBuf::from(home).join(".config/blender"));
        }
        roots.push(PathBuf::from("/usr/share/blender"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            roots.push(PathBuf::from(home).join("Library/Application Support/Blender"));
        }
        roots.push(PathBuf::from("/Applications/Blender.app/Contents/Resources"));
    }

    #[cfg(target_os = "windows")]
    {
        // Addon user data always lives under APPDATA, never Program Files.
        // Writing to Program Files requires UAC elevation; Blender itself
        // never installs user addons there.
        if let Ok(appdata) = std::env::var("APPDATA") {
            roots.push(PathBuf::from(appdata).join("Blender Foundation/Blender"));
        }
    }

    let mut compatible: Vec<BlenderAddonTarget> = Vec::new();
    let mut has_incompatible = false;

    for root in roots {
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let initial_addons_path = path.join("scripts").join("addons");
            // Detect the version before checking whether the addons directory
            // exists.  On a fresh Blender install the user data directory
            // (e.g. %APPDATA%\Blender Foundation\Blender\5.1\scripts\addons)
            // may not exist yet — Blender creates it lazily on first launch.
            // We still want to offer it as an install target so that the
            // installer can create the directory rather than refusing with
            // "requires 5.0+ / upgrade Blender" when a compatible version is
            // installed but the profile directory hasn't been initialised yet.
            // If the addons directory does not exist *and* the version is not
            // compatible, skip silently (no has_incompatible flag — we don't
            // want non-Blender directories to pollute the error state).
            let blender_version =
                detect_blender_version(&initial_addons_path, &dir_name).unwrap_or_else(|| dir_name.clone());
            #[cfg(target_os = "windows")]
            let profile_dir = blender_profile_version_dir(&blender_version);
            #[cfg(not(target_os = "windows"))]
            let profile_dir = dir_name.clone();
            let addons_path = root.join(&profile_dir).join("scripts").join("addons");
            let is_compatible = version_meets_minimum(&blender_version);
            if !addons_path.exists() {
                // Include the target only if we are confident the version
                // is compatible.  Skip entries we cannot confirm at all.
                if is_compatible {
                    compatible.push(BlenderAddonTarget {
                        blender_version,
                        addons_path,
                    });
                }
                // Do NOT set has_incompatible for missing-directory entries;
                // that would trigger the misleading "upgrade Blender" error.
                continue;
            }
            if is_compatible {
                compatible.push(BlenderAddonTarget {
                    blender_version,
                    addons_path,
                });
            } else {
                has_incompatible = true;
            }
        }
    }

    // Windows: also probe Program Files to discover Blender installations whose
    // APPDATA profile directory hasn't been created yet (i.e. Blender was installed
    // but never launched).  We read the version from the binary, then construct the
    // correct APPDATA-based addon path — we never write to Program Files.
    #[cfg(target_os = "windows")]
    {
        let appdata_blender = std::env::var("APPDATA")
            .ok()
            .map(|a| PathBuf::from(a).join("Blender Foundation/Blender"));
        if let Some(appdata_root) = appdata_blender {
            for pf_root in &[
                r"C:\Program Files\Blender Foundation",
                r"C:\Program Files (x86)\Blender Foundation",
            ] {
                let Ok(entries) = fs::read_dir(pf_root) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let pf_path = entry.path();
                    if !pf_path.is_dir() {
                        continue;
                    }
                    let binary = pf_path.join("blender.exe");
                    if !binary.is_file() {
                        continue;
                    }
                    let Some(ver) = probe_blender_version(&binary) else {
                        continue;
                    };
                    if !version_meets_minimum(&ver) {
                        has_incompatible = true;
                        continue;
                    }
                    let profile_dir = blender_profile_version_dir(&ver);
                    let addons_path = appdata_root
                        .join(profile_dir)
                        .join("scripts")
                        .join("addons");
                    if !compatible.iter().any(|t| t.addons_path == addons_path) {
                        compatible.push(BlenderAddonTarget {
                            blender_version: ver,
                            addons_path,
                        });
                    }
                }
            }
        }
    }

    compatible.sort_by(|a, b| {
        let ka = version_sort_key(&a.blender_version);
        let kb = version_sort_key(&b.blender_version);
        kb.cmp(&ka)
    });
    compatible.dedup_by(|a, b| a.addons_path == b.addons_path);
    BlenderDiscovery {
        compatible,
        has_incompatible,
    }
}

fn addon_source_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest_dir.join("../blender_addon/starbreaker_addon"),
        manifest_dir.join("../../blender_addon/starbreaker_addon"),
    ];
    for candidate in candidates {
        if candidate.join("__init__.py").is_file() {
            return Some(candidate);
        }
    }
    None
}

fn current_addon_version() -> Result<String, AppError> {
    // Use the build-time version string (derived from git commit count) so
    // that every new build automatically shows "Update available" to users
    // who have an older installed copy.  No manual VERSION bump required.
    Ok(env!("ADDON_BUILD_VERSION").to_string())
}

fn installed_addon_version(addons_path: &Path) -> Option<String> {
    let init_path = addons_path.join("starbreaker_addon").join("__init__.py");
    let content = fs::read_to_string(init_path).ok()?;
    parse_addon_version_from_init(&content)
}

fn blender_running() -> bool {
    #[cfg(target_os = "linux")]
    {
        return Command::new("pgrep")
            .args(["-f", "[bB]lender"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
    }

    #[cfg(target_os = "macos")]
    {
        return Command::new("pgrep")
            .args(["-f", "[bB]lender"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq blender.exe"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains("blender.exe"))
            .unwrap_or(false);
    }

    #[allow(unreachable_code)]
    false
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), AppError> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src = entry.path();
        let dst = destination.join(entry.file_name());
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

fn compute_addon_status(
    target: Option<BlenderAddonTarget>,
    incompatible_blender_found: bool,
) -> Result<BlenderAddonStatusDto, AppError> {
    let current_version = current_addon_version()?;
    let running = blender_running();

    let Some(target) = target else {
        let message = if incompatible_blender_found {
            format!(
                "Blender was found but requires version {}.{} or newer. \
                 Please upgrade Blender to use the StarBreaker addon.",
                MIN_BLENDER_MAJOR, MIN_BLENDER_MINOR
            )
        } else {
            "No Blender installation was detected. Install Blender {}.{}+ or provide a manual target path."
                .replace("{}.{}", &format!("{}.{}", MIN_BLENDER_MAJOR, MIN_BLENDER_MINOR))
        };
        return Ok(BlenderAddonStatusDto {
            state: "unavailable".to_string(),
            current_version,
            installed_version: None,
            addons_path: None,
            blender_version: None,
            blender_running: running,
            message: Some(message),
            incompatible_blender_found,
        });
    };

    let installed_version = installed_addon_version(&target.addons_path);
    let state = match installed_version.as_deref() {
        None => "install",
        Some(installed) if installed == current_version => "installed",
        Some(_) => "upgrade",
    };

    Ok(BlenderAddonStatusDto {
        state: state.to_string(),
        current_version,
        installed_version,
        addons_path: Some(target.addons_path.to_string_lossy().to_string()),
        blender_version: Some(target.blender_version),
        blender_running: running,
        message: if running {
            Some(
                "Blender appears to be running. Restart Blender after install/upgrade to reload the addon."
                    .to_string(),
            )
        } else {
            None
        },
        incompatible_blender_found,
    })
}

#[tauri::command]
pub fn get_blender_addon_status() -> Result<BlenderAddonStatusDto, AppError> {
    let discovery = discover_blender_addon_targets();
    let has_incompatible = discovery.has_incompatible;
    compute_addon_status(discovery.compatible.into_iter().next(), has_incompatible)
}

#[tauri::command]
pub fn install_blender_addon(target_path: Option<String>) -> Result<BlenderAddonStatusDto, AppError> {
    let source_dir = addon_source_dir().ok_or_else(|| {
        AppError::Internal("Unable to locate bundled starbreaker_addon source directory".into())
    })?;

    let (target, has_incompatible) = if let Some(path) = target_path {
        // Manual path: trust the user, version is unknown
        (
            BlenderAddonTarget {
                blender_version: "manual".to_string(),
                addons_path: PathBuf::from(path),
            },
            false,
        )
    } else {
        let discovery = discover_blender_addon_targets();
        let has_incompatible = discovery.has_incompatible;
        let target = discovery.compatible.into_iter().next().ok_or_else(|| {
            if has_incompatible {
                AppError::Internal(format!(
                    "Blender found but requires {}.{}+. Please upgrade Blender.",
                    MIN_BLENDER_MAJOR, MIN_BLENDER_MINOR
                ))
            } else {
                AppError::Internal("No Blender installation found.".into())
            }
        })?;
        (target, has_incompatible)
    };

    fs::create_dir_all(&target.addons_path)?;
    let destination = target.addons_path.join("starbreaker_addon");
    if destination.exists() {
        let _ = fs::remove_dir_all(destination.join("__pycache__"));
        fs::remove_dir_all(&destination)?;
    }
    copy_dir_recursive(&source_dir, &destination)?;

    // Stamp the installed __init__.py with the build-time version so that
    // compute_addon_status correctly reports "up to date" after a fresh install
    // rather than always showing "update available".
    let init_path = destination.join("__init__.py");
    if let Ok(content) = fs::read_to_string(&init_path) {
        let build_version = env!("ADDON_BUILD_VERSION");
        let patched = content
            .lines()
            .map(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with("VERSION") && trimmed.contains('=') {
                    format!("VERSION = \"{}\"", build_version)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Preserve a trailing newline if the original had one.
        let patched = if content.ends_with('\n') {
            patched + "\n"
        } else {
            patched
        };
        let _ = fs::write(&init_path, patched);
    }

    let mut status = compute_addon_status(Some(target), has_incompatible)?;
    if status.blender_running {
        status.message = Some(
            "Addon files were installed. Blender is running — restart Blender to load the new version."
                .to_string(),
        );
    }
    Ok(status)
}

/// Attempt to reload the StarBreaker addon in the running Blender instance via its
/// built-in TCP server (port 6264). Falls back gracefully if the server is not running.
#[tauri::command]
pub fn reload_blender_addon() -> Result<String, AppError> {
    if !blender_running() {
        return Ok(
            "Blender is not running. The addon will load automatically on next start.".to_string(),
        );
    }

    // Blender's built-in Python server listens on 127.0.0.1:6264 when enabled.
    // The protocol is: send the Python source followed by a newline, then read the response.
    let snippet = "\
import importlib, sys, bpy; \
mod = sys.modules.get('starbreaker_addon'); \
bpy.ops.preferences.addon_disable(module='starbreaker_addon') if mod else None; \
[sys.modules.pop(k, None) for k in list(sys.modules) if k.startswith('starbreaker_addon')]; \
bpy.ops.preferences.addon_enable(module='starbreaker_addon')\
";

    match TcpStream::connect_timeout(
        &"127.0.0.1:6264".parse().unwrap(),
        Duration::from_millis(500),
    ) {
        Ok(mut stream) => {
            let payload = format!("{snippet}\n");
            match stream.write_all(payload.as_bytes()) {
                Ok(_) => Ok("Reload command sent to Blender.".to_string()),
                Err(e) => Ok(format!(
                    "Connected to Blender but failed to send reload: {e}. Restart Blender manually."
                )),
            }
        }
        Err(_) => Ok(
            "Restart Blender to complete the upgrade (Blender's Python server is not enabled)."
                .to_string(),
        ),
    }
}

// ── DataCore / Export DTOs ──────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct EntityDto {
    pub name: String,
    pub id: String,
    /// Localized display name (e.g., "S-38 Pistol"). None if no translation found.
    pub display_name: Option<String>,
    /// True if not a player-available variant (inclusionMode != "ReadyToInclude").
    /// Covers AI, template, unmanned, and other non-player variants.
    pub is_npc_or_internal: bool,
}

#[derive(Clone, Serialize)]
pub struct CategoryDto {
    pub name: String,
    pub entities: Vec<EntityDto>,
}

/// Scan EntityClassDefinition records from the cached DCB and return categorized entities.
#[tauri::command]
pub async fn scan_categories(state: State<'_, AppState>) -> Result<Vec<CategoryDto>, AppError> {
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?
            .clone()
    };
    let loc = {
        let guard = state.localization.lock();
        guard.clone()
    };

    tokio::task::spawn_blocking(move || {
        let db = starbreaker_datacore::database::Database::from_bytes(&dcb_bytes)?;

        use starbreaker_datacore::QueryResultExt;
        use starbreaker_datacore::query::value::Value;

        // Pre-compile query paths using rooted syntax (StructName.path).
        // .optional() turns TypeFilterMismatch into None (component not in schema),
        // but propagates real errors (typo in path, wrong leaf type, etc.).
        let loc_compiled = db.compile_rooted::<Value>(
            "EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Localization.Name",
        ).optional()?;

        let inclusion_compiled = db.compile_rooted::<Value>(
            "EntityClassDefinition.StaticEntityClassData[EAEntityDataParams].inclusionMode",
        ).optional()?;

        let mut ships = Vec::new();
        let mut ground_vehicles = Vec::new();
        let mut weapons = Vec::new();
        let mut other = Vec::new();

        for record in db.records_by_type_name("EntityClassDefinition") {
            if !db.is_main_record(record) {
                continue;
            }

            let name = db.resolve_string2(record.name_offset).to_string();
            let file_path = db.resolve_string(record.file_name_offset);
            let file_path_lower = file_path.to_lowercase();

            // Look up localized display name from DataCore's localization key.
            // The record stores e.g. "@item_Namebehr_pistol_ballistic_01" — strip
            // the "@" prefix and look up in the INI map.
            let display_name = loc_compiled.as_ref()
                .and_then(|c| db.query_single::<Value>(c, record).ok().flatten())
                .and_then(|v| match v {
                    Value::String(s) | Value::Locale(s) => Some(s.to_string()),
                    other => {
                        eprintln!("WARNING: Localization.Name for {name}: expected String/Locale, got {other:?}");
                        None
                    }
                })
                .filter(|s| !s.is_empty() && s != "@LOC_UNINITIALIZED" && s != "@LOC_EMPTY")
                .and_then(|key| {
                    let stripped = key.strip_prefix('@').unwrap_or(&key);
                    loc.get(&stripped.to_lowercase()).cloned()
                });

            // Non-player variants have inclusionMode != "ReadyToInclude".
            // inclusionMode is a DataCore enum — query as Value.
            // Entities without the component return None (not NPC).
            let is_npc_or_internal = inclusion_compiled.as_ref()
                .and_then(|c| db.query_single::<Value>(c, record).ok().flatten())
                .is_some_and(|v| match v {
                    Value::Enum(s) => s != "ReadyToInclude",
                    _ => false,
                });

            let info = EntityDto {
                name,
                id: format!("{}", record.id),
                display_name,
                is_npc_or_internal,
            };

            if file_path_lower.contains("entities/spaceships") {
                ships.push(info);
            } else if file_path_lower.contains("entities/groundvehicles") {
                ground_vehicles.push(info);
            } else if file_path_lower.contains("weapon") {
                weapons.push(info);
            } else {
                other.push(info);
            }
        }

        // Sort by display name when available, fall back to DataCore name
        let sort_key = |e: &EntityDto| {
            e.display_name.clone().unwrap_or_else(|| e.name.clone())
        };
        ships.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        ground_vehicles.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        weapons.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        other.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));

        Ok::<_, AppError>(vec![
            CategoryDto {
                name: "Ships".to_string(),
                entities: ships,
            },
            CategoryDto {
                name: "Ground Vehicles".to_string(),
                entities: ground_vehicles,
            },
            CategoryDto {
                name: "Weapons".to_string(),
                entities: weapons,
            },
            CategoryDto {
                name: "Other".to_string(),
                entities: other,
            },
        ])
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

// ── Export commands ──────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ExportProgress {
    pub current: usize,
    pub total: usize,
    pub fraction: f32,
    pub entity_name: String,
    pub entity_id: String,
    pub stage: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ExportDone {
    pub success: usize,
    pub errors: usize,
    pub succeeded_ids: Vec<String>,
}

fn bundled_extension(format: starbreaker_3d::ExportFormat) -> &'static str {
    match format {
        starbreaker_3d::ExportFormat::Glb => "glb",
        starbreaker_3d::ExportFormat::Stl => "stl",
        starbreaker_3d::ExportFormat::Blend => "blend",
    }
}

fn prepare_decomposed_output_root(output_root: &Path, package_name: &str) -> Result<(), AppError> {
    if output_root.exists() {
        if output_root.is_file() {
            return Err(AppError::Internal(format!(
                "decomposed output root '{}' already exists as a file",
                output_root.display(),
            )));
        }
    }

    let packages_root = output_root.join("Packages");
    let package_root = packages_root.join(package_name);
    if package_root.exists() {
        std::fs::remove_dir_all(&package_root)?;
    }

    std::fs::create_dir_all(&package_root)?;
    Ok(())
}

fn decomposed_package_directory_name(
    files: &[starbreaker_3d::ExportedFile],
    fallback_name: &str,
) -> String {
    for file in files {
        let Some(rest) = file.relative_path.strip_prefix("Packages/") else {
            continue;
        };
        let Some((package_name, _)) = rest.split_once('/') else {
            continue;
        };
        if !package_name.is_empty() {
            return package_name.to_string();
        }
    }
    fallback_name.to_string()
}

#[derive(Debug, serde::Deserialize)]
pub struct ExportRequest {
    pub record_ids: Vec<String>,
    pub names: Vec<String>,
    pub output_dir: String,
    pub lod: u32,
    pub mip: u32,
    pub export_kind: String,
    /// "none", "colors", "textures", "all"
    pub material_mode: String,
    /// "glb", "stl", or "blend"
    pub format: String,
    pub include_attachments: bool,
    pub include_interior: bool,
    pub include_lights: bool,
    pub threads: usize,
    pub overwrite_existing_assets: bool,
    pub include_nodraw: bool,
    pub include_animations: bool,
}

#[derive(Clone)]
struct ExportProgressSlot {
    entity_name: String,
    entity_id: String,
    progress: Arc<Progress>,
    done: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
enum DecomposedWriteOutcome {
    Written,
    SkippedExisting,
}

fn snapshot_export_progress(slots: &[ExportProgressSlot]) -> ExportProgress {
    if slots.is_empty() {
        return ExportProgress {
            current: 0,
            total: 0,
            fraction: 1.0,
            entity_name: String::new(),
            entity_id: String::new(),
            stage: String::new(),
            error: None,
        };
    }

    let mut completed = 0usize;
    let mut fraction_sum = 0.0f32;
    let mut active_name = String::new();
    let mut active_id = String::new();
    let mut active_stage = String::new();
    let mut active_fraction = -1.0f32;
    let mut fallback_name = String::new();
    let mut fallback_id = String::new();
    let mut fallback_stage = String::new();
    let mut fallback_fraction = -1.0f32;

    for slot in slots {
        let (fraction, stage) = slot.progress.get();
        if slot.done.load(Ordering::Relaxed) {
            completed += 1;
            fraction_sum += 1.0;
        } else {
            // Keep running entities slightly below 100% so the aggregate only
            // reaches 100% after every export slot is marked complete.
            fraction_sum += fraction.min(0.9999);
        }

        let has_activity = fraction > 0.0 || !stage.is_empty();
        if has_activity && fraction >= fallback_fraction {
            fallback_fraction = fraction;
            fallback_name = slot.entity_name.clone();
            fallback_id = slot.entity_id.clone();
            fallback_stage = stage.clone();
        }

        if fraction < 1.0 && has_activity && fraction >= active_fraction {
            active_fraction = fraction;
            active_name = slot.entity_name.clone();
            active_id = slot.entity_id.clone();
            active_stage = stage;
        }
    }

    if active_name.is_empty() {
        active_name = fallback_name;
        active_id = fallback_id;
        active_stage = fallback_stage;
    }

    ExportProgress {
        current: completed,
        total: slots.len(),
        fraction: (fraction_sum / slots.len() as f32).clamp(0.0, 1.0),
        entity_name: active_name,
        entity_id: active_id,
        stage: active_stage,
        error: None,
    }
}

async fn emit_export_progress_until_done(
    app: AppHandle,
    slots: Vec<ExportProgressSlot>,
    done: Arc<AtomicBool>,
) {
    loop {
        let _ = app.emit("export-progress", snapshot_export_progress(&slots));
        if done.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn should_skip_existing_decomposed_asset(
    file: &starbreaker_3d::ExportedFile,
    overwrite_existing_assets: bool,
) -> bool {
    !overwrite_existing_assets && file.kind.is_mesh_or_texture_asset()
}

fn write_decomposed_file(
    file: &starbreaker_3d::ExportedFile,
    file_path: &Path,
    overwrite_existing_assets: bool,
) -> Result<DecomposedWriteOutcome, AppError> {
    if file_path.exists() {
        if !file_path.is_file() {
            return Err(AppError::Internal(format!(
                "decomposed output path '{}' already exists as a directory",
                file_path.display(),
            )));
        }
        if should_skip_existing_decomposed_asset(file, overwrite_existing_assets) {
            return Ok(DecomposedWriteOutcome::SkippedExisting);
        }
    }

    std::fs::write(file_path, &file.bytes)?;
    Ok(DecomposedWriteOutcome::Written)
}

fn collect_existing_decomposed_assets(output_root: &Path) -> Result<HashSet<String>, AppError> {
    let data_root = output_root.join("Data");
    let mut existing = HashSet::new();
    if !data_root.exists() {
        return Ok(existing);
    }

    let mut pending = vec![data_root];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            if !matches!(extension, "glb" | "png") {
                continue;
            }

            let relative = path
                .strip_prefix(output_root)
                .map_err(|_| {
                    AppError::Internal(format!(
                        "failed to compute relative decomposed asset path for '{}'",
                        path.display(),
                    ))
                })?
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            existing.insert(relative);
        }
    }

    Ok(existing)
}

/// Start exporting selected entities to bundled files.
#[tauri::command]
pub async fn start_export(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ExportRequest,
) -> Result<(), AppError> {
    // Reset cancel flag
    state.export_cancel.store(false, Ordering::SeqCst);

    // Clone data out of state
    let p4k = {
        let guard = state.p4k.lock();
        guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?
            .clone()
    };
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?
            .clone()
    };
    let cancel = state.export_cancel.clone();

    // Parse record IDs upfront
    let record_ids: Vec<CigGuid> = request
        .record_ids
        .iter()
        .map(|s| s.parse::<CigGuid>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let id_strings: Vec<String> = record_ids.iter().map(|id| id.to_string()).collect();
    let export_names: Vec<String> = request
        .names
        .iter()
        .map(|name| sanitize_export_name(&export_entity_name(name)))
        .collect();
    let progress_slots: Vec<ExportProgressSlot> = export_names
        .iter()
        .zip(id_strings.iter())
        .map(|(entity_name, entity_id)| ExportProgressSlot {
            entity_name: entity_name.clone(),
            entity_id: entity_id.clone(),
            progress: Arc::new(Progress::new()),
            done: Arc::new(AtomicBool::new(false)),
        })
        .collect();
    let progress_done = Arc::new(AtomicBool::new(false));

    tauri::async_runtime::spawn(emit_export_progress_until_done(
        app.clone(),
        progress_slots.clone(),
        progress_done.clone(),
    ));

    let material_mode = match request.material_mode.to_lowercase().as_str() {
        "none" => starbreaker_3d::MaterialMode::None,
        "colors" => starbreaker_3d::MaterialMode::Colors,
        "all" => starbreaker_3d::MaterialMode::All,
        _ => starbreaker_3d::MaterialMode::Textures,
    };
    let kind = match request.export_kind.to_lowercase().as_str() {
        "decomposed" => starbreaker_3d::ExportKind::Decomposed,
        _ => starbreaker_3d::ExportKind::Bundled,
    };
    let format = match request.format.to_lowercase().as_str() {
        "stl" => starbreaker_3d::ExportFormat::Stl,
        "blend" => starbreaker_3d::ExportFormat::Blend,
        _ => starbreaker_3d::ExportFormat::Glb,
    };
    let existing_asset_paths = if kind == starbreaker_3d::ExportKind::Decomposed
        && !request.overwrite_existing_assets
    {
        Arc::new(collect_existing_decomposed_assets(Path::new(&request.output_dir))?)
    } else {
        Arc::new(HashSet::new())
    };
    let opts = starbreaker_3d::ExportOptions {
        kind,
        format,
        material_mode,
        include_attachments: request.include_attachments,
        include_interior: request.include_interior,
        include_lights: request.include_lights,
        include_nodraw: request.include_nodraw,
        include_shields: false,
        texture_mip: request.mip,
        lod_level: request.lod,
        include_animations: request.include_animations,
        apply_default_animation_pose: !request.include_animations,
        default_animation_tags: vec!["landing_gear_extend".to_string()],
    };

    log::info!(
        "[export] material_mode={:?} format={:?} include_interior={} include_attachments={} include_lights={} include_nodraw={} lod={} mip={}",
        opts.material_mode,
        opts.format,
        opts.include_interior,
        opts.include_attachments,
        opts.include_lights,
        opts.include_nodraw,
        opts.lod_level,
        opts.texture_mip
    );

    let output_dir = request.output_dir;
    let requested_threads = request.threads;
    let overwrite_existing_assets = request.overwrite_existing_assets;

    tokio::task::spawn_blocking(move || {
        let db = match starbreaker_datacore::database::Database::from_bytes(&dcb_bytes) {
            Ok(db) => db,
            Err(_) => {
                progress_done.store(true, Ordering::Relaxed);
                let _ = app.emit(
                    "export-done",
                    ExportDone {
                        success: 0,
                        errors: record_ids.len(),
                        succeeded_ids: Vec::new(),
                    },
                );
                return;
            }
        };

        let total = record_ids.len();
        let success = AtomicUsize::new(0);
        let errors = AtomicUsize::new(0);
        let succeeded_ids: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

        // 0 = auto (half cores), otherwise use the requested count.
        let num_threads = if requested_threads > 0 {
            requested_threads
        } else {
            (std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                / 2)
            .max(2)
        };
        let pool = match rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build()
        {
            Ok(pool) => pool,
            Err(e) => {
                progress_done.store(true, Ordering::Relaxed);
                let _ = app.emit(
                    "export-done",
                    ExportDone {
                        success: 0,
                        errors: total,
                        succeeded_ids: Vec::new(),
                    },
                );
                eprintln!("failed to build thread pool: {e}");
                return;
            }
        };

        pool.install(|| {
            use rayon::prelude::*;
            record_ids
                .par_iter()
                .zip(export_names.par_iter())
                .zip(id_strings.par_iter())
                .zip(progress_slots.par_iter())
                .for_each(|(((record_id, export_name), id_str), progress_slot)| {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }

                    progress_slot.progress.report(0.01, "Resolving loadout");

                    let filename = format!(
                        "{}.{}",
                        sanitize_filename(export_name),
                        bundled_extension(opts.format),
                    );
                    let output_path = match opts.kind {
                        starbreaker_3d::ExportKind::Bundled => {
                            std::path::PathBuf::from(&output_dir).join(&filename)
                        }
                        starbreaker_3d::ExportKind::Decomposed => std::path::PathBuf::from(&output_dir),
                    };

                    match export_single(
                        &db,
                        &p4k,
                        record_id,
                        &output_path,
                        &opts,
                        export_name,
                        overwrite_existing_assets,
                        Some(progress_slot.progress.as_ref()),
                        Some(existing_asset_paths.as_ref()),
                    ) {
                        Ok(()) => {
                            progress_slot.done.store(true, Ordering::Relaxed);
                            success.fetch_add(1, Ordering::Relaxed);
                            succeeded_ids.lock().unwrap().push(id_str.clone());
                        }
                        Err(e) => {
                            progress_slot.progress.report(1.0, "Failed");
                            progress_slot.done.store(true, Ordering::Relaxed);
                            let mut snapshot = snapshot_export_progress(&progress_slots);
                            snapshot.entity_name = export_name.clone();
                            snapshot.entity_id = id_str.clone();
                            snapshot.stage = "Failed".to_string();
                            snapshot.error = Some(format!("{export_name}: {e}"));
                            let _ = app.emit(
                                "export-progress",
                                snapshot,
                            );
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
        }); // pool.install

        progress_done.store(true, Ordering::Relaxed);

        let _ = app.emit(
            "export-done",
            ExportDone {
                success: success.load(Ordering::Relaxed),
                errors: errors.load(Ordering::Relaxed),
                succeeded_ids: succeeded_ids.into_inner().unwrap(),
            },
        );
    });

    Ok(())
}

/// Cancel an in-progress export.
#[tauri::command]
pub fn cancel_export(state: State<'_, AppState>) {
    state.export_cancel.store(true, Ordering::SeqCst);
}

/// Export a single entity to a bundled file.
fn export_single(
    db: &starbreaker_datacore::database::Database,
    p4k: &MappedP4k,
    record_id: &CigGuid,
    output_path: &Path,
    opts: &starbreaker_3d::ExportOptions,
    export_name: &str,
    overwrite_existing_assets: bool,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Result<(), AppError> {
    let record = db
        .record_by_id(record_id)
        .ok_or_else(|| AppError::Internal("record not found".into()))?;
    let idx = starbreaker_datacore::loadout::EntityIndex::new(db);
    let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);
    let result = starbreaker_3d::assemble_glb_with_loadout_with_progress(
        db,
        p4k,
        record,
        &tree,
        opts,
        progress,
        existing_asset_paths,
    )?;
    match result.kind {
        starbreaker_3d::ExportKind::Bundled => {
            let bundled_bytes = result.bundled_bytes().ok_or_else(|| {
                AppError::Internal(format!(
                    "export returned non-bundled output for {:?}",
                    result.kind,
                ))
            })?;
            if let Some(progress) = progress {
                progress.report(EXPORT_FINAL_WRITE_START, "Writing bundled file");
            }
            std::fs::write(output_path, bundled_bytes)?;
            if let Some(progress) = progress {
                progress.report(1.0, "Done");
            }
        }
        starbreaker_3d::ExportKind::Decomposed => {
            let decomposed = result.decomposed.as_ref().ok_or_else(|| {
                AppError::Internal("export returned no decomposed files".into())
            })?;
            let package_name = decomposed_package_directory_name(&decomposed.files, export_name);
            prepare_decomposed_output_root(output_path, &package_name)?;
            let total_files = decomposed.files.len().max(1);
            for (index, file) in decomposed.files.iter().enumerate() {
                let file_path = output_path.join(&file.relative_path);
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let outcome = write_decomposed_file(file, &file_path, overwrite_existing_assets)?;
                if let Some(progress) = progress {
                    let fraction = (index + 1) as f32 / total_files as f32;
                    let stage = match outcome {
                        DecomposedWriteOutcome::Written => "Writing package files",
                        DecomposedWriteOutcome::SkippedExisting => "Skipping existing assets",
                    };
                    progress.report(
                        EXPORT_FINAL_WRITE_START + (1.0 - EXPORT_FINAL_WRITE_START) * fraction,
                        stage,
                    );
                }
            }
            if let Some(progress) = progress {
                progress.report(1.0, "Done");
            }
        }
    }
    Ok(())
}

fn export_entity_name(name: &str) -> String {
    let trimmed = name.trim_matches('"');
    trimmed
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

fn sanitize_export_name(name: &str) -> String {
    let mut cleaned = String::new();
    let mut last_was_space = false;

    for ch in name.chars() {
        if ch.is_alphanumeric() {
            cleaned.push(ch);
            last_was_space = false;
        } else if ch.is_whitespace() || matches!(ch, '_' | '-') {
            if !cleaned.is_empty() && !last_was_space {
                cleaned.push(' ');
                last_was_space = true;
            }
        }
    }

    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Export".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Sanitize a filename by replacing invalid characters with underscores.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        decomposed_package_directory_name, export_entity_name, sanitize_export_name,
        should_skip_existing_decomposed_asset,
    };

    #[test]
    fn export_entity_name_strips_record_prefix_and_quotes() {
        assert_eq!(export_entity_name("EntityClassDefinition.ARGO_MOLE\""), "ARGO_MOLE");
        assert_eq!(export_entity_name("ARGO_MOLE"), "ARGO_MOLE");
    }

    #[test]
    fn sanitize_export_name_preserves_alphanumerics_and_spaces() {
        assert_eq!(sanitize_export_name("Argo Mole Teach's Special"), "Argo Mole Teachs Special");
        assert_eq!(sanitize_export_name("ARGO_MOLE"), "ARGO MOLE");
    }

    #[test]
    fn decomposed_package_directory_name_preserves_exporter_suffixes() {
        let files = vec![
            starbreaker_3d::ExportedFile {
                relative_path: "Packages/ARGO MOLE_LOD0_TEX0/scene.json".into(),
                bytes: Vec::new(),
                kind: starbreaker_3d::ExportedFileKind::PackageManifest,
            },
            starbreaker_3d::ExportedFile {
                relative_path: "Data/Objects/Test/root.glb".into(),
                bytes: Vec::new(),
                kind: starbreaker_3d::ExportedFileKind::MeshAsset,
            },
        ];

        assert_eq!(
            decomposed_package_directory_name(&files, "Argo Mole"),
            "ARGO MOLE_LOD0_TEX0"
        );
    }

    #[test]
    fn decomposed_package_directory_name_falls_back_when_packages_path_is_absent() {
        let files = vec![starbreaker_3d::ExportedFile {
            relative_path: "Data/Objects/Test/root.glb".into(),
            bytes: Vec::new(),
            kind: starbreaker_3d::ExportedFileKind::MeshAsset,
        }];

        assert_eq!(
            decomposed_package_directory_name(&files, "Argo Mole"),
            "Argo Mole"
        );
    }

    #[test]
    fn skip_existing_assets_only_applies_to_meshes_and_textures() {
        let mesh = starbreaker_3d::ExportedFile {
            relative_path: "Data/Objects/Test/root.glb".into(),
            bytes: Vec::new(),
            kind: starbreaker_3d::ExportedFileKind::MeshAsset,
        };
        let texture = starbreaker_3d::ExportedFile {
            relative_path: "Data/Objects/Test/root.png".into(),
            bytes: Vec::new(),
            kind: starbreaker_3d::ExportedFileKind::TextureAsset,
        };
        let material = starbreaker_3d::ExportedFile {
            relative_path: "Data/Objects/Test/root.materials.json".into(),
            bytes: Vec::new(),
            kind: starbreaker_3d::ExportedFileKind::MaterialSidecar,
        };

        assert!(should_skip_existing_decomposed_asset(&mesh, false));
        assert!(should_skip_existing_decomposed_asset(&texture, false));
        assert!(!should_skip_existing_decomposed_asset(&material, false));
        assert!(!should_skip_existing_decomposed_asset(&mesh, true));
    }
}

/// Generate a GLB preview for a geometry file in the P4K.
/// Accepts .skin, .skinm, .cgf, .cgfm, .cga, .chr paths.
/// Companion files (.skinm/.cgfm) are resolved to their primary (.skin/.cgf).
#[tauri::command]
pub fn preview_geometry(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<u8>, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    // Resolve companion: .skinm -> .skin, .cgfm -> .cgf, .cgam -> .cga
    let primary = if path.ends_with('m') && (path.ends_with(".skinm") || path.ends_with(".cgfm") || path.ends_with(".cgam")) {
        path[..path.len() - 1].to_string()
    } else {
        path.clone()
    };

    // Read companion file (vertex data) — fall back to primary if no companion exists
    let companion = format!("{primary}m");
    let data = p4k
        .read_file(&companion)
        .or_else(|_| p4k.read_file(&primary))?;

    // Read primary file for NMC transforms (scene graph hierarchy).
    // The NMC chunk lives in the .cgf/.skin/.cga, not the companion .cgfm/.skinm/.cgam.
    let metadata = p4k.read_file(&primary).ok();

    let glb = starbreaker_3d::skin_to_glb(&data, metadata.as_deref())?;
    Ok(glb)
}

/// Decode a CryXMLB file from the P4K and return it as formatted XML text.
#[tauri::command]
pub fn preview_xml(state: tauri::State<'_, AppState>, path: String) -> Result<String, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    let data = p4k.read_file(&path)?;

    // Try CryXMLB decode first, fall back to raw UTF-8
    if starbreaker_cryxml::is_cryxmlb(&data) {
        let cryxml = starbreaker_cryxml::from_bytes(&data)?;
        Ok(format!("{cryxml}"))
    } else {
        Ok(String::from_utf8_lossy(&data).into_owned())
    }
}

/// Read a raw file from the P4K. Used for images (PNG, TGA, etc.) that
/// don't need server-side decoding.
#[tauri::command]
pub fn read_p4k_file(state: tauri::State<'_, AppState>, path: String) -> Result<Vec<u8>, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();
    Ok(p4k.read_file(&path)?)
}

/// Progress event for folder extraction.
#[derive(Clone, Serialize)]
pub struct FolderExtractProgress {
    pub current: usize,
    pub total: usize,
    pub name: String,
}

/// Extract all files under a P4k folder path to disk.
#[tauri::command]
pub async fn extract_p4k_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    path_prefix: String,
    output_dir: String,
    filter: Option<String>,
) -> Result<usize, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    tokio::task::spawn_blocking(move || {
        let prefix = if path_prefix.ends_with('\\') {
            path_prefix.clone()
        } else {
            format!("{path_prefix}\\")
        };

        // Parse extension filters (comma-separated, e.g. "mtl,xml")
        let extensions: Vec<String> = filter
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(|s| {
                let s = s.trim().to_lowercase();
                if s.starts_with('.') {
                    s
                } else {
                    format!(".{s}")
                }
            })
            .filter(|s| s.len() > 1)
            .collect();

        let entries: Vec<_> = p4k
            .entries()
            .iter()
            .filter(|e| {
                if !e.name.starts_with(&prefix) || e.uncompressed_size == 0 {
                    return false;
                }
                if extensions.is_empty() {
                    return true;
                }
                let name_lower = e.name.to_lowercase();
                extensions
                    .iter()
                    .any(|ext| name_lower.ends_with(ext.as_str()))
            })
            .collect();

        let count = entries.len();
        let out = std::path::Path::new(&output_dir);

        for (i, entry) in entries.iter().enumerate() {
            if i % 50 == 0 || i + 1 == count {
                let short_name = entry
                    .name
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&entry.name)
                    .to_string();
                let _ = app.emit(
                    "folder-extract-progress",
                    FolderExtractProgress {
                        current: i + 1,
                        total: count,
                        name: short_name,
                    },
                );
            }
            let rel = entry.name.replace('\\', "/");
            let dest = out.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let data = p4k.read(entry)?;
            std::fs::write(&dest, &data)?;
        }

        Ok::<_, AppError>(count)
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

/// Metadata returned alongside a DDS preview so the frontend can show mip controls.
#[derive(serde::Serialize)]
pub struct DdsPreviewResult {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub mip_level: usize,
    pub mip_count: usize,
}

/// P4K-backed sibling reader for split DDS mip files.
struct P4kSiblingReader {
    p4k: std::sync::Arc<starbreaker_p4k::MappedP4k>,
    base_path: String,
}

impl starbreaker_dds::ReadSibling for P4kSiblingReader {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let path = format!("{}{suffix}", self.base_path);
        self.p4k.read_file(&path).ok()
    }
}

/// Decode a DDS texture from the P4K (merging split mip siblings) and return
/// a specific mip level as PNG bytes along with metadata for mip selection.
#[tauri::command]
pub fn preview_dds(
    state: tauri::State<'_, AppState>,
    path: String,
    mip: Option<usize>,
) -> Result<DdsPreviewResult, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    let data = p4k.read_file(&path)?;
    let sibling_reader = P4kSiblingReader {
        p4k: p4k.clone(),
        base_path: path.clone(),
    };
    let dds = starbreaker_dds::DdsFile::from_split(&data, &sibling_reader)?;

    if dds.mip_count() == 0 {
        return Err(AppError::Internal("DDS has no mip data".into()));
    }

    let mip_level = mip.unwrap_or(0).min(dds.mip_count() - 1);
    let (width, height) = dds.dimensions(mip_level);
    let rgba = dds.decode_rgba(mip_level)?;

    let mut png_buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
    image::ImageEncoder::write_image(
        encoder,
        &rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(DdsPreviewResult {
        png: png_buf,
        width,
        height,
        mip_level,
        mip_count: dds.mip_count(),
    })
}

/// Save a DDS texture from the P4K as a PNG file to disk.
#[tauri::command]
pub fn export_dds_png(
    state: tauri::State<'_, AppState>,
    path: String,
    output_path: String,
    mip: Option<usize>,
) -> Result<(), AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    let data = p4k.read_file(&path)?;
    let sibling_reader = P4kSiblingReader {
        p4k: p4k.clone(),
        base_path: path.clone(),
    };
    let dds = starbreaker_dds::DdsFile::from_split(&data, &sibling_reader)?;

    if dds.mip_count() == 0 {
        return Err(AppError::Internal("DDS has no mip data".into()));
    }

    let mip_level = mip.unwrap_or(0).min(dds.mip_count() - 1);
    let (width, height) = dds.dimensions(mip_level);
    let rgba = dds.decode_rgba(mip_level)?;

    let out = std::path::Path::new(&output_path);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(out)?;
    let encoder = image::codecs::png::PngEncoder::new(file);
    image::ImageEncoder::write_image(
        encoder,
        &rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(())
}

/// Extract a single file from the P4K to disk.
#[tauri::command]
pub fn extract_p4k_file(
    state: tauri::State<'_, AppState>,
    path: String,
    output_path: String,
) -> Result<(), AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    let data = p4k.read_file(&path)?;
    let out = std::path::Path::new(&output_path);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, &data)?;

    Ok(())
}
