# UI Architecture and Troubleshooting Runbook

## Architecture Summary

The UI pipeline is split into four stages:

1. Source resolution
- Resolve BuildingBlocks canvases, styles, bindings, and localization.
- Files: `bb_resolve.rs`, `bb_state_filter.rs`, `bb_bindings.rs`, `bb_brand_apply.rs`.

2. Canonical IR compilation
- Compile deterministic `UiIrDocument` output with fidelity fields and provenance.
- File: `ui_ir.rs`.

3. Renderer consumption
- Render from IR only (no source-data probing in renderer).
- Files: `ir_compose.rs`, `hybrid_compose.rs`, `compose.rs` compatibility wrapper.

4. Regression/certification
- Structural snapshot extraction and comparison for representative families.
- File: `ui_snapshot.rs` and example `phase5_certification_dashboard.rs`.

## Standard Validation Commands

- Full UI crate checks:
  - `cargo test -p starbreaker-ui`
- Hardcoding guard:
  - `bash .github/scripts/check_ui_hardcoding.sh`
- Family certification drift check:
  - `cargo run -p starbreaker-ui --example phase5_certification_dashboard`

## Troubleshooting Flow

1. Confirm source provenance first
- Check selected style/SWF source and unresolved references in diagnostics output.

2. Reproduce with deterministic fixture path
- Use representative fixture canvases under `crates/starbreaker-ui/tests/fixtures/canvas/`.

3. Compare structural snapshots
- Run certification dashboard and inspect failures in:
  - `docs/StarBreaker/ui-rework-artifacts/phase-5/certification-dashboard.md`
  - `docs/StarBreaker/ui-rework-artifacts/phase-5/certification-results.json`

4. Classify fault domain
- Source resolution mismatch: investigate `bb_resolve`/bindings/style application.
- IR mismatch: investigate `ui_ir` compilation fields.
- Rendering mismatch: investigate `ir_compose` use of existing IR fields.

5. Add a regression test with the fix
- Add/update a focused unit/integration test in the touched subsystem.
- Re-run `cargo test -p starbreaker-ui`.

## Incident Checklist

- Is this a source-data issue or renderer misuse?
- Is the behavior represented in IR (`UiIrDocument`) correctly?
- Did certification snapshots drift for previously certified screens?
- Is there an undocumented fallback involved?
- Did the fix add a focused regression test?
