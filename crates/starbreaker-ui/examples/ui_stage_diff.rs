//! Pipeline-stage bisection diagnostic for layout/geometry bugs
//! (crates/starbreaker-ui/docs/ui-process-improvements.md item 6).
//!
//! Runs a BuildingBlocks canvas through (a) PARSE-ONLY and (b) FULL RESOLVE
//! (canvas-graph merge + style cascade), lays both out at the same size, and
//! prints per-node typed sizing + computed rects side by side, flagging the
//! first node whose sizing diverges between the stages. This is the fastest
//! way to tell whether a geometry bug lives in parse/layout or in the
//! resolve/style cascade (it cracked the emissions-header collapse: sizing
//! was `Percent(1.0)` standalone, `Fixed(1.0)` after the cascade).
//!
//! Usage:
//!   cargo run -p starbreaker-ui --example ui_stage_diff -- \
//!     <canvas.json> [WxH] [--records-root <dir>] [--filter <name-substring>]
//!
//! - `WxH` defaults to the canvas's authored size.
//! - `--records-root` defaults to the decompiled record mirror
//!   `~/projects/scorg_tools/ships/dcb_canvas/libs/foundry/records`.
//! - Nodes are matched ACROSS stages by NAME (resolve remaps ids); names that
//!   are not unique within a stage are compared positionally per occurrence.

use std::collections::BTreeMap;

fn default_records_root() -> String {
    format!(
        "{}/projects/scorg_tools/ships/dcb_canvas/libs/foundry/records",
        std::env::var("HOME").unwrap_or_default()
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut canvas_path = None;
    let mut size: Option<(u32, u32)> = None;
    let mut records_root = default_records_root().to_string();
    let mut filter = String::new();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--records-root" => records_root = it.next().expect("--records-root <dir>"),
            "--filter" => filter = it.next().expect("--filter <substring>").to_lowercase(),
            other if other.contains('x') && other.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
                let (w, h) = other.split_once('x').expect("WxH");
                size = Some((w.parse().expect("W"), h.parse().expect("H")));
            }
            other => canvas_path = Some(other.to_string()),
        }
    }
    let canvas_path = canvas_path.expect(
        "usage: ui_stage_diff <canvas.json> [WxH] [--records-root <dir>] [--filter <substr>]",
    );

    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&canvas_path).expect("read canvas json"),
    )
    .expect("parse canvas json");

    let record_value = json.get("_RecordValue_").unwrap_or(&json);
    let (w, h) = size.unwrap_or_else(|| {
        let sz = record_value.get("size");
        (
            sz.and_then(|s| s.get("x")).and_then(|v| v.as_f64()).unwrap_or(1600.0) as u32,
            sz.and_then(|s| s.get("y")).and_then(|v| v.as_f64()).unwrap_or(900.0) as u32,
        )
    });

    let parsed = starbreaker_ui::bb_scene::parse_bb_canvas(&json).expect("parse stage");
    let parsed_rows = stage_rows(&parsed, w, h, &filter);

    let root = records_root.clone();
    let fetch = move |url: &str| -> Result<serde_json::Value, String> {
        let rel = url.trim_start_matches("file://./").replace("../", "");
        let candidate = format!("{root}/{}", rel.split("records/").nth(1).unwrap_or(&rel));
        std::fs::read_to_string(&candidate)
            .map_err(|e| format!("{candidate}: {e}"))
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    };
    let defaults = starbreaker_ui::defaults::DefaultValueRegistry::default();
    let resolved = starbreaker_ui::bb_resolve::resolve_canvas_graph_with_defaults(
        &json,
        Some("drak"),
        &fetch,
        None,
        None,
        &defaults,
    )
    .expect("resolve stage");
    let resolved_rows = stage_rows(&resolved, w, h, &filter);

    println!("== ui_stage_diff: {canvas_path} at {w}x{h}");
    println!("-- PARSE-ONLY ({} matching nodes)", parsed_rows.len());
    for row in parsed_rows.values().flatten() {
        println!("  {row}");
    }
    println!("-- FULL RESOLVE ({} matching nodes)", resolved_rows.len());
    for row in resolved_rows.values().flatten() {
        println!("  {row}");
    }

    // Name-matched divergence report (per occurrence index).
    let mut first_divergence: Option<String> = None;
    for (name, p_rows) in &parsed_rows {
        let Some(r_rows) = resolved_rows.get(name) else { continue };
        for (i, (p, r)) in p_rows.iter().zip(r_rows.iter()).enumerate() {
            let p_geom = p.split_once(" :: ").map(|x| x.1).unwrap_or(p);
            let r_geom = r.split_once(" :: ").map(|x| x.1).unwrap_or(r);
            if p_geom != r_geom {
                let line = format!("'{name}'[{i}]\n    parse:   {p_geom}\n    resolve: {r_geom}");
                if first_divergence.is_none() {
                    first_divergence = Some(line.clone());
                }
            }
        }
    }
    match first_divergence {
        Some(d) => println!("\nFIRST DIVERGENCE: {d}"),
        None => println!("\nNO DIVERGENCE among name-matched nodes (try a different --filter)"),
    }
}

/// `name -> [row per occurrence]`, row = "id name [ty] :: sizing=(...) rect=(...)".
fn stage_rows(
    scene: &starbreaker_ui::bb_scene::BbScene,
    w: u32,
    h: u32,
    filter: &str,
) -> BTreeMap<String, Vec<String>> {
    let result = starbreaker_ui::bb_layout::layout(scene, w, h);
    let mut rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, node) in &scene.nodes {
        if !filter.is_empty() && !node.name.to_lowercase().contains(filter) {
            continue;
        }
        if node.name.is_empty() || node.name == "<unnamed>" {
            continue;
        }
        let rect = result
            .rects
            .get(id)
            .map(|r| format!("({:.1},{:.1},{:.1},{:.1})", r.x, r.y, r.w, r.h))
            .unwrap_or_else(|| "(no rect)".to_string());
        rows.entry(node.name.clone()).or_default().push(format!(
            "{id} {} [{:?}] :: sizing=({:?},{:?}) rect={rect}",
            node.name, node.ty, node.sizing.width, node.sizing.height,
        ));
    }
    rows
}
