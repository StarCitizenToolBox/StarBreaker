# brand_palettes_v1.json — provenance

Extracted 2026-06-12 from the decompiled DataCore record mirror
(`dcb_canvas/libs/foundry/records/ui/buildingblocks/styles/`):

| brand key | record | record id |
|---|---|---|
| `s_bioc` | `BuildingBlocks_Style.S_BIOC` | `78e134f3-caa9-4aed-88bc-d71c706a62f3` |
| `s_drak_hud` | `BuildingBlocks_Style.S_DRAK_HUD` | `2adcc682-7322-4e06-92f5-9aa19f222d93` |

`colorStyles` is the verbatim 17-slot palette array in `BB_ColorStyle` enum
order (0 Base … 6 Bright … 16 MissionObjectives; see
`crates/starbreaker-ui/docs/ui-architecture-runbook.md`). `null` entries mirror authored null
slots.

Purpose: unit-test fixtures need REAL brand palette values but must run
without game data (hosted CI). Hard-coding palette values in test source is
banned (`rgba_colour_literals_are_not_hardcoded` guard,
`crates/starbreaker-ui/AGENTS.md` Core rules); tests load this fixture via
`crate::test_palettes` instead. When game data is available,
`brand_palette_fixture_matches_live_records` (same guard file) re-validates
the fixture against the live records — refresh this file (and this note)
when upstream patches change a palette.
