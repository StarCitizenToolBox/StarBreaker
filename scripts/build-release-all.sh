#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT/app"

if command -v pgrep >/dev/null 2>&1; then
  while IFS= read -r pid; do
    [[ -n "$pid" ]] || continue
    kill "$pid" >/dev/null 2>&1 || true
  done < <(pgrep -f '/target/debug/starbreaker-app|/target/release/starbreaker-app' || true)
fi

npm run tauri build -- --no-sign --ci --bundles appimage

cd "$REPO_ROOT"
cargo build --release -p starbreaker -p starbreaker-mcp
