#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
DEFAULT_MANIFEST_PATH="$(find "${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures" -type f -name '*snapshot_manifest.json' | sort | head -n 1)"
UI_REGRESSION_MANIFEST_PATH="${UI_REGRESSION_MANIFEST_PATH:-${DEFAULT_MANIFEST_PATH}}"
OUTPUT_DIR="${REPO_ROOT}/test-artifacts/ui"

if [[ -z "${UI_REGRESSION_MANIFEST_PATH}" || ! -f "${UI_REGRESSION_MANIFEST_PATH}" ]]; then
  echo "error: UI regression manifest not found: ${UI_REGRESSION_MANIFEST_PATH}" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required but not installed" >&2
  exit 1
fi

mkdir -p "${OUTPUT_DIR}"

# Refresh source-generated PNGs before copying them into test-artifacts.
# We infer entity export targets from manifest source paths of the form:
# ships/Data/UI/Generated/ship/<manufacturer>/<ShipName>/...
if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required to regenerate UI source images" >&2
  exit 1
fi

mapfile -t _manifest_sources < <(
  jq -r '.targets[] | select(.source_generated_png != null) | .source_generated_png' "${UI_REGRESSION_MANIFEST_PATH}" \
    | sed '/^$/d' \
    | sort -u
)

declare -A _entities=()
for source_png in "${_manifest_sources[@]}"; do
  if [[ "${source_png}" =~ ^ships/Data/UI/Generated/ship/([^/]+)/([^/]+)/ ]]; then
    manufacturer="${BASH_REMATCH[1]}"
    ship_name="${BASH_REMATCH[2]}"
    ship_slug="$(echo "${ship_name}" | tr '[:upper:]' '[:lower:]' | tr ' ' '_' | tr '-' '_')"
    entity_slug="${manufacturer}_${ship_slug}"
    _entities["${entity_slug}"]=1
  fi
done

if [[ "${#_entities[@]}" -eq 0 ]]; then
  echo "error: no exportable source paths found in manifest" >&2
  exit 1
fi

for entity_slug in "${!_entities[@]}"; do
  echo "generating source UI images via export: ${entity_slug}"
  (
    cd "${REPO_ROOT}"
    cargo run -p starbreaker -- entity export "${entity_slug}" "${WORKSPACE_ROOT}/ships" --kind decomposed --lod 0 --mip 0 --materials all
  )
done

count=0
while IFS=$'\t' read -r target_id source_png tier; do
  [[ -n "${target_id}" ]] || continue
  [[ -n "${source_png}" ]] || continue

  if [[ "${source_png}" = /* ]]; then
    source_path="${source_png}"
  elif [[ "${source_png}" = ships/* ]]; then
    source_path="${WORKSPACE_ROOT}/${source_png}"
  else
    source_path="${REPO_ROOT}/${source_png}"
  fi

  if [[ ! -f "${source_path}" ]]; then
    echo "error: source image missing for ${target_id}: ${source_path}" >&2
    exit 1
  fi

  output_path="${OUTPUT_DIR}/${target_id}.png"
  cp -f "${source_path}" "${output_path}"
  count=$((count + 1))
  echo "copied ${target_id} (${tier}) -> ${output_path}"
done < <(jq -r '.targets[] | select(.source_generated_png != null) | [.id, .source_generated_png, .tier] | @tsv' "${UI_REGRESSION_MANIFEST_PATH}")

if [[ "${count}" -eq 0 ]]; then
  echo "error: no targets with source_generated_png found in UI regression manifest" >&2
  exit 1
fi

echo "generated ${count} UI artifact(s) in ${OUTPUT_DIR}"
