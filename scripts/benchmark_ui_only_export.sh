#!/usr/bin/env bash
set -euo pipefail

# Benchmark and verify UI-only decomposed export parity against full export.
# Fails when UI output completeness/parity breaks or runtime exceeds threshold.

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <entity_name> [output_prefix]"
  echo "Example: $0 drak_clipper /tmp/clipper_ui_benchmark"
  exit 2
fi

entity_name="$1"
output_prefix="${2:-/tmp/${entity_name}_ui_benchmark}"

if [[ -z "${SC_DATA_P4K:-}" ]]; then
  echo "SC_DATA_P4K is required"
  exit 2
fi

if [[ ! -f "$SC_DATA_P4K" ]]; then
  echo "SC_DATA_P4K does not exist: $SC_DATA_P4K"
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

full_root="${output_prefix}_full"
ui_root="${output_prefix}_ui_only"
full_ui_list="${output_prefix}_full_ui_files.txt"
ui_ui_list="${output_prefix}_ui_ui_files.txt"
full_scene_list="${output_prefix}_full_scene_files.txt"
ui_scene_list="${output_prefix}_ui_scene_files.txt"

rm -rf "$full_root" "$ui_root"

pushd "$repo_root" >/dev/null
# Avoid skewing ratio with one-time compile/link overhead.
cargo build --release -p starbreaker >/dev/null 2>&1
popd >/dev/null

run_export() {
  local mode="$1"
  local out_root="$2"
  local start_ns end_ns elapsed_ms

  start_ns="$(date +%s%N)"
  if [[ "$mode" == "full" ]]; then
    cargo run --release -p starbreaker -- entity export "$entity_name" "$out_root" --kind decomposed --lod 0 --mip 0 --materials all >/dev/null 2>&1
  else
    cargo run --release -p starbreaker -- entity export "$entity_name" "$out_root" --kind decomposed --lod 0 --mip 0 --materials all --ui-only-files >/dev/null 2>&1
  fi
  end_ns="$(date +%s%N)"
  elapsed_ms=$(((end_ns - start_ns) / 1000000))
  echo "$elapsed_ms"
}

pushd "$repo_root" >/dev/null
full_ms="$(run_export full "$full_root")"
ui_ms="$(run_export ui "$ui_root")"
popd >/dev/null

if [[ ! -d "$full_root/Data/UI/Generated" || ! -d "$ui_root/Data/UI/Generated" ]]; then
  echo "Missing Data/UI/Generated in full or UI-only export"
  exit 1
fi

(
  cd "$full_root"
  find Data/UI/Generated -type f | sort
) >"$full_ui_list"
(
  cd "$ui_root"
  find Data/UI/Generated -type f | sort
) >"$ui_ui_list"

diff -u "$full_ui_list" "$ui_ui_list" >/dev/null

while IFS= read -r rel_path; do
  full_hash="$(sha256sum "$full_root/$rel_path" | awk '{print $1}')"
  ui_hash="$(sha256sum "$ui_root/$rel_path" | awk '{print $1}')"
  if [[ "$full_hash" != "$ui_hash" ]]; then
    echo "UI hash mismatch: $rel_path"
    exit 1
  fi
done <"$full_ui_list"

(
  cd "$full_root"
  find Packages -type f -name scene.json | sort
) >"$full_scene_list"
(
  cd "$ui_root"
  find Packages -type f -name scene.json | sort
) >"$ui_scene_list"

diff -u "$full_scene_list" "$ui_scene_list" >/dev/null

while IFS= read -r rel_path; do
  full_hash="$(sha256sum "$full_root/$rel_path" | awk '{print $1}')"
  ui_hash="$(sha256sum "$ui_root/$rel_path" | awk '{print $1}')"
  if [[ "$full_hash" != "$ui_hash" ]]; then
    echo "Scene sidecar hash mismatch: $rel_path"
    exit 1
  fi
done <"$full_scene_list"

max_ratio="${UI_ONLY_MAX_RUNTIME_RATIO:-1.10}"
ratio="$(awk -v full_ms="$full_ms" -v ui_ms="$ui_ms" 'BEGIN { if (full_ms == 0) { print 999.0 } else { printf "%.4f", ui_ms / full_ms } }')"

awk -v ratio="$ratio" -v max_ratio="$max_ratio" 'BEGIN { if (ratio > max_ratio) { exit 1 } }'

ui_count="$(wc -l < "$full_ui_list")"
echo "UI parity check passed for ${ui_count} Data/UI/Generated files"
echo "Full export runtime: ${full_ms} ms"
echo "UI-only export runtime: ${ui_ms} ms"
echo "Runtime ratio (ui/full): ${ratio} (limit ${max_ratio})"
