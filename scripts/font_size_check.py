#!/usr/bin/env python3
"""UI font-size regression check.

Compares an `SB_UI_FONT_DUMP` capture against the frozen baseline
(`crates/starbreaker-ui/tests/fixtures/font_size_baseline.tsv`), which holds the
committed rendered size of every text element on the four platinum/gold Clipper
screens and is within ~1-2% of the in-game references.

See `docs/ui-font-size-harness.md` for the full workflow.

Usage:
    # 1. capture a dump from any render (stderr -> file):
    SB_UI_FONT_DUMP=1 cargo run -q -p starbreaker -- ui render \
        --scene "<scene.json>" --out-dir /tmp/x 2>&1 \
        | grep '^FONTDUMP' > /tmp/font_dump.tsv

    # 2. check it:
    python3 scripts/font_size_check.py /tmp/font_dump.tsv [tolerance_pct] [baseline.tsv]

Each FONTDUMP line:
    FONTDUMP \\t canvas \\t node \\t font \\t size_px \\t visible_px \\t em \\t text
The held invariant is VISIBLE_px (final rendered text height), keyed by
(canvas, node, text). Reports every element that drifts beyond `tolerance_pct`
(default 2.5%) and exits non-zero on any drift, so it can gate CI.
"""
import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_BASELINE = os.path.join(
    REPO, "crates/starbreaker-ui/tests/fixtures/font_size_baseline.tsv"
)
# The four platinum/gold regression targets (see docs/ui-regression-policy.md).
TARGETS = (
    "I_Med_MedicalBed_A",          # platinum  (ui_target_a)
    "I_Med_MedicalEndOfBed_A",     # platinum  (ui_target_b)
    "I_Door_Small_DRAK",           # gold      (clipper_small_door)
    "H_Eng_Annunciator_Master_Left",  # gold   (eng_annunciator_master_left)
)


def harness_error(msg):
    """A harness error means the CHECKER or dump format broke — distinct from
    font drift (exit 1) so format rot can never masquerade as MISSING data
    (docs/ui-process-improvements.md item 3)."""
    print(f"HARNESS ERROR: {msg}", file=sys.stderr)
    sys.exit(2)


def load(path):
    """Return ({(canvas, node, text): visible_px} for target canvases,
    total FONTDUMP line count, set of unexpected cell counts)."""
    out = {}
    total = 0
    odd_widths = set()
    for line in open(path):
        if not line.startswith("FONTDUMP"):
            continue
        total += 1
        cells = [c for c in line.rstrip("\n").split("\t") if c != "FONTDUMP"]
        # 7 data cells = pre-width_px format; 8 = current. Anything else is
        # format drift the checker does not understand.
        if len(cells) not in (7, 8):
            odd_widths.add(len(cells))
            continue
        # Head fields are positional; text is always LAST (the dump gained a
        # width_px column after the baseline was captured — both layouts parse).
        canvas, node, _font, _size, visible, _em = cells[:6]
        text = cells[-1]
        canvas = canvas.replace("BuildingBlocks_Canvas.", "")
        if canvas not in TARGETS:
            continue
        out[(canvas, node, text)] = float(visible)
    return out, total, odd_widths


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    new_path = sys.argv[1]
    tol = float(sys.argv[2]) if len(sys.argv) > 2 else 2.5
    baseline = sys.argv[3] if len(sys.argv) > 3 else DEFAULT_BASELINE

    base, base_total, base_odd = load(baseline)
    new, new_total, new_odd = load(new_path)

    if base_odd or new_odd:
        harness_error(
            f"unexpected FONTDUMP cell counts {sorted(base_odd | new_odd)} "
            "(expected 7 or 8 data cells) — the dump format changed; update "
            "this checker and docs/ui-font-size-harness.md"
        )
    if new_total == 0:
        harness_error(f"no FONTDUMP lines in {new_path} — was SB_UI_FONT_DUMP=1 set?")
    if base and new_total > 0 and not (set(base) & set(new)):
        harness_error(
            f"dump has {new_total} FONTDUMP lines but ZERO match the baseline "
            "keys — wrong scene (the baseline canvases live in the Clipper "
            "LOD1 interior scene) or key-format drift; this is not font drift"
        )
    bad = []
    for key, bv in sorted(base.items()):
        nv = new.get(key)
        if nv is None:
            bad.append((key, bv, None, None))
        else:
            pct = (nv - bv) / bv * 100.0
            if abs(pct) > tol:
                bad.append((key, bv, nv, pct))

    print(f"baseline elements: {len(base)}  matched within {tol}%: {len(base) - len(bad)}")
    for (canvas, node, text), bv, nv, pct in bad:
        if nv is None:
            print(f"  MISSING  {canvas:26} {node:20} base={bv:7.2f}  '{text[:24]}'")
        else:
            print(f"  {pct:+6.1f}%  {canvas:26} {node:20} base={bv:7.2f} new={nv:7.2f}  '{text[:24]}'")
    print("PASS" if not bad else f"FAIL ({len(bad)} drifted)")
    sys.exit(0 if not bad else 1)


if __name__ == "__main__":
    main()
