# starbreaker-ui crate instructions

Scope: everything under `StarBreaker/crates/starbreaker-ui/`.

Read this after the repo-level `StarBreaker/AGENTS.md` and before planning or editing work in this crate.

## Required first reads

Before planning or editing in this crate, read these in order:

1. `StarBreaker/AGENTS.md`
2. `StarBreaker/.github/copilot-instructions.md`
3. `StarBreaker/crates/starbreaker-ui/AGENTS.md` (this file)
4. `StarBreaker/crates/starbreaker-ui/docs/ui-matching-workflow.md` for any UI parity/matching task

Do not rely on stale chat context for UI matching behavior. Re-read
`ui-matching-workflow.md` when switching screens, after long detours, or when
visual fixes stop converging.

## Core rules

- No hard-coding, no name-matching, no ship-specific branches, and no screen-specific branches in production code.
- No heuristic placement rules, blend factors, hand-tuned percentages, or magic offsets unless the surrounding rule is already structurally defined by source data and the new math is derived from that data.
- Fix the structural cause. Do not patch symptoms in one renderer path if the real issue is authored metadata, IR compilation, layout, or text measurement.
- IR is the source of truth for render values. The renderer must not override IR-provided font size, position, alignment, scale, margin, padding, text colour, stroke colour, icon tint, or visibility based on widget names, parent context, or screen-specific checks.
- If rendered output is wrong, correct the owning upstream stage (`bb_layout.rs` / `ui_ir.rs` / source data resolution) so IR values are correct before rendering.
- When a rendered position looks wrong, identify which abstraction owns it before editing:
	- `bb_layout.rs` owns authored layout rects.
	- `ui_ir.rs` owns which authored metadata is preserved into IR.
	- `ir_compose.rs` owns final draw-time rects and renderer-specific adjustments.
	- `text.rs` owns text metrics, baseline, and rendered glyph bounds.

## Required validation loop for visual work

For visual/layout tasks in this crate, work in this order:

1. Identify the owning rect or draw path with the query tools before editing.
2. Form one falsifiable local hypothesis about the bad position.
3. Make one focused change.
4. Run the narrowest relevant validation immediately.
5. Measure the new result numerically.
6. Only then ask the user for final visual confirmation.

Do not chain multiple speculative layout changes before remeasuring.

## Query and debug tools

Prefer the generic query example over ad-hoc logging:

```bash
cargo run -p starbreaker-ui --example query_ui_layout -- \
	--canvas-guid <guid> --query <pattern>
```

The query tool is intended to be generic. Keep it generic when extending it.

Current debug outputs include:

- node `x/y/w/h` layout rect
- resolved `draw_rect`
- `parent_id`
- primary/secondary text rects
- primary/secondary text origins
- primary/secondary drawn glyph bounds
- progress-meter draw rect
- asset reference path when present
- custom-shape metadata when present

If a future investigation needs another measurable draw-time output, add it here generically rather than adding screen-specific debug code.

## Troubleshooting workflow for relative visual feedback

When the user gives relative movement feedback such as “move it up about 20px” or “needs slightly more gap”, use that as a calibration target for investigation, not as the final rule.

The workflow is:

1. Use `query_ui_layout` to measure the current layout rect, draw rect, text rects, and drawn bounds.
2. Compare the measured values with the user’s relative estimate.
3. Trace the mismatch back to authored metadata, layout math, IR loss, or draw-time adjustment.
4. Implement a structural fix that explains the requested movement.
5. Re-run the focused tests and `query_ui_layout` to confirm the measured movement matches the estimate.
6. Re-render the affected screen.
7. Return to the user only for final visual confirmation.

This lets iteration continue without repeatedly asking the user to re-check half-finished passes, while still keeping the final rule structural.

## Minimum checks before claiming a fix

- For `ir_compose.rs` work: run `cargo test -p starbreaker-ui ir_compose --lib`.
- For query/debug tool changes: run the example you changed against a real canvas.
- For any renderer/layout change touching visible output: regenerate the relevant render example and inspect it.
- Before commit, run `cargo test -p starbreaker-ui`.

## Guardrails for code review

Reject a change if it does any of the following:

- branches on `MedGel`, `ui_target_a`, `ui_target_b`, ship names, manufacturer names, or specific asset paths in production logic
- introduces unexplained percentages or offsets for placement
- adds debug output that only works for one screen instead of the generic query path
- fixes a draw-time symptom while leaving a clearly wrong upstream rect or missing metadata untouched

If the source data genuinely does not contain the needed signal, prove that first with the query/debug tools before choosing the narrowest renderer-side fallback.
