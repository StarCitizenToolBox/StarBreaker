#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"
cargo build --release -p starbreaker -p starbreaker-mcp

cd "$REPO_ROOT/app"

pkill -f '/target/debug/starbreaker-app|/target/release/starbreaker-app' >/dev/null 2>&1 || true
npm run tauri build -- --no-sign
