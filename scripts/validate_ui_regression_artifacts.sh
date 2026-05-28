#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
ARTIFACT_DIR="${REPO_ROOT}/test-artifacts/ui"

mapfile -t MANIFEST_CANDIDATES < <(find "${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures" -type f -name '*snapshot_manifest.json' | sort)
DEFAULT_MANIFEST_PATH=""
if [[ "${#MANIFEST_CANDIDATES[@]}" -eq 1 ]]; then
  DEFAULT_MANIFEST_PATH="${MANIFEST_CANDIDATES[0]}"
fi
MANIFEST_PATH="${UI_REGRESSION_MANIFEST_PATH:-${DEFAULT_MANIFEST_PATH}}"

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required but not installed" >&2
  exit 1
fi

if ! command -v magick >/dev/null 2>&1; then
  echo "error: ImageMagick 'magick' command is required but not installed" >&2
  exit 1
fi

if [[ -z "${MANIFEST_PATH}" || ! -f "${MANIFEST_PATH}" ]]; then
  echo "error: UI regression manifest not found: ${MANIFEST_PATH}" >&2
  if [[ "${#MANIFEST_CANDIDATES[@]}" -gt 1 ]]; then
    echo "error: multiple manifest candidates found; pass UI_REGRESSION_MANIFEST_PATH" >&2
    for candidate in "${MANIFEST_CANDIDATES[@]}"; do
      echo "  - ${candidate}" >&2
    done
  fi
  exit 1
fi

if [[ ! -d "${ARTIFACT_DIR}" ]]; then
  echo "error: artifact directory missing: ${ARTIFACT_DIR}" >&2
  exit 1
fi

if ! jq -e '.schema_version == 1 and (.targets | type == "array")' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest schema/targets validation failed: ${MANIFEST_PATH}" >&2
  exit 1
fi

if ! jq -e '(.targets | map(.id)) as $ids | ($ids | length) == ($ids | unique | length)' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest contains duplicate target ids: ${MANIFEST_PATH}" >&2
  exit 1
fi

if ! jq -e '.targets | all(.tier == "gold" or .tier == "platinum")' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest contains invalid tier values (expected gold/platinum)" >&2
  exit 1
fi

if ! jq -e '.targets | all(.source_generated_png != null and (.source_generated_png | type == "string") and (.source_generated_png | length > 0))' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: every target must include non-empty source_generated_png" >&2
  exit 1
fi

errors=0
checked=0

while IFS=$'\t' read -r target_id source_png tier; do
  [[ -n "${target_id}" ]] || continue
  checked=$((checked + 1))

  if [[ "${source_png}" = /* ]]; then
    source_path="${source_png}"
  elif [[ "${source_png}" = ships/* ]]; then
    source_path="${WORKSPACE_ROOT}/${source_png}"
  else
    source_path="${REPO_ROOT}/${source_png}"
  fi

  artifact_path="${ARTIFACT_DIR}/${target_id}.png"

  if [[ ! -f "${source_path}" ]]; then
    echo "error: missing source image for ${target_id}: ${source_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  if [[ ! -f "${artifact_path}" ]]; then
    echo "error: missing artifact image for ${target_id}: ${artifact_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  source_meta="$(magick identify -format '%w %h %[channels]' "${source_path}" 2>/dev/null || true)"
  artifact_meta="$(magick identify -format '%w %h %[channels]' "${artifact_path}" 2>/dev/null || true)"

  if [[ -z "${source_meta}" ]]; then
    echo "error: failed to inspect source image for ${target_id}: ${source_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  if [[ -z "${artifact_meta}" ]]; then
    echo "error: failed to inspect artifact image for ${target_id}: ${artifact_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  read -r src_w src_h src_channels <<< "${source_meta}"
  read -r out_w out_h out_channels <<< "${artifact_meta}"

  if [[ "${src_w}" != "${out_w}" || "${src_h}" != "${out_h}" ]]; then
    echo "error: dimension mismatch for ${target_id}: source=${src_w}x${src_h} artifact=${out_w}x${out_h}" >&2
    errors=$((errors + 1))
  fi

  if [[ "${src_channels}" != "${out_channels}" ]]; then
    echo "error: channel mismatch for ${target_id}: source=${src_channels} artifact=${out_channels}" >&2
    errors=$((errors + 1))
  fi

  echo "ok: ${target_id} (${tier})"
done < <(jq -r '.targets[] | [.id, .source_generated_png, .tier] | @tsv' "${MANIFEST_PATH}")

if [[ "${checked}" -eq 0 ]]; then
  echo "error: no targets found in manifest: ${MANIFEST_PATH}" >&2
  exit 1
fi

if [[ "${errors}" -gt 0 ]]; then
  echo "validation failed: ${errors} issue(s) across ${checked} target(s)" >&2
  exit 1
fi

echo "validation passed: ${checked} target(s) in ${MANIFEST_PATH}"
