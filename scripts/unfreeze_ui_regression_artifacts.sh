#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/unfreeze_ui_regression_artifacts.sh \
    --approver <name> \
    --reason <text> \
    [--freeze-file <path>] \
    [--archive-dir <path>]

Description:
  Unfreezes the current UI regression freeze lock by archiving it with unfreeze
  metadata and removing the active freeze file.

Options:
  --approver      Required approver identity (name/alias)
  --reason        Required unfreeze reason
  --freeze-file   Optional active freeze lock path override
  --archive-dir   Optional archive directory path override
  -h, --help      Show this help text
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

FREEZE_FILE="${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json"
ARCHIVE_DIR="${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/freeze-history"
APPROVER=""
REASON=""

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
    --freeze-file)
      FREEZE_FILE="${2:-}"
      shift 2
      ;;
    --archive-dir)
      ARCHIVE_DIR="${2:-}"
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

if [[ -z "${APPROVER}" || -z "${REASON}" ]]; then
  echo "error: --approver and --reason are required" >&2
  usage >&2
  exit 1
fi

if [[ ! -f "${FREEZE_FILE}" ]]; then
  echo "error: freeze file not found: ${FREEZE_FILE}" >&2
  exit 1
fi

if ! jq -e '.schema_version == 1 and (.artifacts | type == "array")' "${FREEZE_FILE}" >/dev/null 2>&1; then
  echo "error: invalid freeze file: ${FREEZE_FILE}" >&2
  exit 1
fi

mkdir -p "${ARCHIVE_DIR}"

stamp="$(date -u +"%Y%m%dT%H%M%S.%NZ")"
archive_file="${ARCHIVE_DIR}/ui_regression_freeze_${stamp}.json"
unfrozen_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

jq \
  --arg unfrozen_at "${unfrozen_at}" \
  --arg unfreeze_approver "${APPROVER}" \
  --arg unfreeze_reason "${REASON}" \
  '. + {
    unfrozen_at: $unfrozen_at,
    unfreeze_approver: $unfreeze_approver,
    unfreeze_reason: $unfreeze_reason
  }' "${FREEZE_FILE}" > "${archive_file}"

rm -f "${FREEZE_FILE}"

echo "archived freeze file: ${archive_file}"
echo "removed active freeze file: ${FREEZE_FILE}"
