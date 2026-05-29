//! SWF library loading and import resolution.

use std::collections::HashSet;
use std::io::Cursor;

use crate::swf_assets::SwfAssetLibrary;
use swf::Tag;

use super::super::SwfFetcher;

pub(crate) fn load_first_swf(paths: &[String], fetcher: &dyn SwfFetcher) -> SwfAssetLibrary {
    for path in paths {
        match fetcher.fetch_swf_bytes(path) {
            Ok(bytes) => {
                let mut root_lib = match SwfAssetLibrary::new(bytes.clone()) {
                    Ok(lib) => lib,
                    Err(e) => {
                        log::warn!("pipeline: failed to parse SWF '{}': {}", path, e);
                        continue;
                    }
                };
                let mut pending = vec![(normalize_p4k_swf_path(path), bytes)];
                let mut seen = HashSet::new();

                let canvas_path = "Data/UI/BuildingBlocks/assets/SWF/Canvas.swf".to_string();
                if let Ok(canvas_bytes) = fetcher.fetch_swf_bytes(&canvas_path) {
                    if let Err(e) = root_lib.merge_swf_bytes(&canvas_bytes) {
                        log::debug!(
                            "pipeline: failed to merge BuildingBlocks canvas SWF '{}': {}",
                            canvas_path,
                            e
                        );
                    }
                    pending.push((canvas_path, canvas_bytes));
                }

                let shared_fonts_path = "Data/UI/fonts/Shared/fonts_en.gfx".to_string();
                if let Ok(shared_bytes) = fetcher.fetch_swf_bytes(&shared_fonts_path) {
                    if let Err(e) = root_lib.merge_swf_bytes(&shared_bytes) {
                        log::debug!(
                            "pipeline: failed to merge shared fonts SWF '{}': {}",
                            shared_fonts_path,
                            e
                        );
                    }
                    pending.push((shared_fonts_path, shared_bytes));
                }

                while let Some((current_path, current_bytes)) = pending.pop() {
                    if !seen.insert(current_path.clone()) {
                        continue;
                    }
                    for import_path in collect_import_swf_paths(&current_path, &current_bytes) {
                        if seen.contains(&import_path) {
                            continue;
                        }
                        match fetcher.fetch_swf_bytes(&import_path) {
                            Ok(import_bytes) => {
                                if let Err(e) = root_lib.merge_swf_bytes(&import_bytes) {
                                    log::debug!(
                                        "pipeline: failed to merge imported SWF '{}': {}",
                                        import_path,
                                        e
                                    );
                                }
                                pending.push((import_path, import_bytes));
                            }
                            Err(e) => {
                                log::debug!(
                                    "pipeline: import SWF fetch failed for '{}': {}",
                                    import_path,
                                    e
                                );
                            }
                        }
                    }
                }
                return root_lib;
            }
            Err(e) => {
                log::debug!("pipeline: SWF fetch failed for '{}': {}", path, e);
            }
        }
    }

    let minimal: Vec<u8> = vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0, 0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    SwfAssetLibrary::new(minimal).expect("minimal SWF is always valid")
}

fn normalize_p4k_swf_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    if replaced.to_ascii_lowercase().starts_with("data/") {
        replaced
    } else {
        format!("Data/{replaced}")
    }
}

fn resolve_relative_swf_path(base_path: &str, import_url: &str) -> String {
    let import = import_url.replace('\\', "/");
    if import.is_empty() {
        return String::new();
    }
    if import.to_ascii_lowercase().starts_with("data/") {
        return import;
    }

    let base_norm = base_path.replace('\\', "/");
    let mut base_parts: Vec<&str> = base_norm.split('/').collect();
    if !base_parts.is_empty() {
        base_parts.pop();
    }

    for part in import.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !base_parts.is_empty() {
                    base_parts.pop();
                }
            }
            other => base_parts.push(other),
        }
    }

    let joined = base_parts.join("/");
    if joined.to_ascii_lowercase().starts_with("data/") {
        joined
    } else {
        format!("Data/{joined}")
    }
}

fn collect_import_swf_paths(current_path: &str, bytes: &[u8]) -> Vec<String> {
    let Ok(swf_buf) = swf::decompress_swf(Cursor::new(bytes)) else {
        return Vec::new();
    };
    let Ok(parsed) = swf::parse_swf(&swf_buf) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for tag in &parsed.tags {
        if let Tag::ImportAssets { url, .. } = tag {
            let url_text = url.to_string_lossy(swf::UTF_8);
            let resolved = resolve_relative_swf_path(current_path, &url_text);
            if !resolved.is_empty() {
                out.push(resolved);
            }
        }
    }
    out
}
