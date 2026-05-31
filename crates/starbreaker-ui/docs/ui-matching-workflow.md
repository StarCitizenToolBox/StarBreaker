# UI Matching Workflow

This document is the reference workflow for UI-matching changes in `starbreaker-ui`.

## Non-Negotiable Rules

1. Always run the `starbreaker-ui` test suite after making UI-related changes.
2. Treat platinum/gold image test failures as regressions in source behavior.
3. Do not "fix" regressions by changing tests or baselines first.
4. Fix root causes structurally; do not use hard-coding or heuristics.
5. Keep Rust source files under 500 lines (enforced by tests).
6. Only add, remove, or update regression targets/baselines when the user explicitly instructs it.

## Regression Policy

- The visual regression tests are the guardrail for platinum/gold standard outputs.
- If any platinum/gold image test fails, assume the code regressed.
- Fix production code or source-data handling so the expected images remain unchanged.
- Never bypass or weaken these tests to make them pass.
- Do not add per-image hand-rolled regression tests when a manifest-driven generic path can express the same invariant.

## Source Of Truth

- The single source of truth is game data.
- Use StarBreaker MCP tools to inspect DataCore records, P4k assets, and related source structures.
- Use Blender MCP tools with the connected Blender instance for scene/material/transform/render validation.

## Validation Loop

1. Make a focused code change.
2. Run the required full-scope UI regression path:

```bash
cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture
cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture
cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture
./scripts/validate_ui_snapshot_freeze.sh
./scripts/validate_ui_regression_artifacts.sh --quick
```

3. Then run `cargo test -p starbreaker-ui --tests` when broad crate validation is needed.
4. Treat `cargo test -p starbreaker-ui --test manifest_visual_regression -- --ignored --nocapture` and `./scripts/validate_ui_regression_artifacts.sh --full` as optional artifact-hash diagnostics, not required gating.
5. If tests fail, fix the root cause in source logic/data handling.
6. Re-run tests until all pass.
7. Repeat this loop frequently during implementation.

## Repo-Only CI-Safe Validation

- Hosted CI without local game data should run `./scripts/validate_ui_regression_repo_only.sh`.
- This repo-only path validates manifest/snapshot regression behavior and IR-freeze parity from checked-in data only.
- The live guard and local visual artifact checks remain required for local developer validation on machines with game data.

## If You Are Unsure About Visual Output

- Generate UI regression artifacts:

```bash
./scripts/generate_ui_regression_artifacts.sh
```

- Then ask the user to verify via the question tool.

## Target Onboarding Happy Path

1. Add or replace the manifest target with `./scripts/add_ui_regression_target.sh`.
2. Generate local visual artifacts with `./scripts/generate_ui_regression_artifacts.sh` when visual inspection is needed.
3. Generate or refresh the IR-only baseline with `./scripts/freeze_ui_snapshot_ir.sh --approver <name> --reason <why>`.
4. Run `./scripts/validate_ui_snapshot_freeze.sh`.
5. Run `./scripts/validate_ui_regression_artifacts.sh --quick`.
6. Run the required full-scope UI regression path.
7. Ask for explicit approval before committing any manifest, tier, or freeze update.

## Tier Change Checklist

- Confirm the target belongs in `gold` or `platinum` based on expected stability and review standard.
- Do not change tier to silence unexplained drift.
- Re-run `./scripts/freeze_ui_snapshot_ir.sh --approver <name> --reason <why>` after an approved tier change.
- Run `./scripts/validate_ui_snapshot_freeze.sh` and `./scripts/validate_ui_regression_artifacts.sh --quick` after the tier change.
- Re-run the required full-scope UI regression path.
- Record the reason for the tier change in the review/commit context.

## Troubleshooting Matrix

- `font_size drift` or other semantic field drift: treat this as the primary failure. Fix source behavior first; do not refresh baselines first.
- Manifest/freeze id mismatch: update the manifest or regenerate the IR freeze so ids are exactly 1:1.
- Missing `baseline_snapshot` payload: regenerate the IR freeze with `./scripts/freeze_ui_snapshot_ir.sh`.
- Optional artifact-hash mismatch: use it only as a secondary debugging signal after semantic failures are understood.
- Missing source/generated image artifacts: regenerate local artifacts for debugging; do not add image binaries to git.
- Placeholder text or missing font metadata in live guard: fix localization or font-resolution data flow before any baseline change.

## Approval Checklist

- The regression change is intentional and source-backed.
- Semantic failures are explained.
- The IR freeze was regenerated with an explicit approver and reason when required.
- `./scripts/validate_ui_snapshot_freeze.sh` passes.
- `./scripts/validate_ui_regression_artifacts.sh --quick` passes.
- The required full-scope UI regression path was re-run.
- No image binaries are being committed as part of the freeze update.

## Test Scope Guidance

- During UI matching, do not add one-off tests for visibility/text/font/shape positioning just to chase regressions.
- Once fixes are correct, gold/platinum image standards freeze those aspects under rigorous regression coverage.
- Do not treat a targeted single-test run as equivalent to the required full-scope UI regression path.
- The required full-scope path is manifest-driven; target-specific probes are for debugging only, not for compliance.
