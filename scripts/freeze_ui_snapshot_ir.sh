#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/freeze_ui_snapshot_ir.sh \
    --approver <name> \
    --reason <text> \
    [--signature <text>] \
    [--manifest <path>] \
    [--output <path>]

Description:
  Generates an IR-only UI snapshot freeze file from the manifest targets.
  This stores canonical serialized UiScreenSnapshot baselines in git and does
  not write image binaries into the freeze payload.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MANIFEST_PATH="${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json"
OUTPUT_PATH="${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json"
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

if [[ -z "${APPROVER}" || -z "${REASON}" ]]; then
  echo "error: --approver and --reason are required" >&2
  usage >&2
  exit 1
fi

cd "${REPO_ROOT}"
cmd=(
  cargo run -p starbreaker-ui --example freeze_ui_snapshot_ir --
  --manifest "${MANIFEST_PATH}"
  --output "${OUTPUT_PATH}"
  --approver "${APPROVER}"
  --reason "${REASON}"
)

if [[ -n "${SIGNATURE}" ]]; then
  cmd+=(--signature "${SIGNATURE}")
fi

"${cmd[@]}"
