//! Trace resolved scene style context for text nodes from a compiled IR dump.
//!
//! Usage:
//!   cargo run -p starbreaker-ui --example trace_text_style_context -- \
//!     <ir.json> <root_canvas.json> <json_root_dir>

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use starbreaker_ui::bb_resolve::resolve_canvas_graph_with_loc;
use starbreaker_ui::pipeline::extract_record_name;

fn walk_collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_collect(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(p);
        }
    }
}

fn node_style(raw: &serde_json::Value) -> Option<String> {
    raw.get("labelProperties")
        .and_then(|v| v.get("style"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn style_tags(raw: &serde_json::Value) -> Vec<String> {
    raw.get("styleTags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|tag| tag.get("_RecordId_").and_then(|id| id.as_str()))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        anyhow::bail!(
            "Usage: trace_text_style_context <ir.json> <root_canvas.json> <json_root_dir>"
        );
    }

    let ir_raw = std::fs::read_to_string(&args[0]).with_context(|| "read ir json")?;
    let ir_json: serde_json::Value = serde_json::from_str(&ir_raw).with_context(|| "parse ir")?;
    let target_nodes = ir_json
        .get("nodes")
        .and_then(|v| v.as_array())
        .context("ir missing nodes[]")?;

    let mut text_targets: HashMap<u64, (String, String)> = HashMap::new();
    for n in target_nodes {
        let Some(id) = n.get("id").and_then(|v| v.as_u64()) else {
            continue;
        };
        let text = n
            .get("text_payload")
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let name = n
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("<unnamed>")
            .to_string();
        text_targets.insert(id, (name, text));
    }

    let root_canvas_raw = std::fs::read_to_string(&args[1]).with_context(|| "read root canvas")?;
    let root_canvas: serde_json::Value =
        serde_json::from_str(&root_canvas_raw).with_context(|| "parse root canvas")?;

    let mut all: Vec<PathBuf> = Vec::new();
    walk_collect(Path::new(&args[2]), &mut all);

    let fetch = |path: &str| -> std::result::Result<serde_json::Value, String> {
        let rec = extract_record_name(path);
        let rec_l = rec.to_lowercase();
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

    let scene = resolve_canvas_graph_with_loc(&root_canvas, None, &fetch, None)
        .map_err(anyhow::Error::msg)
        .context("resolve scene")?;

    let target_ids: HashSet<u64> = text_targets.keys().copied().collect();
    println!("id\tname\ttext\tnode_style\tnode_tags\tancestor_context");
    for (id, node) in &scene.nodes {
        let id_u64 = *id as u64;
        if !target_ids.contains(&id_u64) {
            continue;
        }
        let (name, text) = text_targets
            .get(&id_u64)
            .cloned()
            .unwrap_or_else(|| (node.name.clone(), String::new()));

        let own_style = node_style(&node.raw).unwrap_or_default();
        let own_tags = style_tags(&node.raw).join("|");

        let mut ctx = Vec::new();
        let mut current = node.parent;
        while let Some(pid) = current {
            if let Some(parent) = scene.nodes.get(&pid) {
                let p_style = node_style(&parent.raw).unwrap_or_default();
                let p_tags = style_tags(&parent.raw).join("|");
                if !p_style.is_empty() || !p_tags.is_empty() {
                    ctx.push(format!("{}:{}:{}", parent.name, p_style, p_tags));
                }
                current = parent.parent;
            } else {
                break;
            }
        }

        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            id_u64,
            name,
            text.replace('\t', " "),
            own_style,
            own_tags,
            ctx.join(" <- ")
        );
    }

    Ok(())
}