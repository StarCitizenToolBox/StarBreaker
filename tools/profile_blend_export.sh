#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" || -z "${1:-}" ]]; then
  cat <<'USAGE'
Usage: tools/profile_blend_export.sh <entity> [output-root] [threads]

Profiles a native decomposed .blend export and records elapsed time, exporter
phase timings, output size/count, and representative md5 sums.

Environment:
  SC_DATA_P4K  Path to Star Citizen Data.p4k. If unset, the CLI auto-detects
               common install paths.

Arguments:
  entity       Entity search string, e.g. DRAK_Clipper or aurora_mk2.
  output-root  Export root to create. Default: /tmp/starbreaker_blend_profile_<entity>_<pid>
  threads      Export worker threads. 0 = auto/all cores, 1 = sequential.
               Default: 0.
USAGE
  exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENTITY="$1"
SAFE_ENTITY="$(printf '%s' "$ENTITY" | tr -cs '[:alnum:]_-' '_')"
OUTPUT_ROOT="${2:-/tmp/starbreaker_blend_profile_${SAFE_ENTITY}_$$}"
THREADS="${3:-0}"
PROFILE_DIR="$OUTPUT_ROOT/profile"
LOG_PATH="$PROFILE_DIR/export.log"
MD5_PATH="$PROFILE_DIR/md5.txt"
SUMMARY_PATH="$PROFILE_DIR/summary.txt"
RUST_LOG="${RUST_LOG:-starbreaker_3d::pipeline::glb_assembly=info,starbreaker_3d::pipeline::blend_assembly=info,starbreaker_3d::decomposed=info}"

mkdir -p "$PROFILE_DIR"

if [[ ! -x "$ROOT/target/release/starbreaker" ]]; then
  cargo build --manifest-path "$ROOT/Cargo.toml" --release --bin starbreaker
fi

START_NS="$(date +%s%N)"
(
  cd "$ROOT"
  RUST_LOG="$RUST_LOG" target/release/starbreaker entity export \
    --kind decomposed --lod 0 --mip 0 --materials all --threads "$THREADS" \
    "$ENTITY" "$OUTPUT_ROOT"
) 2>&1 | tee "$LOG_PATH"
END_NS="$(date +%s%N)"

elapsed_ms=$(( (END_NS - START_NS) / 1000000 ))
file_count="$(find "$OUTPUT_ROOT" -type f ! -path "$PROFILE_DIR/*" | wc -l)"
total_bytes="$(find "$OUTPUT_ROOT" -type f ! -path "$PROFILE_DIR/*" -printf '%s\n' | awk '{sum += $1} END {print sum + 0}')"

{
  printf 'entity=%s\n' "$ENTITY"
  printf 'output_root=%s\n' "$OUTPUT_ROOT"
  printf 'threads=%s\n' "$THREADS"
  printf 'elapsed_ms=%s\n' "$elapsed_ms"
  printf 'file_count=%s\n' "$file_count"
  printf 'total_bytes=%s\n' "$total_bytes"
  printf 'timings:\n'
  grep -E '\[timing\]' "$LOG_PATH" || true
} > "$SUMMARY_PATH"

: > "$MD5_PATH"
find "$OUTPUT_ROOT" -type f \
  \( -name 'scene.blend' -o -name '*_LOD0.blend' -o -name '*.materials.json' \) \
  ! -path "$PROFILE_DIR/*" \
  | sort \
  | head -80 \
  | xargs -r md5sum >> "$MD5_PATH"

cat "$SUMMARY_PATH"
cat "$MD5_PATH"
