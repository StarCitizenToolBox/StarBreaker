#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<'USAGE'
Usage: tools/profile_aurora_blend.sh [output-root] [threads]

Profiles the Aurora Mk2 native decomposed .blend export and records md5 sums
for representative deterministic output files.

Environment:
  SC_DATA_P4K  Path to Star Citizen Data.p4k. If unset, the workspace default
               Wine/Proton path under $HOME is used.

Arguments:
  output-root  Export root to create. Default: /tmp/starbreaker_aurora_profile_<pid>
  threads      Export worker threads. 0 = auto/all cores, 1 = sequential.
               Default: 0.
USAGE
  exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_ROOT="${1:-/tmp/starbreaker_aurora_profile_$$}"
THREADS="${2:-0}"
SC_DATA_P4K="${SC_DATA_P4K:-$HOME/Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k}"
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
  cd "$ROOT/.."
  SC_DATA_P4K="$SC_DATA_P4K" RUST_LOG="$RUST_LOG" "$ROOT/target/release/starbreaker" entity export \
    "aurora_mk2" "$OUTPUT_ROOT" \
    --kind decomposed --format blend --lod 0 --mip 0 --materials all \
    --threads "$THREADS"
) 2>&1 | tee "$LOG_PATH"
END_NS="$(date +%s%N)"

elapsed_ms=$(( (END_NS - START_NS) / 1000000 ))
file_count="$(find "$OUTPUT_ROOT" -type f ! -path "$PROFILE_DIR/*" | wc -l)"
total_bytes="$(find "$OUTPUT_ROOT" -type f ! -path "$PROFILE_DIR/*" -printf '%s\n' | awk '{sum += $1} END {print sum + 0}')"

cat > "$SUMMARY_PATH" <<SUMMARY
output_root=$OUTPUT_ROOT
threads=$THREADS
elapsed_ms=$elapsed_ms
file_count=$file_count
total_bytes=$total_bytes
SUMMARY

targets=(
  "Packages/RSI Aurora Mk2_LOD0_TEX0/scene.blend"
  "Data/Objects/Spaceships/Ships/RSI/aurora_mk2/interior/rsi_aurora_mk2_foyer_floor_LOD0.blend"
  "Data/Objects/Spaceships/Ships/RSI/aurora_mk2/interior/rsi_aurora_mk2_cockpit_wall_right_LOD0.blend"
  "Data/Objects/Spaceships/Ships/RSI/aurora_mk2/interior/rsi_aurora_mk2_armor_locker_LOD0.blend"
  "Data/Objects/Spaceships/Ships/RSI/aurora_mk2/interior/rsi_aurora_mk2_foyer_weapon_rack_LOD0.blend"
  "Data/Objects/Spaceships/Ships/RSI/aurora_mk2/exterior/rsi_aurora_mk2_LOD0.blend"
  "Data/Objects/Spaceships/Weapons/KLWE/KLWE_Merged_Weapons/KLWE_Las_Rep_S2/KLWE_Las_Rep_S2_L1_LOD0.blend"
  "Data/Objects/fps_weapons/gadgets/kegr/fire_extinguisher/gdgt_fps_kegr_fire_extinguisher_LOD0.blend"
)

: > "$MD5_PATH"
for target in "${targets[@]}"; do
  path="$OUTPUT_ROOT/$target"
  if [[ -f "$path" ]]; then
    md5sum "$path" >> "$MD5_PATH"
  else
    printf 'MISSING  %s\n' "$target" >> "$MD5_PATH"
  fi
done

cat "$SUMMARY_PATH"
cat "$MD5_PATH"
