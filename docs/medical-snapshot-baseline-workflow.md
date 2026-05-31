# Medical Snapshot Baseline Workflow

Phase 7 baseline policy for `starbreaker-ui` medical snapshot regression checks.

Medical1 and Medical2 are treated as gold-standard outputs. Deviations are presumed broken until proven to be intentional, source-backed contract changes with explicit user approval.

## Guardrails

- Normal test runs must never auto-write or auto-bless baselines.
- Snapshot baseline changes are explicit, manual, and code-reviewed.
- Baseline updates are allowed only in dedicated commits that explain why drift is expected.
- Adding, removing, or updating manifest regression targets requires explicit user instruction.
- If a medical render or live IR shows obvious breakage such as visible placeholder text, stop. That is a product regression signal, not baseline drift.
- Do not update medical baselines unless the user explicitly approves the baseline refresh after root-cause investigation.

## Current Baseline Assets

- IR fixtures:
  - `crates/starbreaker-ui/tests/fixtures/medical_ir/medical1-screen_16x9_a-ir.json`
  - `crates/starbreaker-ui/tests/fixtures/medical_ir/medical2-mesh_end_screen_plane-ir.json`
- Regression tests:
  - `crates/starbreaker-ui/tests/manifest_snapshot_regression.rs`
  - `crates/starbreaker-ui/tests/manifest_visual_regression.rs`
  - `crates/starbreaker-ui/tests/manifest_live_ir_guard.rs`

## Standard Scripts

- Add or update a manifest target: `./scripts/add_ui_regression_target.sh`
- Generate local artifact images from manifest targets: `./scripts/generate_ui_regression_artifacts.sh`
- Validate manifest/artifact/source integrity: `./scripts/validate_ui_regression_artifacts.sh`
- Freeze approved artifact hashes/metadata: `./scripts/freeze_ui_regression_artifacts.sh`
- Unfreeze by approved reason (archives prior freeze): `./scripts/unfreeze_ui_regression_artifacts.sh`

Example: add a new platinum target and then regenerate artifacts.

```bash
./scripts/add_ui_regression_target.sh \
  --id clipper_new_panel \
  --tier platinum \
  --source-generated-png ships/Data/UI/Generated/ship/drak/Clipper/clipper_new_panel.png

./scripts/generate_ui_regression_artifacts.sh
./scripts/validate_ui_regression_artifacts.sh --full
./scripts/freeze_ui_regression_artifacts.sh \
  --approver "<name>" \
  --reason "Approve baseline refresh"
```

Modes for validator:

- Quick mode (manifest + freeze lock coherence): `./scripts/validate_ui_regression_artifacts.sh --quick`
- Full mode (source/artifact/freeze image checks): `./scripts/validate_ui_regression_artifacts.sh --full`

## Update Procedure

1. Confirm the drift is intentional and source-driven (not a hardcoded workaround).
2. Confirm the failure is not an obvious regression signal such as visible placeholder text, unresolved placeholder keys, or missing major UI elements.
3. If you need to register a new target, add it via `./scripts/add_ui_regression_target.sh`.
4. Refresh local visual artifacts via `./scripts/generate_ui_regression_artifacts.sh`.
  - Generated files are written to `StarBreaker/test-artifacts/ui`.
5. Run `./scripts/validate_ui_regression_artifacts.sh` to verify manifest/source/artifact consistency.
6. Freeze approved baselines via `./scripts/freeze_ui_regression_artifacts.sh --approver "<name>" --reason "<why>"`.
7. Re-run focused validation before any baseline updates:
   - `cargo test -p starbreaker-ui ui_snapshot --lib`
  - `cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture`
  - `cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture`
  - `cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture`
8. If the failure is a regression signal, investigate and fix the product path first; do not touch the baselines.
9. If source IR has legitimately changed, refresh fixture IR files from the approved capture flow.
10. Keep each baseline update in an explicit commit with a message that includes:
   - why drift is expected,
   - what source rule changed,
   - what tests were run.
11. Re-run `cargo test -p starbreaker-ui` before merge.

Policy:

- Failing regression checks are product regressions to investigate and fix, not tests to loosen or baselines to rewrite blindly.

## Regression Failure Playbook

### Reproduce Locally

1. Refresh local artifacts from the manifest:
  - `./scripts/generate_ui_regression_artifacts.sh`
2. Validate lock/manfiest/artifact coherence:
  - quick: `./scripts/validate_ui_regression_artifacts.sh --quick`
  - full: `./scripts/validate_ui_regression_artifacts.sh --full`
3. Run focused regression suites:
  - `cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture`
  - `cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture`
  - `cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture`

### Read Delta Output

1. Start from the first failing target id in test output.
2. Note failure class: geometry, text/font, color/channel, missing artifact, undeclared artifact, freeze mismatch.
3. Use the printed file paths (source artifact, generated artifact, freeze path) as the investigation entrypoints.
4. Treat tolerance failures as behavior deltas, not automatic baseline-update signals.

### Required Investigation Sequence

1. Source data layer:
  - manifest target entry
  - `source_generated_png` path
  - relevant canvas/style/IR fixture records
2. Renderer/transform layer:
  - renderer logic and comparator behavior for the failing class
3. Output layer:
  - generated artifact file
  - freeze lock hash/dim/channel entry

### Prohibited Responses

- Do not loosen tier tolerances to make a failure pass without a source-backed rule change.
- Do not rewrite or freeze new baselines to hide unexplained regressions.
- Do not bypass validator failures by removing freeze entries or skipping target ids.

## Baseline Onboarding Guide For New Targets

### Tier Selection (Platinum vs Gold)

- `platinum`:
  - critical UI surfaces where drift must stay tight
  - use for medical, safety, and high-trust readouts
- `gold`:
  - important but moderately variable surfaces
  - use where controlled drift is acceptable
- Final tier choice remains user-approved.

### Baseline Capture Procedure

1. Add target metadata to manifest:
  - `./scripts/add_ui_regression_target.sh --id <id> --tier <gold|platinum> --source-generated-png <path>`
2. Generate artifacts from manifest:
  - `./scripts/generate_ui_regression_artifacts.sh`
3. Validate consistency:
  - `./scripts/validate_ui_regression_artifacts.sh --full`
4. Freeze with explicit approval/reason:
  - `./scripts/freeze_ui_regression_artifacts.sh --approver "<name>" --reason "<why>"`

### Approval Workflow And Review Checklist

1. Visual approval from user/operator on generated target image.
2. Structural validation pass (`--full`) with no errors.
3. Regression suites pass for snapshot, visual, and live guards.
4. Commit message explains reason and impacted target ids.

### Required Metadata Per Target

- `id`
- `tier`
- `category`
- `source_generated_png`
- `baseline_path`
- `current_path`
- `roi` (`x`, `y`, `w`, `h`)

## Baseline Examples

### Platinum Examples

- `medical1`
- `medical2`

### Gold Example

- `clipper_small_door`

## Review Checklist

- No screen/name-specific production hardcoding was introduced.
- Drift diagnostics are field-granular and actionable.
- Manifest snapshot tests fail for controlled drift and pass for expected state.
- Baseline refresh approval is explicit and recorded; placeholder-text regressions were ruled out before any fixture update.
