//! Extract every embedded font from an SWF as a `.ttf` file.
//!
//! Usage: `cargo run -p starbreaker-swf --example extract_ttf -- \
//!   <input.swf> <output-dir> [--only-id <N>]`

use std::path::PathBuf;
use swf::{Tag, decompress_swf, parse_swf};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let input: PathBuf = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: extract_ttf <swf> <output-dir> [--only-id N]"))?
        .into();
    let outdir: PathBuf = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing output-dir"))?
        .into();
    let mut only_id: Option<u16> = None;
    while let Some(flag) = args.next() {
        if flag == "--only-id" {
            only_id = Some(args.next().expect("--only-id needs a value").parse()?);
        } else {
            anyhow::bail!("unknown flag: {flag}");
        }
    }

    std::fs::create_dir_all(&outdir)?;

    let bytes = std::fs::read(&input)?;
    let buf = decompress_swf(&bytes[..])?;
    let swf_data = parse_swf(&buf)?;

    let mut extracted = 0usize;
    for tag in &swf_data.tags {
        if let Tag::DefineFont2(font) = tag {
            if let Some(id) = only_id
                && font.id != id
            {
                continue;
            }
            let name = font.name.to_string_lossy(swf::UTF_8);
            let suffix = if font.flags.contains(swf::FontFlag::IS_BOLD)
                && font.flags.contains(swf::FontFlag::IS_ITALIC)
            {
                "-BoldItalic"
            } else if font.flags.contains(swf::FontFlag::IS_BOLD) {
                "-Bold"
            } else if font.flags.contains(swf::FontFlag::IS_ITALIC) {
                "-Italic"
            } else {
                "-Regular"
            };
            let filename: String = format!("{name}{suffix}-id{}.ttf", font.id)
                .chars()
                .map(|c| if c == ' ' { '_' } else { c })
                .collect();
            let out_path = outdir.join(&filename);
            let ttf = starbreaker_swf::font_to_ttf(font)?;
            std::fs::write(&out_path, &ttf)?;
            println!(
                "id={:>3} {:<24} -> {} ({} bytes, {} glyphs)",
                font.id,
                name,
                filename,
                ttf.len(),
                font.glyphs.len()
            );
            extracted += 1;
        }
    }
    println!("\nExtracted {extracted} font(s) to {}", outdir.display());
    Ok(())
}
