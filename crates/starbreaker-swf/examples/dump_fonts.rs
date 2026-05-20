//! Day 1 diagnostic: dump every embedded font from an SWF.
//!
//! Usage: `cargo run -p starbreaker-swf --example dump_fonts -- <path-to-swf>`

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let path: PathBuf = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: dump_fonts <path-to-swf>"))?
        .into();

    let bytes = std::fs::read(&path)?;
    let fonts = starbreaker_swf::analyze_fonts(&bytes)?;

    println!("{} fonts in {}", fonts.len(), path.display());
    println!(
        "{:>4}  {:<8} {:<32} {:>6} {:<8} {:<7} {:<9} {}",
        "id", "ver", "name", "glyphs", "layout", "weight", "code", "display_name"
    );
    println!("{}", "-".repeat(110));
    for f in &fonts {
        let weight = if f.is_bold && f.is_italic {
            "Bold-It"
        } else if f.is_bold {
            "Bold"
        } else if f.is_italic {
            "Italic"
        } else {
            "Regular"
        };
        let layout = if f.has_layout { "yes" } else { "no" };
        let code = match f.codepoint_range {
            Some((mn, mx)) => format!("{mn:#06x}-{mx:#06x}"),
            None => "(none)".to_string(),
        };
        let display = f.display_name.as_deref().unwrap_or("");
        println!(
            "{:>4}  v{:<7} {:<32} {:>6} {:<8} {:<7} {:<9} {}",
            f.id, f.version, f.name, f.glyph_count, layout, weight, code, display
        );
    }
    Ok(())
}
