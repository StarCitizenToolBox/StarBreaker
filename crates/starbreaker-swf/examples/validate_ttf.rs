//! Validate an extracted TTF: parse with ttf-parser, dump high-level stats,
//! render the outline of one glyph as an SVG path string.
//!
//! Usage: `cargo run -p starbreaker-swf --example validate_ttf -- <font.ttf> [char]`

use std::path::PathBuf;
use ttf_parser::{Face, OutlineBuilder};

struct SvgBuilder {
    d: String,
}
impl OutlineBuilder for SvgBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.d.push_str(&format!("M{x},{y} "));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.d.push_str(&format!("L{x},{y} "));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.d.push_str(&format!("Q{x1},{y1} {x},{y} "));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.d.push_str(&format!("C{x1},{y1} {x2},{y2} {x},{y} "));
    }
    fn close(&mut self) {
        self.d.push_str("Z ");
    }
}

fn main() -> anyhow::Result<()> {
    let path: PathBuf = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: validate_ttf <ttf> [char]"))?
        .into();
    let test_char: char = std::env::args().nth(2).and_then(|s| s.chars().next()).unwrap_or('A');

    let bytes = std::fs::read(&path)?;
    let face = Face::parse(&bytes, 0)?;

    println!("=== {} ===", path.display());
    println!(
        "family:        {:?}",
        face.names().into_iter().find(|n| n.name_id == 1).map(|n| n.to_string())
    );
    println!(
        "subfamily:     {:?}",
        face.names().into_iter().find(|n| n.name_id == 2).map(|n| n.to_string())
    );
    println!(
        "full name:     {:?}",
        face.names().into_iter().find(|n| n.name_id == 4).map(|n| n.to_string())
    );
    println!("units per em:  {}", face.units_per_em());
    println!("ascender:      {}", face.ascender());
    println!("descender:     {}", face.descender());
    println!("line gap:      {}", face.line_gap());
    println!("num glyphs:    {}", face.number_of_glyphs());
    println!("global bbox:   {:?}", face.global_bounding_box());
    println!("is monospaced: {}", face.is_monospaced());
    println!("is bold:       {}", face.is_bold());
    println!("is italic:     {}", face.is_italic());
    println!("weight:        {:?}", face.weight());

    let gid = face.glyph_index(test_char);
    println!("\n--- glyph for {test_char:?} (U+{:04X}) ---", test_char as u32);
    if let Some(gid) = gid {
        println!("glyph id:      {}", gid.0);
        println!("advance:       {:?}", face.glyph_hor_advance(gid));
        println!("bbox:          {:?}", face.glyph_bounding_box(gid));
        let mut sb = SvgBuilder { d: String::new() };
        if face.outline_glyph(gid, &mut sb).is_some() {
            // Print as a viewable SVG (1024x1024 box, glyph centered + scaled)
            let upem = face.units_per_em() as f32;
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-{0} -{1} {2} {2}">
  <path d="{3}" fill="black" transform="scale(1,-1)"/>
</svg>"#,
                upem * 0.1,
                upem * 0.9,
                upem,
                sb.d.trim()
            );
            let out = path.with_extension(format!("glyph-{}.svg", test_char as u32));
            std::fs::write(&out, &svg)?;
            println!("svg outline:   wrote {} ({} chars)", out.display(), sb.d.len());
        } else {
            println!("svg outline:   (no outline data)");
        }
    } else {
        println!("(no glyph for this character)");
    }

    Ok(())
}
