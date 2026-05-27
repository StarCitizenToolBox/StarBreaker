# UI Regression Policy

This policy defines required regression coverage for `starbreaker-ui` changes.

## Required for Every UI Defect Fix

When fixing a UI defect category, add or update at least one regression guard in the same change:

- Structural/layout drift: snapshot or layout assertion test.
- Typography drift: text style/spacing/font-selection assertion.
- Image/shape/tint drift: snapshot field or renderer metadata assertion.
- Binding/localization drift: binding-resolution assertion in IR compile path.

## Mandatory CI Checks

The following checks are required in CI for UI changes:

- `cargo test -p starbreaker-ui`
- `bash .github/scripts/check_ui_hardcoding.sh`
- `cargo run -p starbreaker-ui --example phase5_certification_dashboard`

## Contributor Guardrails

- New fallback logic requires:
  - entry in `docs/ui-fallback-register.md`,
  - explicit trigger signal,
  - retirement target.
- Hardcoded ship/manufacturer/screen/name/path behavior in production UI code is forbidden.
- Source-backed IR fields should be preferred over renderer-time inference.
- Any snapshot baseline update must be intentional and reviewed.

## Review Checklist Addendum

Reviewers should verify:

- A regression test was added for the fixed defect category.
- Existing certified-family cases do not regress in the dashboard output.
- No undocumented fallback logic was introduced.
- Hardcoding guard remains green.
