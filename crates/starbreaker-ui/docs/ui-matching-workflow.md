# UI Matching Workflow

This document is the reference workflow for UI-matching changes in `starbreaker-ui`.

## Non-Negotiable Rules

1. Always run the `starbreaker-ui` test suite after making UI-related changes.
2. Treat platinum/gold image test failures as regressions in source behavior.
3. Do not "fix" regressions by changing tests or baselines first.
4. Fix root causes structurally; do not use hard-coding or heuristics.
5. Keep Rust source files under 500 lines (enforced by tests).
6. Only add, remove, or update regression targets/baselines when the user explicitly instructs it.
7. Do not null style/tint fields in required live-guard checks; required checks must verify tint/brand semantics.
8. For end-to-end UI parity runs in this crate, do not use VFL stop-and-ask cadence after every micro-change; run an autonomous diff/fix loop and ask the user at meaningful milestones or final verification.
9. Remove no-effect experiments immediately (dead helpers, fallback branches that do not change output, stale one-off probes).

## Required Reads For UI Matching

Before starting a matching pass, re-read:

1. `StarBreaker/AGENTS.md`
2. `StarBreaker/.github/copilot-instructions.md`
3. `StarBreaker/crates/starbreaker-ui/AGENTS.md`
4. `StarBreaker/crates/starbreaker-ui/docs/ui-matching-workflow.md` (this file)

If you are resuming from a long chat or switching to a different screen/canvas,
re-read this file before touching code.

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

MCP-first policy for UI matching:

- Prefer StarBreaker MCP query tools first when investigating assets, style records, DataCore values, and chunk/material semantics.
- Use CLI export commands for end-to-end render generation and regression artifact production, not for exploratory data archaeology that MCP can answer faster.

IR style authority rule:

- IR is the sole styling authority for rendered UI semantics.
- Render/composition code must consume explicit IR style fields and must not invent style semantics from style tags, label names, or palette heuristics at draw time.
- If a style effect is needed and not represented in IR yet, add it to `ui_ir` preprocessing/schema first, then consume it in the renderer.

Compose-time alteration rule:

- If a visible UI effect is produced at compose time (for example manufacturer logo tint derivation), that effect must be represented in IR/snapshot semantics so regression checks can detect drift.
- Prefer moving structural style decisions into IR generation where possible.
- If a compose-time derivation must remain, mirror its effective semantic output in snapshot metadata and include it in required comparisons.

## Style Tag Notes (Annunciator/Common Patterns)

When debugging style/tag-driven UI behavior, verify these semantics in IR before renderer edits:

- State tags: `StateModerate`, `StateCritical`, `StateFlashing`
- Screen/group tags: `AnnunciatorScreen`, `AnnunciatorItem`
- Token effects in IR: `Accent2` (often warning/moderate), `Accent3`, `Base`, `Background`

Do not add renderer-side name checks for these effects. If tags drive a visible
effect, the effect must be explicit in IR fields and snapshots.

## End-To-End Matching Loop (Autonomous)

Use this loop until all tracked differences are closed:

1. Compare reference vs generated image and produce a concrete difference catalog.
2. Classify each difference by layer/type:
	- missing/extra shape
	- missing/extra image
	- wrong text content/font/size/weight
	- wrong position/scale/alignment
	- wrong color/tint/alpha/blend
	- wrong border/stroke/fill semantics
3. Map each difference to owning stage:
	- source data resolution
	- `bb_layout`
	- IR compile/normalization
	- `ir_compose` draw path
4. Implement one minimal structural fix per difference class.
5. Run required regression commands and quick visual regeneration.
6. Re-catalog remaining differences (do not assume all are fixed).
7. Remove no-effect code from failed experiments before next iteration.
8. Continue until parity is achieved or a true blocker is proven.

Important:

- Do not leave speculative branches in place when they fail to affect output.
- Keep changes lean and reversible while iterating.

## Style Fallback Migration Guidance

When you find a renderer-side style fallback:

1. Classify it as one of: IR-authoritative, temporary migration fallback, or prohibited.
2. Move style-tag/token interpretation into `ui_ir` compile-time normalization.
3. Emit explicit IR fields (token and/or resolved colour/blend fields as appropriate).
4. Update renderer callsites to consume only the explicit IR fields.
5. Add tests proving style tags alone no longer change draw-time behavior unless IR carries that semantic explicitly.
6. Run the required regression path and update freeze/snapshot docs if schema changed.

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

During end-to-end parity passes, run this loop at every meaningful fix batch,
not only once at the end.

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
- Footer/logo tint mismatch: verify the drift is represented in snapshot semantics (RGBA and/or tint token); required live-guard checks must not null tint fields.
- "Change had no visible effect": treat as a failed hypothesis, revert/clean the added logic, and re-check owning stage with IR/query evidence before trying another fix.

## Approval Checklist

- The regression change is intentional and source-backed.
- Semantic failures are explained.
- The IR freeze was regenerated with an explicit approver and reason when required.
- `./scripts/validate_ui_snapshot_freeze.sh` passes.
- `./scripts/validate_ui_regression_artifacts.sh --quick` passes.
- The required full-scope UI regression path was re-run.
- No image binaries are being committed as part of the freeze update.
- Style-related changes were reviewed for IR-authority compliance (no new draw-time semantic invention in renderer paths).

## Test Scope Guidance

- During UI matching, do not add one-off tests for visibility/text/font/shape positioning just to chase regressions.
- Once fixes are correct, gold/platinum image standards freeze those aspects under rigorous regression coverage.
- Do not treat a targeted single-test run as equivalent to the required full-scope UI regression path.
- The required full-scope path is manifest-driven; target-specific probes are for debugging only, not for compliance.

## Code Hygiene During Matching

- Remove dead helper functions and stale branches in the same change where they become obsolete.
- Avoid stacking multiple overlapping fallback strategies; keep one structural rule per behavior.
- If a patch does not measurably change queried IR/draw values or rendered output, remove it and record the failed hypothesis in notes.
