//! Star Citizen SWF / GFX font extraction.
//!
//! Star Citizen ships its UI font library as Scaleform-flavored SWF files
//! (`Data/UI/fonts/Shared/fonts_en.swf`). The glyph outlines live inside
//! `DefineFont3` tags as Flash shape records; CIG does not ship the actual
//! `.ttf`/`.otf` source nor the `.slug` files referenced from DataCore. This
//! crate parses those SWFs and re-emits each contained font as a standalone
//! TrueType file, so downstream tools (Blender exporter, BB canvas renderer,
//! documentation generators) can use the real CIG-shipped glyphs.
//!
//! Day 1 (current): `analyze_fonts` enumerates every `DefineFont3` plus its
//! companion `DefineFontInfo*` / `DefineFontName` tag and returns a
//! `FontInfo` struct per font. Later days will add shape→contour conversion
//! and TTF writing.

mod error;
mod font_info;
pub mod shape;
mod ttf;

pub use error::SwfError;
pub use font_info::{FontEntry, analyze_fonts};
pub use ttf::font_to_ttf;

/// Re-export `swf::Font` for downstream crates that want to work with the
/// parsed font directly before TTF conversion.
pub use swf::Font;
