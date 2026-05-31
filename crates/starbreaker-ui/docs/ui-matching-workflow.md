# UI Matching Workflow

This document is the reference workflow for UI-matching changes in `starbreaker-ui`.

## Non-Negotiable Rules

1. Always run the `starbreaker-ui` test suite after making UI-related changes.
2. Treat platinum/gold image test failures as regressions in source behavior.
3. Do not "fix" regressions by changing tests or baselines first.
4. Fix root causes structurally; do not use hard-coding or heuristics.
5. Keep Rust source files under 500 lines (enforced by tests).

## Regression Policy

- The visual regression tests are the guardrail for platinum/gold standard outputs.
- If any platinum/gold image test fails, assume the code regressed.
- Fix production code or source-data handling so the expected images remain unchanged.
- Never bypass or weaken these tests to make them pass.

## Source Of Truth

- The single source of truth is game data.
- Use StarBreaker MCP tools to inspect DataCore records, P4k assets, and related source structures.
- Use Blender MCP tools with the connected Blender instance for scene/material/transform/render validation.

## Validation Loop

1. Make a focused code change.
2. Run `cargo test -p starbreaker-ui --tests`.
3. If tests fail, fix the root cause in source logic/data handling.
4. Re-run tests until all pass.
5. Repeat this loop frequently during implementation.

## If You Are Unsure About Visual Output

- Generate UI regression artifacts:

```bash
./generate_ui_regression_artifacts.sh
```

- Then ask the user to verify via the question tool.

## Test Scope Guidance

- During UI matching, do not add one-off tests for visibility/text/font/shape positioning just to chase regressions.
- Once fixes are correct, gold/platinum image standards freeze those aspects under rigorous regression coverage.
