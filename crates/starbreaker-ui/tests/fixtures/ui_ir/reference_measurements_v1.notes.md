# reference_measurements_v1.json — provenance notes

The measurement bank (plan P1.4, ledger item 19): settled in-game reference
measurements so pixel adjudications are lookups, not re-measurements.
Adjudications consult this bank FIRST (`crates/starbreaker-ui/docs/ui-workflow.md` §4).

## Provenance

- **Captures**: manual in-game screenshots in
  `~/projects/scorg_tools/reference/in-game/Clipper/` (workspace
  path; the `capture` field is workspace-relative). NOT rectified — all
  boxes are raw capture pixels. `Screen_Left_Lower_RTT.png` is 1600x1200
  (matches the render space); `Screen_Right_Upper_RTT.png` is **1959x1513**
  (scale by 1600/1959 before comparing against renders).
- **Method**: `python3 scripts/ui_measure.py <capture> --box <box> --delta
  <delta from the entry's notes>` (the contamination-guarded measurer,
  plan P1.2). `cap_height_px` = the tallest non-suspect bright-row run.
- **Measured**: 2026-06-12, re-measured with the tool from the values
  settled during the 2026-06-12 power/MFD text arc
  (`crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md` §13-§16).

## Re-measurement deltas vs the arc's session numbers

Most arc numbers reproduced exactly (footer P 52, IR 53, 3.5K 54, 294.1 55,
OUTPUT 41, "2" 93, /16 58, ºC 37, NO TARGET 77, footer 64). Three battery-
card entries did NOT and are recorded at the tool-reproduced value, flagged
SOFT in their notes:

| element | arc value | bank value | why soft |
|---|---|---|---|
| power.battery.zeros | 53 | 51 | dim card, AA-sensitive at low delta |
| power.battery.slash | 58 | 55 | same |
| power.battery.OFFLINE | 43 | 49 | the 43 was already "unexplained (band-fit?)" in the handoff §13 |

The bank records what `ui_measure.py` reproduces — a value nobody can
re-derive is not an adjudication anchor. Revisit these three when the P3
text-calibration arc touches battery-card text.

## Traps carried in entry notes

- power footer: the bar line crosses the box (the ledger-19 trap that
  produced the wrong "ink x1.4" verdict) — the tool flags it suspect.
- power "2": keep the box above the card underline (a wide box once
  measured cap 218).
- power ºC: bright pip slab ends ~x1125; keep x0 >= 1150.
- target screen: 1959-wide capture — every target.* value is in capture
  scale (@1959w).

## Updating

Append entries via the same method (tool + delta + box in notes). No
machine validator exists (references cannot be re-derived from game data) —
this file IS the audit trail: record capture, date, method, and any
disagreement with prior numbers.
