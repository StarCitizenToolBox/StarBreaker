//! Library entry point for inspecting StarEngine GFx/SWF UI assets.
//!
//! The crate exposes read-only GFx/SWF metadata, source-derived default still
//! generation, and dependency resolution for related UI assets.

pub mod error;
pub mod inspect;
pub mod parser;
pub mod raster;
pub mod render;
pub mod resolver;
pub mod swf_interpreter;
pub mod types;

pub use error::{GfxError, GfxResult};
pub use inspect::{GfxMetadata, dump_metadata};
pub use parser::parse_gfx;
pub use raster::RasterContext;
pub use render::{
    UiLightCue, UiStillBinding, UiStillSpec, render_default_still_png,
    render_display_list_still_png, render_gfx_still_png, select_default_still,
};
pub use resolver::{AssetResolver, ResolvedAsset, ResolvedAssetKind};
pub use swf_interpreter::render_swf_to_png;
pub use types::{
    BytecodeTag, ColorTransform, FrameLabel, FrameSelection, GfxFile, GfxHeader, GfxSignature,
    ImportedResource, ImportedResourceKind, Matrix, Movie, OutputIdentity, PlaceObject,
    RenderNode, RenderNodeKind, RenderTree, SwfTag, SwfTagKind, Symbol, SymbolTable, Timeline,
};
