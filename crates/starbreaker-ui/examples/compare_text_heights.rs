//! Compare externally measured text heights against UI IR text-style sizes.
//!
//! Usage:
//!   cargo run -p starbreaker-ui --example compare_text_heights -- \
//!     <measurements.csv> <ir.json>
//!
//! CSV format (header optional):
//!   full string, measured height in pixels[, measured width in pixels]
//! Example:
//!   MEDICAL ASSISTANT,38,286

use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
struct Measurement {
    text: String,
    measured_px: f32,
    measured_width_px: Option<f32>,
}

#[derive(Debug, Clone)]
struct IrTextEntry {
    node_name: String,
    text: String,
    font_size: f32,
}

fn parse_measurements(csv_path: &str) -> Result<Vec<Measurement>> {
    let raw = fs::read_to_string(csv_path)
        .with_context(|| format!("failed reading measurements csv: {csv_path}"))?;
    let mut out = Vec::new();

    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Allow an optional header row.
        if idx == 0 && line.to_ascii_lowercase().contains("measured") {
            continue;
        }

        let mut parts = line.rsplitn(3, ',');
        let last = parts
            .next()
            .map(str::trim)
            .context("csv parse failed: missing measured column")?;
        let middle = parts.next().map(str::trim);
        let text = parts
            .next()
            .map(str::trim)
            .context("csv parse failed: missing full-string column")?;

        let (measured_px, measured_width_px) = if let Some(height) = middle {
            let measured_px: f32 = height
                .parse()
                .with_context(|| format!("invalid measured height '{height}' in line '{line}'"))?;
            let measured_width_px: f32 = last
                .parse()
                .with_context(|| format!("invalid measured width '{last}' in line '{line}'"))?;
            (measured_px, Some(measured_width_px))
        } else {
            let measured_px: f32 = last
                .parse()
                .with_context(|| format!("invalid measured height '{last}' in line '{line}'"))?;
            (measured_px, None)
        };

        if text.is_empty() {
            continue;
        }

        out.push(Measurement {
            text: text.to_string(),
            measured_px,
            measured_width_px,
        });
    }

    Ok(out)
}

fn parse_ir_text_entries(ir_path: &str) -> Result<Vec<IrTextEntry>> {
    let raw = fs::read_to_string(ir_path)
        .with_context(|| format!("failed reading ir json: {ir_path}"))?;
    let json: Value = serde_json::from_str(&raw).context("failed parsing ir json")?;

    let nodes = json
        .get("nodes")
        .and_then(Value::as_array)
        .context("ir json missing nodes[]")?;

    let mut out = Vec::new();
    for node in nodes {
        let text = node
            .get("text_payload")
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        if text.is_empty() {
            continue;
        }

        let font_size = node
            .get("text_style")
            .and_then(|v| v.get("font_size"))
            .and_then(|v| v.get("value"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32;
        if font_size <= 0.0 {
            continue;
        }

        let node_name = node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unnamed>")
            .to_string();

        out.push(IrTextEntry {
            node_name,
            text: text.to_string(),
            font_size,
        });
    }

    Ok(out)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        anyhow::bail!(
            "Usage: compare_text_heights <measurements.csv> <ir.json>\nCSV: full string, measured height in pixels[, measured width in pixels]"
        );
    }

    let measurements = parse_measurements(&args[0])?;
    let ir_entries = parse_ir_text_entries(&args[1])?;

    let mut ir_by_text: HashMap<String, Vec<IrTextEntry>> = HashMap::new();
    for entry in ir_entries {
        ir_by_text.entry(entry.text.clone()).or_default().push(entry);
    }

    println!("string,measured_px,measured_width_px,ir_font_size_px,measured_to_ir_ratio,node_name,match_count");
    for m in measurements {
        let matches = ir_by_text.get(&m.text).cloned().unwrap_or_default();
        if matches.is_empty() {
            let measured_width = m
                .measured_width_px
                .map(|v| format!("{v:.2}"))
                .unwrap_or_default();
            println!(
                "\"{}\",{:.2},{},,,NO_IR_MATCH,0",
                m.text, m.measured_px, measured_width
            );
            continue;
        }

        let preferred = matches
            .iter()
            .max_by(|a, b| {
                a.font_size
                    .partial_cmp(&b.font_size)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("non-empty matches");
        let ratio = if preferred.font_size > 0.0 {
            m.measured_px / preferred.font_size
        } else {
            0.0
        };

        let measured_width = m
            .measured_width_px
            .map(|v| format!("{v:.2}"))
            .unwrap_or_default();
        println!(
            "\"{}\",{:.2},{},{:.2},{:.3},\"{}\",{}",
            m.text,
            m.measured_px,
            measured_width,
            preferred.font_size,
            ratio,
            preferred.node_name,
            matches.len()
        );
    }

    Ok(())
}