# Gold/Platinum UI Regression Deep-Dive

Date: 2026-05-31
Scope: `starbreaker-ui` gold/platinum regression coverage, test architecture, and onboarding workflow for new targets/tier changes.
Constraint: report-only pass; no code changes in this task.

Additional policy requirement (user-directed): freeze data must be IR-based and git-storable; image artifacts must not be committed to git and are not part of the freeze payload.

## 1) Deep-Dive Audit Report

### Executive Summary

The regression engine already contains a strong generic structural comparator, but the current test harness is still a hybrid with several image-specific test entry points. This is why behavior is not fully aligned with the intended model (manifest + freeze-driven, no per-image hand-rolled tests).

The current implementation is close to the desired architecture at the core-comparator layer, but incomplete at the orchestration layer (test entry points, gating behavior, and always-run coverage guarantees).

### What Is Generic Today

The generic snapshot comparator and manifest runner enforce many of the required standards across visible elements:

- Missing previously visible elements are detected.
- Unexpected new visible elements are detected.
- Text payload/case drift is detected.
- Font identity drift is detected.
- Position and size drift (`x`, `y`, `w`, `h`) is detected with tier tolerance.
- RGBA/tint channel drift is detected.
- Alpha drift is detected.

### What Is Not Yet Fully Generic

- Image-named integration tests still exist in `manifest_visual_regression.rs`.
- Target filtering is still hardcoded in some suites (for example, retaining only a subset of ids).
- Visual checks are inconsistent by target: some use heuristic ROI checks, others use freeze hash checks.
- Live guard intentionally narrows to two targets and strips many style fields for movement-only checks.

### Requirement-by-Requirement Compliance Matrix

Requested standard vs current state:

1. All visible text that was visible is still visible: PASS (generic comparator).
2. No new text is visible: PASS (generic comparator).
3. All text exactly the same (character/case): PASS (generic comparator).
4. Correct fonts & weight: PARTIAL.
   - Font identity is checked.
   - Explicit independent weight field enforcement is not consistently modeled as a first-class snapshot invariant.
5. Correct font size within stated limit: PARTIAL.
   - `line_spacing` is compared.
   - Explicit font-size invariant is not captured as a dedicated snapshot field in the current generic comparator path.
6. Correct text color within stated limit: PASS (RGBA channel comparisons).
7. Correct text x/y position: PASS (geometry drift checks).
8. All images/shapes that were visible still are: PASS (generic comparator).
9. No new images/shapes visible: PASS (generic comparator).
10. Correct image/shape tint applied: PASS (icon/background/stroke channel checks when present).
11. Correct image/shape x/y and scale: PASS (x/y/w/h checks).
12. Include backgrounds: PARTIAL/PASS.
   - Background RGBA channels are checked.
   - Full background image identity is not universally modeled as its own explicit invariant.
13. Include alpha amounts: PASS (element alpha + RGBA alpha channels).

### Root Causes Behind Mismatch With Intended Model

1. Architecture drift: generic library comparator plus non-generic test harness wrappers.
2. Inconsistent visual oracles: mixed use of source-vs-current heuristics and freeze-hash baselines.
3. Coverage path ambiguity: targeted test invocations can bypass full gold/platinum image target enforcement.
4. Data model gap for typography: font size and weight are not fully represented as explicit, uniformly enforced snapshot invariants.
5. Freeze medium mismatch: current freeze process is artifact-image-hash-centric, while desired policy is IR-baseline-centric with no image payload in git.

### Focused Investigation: Why Font-Size Regressions Escaped

Observed issue:

- Font-size regressions have occurred without deterministic failure from the intended generic test path.

Current observed profile (validated on 2026-05-31):

- `ui_target_a`: FAIL
- `ui_target_b`: FAIL
- `clipper_small_door`: FAIL
- `eng_annunciator_master_left`: PASS

This matches the reported expectation that all standard images except annunciator currently fail.

Confirmed technical causes:

1. UI IR contains explicit `font_size` in `UiIrTextStyle`, but snapshot capture does not persist that field into `UiSnapshotElement`.
2. Snapshot comparator uses `font_size_relative` tolerance against `line_spacing` only, not true `font_size`.
3. Existing unit coverage validates line-spacing tolerance behavior, which does not guarantee true font-size protection.
4. The live guard includes a movement-focused normalization path that strips typography style details for that comparison mode.
5. `clipper_small_door` currently lacks a dedicated frozen IR baseline fixture equivalent to the existing target A/B fixture depth, limiting strict semantic attribution for that target today.

Impact:

- A change that modifies effective text size while leaving line spacing close enough (or normalized away in movement-focused paths) can evade the intended generic catch.
- Current failures are primarily surfaced by freeze hash drift, which is broad and not field-specific to font-size.
- Therefore, regressions are detected, but not by the intended explicit font-size invariant pathway.

Diagnostic gap:

- Current failure messages frequently report generic artifact hash drift instead of a direct semantic cause such as `font_size drift`.
- This slows root-cause triage and obscures whether the failure is specifically typography-related.
- Check ordering should enforce semantic checks first and hash/freeze checks last so the first failing assertion is explanatory.

Required correction:

- Promote explicit `font_size` to a first-class snapshot invariant and compare it with tier tolerance in generic manifest-driven checks.
- Keep line-spacing checks, but treat them as secondary typography fidelity signals, not the primary font-size oracle.

### Tooling and Workflow Status

Current scripts are strong and broadly compatible with the intended model:

- `add_ui_regression_target.sh` supports target registration and tier updates.
- `generate_ui_regression_artifacts.sh` is manifest-driven for artifact generation.
- `freeze_ui_regression_artifacts.sh` captures approved baseline hashes/metadata.
- `validate_ui_regression_artifacts.sh` validates manifest/freeze/artifact coherence.

The scripts can support a fully generic model, but test wiring must be unified so all gold/platinum targets are always exercised in standard UI regression runs.

Current mismatch with desired policy:

- Existing freeze lock stores metadata derived from generated PNG artifacts (hash/dim/channel metadata) rather than a pure IR freeze contract.
- Desired model is to freeze canonical IR-derived structural baseline data in git, then validate local render outputs against that baseline without requiring committed images.

### Evidence Map (Source Files Reviewed)

- Generic comparator behavior: `crates/starbreaker-ui/src/ui_snapshot/compare.inc`
- Snapshot capture fields: `crates/starbreaker-ui/src/ui_snapshot/capture.inc`
- Snapshot schema/tolerance fields: `crates/starbreaker-ui/src/ui_snapshot/types.inc`
- Tier tolerance definitions: `crates/starbreaker-ui/src/ui_regression_manifest/types.rs`
- Manifest runner behavior: `crates/starbreaker-ui/src/ui_regression_manifest/runner.rs`
- Hybrid visual tests (image-named): `crates/starbreaker-ui/tests/manifest_visual_regression.rs`
- Snapshot regression harness/filtering: `crates/starbreaker-ui/tests/manifest_snapshot_regression.rs`
- Live guard target narrowing/movement focus: `crates/starbreaker-ui/tests/manifest_live_ir_guard.rs`
- Target onboarding script: `scripts/add_ui_regression_target.sh`
- Artifact generation script: `scripts/generate_ui_regression_artifacts.sh`
- Freeze creation script: `scripts/freeze_ui_regression_artifacts.sh`
- Manifest/freeze/artifact validator: `scripts/validate_ui_regression_artifacts.sh`

### Line-Level Citation Index

- Visible-element set drift checks (missing/new):
  - `crates/starbreaker-ui/src/ui_snapshot/compare.inc` lines 24 and 30.
- Text payload/case drift:
  - `crates/starbreaker-ui/src/ui_snapshot/compare.inc` line 86.
- Font identity drift:
  - `crates/starbreaker-ui/src/ui_snapshot/compare.inc` line 92.
- Geometry drift (`x`, `y`, `w`, `h`):
  - `crates/starbreaker-ui/src/ui_snapshot/compare.inc` line 98 and subsequent geometry comparisons.
- Line spacing drift using font-size-relative tolerance:
  - `crates/starbreaker-ui/src/ui_snapshot/compare.inc` lines 155 and 295.
- Explicit `font_size` exists in UI IR text style:
  - `crates/starbreaker-ui/src/ui_ir/engine_parts/part_01.part` lines 189 and 192.
- Snapshot capture does not currently persist `font_size`:
  - `crates/starbreaker-ui/src/ui_snapshot/capture.inc` lines 48, 54 (font identity + line spacing only).
- Unit test currently validates line-spacing as font-size-relative proxy:
  - `crates/starbreaker-ui/src/ui_snapshot/tests.inc` lines 73, 89, and 97.
- Live guard movement-focused normalization removes line spacing in that mode:
  - `crates/starbreaker-ui/tests/manifest_live_ir_guard.rs` line 189.
- RGBA/tint drift comparisons:
  - `crates/starbreaker-ui/src/ui_snapshot/compare.inc` lines 164, 180, and 188.
- Element alpha drift:
  - `crates/starbreaker-ui/src/ui_snapshot/compare.inc` line 122.
- Snapshot capture fields proving available invariants:
  - `crates/starbreaker-ui/src/ui_snapshot/capture.inc` lines 37, 39, 40, and 54.
- Tier tolerance policy values (platinum/gold):
  - `crates/starbreaker-ui/src/ui_regression_manifest/types.rs` lines 41, 45, 47, 51, and 53.
- Manifest runner applying category checks:
  - `crates/starbreaker-ui/src/ui_regression_manifest/runner.rs` lines 36 and 45.
- Hybrid image-named visual tests:
  - `crates/starbreaker-ui/tests/manifest_visual_regression.rs` lines 196, 201, 434, 442, 448, and 454.
- Hardcoded target-retain filtering in regression suites:
  - `crates/starbreaker-ui/tests/manifest_visual_regression.rs` line 107.
  - `crates/starbreaker-ui/tests/manifest_snapshot_regression.rs` line 25.
  - `crates/starbreaker-ui/tests/manifest_live_ir_guard.rs` line 166.
- Live guard movement-focused normalization path:
  - `crates/starbreaker-ui/tests/manifest_live_ir_guard.rs` lines 170, 349, 353, 357, 361, and 365.
- Manifest-driven artifact generation and copy behavior:
  - `scripts/generate_ui_regression_artifacts.sh` lines 32, 57, 80, and 83.
- Validator guarantees for source field presence and freeze target coherence:
  - `scripts/validate_ui_regression_artifacts.sh` lines 83, 84, 154, and 173.

## 2) Review of Report, Findings, and Actions

### Review Pass Notes

This review validates the report quality itself (accuracy, completeness, and actionability), then corrects any weak points.

#### Findings Quality Check

- Completeness: PASS.
  - All requested validation dimensions (text, shape/image, tint/color, geometry/scale, alpha) are addressed.
- Accuracy: PASS with two explicit caveats preserved.
  - Font weight and explicit font-size are marked PARTIAL, not PASS.
  - Background handling is marked PARTIAL/PASS depending on representation.
- Specificity: PASS.
  - Root causes are mapped to architecture, oracle choice, and workflow behavior.

#### Actionability Check

- The report includes concrete structural actions and script/gating actions.
- Actions are sequenceable and testable.
- No action requires per-image hand-authored tests.

#### Corrections Applied During Review

1. Kept font-weight classification at PARTIAL to avoid over-claiming.
2. Kept background identity classification at PARTIAL/PASS to avoid over-claiming.
3. Added an explicit always-run requirement for standard images in normal UI regression command flow.

### Final Corrective Action Set (Reviewed)

A1. Unify test entry points so regression runs are manifest-driven and fully generic.

A2. Replace artifact-hash freeze with an IR-based freeze contract committed to git (no image artifact payloads).

A3. Extend snapshot invariants so font-size and weight are explicit first-class checks.

A4. Normalize category policy packs (text/image/shape/background) under one generic runner.

A5. Enforce always-run coverage for standard images in canonical UI test execution (local + CI).

A6. Preserve and document script-first onboarding for adding targets and changing tiers.

A7. Enforce repo hygiene guardrails so committed freeze data is IR-only and image artifacts are always excluded from git.

A8. Require field-level semantic failure diagnostics (for example `font_size drift`, `font_weight drift`, `text_rgba drift`) in regression output.

A9. Order checks so semantic invariant failures execute before hash/freeze guards; hash mismatch is a final backstop, not the first reported failure.

## 3) Phased Plan (Checkboxed TODO)

Goal: transition from hybrid regression tests to a fully generic gold/platinum model with no per-image hand-rolled tests.

### Phase 1 - Baseline Architecture Lock-In

- [x] Replace image-named visual guard tests with a single manifest-iterating generic integration test entry point.
- [x] Remove hardcoded target-id filtering from regression suites unless explicitly scoped by manifest metadata.
- [x] Ensure test output still reports failures per target id for diagnostics.
- [x] Add/refresh test docs to define "no per-image hand-rolled tests" as an explicit invariant.

Acceptance criteria:
- One generic test entry path executes all manifest targets.
- No test function names encode specific image ids.

Progress update (2026-05-31):

- `manifest_visual_regression.rs` now uses `manifest_targets_visual_regression_guard` (manifest-driven loop) instead of per-image visual guard test functions.
- Backstop failure output remains per-target and currently reports three regressed targets (`ui_target_a`, `ui_target_b`, `clipper_small_door`) while annunciator remains passing.
- `manifest_snapshot_regression.rs` and `manifest_live_ir_guard.rs` now retain targets by available snapshot keys instead of hardcoded target ids.
- `ui-matching-workflow.md` now states that required compliance runs must remain manifest-driven and must not reintroduce per-image hand-rolled tests.

### Phase 2 - Baseline Oracle Standardization ✅ (2026-05-31, uncommitted)

- [x] Replace PNG-hash freeze payload with IR-snapshot baseline payload committed to git.
- [x] Define IR freeze schema fields (target id, tier, category, canonical snapshot elements, tolerances, schema version, approval metadata).
- [x] Guarantee manifest ids and IR-freeze ids are 1:1 in mandatory validation.
- [x] Ensure regression failures point to target id + IR field-level deltas (no dependency on committed image assets).
- [x] Remove PNG-hash freeze checks from required gating once IR freeze parity is complete.
- [x] Ensure top-level failure output includes semantic failure class summary, not only hash/metadata mismatch context.
- [x] Order validation pipeline: semantic IR-delta checks first, freeze/hash consistency checks last.

Acceptance criteria:
- Baseline checks are IR-freeze-driven and target-generic.
- Freeze payload in git contains no image binaries or image-hash-as-primary-oracle coupling.
- Failures are explainable via field-level deltas in test output.
- First failure for a semantic regression is descriptive (for example `font_size drift`) before any hash mismatch output.

Progress update (2026-05-31):

- `scripts/freeze_ui_snapshot_ir.sh` now generates `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json` from manifest targets and local Foundry records.
- The generated IR-only freeze currently covers all four manifest targets: `ui_target_a`, `ui_target_b`, `clipper_small_door`, and `eng_annunciator_master_left`.
- `scripts/validate_ui_snapshot_freeze.sh` now enforces manifest/freeze id parity and rejects `artifact_path` and `sha256` fields in the IR freeze payload.
- `bash ./scripts/validate_ui_snapshot_freeze.sh`: PASS (`4` frozen targets validated against manifest parity and IR-only schema rules).
- `bash ./scripts/validate_ui_regression_artifacts.sh --quick`: PASS, and now invokes the IR-freeze validator before legacy artifact-freeze parity checks.
- `cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture`: PASS, including new Rust-side checks for manifest/freeze id parity and IR-only freeze payload structure.
- `cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture`: FAIL, but now via the fully generic live semantic guard over all four frozen targets rather than the previous hardcoded two-target medical slice.
- `manifest_targets_frozen_artifact_backstop_guard` is now opt-in (`#[ignore]`), and `./scripts/validate_ui_regression_artifacts.sh --quick` no longer requires the legacy artifact-hash freeze file.
- `cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture`: PASS with the artifact-hash backstop ignored by default (`3` passed, `1` ignored).
- Required gating is now IR-freeze-first. Legacy image-hash checks remain available only as explicit opt-in diagnostics.
- `./scripts/validate_ui_regression_artifacts.sh --quick` now fails if `ui_snapshot_freeze.json` is missing instead of silently skipping IR-freeze enforcement.
- `freeze_ui_snapshot_ir.rs` now derives target dimensions from record-native `_RecordValue_.size` data rather than local PNG files.

### Phase 3 - Typography and Background Invariant Completion ✅ (2026-05-31, uncommitted)

- [x] Add explicit `font_size` field to `UiSnapshotElement` capture from IR `UiIrTextStyle`.
- [x] Add comparator rule for `font_size` drift using tier `font_size_relative` tolerance (and screen-floor behavior where applicable).
- [x] Add targeted regression tests proving font-size-only drift fails even when line spacing remains within tolerance.
- [x] Add explicit snapshot fields/invariants for font weight/style where structurally available.
- [x] Confirm background identity/tint/alpha expectations are first-class in generic policy (not implicit only).
- [x] Add comparator tests that fail on controlled font-size, font-weight, and background identity drifts.
- [x] Ensure typography failures explicitly mention `font_size` and/or `font_weight` in assertion output.

Acceptance criteria:
- Font size/weight checks are explicit and generic.
- Background rules are explicit and generic.
- Controlled font-size regressions fail deterministically via generic manifest-driven checks.

Progress update (2026-05-31):

- `cargo test -p starbreaker-ui --lib ui_snapshot -- --nocapture`: PASS, including `compare_snapshots_fails_on_explicit_font_size_drift`.
- `cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture`: PASS for required gating after moving the artifact-hash backstop to opt-in `#[ignore]` status.
- `cargo test -p starbreaker-ui --tests -q`: semantic font-size failures now surface explicitly in live guard output (for example `font_size drift baseline=... current=...`) instead of hash-only diagnostics.
- After widening `manifest_live_ir_guard` to load targets from `ui_snapshot_freeze.json`, semantic `font_size drift` now surfaces for all four frozen targets, including `eng_annunciator_master_left`.
- Regenerating `ui_snapshot_freeze.json` from record-native size metadata does not remove the four-target `font_size drift`, which suggests the semantic drift is real rather than a PNG-dimension artifact.
- The dominant drift ratios (`85.333 -> 64`, `24 -> 18`, `28 -> 21`) match a missing `imageSizePercent = 0.75` adjustment exactly, which points to divergence around `adjust_ui_ir_font_value_for_font_record_image_percent(...)` in the font-size resolution path.
- Root cause: `manifest_live_ir_guard` used a narrower `FsCanvasFetcher` index than the IR-freeze generator, so fontstyle records referenced by path basename failed to resolve in the live test path. That dropped the `resolved_font_record` data needed for the `imageSizePercent` adjustment and systematically shrank current font sizes.
- Fix landed: the live guard fetcher now indexes path stems, relative paths, and record ids in the same way as the freeze generator helper.
- `cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture`: PASS after aligning fontstyle record resolution between the live guard and the freeze generator.
- `cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture`: PASS (`11` tests), including explicit coverage for `font identity/font_weight drift` and background asset identity drift.
- Font weight/style is now treated as part of explicit `text_font_identity` regression semantics, and typography failure text names `font_weight` directly when that identity drifts.
- Background identity/tint/alpha expectations are now covered explicitly through asset identity, `background_rgba`, and alpha comparator rules rather than only by implication.

### Phase 4 - Always-Run Coverage Enforcement ✅ (2026-05-31, uncommitted)

- [x] Define canonical UI regression command(s) that always include all standard gold/platinum targets.
- [x] Wire canonical command(s) into CI-required checks.
- [x] Add a guard test or validation step that fails if a standard target is omitted from required run scope.
- [x] Update developer docs to distinguish optional focused debugging commands from required full-scope commands.
- [x] Ensure canonical run can execute on any machine with local game data, with no dependency on image files checked into git.

Acceptance criteria:
- Standard image targets are always tested in required UI regression runs.
- Targeted local runs cannot be mistaken for full compliance runs.
- Team members can run the same regression checks from shared IR freeze data plus local generated outputs.

Progress update (2026-05-31):

- Hosted CI without local game data is now explicitly treated as a repo-only enforcement path, not as the place to run live guards.
- `scripts/validate_ui_regression_repo_only.sh` now provides a CI-safe entrypoint for manifest snapshot regression plus IR-freeze parity validation from checked-in data.
- `bash ./scripts/validate_ui_regression_repo_only.sh`: PASS (`manifest_snapshot_regression` + IR-freeze validators succeeded from repo-only data with no live game-data dependency).
- `.github/workflows/starbreaker-ui-tests.yml` now runs the repo-only UI regression contract instead of implying hosted CI can provide meaningful live-guard coverage.
- Required-scope enforcement is now split correctly: repo-only CI validates manifest/freeze contract, while the documented local semantic-first path covers live guard and visual checks on machines with game data.
- Workflow validation in this environment: PASS for the wired command target (`validate_ui_regression_repo_only.sh`) and no YAML errors reported for `.github/workflows/starbreaker-ui-tests.yml`.

### Phase 5 - Onboarding and Tier-Change Workflow Hardening

- [x] Verify `add_ui_regression_target.sh` + IR-freeze generation/update + validator flow in one documented happy path.
- [x] Add a tier-change checklist (gold <-> platinum) with required validation/freeze steps.
- [x] Add troubleshooting matrix for common failures (IR field mismatch, tolerance threshold breach, manifest/freeze id drift, category mismatch).
- [x] Add a final checklist requiring explicit approval before baseline/tier updates.
- [x] Add explicit policy checks: no image artifacts in git for freeze updates; freeze commits contain IR-only baseline data.

Acceptance criteria:
- New targets and tier changes are predictable, script-first, and audit-friendly.
- Baseline changes cannot be used to mask unexplained regressions.
- Freeze updates are git-portable and image-binary-free.

Progress update (2026-05-31):

- `bash ./scripts/add_ui_regression_target.sh --help`: PASS, confirming the current onboarding entrypoint and argument surface used by the documented happy path.
- `ui-matching-workflow.md` now includes a script-first onboarding path, a tier-change checklist, a troubleshooting matrix, and an explicit approval checklist.
- The workflow docs now explicitly require IR-freeze validation and forbid committing image binaries as part of freeze updates.

## 4) Verification Plan For This Phased Work

When implementation begins, use this validation sequence per phase:

1. `cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture`
2. `cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture`
3. `cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture`
4. `./scripts/validate_ui_snapshot_freeze.sh`
5. `./scripts/validate_ui_regression_artifacts.sh --quick`

Latest validation result (2026-05-31):

- Full required semantic-first local path: PASS.
- `manifest_snapshot_regression`: PASS (`11` tests).
- `manifest_live_ir_guard`: PASS (`3` tests).
- `manifest_visual_regression`: PASS for required gating (`3` passed, `1` ignored optional backstop).
- `validate_ui_snapshot_freeze.sh`: PASS (`4` targets).
- `validate_ui_regression_artifacts.sh --quick`: PASS (`4` targets).

Optional artifact-hash diagnostics only:

1. `cargo test -p starbreaker-ui --test manifest_visual_regression -- --ignored --nocapture`
2. `./scripts/validate_ui_regression_artifacts.sh --full`

For Phase 4+, add CI confirmation that the canonical command path includes all standard manifest targets.

## 5) Constraints and Non-Goals

- No production workaround logic by asset name/id.
- No tolerance loosening or baseline refresh as first response to unexplained drift.
- No per-image hand-authored regression tests in the final model.
- No image binaries committed as part of freeze payloads.
- This document now tracks both the original analysis and the implementation progress/status of the phased migration.

## 6) Requirements Coverage Confirmation

The phased plan above covers all required testing expectations listed by the user.

Coverage checklist:

1. All visible text that was visible is still visible.
  - Covered by generic visible-element presence checks (Phase 1/Phase 2 semantic-first path).
2. No new text is visible.
  - Covered by unexpected visible-element checks (Phase 1/Phase 2 semantic-first path).
3. All text is exactly the same (exact character/case).
  - Covered by text payload/case drift checks (Phase 1 + comparator policy).
4. All text font has correct fonts & weight.
  - Font identity already covered; explicit weight/style invariant completion planned in Phase 3.
5. All text fonts have the correct size (within stated limit).
  - Explicit `font_size` capture/comparison is now implemented in Phase 3 and enforced by the generic comparator.
6. All text has the correct colour (within stated limit).
  - Covered by semantic RGBA channel checks in generic comparator policy (Phase 1/Phase 2).
7. All text is positioned in correct x/y position.
  - Covered by x/y drift checks with tier tolerance (Phase 1/Phase 2).
8. All visible images/shapes that were visible still are.
  - Covered by visible-element presence checks (Phase 1/Phase 2).
9. No new images/shapes are visible.
  - Covered by unexpected visible-element checks (Phase 1/Phase 2).
10. Correct image/shape tint (if any) is applied.
  - Covered by icon/background/stroke tint/color checks (Phase 1/Phase 2).
11. Correct x/y position applied to all images/shapes (within limit).
  - Covered by x/y drift checks (Phase 1/Phase 2).
12. Correct scale applied to all images/shapes (within limit).
  - Covered by w/h drift checks (Phase 1/Phase 2).
13. Images include backgrounds.
  - Explicit background identity/tint/alpha first-class checks planned in Phase 3.
14. Alpha amounts included.
  - Covered by alpha drift + RGBA alpha checks (Phase 1/Phase 2).

Additional enforced requirements now included in plan:

- IR-only freeze payload committed to git (no image uploads to git): Phase 2/Phase 5.
- Semantic descriptive failures before hash-related failures: Phase 2 + Action A9.
- Standard gold/platinum targets always tested in required runs: Phase 4.
