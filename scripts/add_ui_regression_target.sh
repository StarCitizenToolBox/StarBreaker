#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/add_ui_regression_target.sh \
    --id <target_id> \
    --tier <gold|platinum> \
    --source-generated-png <path> \
    [--category <image|shape|text|font>] \
    [--baseline-path <path>] \
    [--current-path <path>] \
    [--roi <x,y,w,h>] \
    [--manifest <path>] \
    [--replace]

Description:
  Adds a regression target to the UI regression manifest used by the visual
  artifact workflow. By default this script appends a new target and fails if
  the target id already exists.

Options:
  --id                    Target id (for example: clipper_small_door)
  --tier                  Quality tier: gold or platinum
  --source-generated-png  Source image path used by artifact generation script
  --category              Regression category (default: image)
  --baseline-path         Snapshot baseline key/path (default: <id>.baseline)
  --current-path          Snapshot current key/path (default: <id>.current)
  --roi                   Normalized ROI as x,y,w,h (default: 0,0,1,1)
  --manifest              Explicit manifest path override
  --replace               Replace an existing target with the same id
  -h, --help              Show this help text

Environment:
  UI_REGRESSION_MANIFEST_PATH  Optional manifest path override.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DEFAULT_MANIFEST_PATH="$(find "${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures" -type f -name '*snapshot_manifest.json' | sort | head -n 1)"

TARGET_ID=""
TIER=""
CATEGORY="image"
SOURCE_PNG=""
BASELINE_PATH=""
CURRENT_PATH=""
ROI_SPEC="0,0,1,1"
MANIFEST_PATH="${UI_REGRESSION_MANIFEST_PATH:-${DEFAULT_MANIFEST_PATH}}"
REPLACE_EXISTING=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --id)
      TARGET_ID="${2:-}"
      shift 2
      ;;
    --tier)
      TIER="${2:-}"
      shift 2
      ;;
    --category)
      CATEGORY="${2:-}"
      shift 2
      ;;
    --source-generated-png)
      SOURCE_PNG="${2:-}"
      shift 2
      ;;
    --baseline-path)
      BASELINE_PATH="${2:-}"
      shift 2
      ;;
    --current-path)
      CURRENT_PATH="${2:-}"
      shift 2
      ;;
    --roi)
      ROI_SPEC="${2:-}"
      shift 2
      ;;
    --manifest)
      MANIFEST_PATH="${2:-}"
      shift 2
      ;;
    --replace)
      REPLACE_EXISTING=1
      shift
      ;;
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
done

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required but not installed" >&2
  exit 1
fi

if [[ -z "${MANIFEST_PATH}" || ! -f "${MANIFEST_PATH}" ]]; then
  echo "error: UI regression manifest not found: ${MANIFEST_PATH}" >&2
  exit 1
fi

if [[ -z "${TARGET_ID}" || -z "${TIER}" || -z "${SOURCE_PNG}" ]]; then
  echo "error: --id, --tier, and --source-generated-png are required" >&2
  usage >&2
  exit 1
fi

case "${TIER}" in
  gold|platinum) ;;
  *)
    echo "error: --tier must be one of: gold, platinum" >&2
    exit 1
    ;;
esac

case "${CATEGORY}" in
  image|shape|text|font) ;;
  *)
    echo "error: --category must be one of: image, shape, text, font" >&2
    exit 1
    ;;
esac

IFS=',' read -r ROI_X ROI_Y ROI_W ROI_H <<< "${ROI_SPEC}"
if [[ -z "${ROI_X:-}" || -z "${ROI_Y:-}" || -z "${ROI_W:-}" || -z "${ROI_H:-}" ]]; then
  echo "error: --roi must be provided as x,y,w,h" >&2
  exit 1
fi

BASELINE_PATH="${BASELINE_PATH:-${TARGET_ID}.baseline}"
CURRENT_PATH="${CURRENT_PATH:-${TARGET_ID}.current}"

if ! jq -e . "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest is not valid JSON: ${MANIFEST_PATH}" >&2
  exit 1
fi

exists="$(jq --arg id "${TARGET_ID}" 'any(.targets[]; .id == $id)' "${MANIFEST_PATH}")"
if [[ "${exists}" == "true" && "${REPLACE_EXISTING}" -ne 1 ]]; then
  echo "error: target id already exists: ${TARGET_ID} (use --replace to update)" >&2
  exit 1
fi

tmp_file="$(mktemp)"
cleanup() {
  rm -f "${tmp_file}"
}
trap cleanup EXIT

jq \
  --arg id "${TARGET_ID}" \
  --arg category "${CATEGORY}" \
  --arg baseline_path "${BASELINE_PATH}" \
  --arg current_path "${CURRENT_PATH}" \
  --arg source_generated_png "${SOURCE_PNG}" \
  --arg tier "${TIER}" \
  --argjson x "${ROI_X}" \
  --argjson y "${ROI_Y}" \
  --argjson w "${ROI_W}" \
  --argjson h "${ROI_H}" \
  --argjson replace_existing "${REPLACE_EXISTING}" \
  '
  . as $root
  | {
      id: $id,
      category: $category,
      baseline_path: $baseline_path,
      current_path: $current_path,
      source_generated_png: $source_generated_png,
      tier: $tier,
      roi: { x: $x, y: $y, w: $w, h: $h }
    } as $new_target
  | if $replace_existing == 1 then
      .targets = (.targets | map(if .id == $id then $new_target else . end))
    else
      .targets = (.targets + [$new_target])
    end
  ' "${MANIFEST_PATH}" > "${tmp_file}"

mv "${tmp_file}" "${MANIFEST_PATH}"
trap - EXIT

if [[ "${exists}" == "true" ]]; then
  echo "updated target ${TARGET_ID} in ${MANIFEST_PATH}"
else
  echo "added target ${TARGET_ID} to ${MANIFEST_PATH}"
fi
