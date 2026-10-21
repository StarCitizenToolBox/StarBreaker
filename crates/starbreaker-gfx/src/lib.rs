//! Library entry point for inspecting StarEngine GFx/SWF UI assets.
//!
//! The crate exposes read-only GFx/SWF metadata, source-derived default still
//! generation, and dependency resolution for related UI assets.

pub mod error;
pub mod inspect;
pub mod mesh_holo;
pub mod parser;
pub mod radar_plane;
pub mod raster;
pub mod render;
pub mod resolver;
pub mod types;

pub use error::{GfxError, GfxResult};
pub use inspect::{GfxMetadata, dump_metadata};
pub use mesh_holo::{HologramParams, render_vehicle_hologram};
pub use parser::parse_gfx;
pub use radar_plane::{
    HeadingRingParams, RadarPlaneParams, RadarSpoke, cardinal_marker_cell, project_radar_disc,
    sweep_wedge_geometry,
};
pub use raster::RasterContext;
pub use render::{
    UiLightCue, UiStillBinding, UiStillSpec,
    render_gfx_still_png, select_default_still,
};
pub use resolver::{AssetResolver, ResolvedAsset, ResolvedAssetKind};
pub use types::{
    BytecodeTag, ColorTransform, FrameLabel, FrameSelection, GfxFile, GfxHeader, GfxSignature,
    ImportedResource, ImportedResourceKind, Matrix, Movie, OutputIdentity, PlaceObject,
    RenderNode, RenderNodeKind, RenderTree, SwfTag, SwfTagKind, Symbol, SymbolTable, Timeline,
};
