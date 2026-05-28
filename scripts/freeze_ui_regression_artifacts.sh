#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/freeze_ui_regression_artifacts.sh \
    --approver <name> \
    --reason <text> \
    [--signature <text>] \
    [--manifest <path>] \
    [--artifact-dir <path>] \
    [--output <path>]

Description:
  Freezes current UI regression artifacts by writing a metadata lock file with
  per-target hashes, dimensions, channels, tier, and source path.

Options:
  --approver      Required approver identity (name/alias)
  --reason        Required approval reason
  --signature     Optional signature/token note for audit trail
  --manifest      Optional manifest path override
  --artifact-dir  Optional artifact directory override
  --output        Optional freeze lock output path override
  -h, --help      Show this help text

Environment:
  UI_REGRESSION_MANIFEST_PATH  Optional manifest path override.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"

mapfile -t MANIFEST_CANDIDATES < <(find "${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures" -type f -name '*snapshot_manifest.json' | sort)
DEFAULT_MANIFEST_PATH=""
if [[ "${#MANIFEST_CANDIDATES[@]}" -eq 1 ]]; then
  DEFAULT_MANIFEST_PATH="${MANIFEST_CANDIDATES[0]}"
fi

MANIFEST_PATH="${UI_REGRESSION_MANIFEST_PATH:-${DEFAULT_MANIFEST_PATH}}"
ARTIFACT_DIR="${REPO_ROOT}/test-artifacts/ui"
OUTPUT_PATH="${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json"
APPROVER=""
REASON=""
SIGNATURE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --approver)
      APPROVER="${2:-}"
      shift 2
      ;;
    --reason)
      REASON="${2:-}"
      shift 2
      ;;
    --signature)
      SIGNATURE="${2:-}"
      shift 2
      ;;
    --manifest)
      MANIFEST_PATH="${2:-}"
      shift 2
      ;;
    --artifact-dir)
      ARTIFACT_DIR="${2:-}"
      shift 2
      ;;
    --output)
      OUTPUT_PATH="${2:-}"
      shift 2
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

if ! command -v magick >/dev/null 2>&1; then
  echo "error: ImageMagick 'magick' command is required but not installed" >&2
  exit 1
fi

if ! command -v sha256sum >/dev/null 2>&1; then
  echo "error: sha256sum is required but not installed" >&2
  exit 1
fi

if [[ -z "${APPROVER}" || -z "${REASON}" ]]; then
  echo "error: --approver and --reason are required" >&2
  usage >&2
  exit 1
fi

if [[ -z "${MANIFEST_PATH}" || ! -f "${MANIFEST_PATH}" ]]; then
  echo "error: UI regression manifest not found: ${MANIFEST_PATH}" >&2
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

entries_file="$(mktemp)"
out_tmp="$(mktemp)"
cleanup() {
  rm -f "${entries_file}" "${out_tmp}"
}
trap cleanup EXIT

count=0
while IFS=$'\t' read -r target_id source_png tier; do
  [[ -n "${target_id}" ]] || continue

  if [[ "${source_png}" = /* ]]; then
    source_path="${source_png}"
  elif [[ "${source_png}" = ships/* ]]; then
    source_path="${WORKSPACE_ROOT}/${source_png}"
  else
    source_path="${REPO_ROOT}/${source_png}"
  fi

  artifact_path="${ARTIFACT_DIR}/${target_id}.png"
  if [[ "${artifact_path}" == "${REPO_ROOT}/"* ]]; then
    artifact_rel="${artifact_path#${REPO_ROOT}/}"
  else
    artifact_rel="${artifact_path}"
  fi

  if [[ ! -f "${source_path}" ]]; then
    echo "error: source image missing for ${target_id}: ${source_path}" >&2
    exit 1
  fi

  if [[ ! -f "${artifact_path}" ]]; then
    echo "error: artifact image missing for ${target_id}: ${artifact_path}" >&2
    exit 1
  fi

  hash="$(sha256sum "${artifact_path}" | awk '{print $1}')"
  meta="$(magick identify -format '%w %h %[channels]' "${artifact_path}")"
  read -r width height channels_raw <<< "${meta}"
  channels="$(echo "${channels_raw}" | awk '{print tolower($1)}')"

  jq -n \
    --arg id "${target_id}" \
    --arg tier "${tier}" \
    --arg artifact_path "${artifact_rel}" \
    --arg source_generated_png "${source_png}" \
    --arg sha256 "${hash}" \
    --argjson width "${width}" \
    --argjson height "${height}" \
    --arg channels "${channels}" \
    '{
      id: $id,
      tier: $tier,
      source_generated_png: $source_generated_png,
      artifact_path: $artifact_path,
      sha256: $sha256,
      width: $width,
      height: $height,
      channels: $channels
    }' >> "${entries_file}"

  count=$((count + 1))
  echo "frozen: ${target_id} (${tier})"
done < <(jq -r '.targets[] | select(.source_generated_png != null) | [.id, .source_generated_png, .tier] | @tsv' "${MANIFEST_PATH}")

if [[ "${count}" -eq 0 ]]; then
  echo "error: no targets with source_generated_png found in manifest" >&2
  exit 1
fi

frozen_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
manifest_rel="${MANIFEST_PATH#${REPO_ROOT}/}"

jq -s \
  --arg schema_version "1" \
  --arg frozen_at "${frozen_at}" \
  --arg approver "${APPROVER}" \
  --arg reason "${REASON}" \
  --arg signature "${SIGNATURE}" \
  --arg manifest_path "${manifest_rel}" \
  '
  {
    schema_version: ($schema_version | tonumber),
    frozen_at: $frozen_at,
    approver: $approver,
    reason: $reason,
    signature: (if $signature == "" then null else $signature end),
    manifest_path: $manifest_path,
    artifacts: .
  }
  ' "${entries_file}" > "${out_tmp}"

mkdir -p "$(dirname "${OUTPUT_PATH}")"
mv "${out_tmp}" "${OUTPUT_PATH}"
trap - EXIT
rm -f "${entries_file}"

echo "freeze file written: ${OUTPUT_PATH}"
