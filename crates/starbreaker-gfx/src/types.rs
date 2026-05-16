//! Public data types for GFx/SWF inspection, identities, and render planning.

use crate::error::{GfxError, GfxResult};

/// Parsed GFx/SWF file metadata and future decoded contents.
#[derive(Debug, Clone, PartialEq)]
pub struct GfxFile {
    /// Container header.
    pub header: GfxHeader,
    /// Movie-level timeline metadata.
    pub movie: Movie,
    /// Symbols exported or referenced by the movie.
    pub symbols: SymbolTable,
    /// External resources imported by the movie.
    pub imports: Vec<ImportedResource>,
    /// Top-level tags decoded from the movie stream.
    pub tags: Vec<SwfTag>,
    /// Runtime bytecode tags preserved without execution.
    pub bytecode: Vec<BytecodeTag>,
    /// Renderer-oriented initial display list extracted from the first frame.
    pub render_tree: RenderTree,
}

/// GFx/SWF container header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GfxHeader {
    /// File signature.
    pub signature: GfxSignature,
    /// Container version byte.
    pub version: u8,
    /// File length declared by the header.
    pub declared_len: u32,
    /// Number of bytes supplied to the parser.
    pub actual_len: usize,
    /// Number of bytes after decompressing the movie body, when applicable.
    pub decoded_len: usize,
}

/// Supported StarEngine GFx and standard SWF signatures.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum GfxSignature {
    /// StarEngine/Scaleform GFx container (`GFX`).
    Gfx,
    /// Uncompressed SWF container (`FWS`).
    Fws,
    /// Zlib-compressed SWF container (`CWS`).
    Cws,
    /// LZMA-compressed SWF container (`ZWS`).
    Zws,
}

impl GfxSignature {
    pub(crate) fn parse(bytes: &[u8]) -> GfxResult<Self> {
        match bytes {
            b"GFX" => Ok(Self::Gfx),
            b"FWS" => Ok(Self::Fws),
            b"CWS" => Ok(Self::Cws),
            b"ZWS" => Ok(Self::Zws),
            other => Err(GfxError::malformed(format!(
                "unsupported signature {:?}",
                String::from_utf8_lossy(other)
            ))),
        }
    }
}

/// Movie-level information decoded from the container.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Movie {
    /// Root timeline, when decoded.
    pub root_timeline: Option<Timeline>,
    /// Nominal frame count, when decoded.
    pub frame_count: Option<u16>,
    /// Nominal frame rate in frames per second, when decoded.
    pub frame_rate: Option<f32>,
    /// Stage width in twips, when decoded.
    pub stage_width_twips: Option<i32>,
    /// Stage height in twips, when decoded.
    pub stage_height_twips: Option<i32>,
}

/// Summary of a top-level SWF/GFx tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwfTag {
    /// Numeric SWF tag code.
    pub code: u16,
    /// Semantic class for the tag.
    pub kind: SwfTagKind,
    /// Byte length of the tag body.
    pub len: u32,
    /// Zero-based frame index active when the tag was encountered.
    pub frame: u32,
}

/// Semantic classes used by the default-state preparation pass.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum SwfTagKind {
    /// End of tag stream.
    End,
    /// Frame boundary.
    ShowFrame,
    /// Named frame label.
    FrameLabel,
    /// Shape definition.
    Shape,
    /// Bitmap definition.
    Bitmap,
    /// Text or font definition.
    Text,
    /// Sprite/movie-clip definition.
    Sprite,
    /// Display-list placement or modification.
    PlaceObject,
    /// Import/export/symbol table data.
    SymbolMetadata,
    /// Runtime ActionScript/ABC bytecode preserved but not executed.
    Bytecode,
    /// Metadata or attributes that influence interpretation.
    Metadata,
    /// Known but currently unsupported tag.
    Other,
}

/// Preserved bytecode tag metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytecodeTag {
    /// Numeric SWF tag code.
    pub code: u16,
    /// Zero-based frame index where the bytecode appears.
    pub frame: u32,
    /// Byte length of the tag body.
    pub len: u32,
}

/// Timeline labels and frame metadata.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Timeline {
    /// Timeline or symbol id, if present in source data.
    pub id: Option<u32>,
    /// Human-readable timeline labels.
    pub labels: Vec<FrameLabel>,
    /// Number of ShowFrame boundaries encountered.
    pub show_frames: u32,
}

/// Named frame label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameLabel {
    /// Zero-based frame index.
    pub frame: u32,
    /// Source-authored label.
    pub label: String,
}

/// Symbol table for exported and referenced movie symbols.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SymbolTable {
    /// Known symbols.
    pub symbols: Vec<Symbol>,
}

/// A symbol exported or referenced by a movie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Source symbol id.
    pub id: u32,
    /// Optional source symbol name.
    pub name: Option<String>,
    /// Source tag kind that introduced this symbol.
    pub kind: Option<SwfTagKind>,
}

/// External movie, image, or font asset imported by a GFx/SWF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedResource {
    /// Source-authored resource path or URI.
    pub source: String,
    /// Resource class.
    pub kind: ImportedResourceKind,
}

/// Known imported resource classes.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ImportedResourceKind {
    /// Nested GFx/SWF movie.
    Movie,
    /// Texture or bitmap input.
    Texture,
    /// Font resource.
    Font,
    /// Source resource whose class is not known yet.
    Unknown,
}

/// How a caller selects the frame/state to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameSelection {
    /// First displayable frame.
    FirstFrame,
    /// Explicit zero-based frame index.
    Frame(u32),
    /// Source-authored frame label.
    Label(String),
    /// Game/UI default state name from BuildingBlocks or DataCore.
    DefaultState(String),
}

/// A render tree to be consumed by a future raster backend.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RenderTree {
    /// Root render node.
    pub root: Option<RenderNode>,
    /// Display-list placements encountered before the first frame boundary.
    pub initial_placements: Vec<PlaceObject>,
}

/// Transform matrix for display-list placement.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    /// Scale X (usually 1.0 = no scaling)
    pub scale_x: f32,
    /// Scale Y (usually 1.0 = no scaling)
    pub scale_y: f32,
    /// Skew/rotation component 0
    pub skew0: f32,
    /// Skew/rotation component 1
    pub skew1: f32,
    /// Translation X in twips
    pub translate_x: i32,
    /// Translation Y in twips
    pub translate_y: i32,
}

/// Color transform for display-list placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorTransform {
    /// Red multiply (0-255, default 255)
    pub multiply_r: u8,
    /// Green multiply (0-255, default 255)
    pub multiply_g: u8,
    /// Blue multiply (0-255, default 255)
    pub multiply_b: u8,
    /// Alpha multiply (0-255, default 255)
    pub multiply_a: u8,
    /// Red offset (-255 to 255)
    pub add_r: i16,
    /// Green offset (-255 to 255)
    pub add_g: i16,
    /// Blue offset (-255 to 255)
    pub add_b: i16,
    /// Alpha offset (-255 to 255)
    pub add_a: i16,
}

/// Summary of a display-list placement.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceObject {
    /// Referenced character id, if present in the tag variant.
    pub character_id: Option<u16>,
    /// Display-list depth, when decoded.
    pub depth: Option<u16>,
    /// Transform matrix when present.
    pub matrix: Option<Matrix>,
    /// Color transform when present.
    pub color_transform: Option<ColorTransform>,
    /// Clipping depth when present.
    pub clip_depth: Option<u16>,
    /// Frame index where the placement appears.
    pub frame: u32,
}

/// Node in a renderer-oriented intermediate scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderNode {
    /// Node class.
    pub kind: RenderNodeKind,
    /// Child nodes in painter order.
    pub children: Vec<RenderNode>,
}

/// Renderer-oriented node classes.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum RenderNodeKind {
    /// Vector shape.
    Shape,
    /// Bitmap or texture image.
    Bitmap,
    /// Text run.
    Text,
    /// Empty transform/group node.
    Group,
}

/// Structural identity used to deduplicate generated UI outputs.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct OutputIdentity {
    /// Ordered source-derived identity components.
    pub components: Vec<String>,
}

impl OutputIdentity {
    /// Create an empty identity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a source-derived identity component.
    pub fn with_component(mut self, component: impl Into<String>) -> Self {
        self.components.push(component.into());
        self
    }
}
