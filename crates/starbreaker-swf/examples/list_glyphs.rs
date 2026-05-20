//! Print the printable-character coverage of a TTF.
//!
//! Usage: `cargo run -p starbreaker-swf --example list_glyphs -- <font.ttf>`

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let path: PathBuf = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: list_glyphs <ttf>"))?
        .into();
    let bytes = std::fs::read(&path)?;
    let face = ttf_parser::Face::parse(&bytes, 0)?;

    let family = face
        .names()
        .into_iter()
        .find(|n| n.name_id == 1)
        .and_then(|n| n.to_string())
        .unwrap_or_default();
    println!("=== {} ===", family);
    println!("Total glyphs: {} (incl. .notdef)", face.number_of_glyphs());

    let mut present: Vec<(u32, char)> = Vec::new();
    for cp in 0x20u32..=0x2FFFu32 {
        if let Some(ch) = char::from_u32(cp)
            && face.glyph_index(ch).is_some()
        {
            present.push((cp, ch));
        }
    }
    println!("Mapped characters: {}", present.len());
    println!();

    // Print as a grid grouped by Unicode block
    let mut current_block: u32 = 0;
    for (cp, ch) in &present {
        let block = cp / 0x80;
        if block != current_block {
            current_block = block;
            print!("\nU+{:04X}–: ", block * 0x80);
        }
        if ch.is_ascii_graphic() || (*cp >= 0x20 && *cp < 0x7F) {
            print!("{ch}");
        } else {
            print!("·");
        }
    }
    println!("\n");

    // List the missing-but-common letters/digits
    let common_basic_latin = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let missing: String = common_basic_latin
        .chars()
        .filter(|c| face.glyph_index(*c).is_none())
        .collect();
    if !missing.is_empty() {
        println!("Missing from basic alphabet/digits: {missing:?}");
    } else {
        println!("Full basic alphabet + digits present.");
    }

    Ok(())
}
