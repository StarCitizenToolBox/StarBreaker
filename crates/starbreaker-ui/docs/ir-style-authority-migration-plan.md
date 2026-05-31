# IR Style Authority Migration Plan

## Goal

Make UI visual style authority live in IR processing, and make render/composition strictly consume resolved IR styling without mutating style intent at draw time.

## Scope

- Crate: `crates/starbreaker-ui`
- Primary modules:
  - `src/ui_ir/**` (style semantics and resolution)
  - `src/ir_compose/**` (renderer consumption)
  - `src/compose/**` (legacy path parity and cleanup)
  - `tests/manifest_live_ir_guard.rs` and snapshot compare/capture

## Acceptance Criteria

- Renderers do not derive colour/tint semantics from runtime style defaults when equivalent IR semantics are expected.
- Style-tag and label-style semantic interpretation is performed before draw dispatch.
- Live guard captures style semantics that can drift at compose time.
- Regressions are caught by deterministic tests.

## Phase A - Contract and Inventory

- [x] Audit render-side style mutation points in `ir_compose` and legacy `compose`.
- [x] Define explicit "style authority contract" in docs and enforceable code comments.
- [x] Classify each current fallback as either: IR-authoritative, temporary migration fallback, or prohibited.

Fallback classification snapshot:

- IR-authoritative:
  - explicit `UiIrTextStyle.colour` / `colour_token`
  - explicit node colour/tint fields (`icon_tint_colour`, `background_fill_colour`, `stroke_colour`)
- Temporary migration fallback:
  - token resolution against manufacturer palette in render path while IR lacks resolved RGBA fields
  - selected style source -> brand slug mapping in render path
- Prohibited (removed in this iteration):
  - style-derived accent/backlight fallback for manufacturer logo tint when IR has no tint semantics
  - style-derived accent/backlight fallback for secondary close button tint when IR has no tint semantics
  - draw-time text-colour semantic override chain that bypassed explicit IR text style fields

## Phase B - Colour/Tint Authority (Initial Implementation)

- [x] Remove style-derived fallback tints from high-impact drift paths (manufacturer logo / close button) where IR already provides semantics.
- [x] Keep renderer fallback neutral (non-style-derived) when IR has no style instruction.
- [x] Add unit tests proving renderer preference order is IR fields first and neutral fallback last.

Outcome:

- `cargo test -p starbreaker-ui ir_compose -- --nocapture` passed (36/36 ir_compose tests).

## Phase C - IR Resolution Expansion

- [x] Move style-tag and token interpretation rules out of draw-time paths into IR preprocessing/normalization.
- [x] Add resolved style fields where needed so draw functions consume resolved values directly.
- [x] Keep old fields temporarily for compatibility while migrating callsites.

Progress:

- Removed draw-time text-colour semantic override chain in `ir_compose` (`resolved_text_colour` now consumes explicit IR text style colour/colour_token only).
- Updated ir_compose tests to reflect IR-authoritative text colour contract.
- Moved additional style-tag semantics into `ui_ir` compile-time token mapping (`UI_Generic_Flag_03`, `StateModerate`, `StateCritical`, `Flashing`, `Modify`, untagged `Heading1`/`Heading3`).
- Added ui_ir compile tests proving these mappings are materialized into IR text style colour tokens.
- Moved custom-shape style-tag tint semantics (`Primary`/`Modify`) into `ui_ir` compile-time icon tint token mapping.
- Removed renderer-side custom-shape style-tag tint inference; renderer now uses explicit IR tint fields only.
- Added tests proving custom-shape tint respects explicit IR token and does not infer from style tags at draw time.
- Moved custom-shape `Modify` additive blend semantics into `ui_ir` compile-time `colour_blend_mode` mapping.
- Removed renderer-side additive blend inference from `resolved_style_tags`; renderer now uses IR-authored `colour_blend_mode`.
- Added tests proving style tags alone no longer imply additive SVG blend mode at draw time.
- Removed renderer-side border/separator fallback derivation to primary/accent defaults; these paths now consume explicit IR colour/token semantics only.
- Added compile-time warnings for strict-renderer style semantic gaps (separator colour semantics and border side colour semantics).
- Verification: `cargo test -p starbreaker-ui ui_ir -- --nocapture && cargo test -p starbreaker-ui ir_compose -- --nocapture` passed.

Outcome:

- Phase C migration is complete for colour/tint/blend style semantics: style-tag interpretation now occurs in `ui_ir`, and `ir_compose` consumes explicit IR fields/tokens without style-tag inference at draw time.

## Phase F - Documentation and Contract Reinforcement

- [x] Update regression docs to explicitly state: IR is the sole styling authority; renderers must not invent style semantics.
- [x] Add migration guidance for moving any future style fallback from draw-time to IR preprocessing.
- [x] Add a reviewer checklist item requiring IR-authority verification on style-related PRs.

Outcome:

- Updated `ui-matching-workflow.md` with explicit IR style-authority contract, style-fallback migration procedure, and reviewer checklist guard.

## Phase D - Renderer Strictness

- [x] Remove remaining draw-time style inference branches.
- [x] Route all colour/tint/font policy through resolved IR values.
- [x] Add diagnostics for missing required resolved fields instead of silently deriving style.

Outcome:

- Removed draw-time style-primary fallback heuristics for border/separator rendering and token resolution (`Accent1`/`Accent2`/`Accent4` no longer infer from `primary_tint`).
- Added strict-renderer diagnostic warnings in IR compile output for nodes missing required colour semantics.
- Verification: `cargo test -p starbreaker-ui ui_ir -- --nocapture && cargo test -p starbreaker-ui ir_compose -- --nocapture` passed.

## Phase E - Regression and Baseline Policy

- [x] Extend live guard snapshots to include resolved style semantics required for drift detection.
- [x] Update fixture and freeze migration policy for schema additions.
- [x] Re-run required sequence and document expected failures/approvals.

Outcome:

- Extended `UiSnapshotElement` semantics with `stroke_tint_token` and `text_tint_token` and wired capture/comparison logic.
- Updated `ir-freeze-schema.md` with explicit schema-addition/migration policy for new semantic fields.
- Validation sequence results:
  - `cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture`: pass (11/11)
  - `cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture`: pass (3 passed, 1 ignored optional backstop)
  - `cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture`: expected failures requiring approval/triage
    - existing known font-size drift on `ui_target_a` / `ui_target_b`
    - `ui_target_b` tint semantics now report blend-mode drift (`None` -> `additive`) after strict IR-style-authority migration
  - `bash ./scripts/validate_ui_snapshot_freeze.sh`: pass (4 targets)
  - `./scripts/validate_ui_regression_artifacts.sh --quick`: pass (4 targets)

## Risks

- Font behavior regressions when draw-time heuristics are removed.
- Freeze/schema churn while adding resolved fields.
- Temporary divergence between `compose` and `ir_compose` paths.

## Mitigations

- Migrate colours/tints first; defer typography policy changes.
- Add parity tests around high-value targets before deleting fallback logic.
- Stage strictness behind progressive test gates.

## Immediate Next Steps

1. Triage/approve `manifest_live_ir_guard` drift set (`ui_target_a`, `ui_target_b`) before any freeze/baseline updates.
2. If approved, refresh freeze snapshots via `scripts/freeze_ui_snapshot_ir.sh` with explicit approver/reason and re-run required validations.
