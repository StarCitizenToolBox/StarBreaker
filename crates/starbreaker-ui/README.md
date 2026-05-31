# starbreaker-ui

`starbreaker-ui` compiles Star Citizen UI data into deterministic, testable outputs.

## Overview

This crate provides a data-driven pipeline that:
- Parses and resolves BuildingBlocks canvas graphs.
- Resolves bindings, localization, default state, and brand/style modifiers.
- Computes layout and compiles canonical UI IR.
- Renders static outputs from canonical IR.
- Produces snapshot/metadata artifacts for regression validation.

Core constraints:
- No production hard-coded ship/canvas/asset name heuristics.
- Deterministic output for equivalent inputs.
- Structural, modular organization with a 500-line guardrail for `src/**/*.rs` and `src/**/*.part` files.

Engine split-part directories (`engine.inc` include targets):
- `src/bb_layout/engine_parts/`
- `src/bb_resolve/engine_parts/`
- `src/ir_compose/engine_parts/`
- `src/ui_ir/engine_parts/`

## Key Entry Points

- `src/lib.rs`: crate exports and top-level module map.
- `src/pipeline/mod.rs`: binding-driven orchestration from sources to IR/image.
- `src/ui_ir/mod.rs`: canonical IR schema + compiler.
- `src/ir_compose/mod.rs`: canonical IR renderer.
- `src/ui_snapshot/mod.rs`: structural + metadata snapshot comparison.

## Quick Navigation

Render path (BB data -> image):
1. `pipeline::compile_ir_for_binding`
2. `ui_ir::compile_ui_ir_from_scene`
3. `ir_compose::render_ui_ir_document`
4. Optional post-process in `postprocess`

Defaults/state path:
1. `defaults::DefaultValueRegistry`
2. `bb_bindings` resolver lookups
3. `bb_state_filter` static visibility filtering
4. `bb_brand_apply` style/tag-driven modifiers

Regression path:
1. `ui_snapshot::snapshot_from_ui_ir`
2. `ui_snapshot::compare_snapshots`
3. `ui_regression_manifest::compare_manifest_targets_with_loader`
4. integration tests under `tests/manifest_*`

## Regression Policy

- Visual and snapshot regression failures are product regressions until proven otherwise.
- First response must be root-cause investigation and code/data-flow fixes.
- Do not change baselines, thresholds, or regression tests as an initial workaround.
- Baseline/test updates are allowed only after root-cause fix is implemented and validated.

## Complete Rust File List (1-line each)

### Source modules (`src/`)

- `src/lib.rs` - Crate root exports and module index.
- `src/error.rs` - Shared crate error types.
- `src/bb_assets.rs` - UI asset reference normalization and overlay classification.
- `src/bb_brand_style.rs` - Brand-style source selection helpers.
- `src/bb_loc.rs` - Generic localization resolver interface and logic.
- `src/bb_loc_p4k.rs` - P4K-backed localization table loader.
- `src/bb_svg.rs` - SVG rasterization, tinting, and nine-slice helpers.
- `src/hybrid_compose.rs` - Hybrid renderer path coordination.

### BB atlas (`src/bb_atlas/`)

- `src/bb_atlas/mod.rs` - Atlas API surface and orchestration.
- `src/bb_atlas/decode.rs` - Decode/loading helpers for atlas assets.
- `src/bb_atlas/path.rs` - Atlas path normalization and lookup rules.
- `src/bb_atlas/tests.rs` - Atlas unit tests.

### BB bindings (`src/bb_bindings/`)

- `src/bb_bindings/mod.rs` - Binding resolver public API.
- `src/bb_bindings/build.rs` - Resolver graph construction.
- `src/bb_bindings/eval.rs` - Numeric/boolean/value evaluation logic.
- `src/bb_bindings/eval_string.rs` - String evaluation and coercion helpers.
- `src/bb_bindings/resolve_text.rs` - Text-field and localization text resolution.
- `src/bb_bindings/util.rs` - Binding utility helpers.
- `src/bb_bindings/tests.rs` - Binding resolver unit tests.

### BB brand apply (`src/bb_brand_apply/`)

- `src/bb_brand_apply/mod.rs` - Brand modifier application entrypoints.
- `src/bb_brand_apply/modifiers.rs` - Field-level modifier application.
- `src/bb_brand_apply/colors.rs` - Color token/value interpretation helpers.
- `src/bb_brand_apply/tests_support.rs` - Shared unit-test scaffolding.
- `src/bb_brand_apply/tests_conditions.rs` - Condition matching tests.
- `src/bb_brand_apply/tests_modifiers.rs` - Modifier behavior tests.
- `src/bb_brand_apply/tests_colors.rs` - Color conversion and token tests.

### BB layout (`src/bb_layout/`)

- `src/bb_layout/mod.rs` - Layout module wrapper and exported layout API.

### BB resolve (`src/bb_resolve/`)

- `src/bb_resolve/mod.rs` - Canvas graph resolver wrapper and exports.

### BB scene (`src/bb_scene/`)

- `src/bb_scene/mod.rs` - Scene parse API surface.
- `src/bb_scene/types.rs` - Parsed scene and node type definitions.
- `src/bb_scene/parse.rs` - Canvas-to-scene parsing logic.
- `src/bb_scene/fields.rs` - Field extraction/parsing helpers.
- `src/bb_scene/tests.rs` - Scene parser tests.

### BB state filter (`src/bb_state_filter/`)

- `src/bb_state_filter/mod.rs` - State-filter API entrypoints.
- `src/bb_state_filter/eval.rs` - Condition expression evaluation.
- `src/bb_state_filter/idle_defaults.rs` - Idle/default state derivation.
- `src/bb_state_filter/tests_a.rs` - Primary state-filter test set.
- `src/bb_state_filter/tests_b.rs` - Additional state-filter test set.

### Canvas (`src/canvas/`)

- `src/canvas/mod.rs` - Canvas parse/resolve API re-exports.
- `src/canvas/types.rs` - Canvas record/type definitions.
- `src/canvas/parser.rs` - Raw record parser.
- `src/canvas/resolver.rs` - Sub-canvas resolver and tree assembly.
- `src/canvas/tests.rs` - Canvas parser/resolver tests.

### Compose (`src/compose/`)

- `src/compose/mod.rs` - Compose API and orchestration.
- `src/compose/draw_node.rs` - Per-node dispatch logic.
- `src/compose/draw_primitives.rs` - Primitive shape rendering helpers.
- `src/compose/text_draw.rs` - Text drawing bridge calls.
- `src/compose/raw_assets.rs` - Raw image/SWF asset draw helpers.
- `src/compose/blit.rs` - Blit/compositing utilities.
- `src/compose/tests.rs` - Compose module tests.

### Defaults (`src/defaults/`)

- `src/defaults/mod.rs` - Default value registry API.
- `src/defaults/registry.rs` - Registry implementation and ingest logic.
- `src/defaults/tests.rs` - Defaults tests.

### IR compose (`src/ir_compose/`)

- `src/ir_compose/mod.rs` - Canonical UI IR renderer wrapper.

### Pipeline (`src/pipeline/`)

- `src/pipeline/mod.rs` - Pipeline orchestration entrypoints.
- `src/pipeline/asset_manifest.rs` - Asset-manifest extraction and reporting.
- `src/pipeline/style_selection.rs` - Style-source selection logic.
- `src/pipeline/tests.rs` - Pipeline tests.

### Pipeline SWF selection (`src/pipeline/swf_selection/`)

- `src/pipeline/swf_selection/mod.rs` - SWF candidate selection API.
- `src/pipeline/swf_selection/candidates.rs` - Candidate list generation.
- `src/pipeline/swf_selection/flash_paths.rs` - Flash path normalization/ranking.
- `src/pipeline/swf_selection/loader.rs` - SWF loader integration helpers.
- `src/pipeline/swf_selection/tests.rs` - SWF selection tests.

### Postprocess (`src/postprocess/`)

- `src/postprocess/mod.rs` - Postprocess API and option wiring.
- `src/postprocess/passes.rs` - Tint/scanline/vignette passes.
- `src/postprocess/tests.rs` - Postprocess tests.

### Style (`src/style/`)

- `src/style/mod.rs` - Style module API surface.
- `src/style/types.rs` - Style data structures.
- `src/style/parse.rs` - Raw style record parsing.
- `src/style/loader.rs` - Style loading/fallback logic.
- `src/style/tests.rs` - Style module tests.

### SWF assets (`src/swf_assets/`)

- `src/swf_assets/mod.rs` - SWF asset extraction API.
- `src/swf_assets/types.rs` - SWF asset/library type definitions.
- `src/swf_assets/decode.rs` - SWF decode helpers.
- `src/swf_assets/extract.rs` - Shape/image/font extraction logic.
- `src/swf_assets/stage.rs` - Stage traversal helpers.
- `src/swf_assets/library.rs` - Library assembly/indexing.
- `src/swf_assets/tests.rs` - SWF asset tests.

### SWF render (`src/swf_render/`)

- `src/swf_render/mod.rs` - SWF renderer API.
- `src/swf_render/stage.rs` - Stage render traversal.
- `src/swf_render/shape.rs` - Shape rasterization logic.
- `src/swf_render/rgba.rs` - RGBA conversion helpers.
- `src/swf_render/tests.rs` - SWF render tests.

### Text (`src/text/`)

- `src/text/mod.rs` - Text API and high-level draw/measure entrypoints.
- `src/text/ttf_draw.rs` - TTF text draw/measure implementation.
- `src/text/swf_draw.rs` - SWF text rendering path.
- `src/text/tests.rs` - Text subsystem tests.

### UI IR (`src/ui_ir/`)

- `src/ui_ir/mod.rs` - Canonical UI IR schema/compiler wrapper.

### UI regression manifest (`src/ui_regression_manifest/`)

- `src/ui_regression_manifest/mod.rs` - Manifest API exports.
- `src/ui_regression_manifest/types.rs` - Manifest schema/type definitions.
- `src/ui_regression_manifest/runner.rs` - Manifest comparison runner.
- `src/ui_regression_manifest/tests.rs` - Manifest tests.

### UI snapshot (`src/ui_snapshot/`)

- `src/ui_snapshot/mod.rs` - Snapshot module exports.
- `src/ui_snapshot/tests_support.rs` - Shared unit-test fixture builders.

## Integration Tests (`tests/`)

- `tests/line_count_guard.rs` - Guardrail for `src/**/*.rs` and `src/**/*.part` (max 500 lines).
- `tests/manifest_live_ir_guard.rs` - Live manifest guard tests.
- `tests/manifest_snapshot_regression.rs` - Snapshot drift regression tests.
- `tests/manifest_visual_regression.rs` - Visual regression target tests.
- `tests/pipeline_ir.rs` - Pipeline-to-IR integration tests.
- `tests/regression_hashes.rs` - Hash/regression stability checks.
- `tests/source_hardcoding_guards.rs` - Source-level hardcoding guard tests.
- `tests/ui_ir_representative.rs` - Representative fixture IR validations.
- `tests/visual_diff.rs` - Visual diff utility behavior tests.

## Example Binaries (`examples/`)

- `examples/as2_spike.rs` - SWF/AS2 exploratory decode probe.
- `examples/bb_dump_scene.rs` - Dump parsed BB scene structure.
- `examples/bb_layout_wireframe.rs` - Render layout wireframe preview.
- `examples/compare_text_heights.rs` - Compare text metric paths.
- `examples/dump_phase3_ui_target_traces.rs` - Emit phase trace diagnostics.
- `examples/dump_representative_ir.rs` - Dump IR for representative fixtures.
- `examples/dump_ui_ir_targets.rs` - Dump IR for selected targets.
- `examples/phase5_certification_dashboard.rs` - Phase 5 cert dashboard helper.
- `examples/query_ui_layout.rs` - Query node layout/debug output.
- `examples/render_phase2_comparison.rs` - Render phase comparison output.
- `examples/render_ui_targets_current.rs` - Render current target set.
- `examples/swf_inventory.rs` - SWF inventory listing tool.
- `examples/swf_place_probe.rs` - SWF placement probing utility.
- `examples/swf_text_probe.rs` - SWF text probing utility.
- `examples/trace_text_style_context.rs` - Trace resolved text-style context.

## Notes

- Split-wrapper modules (`bb_layout`, `bb_resolve`, `ir_compose`, `ui_ir`) include preserved implementation files (`engine.inc`) to keep wrapper entries compact while preserving behavior.
- Keep this README aligned with `src/` and `tests/` whenever modules move or are renamed.
