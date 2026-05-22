#!/usr/bin/env bash
set -euo pipefail

checks=(
  "crates/starbreaker-ui/src/ir_compose.rs::is_medical1_layout"
  "crates/starbreaker-ui/src/ir_compose.rs::medical_cyan_tint"
  "crates/starbreaker-ui/src/ir_compose.rs::Top_seperator"
  "crates/starbreaker-ui/src/ir_compose.rs::MedGelFillMeter"
  "crates/starbreaker-ui/src/ir_compose.rs::i_med_bioc_bottom-bar"
  "crates/starbreaker-ui/src/ir_compose.rs::BGDots"
  "crates/starbreaker-ui/src/ir_compose.rs::MainMenuCanvas"

  "crates/starbreaker-ui/src/compose.rs::base_animatedelements"
  "crates/starbreaker-ui/src/compose.rs::BGDots"
  "crates/starbreaker-ui/src/compose.rs::MainMenuCanvas"
  "crates/starbreaker-ui/src/compose.rs::s_bioc"
  "crates/starbreaker-ui/src/compose.rs::s_rsi"
  "crates/starbreaker-ui/src/compose.rs::s_aegs"

  "crates/starbreaker-ui/src/ui_ir.rs::nominal_font_size_from_label_style"
  "crates/starbreaker-ui/src/ui_ir.rs::BGDots"
  "crates/starbreaker-ui/src/ui_ir.rs::MainMenuCanvas"
  "crates/starbreaker-ui/src/ui_ir.rs::base_animatedelements"
)

failed=0
for check in "${checks[@]}"; do
  file="${check%%::*}"
  marker="${check##*::}"
  if rg --fixed-strings --line-number -- "$marker" "$file" >/dev/null; then
    echo "Hardcoding marker found: $marker in $file"
    rg --fixed-strings --line-number -- "$marker" "$file" || true
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  echo "UI hardcoding guard failed."
  exit 1
fi

echo "UI hardcoding guard passed."
