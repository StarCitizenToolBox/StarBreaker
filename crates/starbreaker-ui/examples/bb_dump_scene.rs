//! Dump merged BbScene with per-node rects for debugging visual fidelity.
//!
//! Usage:
//!   cargo run --release --example bb_dump_scene -p starbreaker-ui -- \
//!     <fixture.json> <dir> [--width W] [--height H]
//!
//! Walks `<dir>` (recursive) to resolve sibling canvas records.

use std::path::{Path, PathBuf};

use starbreaker_ui::bb_layout;
use starbreaker_ui::bb_resolve::resolve_canvas_graph_with_loc;
use starbreaker_ui::bb_scene::{BbNodeType, BbScene};
use starbreaker_ui::pipeline::extract_record_name;

fn walk_collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_collect(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: bb_dump_scene <fixture.json> <flat_dir> [--width W] [--height H]");
        std::process::exit(1);
    }
    let fixture = &args[1];
    let dir = PathBuf::from(&args[2]);
    let mut tw: u32 = 1600;
    let mut th: u32 = 900;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--width" => { i += 1; tw = args[i].parse().unwrap_or(1600); }
            "--height" => { i += 1; th = args[i].parse().unwrap_or(900); }
            _ => {}
        }
        i += 1;
    }

    let mut all: Vec<PathBuf> = Vec::new();
    walk_collect(&dir, &mut all);
    eprintln!("indexed {} JSON files under {}", all.len(), dir.display());

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();

    let fetch = |path: &str| -> Result<serde_json::Value, String> {
        let rec = extract_record_name(path);
        let rec_l = rec.to_lowercase();
        // Prefer exact stem match; fall back to prefix only if no exact match found.
        // Prefix matching ("h_eng_annunciator_" matching "h_eng_annunciator_master_combined")
        // is ambiguous and causes false cycle expansion — exact match must win.
        let mut prefix_hit: Option<&PathBuf> = None;
        for p in &all {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let stem_l = stem.to_lowercase();
            if stem_l == rec_l {
                let text = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
                return serde_json::from_str(&text).map_err(|e| e.to_string());
            }
            if prefix_hit.is_none() && stem_l.starts_with(&format!("{rec_l}_")) {
                prefix_hit = Some(p);
            }
        }
        if let Some(p) = prefix_hit {
            let text = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
            return serde_json::from_str(&text).map_err(|e| e.to_string());
        }
        Err(format!("not found: {rec}"))
    };

    let scene = resolve_canvas_graph_with_loc(&json, None, &fetch, None)
        .unwrap_or_else(|e| panic!("resolve failed: {e}"));

    let res = bb_layout::layout(&scene, tw, th);
    print_dump(&scene, &res, tw, th);
}

fn print_dump(scene: &BbScene, res: &bb_layout::LayoutResult, tw: u32, th: u32) {
    println!("# Canvas {}x{} (target {}x{})", scene.canvas_size.0, scene.canvas_size.1, tw, th);
    println!("# Nodes: {}, Roots: {}", scene.nodes.len(), scene.roots.len());
    println!("# id\ttype\tname\tactive\tparent\trect(x,y,w,h)\tsvgFill\tcustomIcon\ttext\tbg.color");

    // Print in draw order, with depth indentation by tracing parent chain.
    for nid in res.draw_order.iter() {
        let Some(n) = scene.nodes.get(nid) else { continue };
        let mut depth = 0;
        let mut p = n.parent;
        while let Some(pp) = p { depth += 1; p = scene.nodes.get(&pp).and_then(|x| x.parent); }
        let indent = "  ".repeat(depth);
        let rect = res.rects.get(nid);
        let (x, y, w, h) = rect.map(|r| (r.x, r.y, r.w, r.h)).unwrap_or((0.0, 0.0, 0.0, 0.0));
        let ty = match &n.ty {
            BbNodeType::Other(s) => format!("Other:{s}"),
            t => format!("{:?}", t),
        };
        let svg = n.background.as_ref().and_then(|b| b.svg_fill_path.clone()).unwrap_or_default();
        let icon = n.icon.as_ref().and_then(|i| i.image_record.clone()).unwrap_or_default();
        let text = n.text.as_ref().map(|t| t.string.clone()).unwrap_or_default();
        let bg = n.background.as_ref().and_then(|b| b.fill_colour)
            .map(|c| format!("{:.2},{:.2},{:.2},{:.2}", c[0], c[1], c[2], c[3]))
            .unwrap_or_default();
        println!(
            "{indent}{nid}\t{ty}\t{}\t{}\t{:?}\t({:.0},{:.0},{:.0},{:.0})\t{svg}\t{icon}\t{}\t{bg}",
            n.name, n.is_active, n.parent, x, y, w, h,
            text.replace('\n', "\\n").chars().take(40).collect::<String>(),
        );
    }
}
