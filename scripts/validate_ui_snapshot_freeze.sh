#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MANIFEST_PATH="${UI_REGRESSION_MANIFEST_PATH:-${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json}"
FREEZE_PATH="${UI_SNAPSHOT_FREEZE_PATH:-${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json}"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/validate_ui_snapshot_freeze.sh

Environment overrides:
  UI_REGRESSION_MANIFEST_PATH  Path to ui_snapshot_manifest.json
  UI_SNAPSHOT_FREEZE_PATH      Path to ui_snapshot_freeze.json
EOF
}

if [[ $# -gt 0 ]]; then
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required but not installed" >&2
  exit 1
fi

if [[ ! -f "${MANIFEST_PATH}" ]]; then
  echo "error: manifest not found: ${MANIFEST_PATH}" >&2
  exit 1
fi

if [[ ! -f "${FREEZE_PATH}" ]]; then
  echo "error: snapshot freeze not found: ${FREEZE_PATH}" >&2
  exit 1
fi

if ! jq -e '.schema_version == 1 and (.targets | type == "array")' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: invalid manifest schema: ${MANIFEST_PATH}" >&2
  exit 1
fi

if ! jq -e '.schema_version == 1 and (.targets | type == "array")' "${FREEZE_PATH}" >/dev/null 2>&1; then
  echo "error: invalid snapshot freeze schema: ${FREEZE_PATH}" >&2
  exit 1
fi

if ! jq -e '.targets | all(.id and .tier and .category and .source_generated_png)' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest targets missing required fields" >&2
  exit 1
fi

if ! jq -e '.targets | all(.id and .tier and .category and .source_generated_png and .canvas_record_path and .canvas_guid and .baseline_snapshot)' "${FREEZE_PATH}" >/dev/null 2>&1; then
  echo "error: snapshot freeze targets missing required fields" >&2
  exit 1
fi

# baseline_snapshot.schema_version tracks UI_SNAPSHOT_SCHEMA_VERSION (v2 adds the
# rendered text-top fields that visible-position known-outlier overrides anchor on).
if ! jq -e '.targets | all(.baseline_snapshot.schema_version == 2 and (.baseline_snapshot.elements | type == "array"))' "${FREEZE_PATH}" >/dev/null 2>&1; then
  echo "error: snapshot freeze contains invalid baseline_snapshot payloads" >&2
  exit 1
fi

if ! jq -e '(.targets | map(.id)) as $ids | ($ids | length) == ($ids | unique | length)' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest contains duplicate target ids" >&2
  exit 1
fi

if ! jq -e '(.targets | map(.id)) as $ids | ($ids | length) == ($ids | unique | length)' "${FREEZE_PATH}" >/dev/null 2>&1; then
  echo "error: snapshot freeze contains duplicate target ids" >&2
  exit 1
fi

manifest_ids_file="$(mktemp)"
freeze_ids_file="$(mktemp)"
trap 'rm -f "${manifest_ids_file}" "${freeze_ids_file}"' EXIT

jq -r '.targets[].id' "${MANIFEST_PATH}" | sort > "${manifest_ids_file}"
jq -r '.targets[].id' "${FREEZE_PATH}" | sort > "${freeze_ids_file}"

if ! diff -u "${manifest_ids_file}" "${freeze_ids_file}" >/dev/null 2>&1; then
  echo "error: snapshot freeze ids do not match manifest ids" >&2
  diff -u "${manifest_ids_file}" "${freeze_ids_file}" || true
  exit 1
fi

if jq -e '.. | .artifact_path? // empty' "${FREEZE_PATH}" >/dev/null 2>&1; then
  echo "error: snapshot freeze must not contain artifact_path fields" >&2
  exit 1
fi

if jq -e '.. | .sha256? // empty' "${FREEZE_PATH}" >/dev/null 2>&1; then
  echo "error: snapshot freeze must not contain sha256 fields" >&2
  exit 1
fi

count="$(jq '.targets | length' "${FREEZE_PATH}")"
echo "snapshot freeze validation passed: ${count} target(s)"
