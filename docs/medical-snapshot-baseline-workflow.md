# Medical Snapshot Baseline Workflow

Phase 7 baseline policy for `starbreaker-ui` medical snapshot regression checks.

Medical1 and Medical2 are treated as gold-standard outputs. Deviations are presumed broken until proven to be intentional, source-backed contract changes with explicit user approval.

## Guardrails

- Normal test runs must never auto-write or auto-bless baselines.
- Snapshot baseline changes are explicit, manual, and code-reviewed.
- Baseline updates are allowed only in dedicated commits that explain why drift is expected.
- If a medical render or live IR shows obvious breakage such as visible placeholder text, stop. That is a product regression signal, not baseline drift.
- Do not update medical baselines unless the user explicitly approves the baseline refresh after root-cause investigation.

## Current Baseline Assets

- IR fixtures:
  - `crates/starbreaker-ui/tests/fixtures/medical_ir/medical1-screen_16x9_a-ir.json`
  - `crates/starbreaker-ui/tests/fixtures/medical_ir/medical2-mesh_end_screen_plane-ir.json`
- Regression tests:
  - `crates/starbreaker-ui/tests/medical_snapshot_regression.rs`

## Update Procedure

1. Confirm the drift is intentional and source-driven (not a hardcoded workaround).
2. Confirm the failure is not an obvious regression signal such as visible placeholder text, unresolved placeholder keys, or missing major UI elements.
2. Re-run focused validation before any baseline updates:
   - `cargo test -p starbreaker-ui ui_snapshot --lib`
   - `cargo test -p starbreaker-ui --test medical_snapshot_regression`
3. If the failure is a regression signal, investigate and fix the product path first; do not touch the baselines.
4. If source IR has legitimately changed, refresh fixture IR files from the approved capture flow.
5. Keep each baseline update in an explicit commit with a message that includes:
   - why drift is expected,
   - what source rule changed,
   - what tests were run.
6. Re-run `cargo test -p starbreaker-ui` before merge.

## Review Checklist

- No screen/name-specific production hardcoding was introduced.
- Drift diagnostics are field-granular and actionable.
- Medical snapshot tests fail for controlled drift and pass for expected state.
- Baseline refresh approval is explicit and recorded; placeholder-text regressions were ruled out before any fixture update.
