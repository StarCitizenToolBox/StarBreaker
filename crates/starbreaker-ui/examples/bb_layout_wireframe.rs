//! Wireframe overlay generator for BuildingBlocks canvas fixtures.
//!
//! Usage:
//!   cargo run --example bb_layout_wireframe -p starbreaker-ui \
//!     -- <fixture.json> <target.png> [--width W] [--height H] [--merge]
//!
//! - `<fixture.json>` — path to a `BuildingBlocks_Canvas` DataCore JSON file.
//! - `<target.png>`  — output PNG path.
//! - `--width W`     — raster width (default 1600).
//! - `--height H`    — raster height (default 900).
//! - `--merge`       — run `resolve_canvas_graph` before laying out; resolves
//!                     child canvas references by searching for JSON files with
//!                     matching record-name basenames in the same directory as
//!                     the fixture.

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: bb_layout_wireframe <fixture.json> <target.png> \
             [--width W] [--height H] [--merge]"
        );
        std::process::exit(1);
    }

    let fixture_path = &args[1];
    let target_path = &args[2];

    // Parse optional flags.
    let mut target_w: u32 = 1600;
    let mut target_h: u32 = 900;
    let mut merge = false;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--width" => {
                i += 1;
                target_w = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1600);
            }
            "--height" => {
                i += 1;
                target_h = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(900);
            }
            "--merge" => merge = true,
            _ => {}
        }
        i += 1;
    }

    // Load the fixture.
    let fixture_text = std::fs::read_to_string(fixture_path)
        .unwrap_or_else(|e| panic!("cannot read {fixture_path}: {e}"));
    let json: serde_json::Value = serde_json::from_str(&fixture_text)
        .unwrap_or_else(|e| panic!("cannot parse {fixture_path}: {e}"));

    // Determine the directory containing the fixture for sibling lookups.
    let fixture_dir = Path::new(fixture_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let scene = if merge {
        // Resolve child canvases by searching the fixture directory.
        let fetch = |path: &str| -> Result<serde_json::Value, String> {
            let record_name = starbreaker_ui::pipeline::extract_record_name(path);
            // Walk the fixture directory for a file whose stem matches the record name.
            let entries = std::fs::read_dir(&fixture_dir).map_err(|e| e.to_string())?;
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let stem = Path::new(&fname)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                // Match: stem starts with the record name (handles hash suffixes).
                if stem == record_name || stem.starts_with(&format!("{record_name}_")) {
                    let text = std::fs::read_to_string(entry.path())
                        .map_err(|e| e.to_string())?;
                    return serde_json::from_str(&text).map_err(|e| e.to_string());
                }
            }
            Err(format!("fixture not found for record name: {record_name}"))
        };

        starbreaker_ui::bb_resolve::resolve_canvas_graph(&json, None, &fetch)
            .unwrap_or_else(|e| panic!("resolve_canvas_graph failed: {e}"))
    } else {
        starbreaker_ui::bb_scene::parse_bb_canvas(&json)
            .unwrap_or_else(|e| panic!("parse_bb_canvas failed: {e}"))
    };

    eprintln!(
        "Loaded scene: canvas={:.0}x{:.0} nodes={} roots={}",
        scene.canvas_size.0,
        scene.canvas_size.1,
        scene.nodes.len(),
        scene.roots.len(),
    );

    // Render the wireframe.
    let img = starbreaker_ui::bb_layout::render_wireframe(&scene, target_w, target_h);

    // Ensure parent directories exist.
    if let Some(parent) = Path::new(target_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("cannot create output dir: {e}"));
        }
    }

    img.save(target_path)
        .unwrap_or_else(|e| panic!("cannot save {target_path}: {e}"));

    eprintln!("Wrote wireframe to {target_path}");

    // Print layout summary.
    let result = starbreaker_ui::bb_layout::layout(&scene, target_w, target_h);
    eprintln!("Layout: {} rects, {} draw-order entries", result.rects.len(), result.draw_order.len());
}
