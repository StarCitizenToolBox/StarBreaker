# UI Matching Agent Prompt Template

Use this template to prompt an agent for end-to-end UI parity work against a
reference screenshot.

## Copy/Paste Prompt

```text
You are working in StarBreaker, in crate starbreaker-ui.

Before you plan or edit anything, read these files in order:
1. StarBreaker/AGENTS.md
2. StarBreaker/.github/copilot-instructions.md
3. StarBreaker/crates/starbreaker-ui/AGENTS.md
4. StarBreaker/crates/starbreaker-ui/docs/ui-matching-workflow.md

Goal:
Match the generated UI image to the provided reference image end-to-end, and do
not stop until all cataloged differences are resolved or a concrete blocker is
proven.

Important context:
- Reference screenshots are imperfect. They may be non-perpendicular, skewed,
	perspective-distorted, offset, partially occluded, not pixel-identical in
	resolution, or include in-game rendering artifacts.
- Do not assume perfect 1:1 pixel alignment from the screenshot alone.
- Use structural comparison, not naive pixel matching.

Operating rules:
1. Follow `crates/starbreaker-ui/docs/ui-matching-workflow.md`.
2. Use StarBreaker MCP tools first for investigation (DataCore/P4k/material/
	 chunk lookups) whenever possible; use CLI export primarily for rendering and
	 regression artifact generation.
3. Keep IR as styling authority. Do not invent style semantics in renderer code.
4. No hard-coded per-screen or per-name branches in production logic.
5. If a change has no measurable effect, remove it immediately.
6. Keep code lean: no dead helpers, no stale fallback paths, no layered
	 speculative logic left behind.
7. Run regular regression checks to prevent frozen-image regressions.

Required workflow:

Phase A - Baseline and decomposition
- Load both images (reference + latest generated).
- Identify UI regions/components and catalog every difference:
	- extra/missing shapes
	- extra/missing images
	- text differences (content, font, weight, size)
	- positioning/alignment/scale differences
	- color/tint/alpha/blend differences
	- border/stroke/fill differences
- For each difference, assign probable ownership stage:
	- source data resolution
	- bb_layout
	- ui_ir compile/normalization
	- ir_compose draw-time behavior

Phase B - Plan
- Produce a concrete execution plan from the catalog.
- Order items by dependency and regression risk.
- Define success criteria for each item using measurable outcomes (IR/query
	values and rendered results), not only visual opinion.

Phase C - Execute iteratively
- Implement one focused fix at a time.
- After each fix:
	1) run the smallest relevant test/query checks,
	2) regenerate the target artifact,
	3) compare against the same catalog,
	4) update the remaining-differences list.
- Do not keep no-effect code.

Phase D - Regression safety (run frequently, not just once)
- Run required UI regression path from ui-matching-workflow.md.
- If any platinum/gold regresses, fix root cause; do not weaken tests.

Phase E - Completion
- Continue until all cataloged differences are resolved.
- Provide final report:
	- resolved differences
	- remaining differences (if any) with proven blocker evidence
	- tests run and outcomes
	- final code cleanup summary (what was removed as no-effect/stale)

Additional analysis expectations:
- Account for perspective/skew when interpreting shape position and size.
- Prefer comparing relative layout relationships (spacing, alignment groups,
	visual hierarchy) rather than absolute raw pixel offsets from imperfect
	screenshots.
- For text, separate typography issues from placement issues.
- Validate inferred style-tag behavior in IR and query outputs before changing
	compose code.

Output requirements from you:
1. Initial difference catalog table.
2. Ordered fix plan.
3. Per-iteration delta log (what changed, what improved, what regressed).
4. Final parity assessment tied to the original catalog.
```

## Usage Notes

- Provide the agent with both image paths explicitly.
- Include the canvas GUID, target name, and render command currently used.
- If you already know recurring pain points, append a short "watch for" list,
	such as style-tag drift, alpha suppression drift, or text metric drift.
- Keep added context concise; rely on referenced docs for detailed policy.
