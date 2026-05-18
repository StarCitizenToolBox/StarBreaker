//! `starbreaker-ui` — Static UI atom extractor and canvas composer.
//!
//! Extracts visual atoms (bitmaps, shapes, fonts, sprite first-frames) from SWF tag
//! streams using the `swf` crate as a read-only parser. No AVM1/AVM2 bytecode is
//! decoded or executed. Composites BuildingBlocks canvas records into deterministic
//! `RgbaImage` outputs for ship screen textures.
//!
//! # Modules
//! - [`canvas`]      — BuildingBlocks canvas record parser and widget tree resolver.
//! - [`compose`]     — Canvas-to-image compositor via `tiny-skia`.
//! - [`defaults`]    — Default "switched on" state values for game-state-bound widgets.
//! - [`error`]       — Unified [`UiError`] type.
//! - [`postprocess`] — Manufacturer post-process passes (tint, scanlines, vignette).
//! - [`style`]       — Manufacturer style (tint, CRT params) loader.
//! - [`swf_assets`]  — SWF static-atom extractor and [`SwfAssetLibrary`].

pub mod bb_layout;
pub mod bb_resolve;
pub mod bb_scene;
pub mod canvas;
pub mod compose;
pub mod defaults;
pub mod error;
pub mod pipeline;
pub mod postprocess;
pub mod style;
pub mod swf_assets;

pub use error::UiError;

// Re-export pipeline entry point.
pub use pipeline::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView,
    render_for_binding,
};

pub use canvas::{
    CanvasParser, CanvasRecord, CanvasView, CanvasWidgetTreeResolver, Operation, ResolvedCanvas,
    RgbaColor, SceneItem, Transform2D, Value, ViewComponent,
};

// Re-export defaults registry.
pub use defaults::DefaultValueRegistry;

// Re-export manufacturer style types.
pub use style::{CrtParams, ManufacturerStyle, StyleLoader};

// Re-export composer API.
pub use compose::{ComposeContext, ComposeTarget, encode_png, render_canvas, render_canvas_with_postprocess};

// Re-export post-process API.
pub use postprocess::{PostProcessOptions, PostProcessor};
