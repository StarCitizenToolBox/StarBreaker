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

pub mod bb_assets;
pub mod bb_atlas;
pub mod bb_bindings;
pub mod bb_brand_apply;
pub mod bb_brand_style;
pub mod bb_layout;
pub mod bb_loc;
pub mod bb_loc_p4k;
pub mod bb_resolve;
pub mod bb_scene;
pub mod bb_state_filter;
pub mod bb_svg;
pub mod canvas;
pub mod compose;
pub mod defaults;
pub mod error;
pub mod hybrid_compose;
pub mod ir_compose;
pub mod pipeline;
pub mod postprocess;
pub mod style;
pub mod swf_assets;
pub mod swf_render;
pub mod text;
pub mod ui_ir;
pub mod ui_regression_manifest;
pub mod ui_snapshot;

pub use error::UiError;

// Re-export pipeline entry point.
pub use pipeline::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView,
    compile_ir_for_binding, render_for_binding, render_for_binding_ir,
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

pub use ir_compose::render_ui_ir_document;

// Re-export post-process API.
pub use postprocess::{PostProcessOptions, PostProcessor};

// Re-export canonical UI IR schema/compiler.
pub use ui_ir::{
    UI_IR_SCHEMA_VERSION, UiIrDocument, UiIrNode, UiIrRect, UiIrTextPayload, UiIrValue,
    UiIrTextStyle, UiRendererHint, compile_ui_ir_from_scene, stable_hash_ui_ir,
    validate_ui_ir_document,
};

pub use ui_snapshot::{
    UI_SNAPSHOT_SCHEMA_VERSION, UiScreenSnapshot, UiSnapshotComparison, UiSnapshotElement,
    UiSnapshotElementCategory, UiSnapshotTolerance, UiRendererMetadataElement,
    UiRendererMetadataSnapshot, compare_renderer_metadata_snapshots, compare_snapshots,
    renderer_metadata_snapshot_from_ui_ir, snapshot_from_ui_ir,
};

pub use ui_regression_manifest::{
    UI_REGRESSION_MANIFEST_SCHEMA_VERSION, UiRegressionCategory, UiRegressionManifest,
    UiRegressionRoi, UiRegressionTarget, UiRegressionTier,
};
