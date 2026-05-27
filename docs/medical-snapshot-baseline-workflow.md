# Medical Snapshot Baseline Workflow

Phase 7 baseline policy for `starbreaker-ui` medical snapshot regression checks.

## Guardrails

- Normal test runs must never auto-write or auto-bless baselines.
- Snapshot baseline changes are explicit, manual, and code-reviewed.
- Baseline updates are allowed only in dedicated commits that explain why drift is expected.

## Current Baseline Assets

- IR fixtures:
  - `crates/starbreaker-ui/tests/fixtures/medical_ir/medical1-screen_16x9_a-ir.json`
  - `crates/starbreaker-ui/tests/fixtures/medical_ir/medical2-mesh_end_screen_plane-ir.json`
- Regression tests:
  - `crates/starbreaker-ui/tests/medical_snapshot_regression.rs`

## Update Procedure

1. Confirm the drift is intentional and source-driven (not a hardcoded workaround).
2. Re-run focused validation before any baseline updates:
   - `cargo test -p starbreaker-ui ui_snapshot --lib`
   - `cargo test -p starbreaker-ui --test medical_snapshot_regression`
3. If source IR has legitimately changed, refresh fixture IR files from the approved capture flow.
4. Keep each baseline update in an explicit commit with a message that includes:
   - why drift is expected,
   - what source rule changed,
   - what tests were run.
5. Re-run `cargo test -p starbreaker-ui` before merge.

## Review Checklist

- No screen/name-specific production hardcoding was introduced.
- Drift diagnostics are field-granular and actionable.
- Medical snapshot tests fail for controlled drift and pass for expected state.
