//! `starbreaker swf …` subcommands.
//!
//! Currently exposes one operation: `font-extract`, which walks every
//! `DefineFont2`/`DefineFont3` tag in an SWF (or Scaleform-flavored SWF)
//! and emits a standalone `.ttf` per font into an output directory.
//!
//! The input path may be either a filesystem path to a `.swf` / `.gfx` file
//! or a P4k-internal path (resolved via `--p4k`). See
//! `docs/swf-font-extraction.md` for the discovery + verification work
//! that informed this command.

use std::path::PathBuf;

use clap::Subcommand;
use swf::{Tag, decompress_swf, parse_swf};

use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum SwfCommand {
    /// List embedded fonts in an SWF and print their basic metadata.
    FontList {
        /// Filesystem path to a `.swf` / `.gfx` file (or P4k-internal path with `--p4k`).
        input: PathBuf,
        /// Path to Data.p4k; if set, `input` is resolved within the archive.
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Extract every embedded font from an SWF as a standalone `.ttf`.
    FontExtract {
        /// Filesystem path to a `.swf` / `.gfx` file (or P4k-internal path with `--p4k`).
        input: PathBuf,
        /// Output directory; will be created if missing.
        #[arg(short, long)]
        output: PathBuf,
        /// Path to Data.p4k; if set, `input` is resolved within the archive.
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Extract only the font with this SWF character ID. Repeatable.
        #[arg(long)]
        only_id: Vec<u16>,
    },
}

impl SwfCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::FontList { input, p4k } => font_list(input, p4k),
            Self::FontExtract { input, output, p4k, only_id } => {
                font_extract(input, output, p4k, only_id)
            }
        }
    }
}

/// Load `.swf` bytes from either a filesystem path or a P4k-internal path.
fn load_swf_bytes(input: &std::path::Path, p4k: Option<&std::path::Path>) -> Result<Vec<u8>> {
    if input.is_file() {
        return std::fs::read(input).map_err(|e| CliError::IoPath {
            source: e,
            path: input.display().to_string(),
        });
    }
    if let Some(p4k_path) = p4k {
        let archive = starbreaker_p4k::MappedP4k::open(p4k_path)?;
        let normalized = input
            .to_string_lossy()
            .replace('/', "\\");
        let with_prefix = if normalized.to_lowercase().starts_with("data\\") {
            normalized
        } else {
            format!("Data\\{normalized}")
        };
        return archive.read_file(&with_prefix).map_err(Into::into);
    }
    Err(CliError::NotFound(format!(
        "input '{}' not found on disk and no --p4k provided",
        input.display()
    )))
}

fn font_list(input: PathBuf, p4k: Option<PathBuf>) -> Result<()> {
    let bytes = load_swf_bytes(&input, p4k.as_deref())?;
    let fonts = starbreaker_swf::analyze_fonts(&bytes)?;

    println!("{} fonts in {}", fonts.len(), input.display());
    println!(
        "{:>4}  {:<8} {:<32} {:>6} {:<8} {:<7} {:<14} {}",
        "id", "ver", "name", "glyphs", "layout", "weight", "codepoints", "display_name"
    );
    println!("{}", "-".repeat(115));
    for f in &fonts {
        let weight = match (f.is_bold, f.is_italic) {
            (true, true) => "Bold-It",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        };
        let layout = if f.has_layout { "yes" } else { "no" };
        let code = match f.codepoint_range {
            Some((mn, mx)) => format!("{mn:#06x}-{mx:#06x}"),
            None => "(none)".to_string(),
        };
        let display = f.display_name.as_deref().unwrap_or("");
        println!(
            "{:>4}  v{:<7} {:<32} {:>6} {:<8} {:<7} {:<14} {}",
            f.id, f.version, f.name, f.glyph_count, layout, weight, code, display
        );
    }
    Ok(())
}

fn font_extract(
    input: PathBuf,
    output: PathBuf,
    p4k: Option<PathBuf>,
    only_id: Vec<u16>,
) -> Result<()> {
    let bytes = load_swf_bytes(&input, p4k.as_deref())?;
    let buf = decompress_swf(&bytes[..])
        .map_err(|e| starbreaker_swf::SwfError::Decompress(e.to_string()))?;
    let swf_data = parse_swf(&buf)
        .map_err(|e| starbreaker_swf::SwfError::Tags(e.to_string()))?;

    std::fs::create_dir_all(&output).map_err(|e| CliError::IoPath {
        source: e,
        path: output.display().to_string(),
    })?;

    let id_filter: Option<std::collections::HashSet<u16>> = if only_id.is_empty() {
        None
    } else {
        Some(only_id.into_iter().collect())
    };

    let mut extracted = 0usize;
    for tag in &swf_data.tags {
        if let Tag::DefineFont2(font) = tag {
            if let Some(ref ids) = id_filter
                && !ids.contains(&font.id)
            {
                continue;
            }
            let name = font.name.to_string_lossy(swf::UTF_8);
            let suffix = match (
                font.flags.contains(swf::FontFlag::IS_BOLD),
                font.flags.contains(swf::FontFlag::IS_ITALIC),
            ) {
                (true, true) => "-BoldItalic",
                (true, false) => "-Bold",
                (false, true) => "-Italic",
                (false, false) => "-Regular",
            };
            let filename: String = format!("{name}{suffix}-id{}.ttf", font.id)
                .chars()
                .map(|c| if c == ' ' { '_' } else { c })
                .collect();
            let out_path = output.join(&filename);
            let ttf = starbreaker_swf::font_to_ttf(font)?;
            std::fs::write(&out_path, &ttf).map_err(|e| CliError::IoPath {
                source: e,
                path: out_path.display().to_string(),
            })?;
            println!(
                "id={:>3} {:<26} -> {} ({} bytes, {} glyphs)",
                font.id,
                name,
                filename,
                ttf.len(),
                font.glyphs.len()
            );
            extracted += 1;
        }
    }

    println!("\nExtracted {extracted} font(s) to {}", output.display());
    Ok(())
}
