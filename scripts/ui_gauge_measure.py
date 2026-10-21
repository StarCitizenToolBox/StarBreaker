#!/usr/bin/env python3
"""Geometry measurements for the circular cockpit HUD gauges (g-force / velocity
ball, countermeasures, radar, compass).

Consolidates the ad-hoc measurements that the g-force parity arc re-wrote >2x
(ledger item 54): the centre-dot position + circularity, the cross-arm V/H
symmetry, and the cardinal-marker on-axis offsets. Run it on a gauge render (and
optionally its reference) instead of hand-writing one-off numpy snippets.

    python3 scripts/ui_gauge_measure.py <render.png> [reference.png] \
        [--montage out.png]

Reports (JSON to stdout):
  * white_dot: centre (x,y), offset from the geometric centre, and a circularity
    ratio (filled area / inscribed-circle area: ~1.00 = circle, ~1.27 = square,
    >1.04 = squircle).
  * cross: orange-axis arm reach up/down/left/right from the dot and the V/H span
    ratio (1.00 = square cross).
  * cardinals: per-direction ring centroid with its perpendicular offset from the
    axis (0 = on-axis) and radius (as a fraction of the half-extent).

With a reference image the same metrics are emitted for it too, plus a
centre-aligned side-by-side montage (`--montage`) at a common scale so marker
positions line up for a vision read.

Colour gate is the generic HUD-orange used across the gauge glyphs; tweak with
--orange-min-r / --orange-max-g if a brand renders a different surface colour.
"""
import argparse
import json
import math
import sys

import numpy as np
from PIL import Image


def load(path):
    return np.asarray(Image.open(path).convert("RGB")).astype(float)


def orange_mask(a, min_r, max_g):
    r, g, b = a[..., 0], a[..., 1], a[..., 2]
    return (r > min_r) & (g > 60) & (g < max_g) & (b < 125) & (r - b > 55)


def white_dot(a):
    """Brightest near-white blob = the gauge centre dot."""
    mn = np.minimum(np.minimum(a[..., 0], a[..., 1]), a[..., 2])
    spread = a.max(axis=2) - a.min(axis=2)
    mask = (mn > 200) & (spread < 40)
    ys, xs = np.where(mask)
    if len(xs) == 0:
        return None
    bx = xs.max() - xs.min() + 1
    by = ys.max() - ys.min() + 1
    area = len(xs)
    diam = (bx + by) / 2.0
    circle = math.pi * (diam / 2.0) ** 2
    return {
        "x": float(xs.mean()),
        "y": float(ys.mean()),
        "bbox_w": int(bx),
        "bbox_h": int(by),
        "area_px": int(area),
        "circularity_ratio": round(area / circle, 3) if circle > 0 else None,
    }


def cross_and_cardinals(a, cx, cy, min_r, max_g):
    om = orange_mask(a, min_r, max_g)
    H, W = a.shape[:2]
    cxi, cyi = int(round(cx)), int(round(cy))
    band = max(2, int(min(W, H) * 0.02))

    col = om[:, max(0, cxi - band):cxi + band].any(axis=1)
    ys = np.where(col)[0]
    up = cyi - ys[ys < cyi].min() if (ys < cyi).any() else 0
    dn = ys[ys > cyi].max() - cyi if (ys > cyi).any() else 0
    row = om[max(0, cyi - band):cyi + band, :].any(axis=0)
    xs = np.where(row)[0]
    lf = cxi - xs[xs < cxi].min() if (xs < cxi).any() else 0
    rt = xs[xs > cxi].max() - cxi if (xs > cxi).any() else 0
    vspan, hspan = up + dn, lf + rt
    cross = {
        "up": int(up), "down": int(dn), "left": int(lf), "right": int(rt),
        "v_over_h": round(vspan / hspan, 3) if hspan else None,
    }

    # Cardinal ring centroids in tight, diagonal-free windows.
    cards = {}
    win = int(min(W, H) * 0.10)
    for name, (dx, dy) in {"top": (0, -1), "bottom": (0, 1),
                           "left": (-1, 0), "right": (1, 0)}.items():
        wx = cx + dx * 0.78 * (W / 2)
        wy = cy + dy * 0.78 * (H / 2)
        x0, x1 = int(max(0, wx - win)), int(min(W, wx + win))
        y0, y1 = int(max(0, wy - win)), int(min(H, wy + win))
        sub = om[y0:y1, x0:x1]
        sy, sx = np.where(sub)
        if len(sx) == 0:
            cards[name] = None
            continue
        mx, my = sx.mean() + x0, sy.mean() + y0
        if name in ("top", "bottom"):
            perp, along, half = mx - cx, abs(my - cy), H / 2
        else:
            perp, along, half = my - cy, abs(mx - cx), W / 2
        cards[name] = {
            "x": round(float(mx), 1), "y": round(float(my), 1),
            "perp_offset": round(float(perp), 1),
            "radius_frac": round(float(along) / half, 3) if half else None,
        }
    return cross, cards


def measure(path, min_r, max_g):
    a = load(path)
    H, W = a.shape[:2]
    dot = white_dot(a)
    cx = dot["x"] if dot else W / 2.0
    cy = dot["y"] if dot else H / 2.0
    cross, cards = cross_and_cardinals(a, cx, cy, min_r, max_g)
    if dot:
        dot["dx_from_centre"] = round(dot["x"] - W / 2.0, 1)
        dot["dy_from_centre"] = round(dot["y"] - H / 2.0, 1)
    return {"size": [W, H], "white_dot": dot, "cross": cross, "cardinals": cards}


def montage(render, reference, out):
    r = Image.open(render).convert("RGB")
    f = Image.open(reference).convert("RGB")

    def centre_crop(im, frac=0.9, sz=480):
        w, h = im.size
        half = int(min(w, h) * frac / 2)
        return im.crop((w // 2 - half, h // 2 - half,
                        w // 2 + half, h // 2 + half)).resize((sz, sz), Image.LANCZOS)

    canvas = Image.new("RGB", (976, 480), (12, 12, 12))
    canvas.paste(centre_crop(r), (0, 0))
    canvas.paste(centre_crop(f), (496, 0))
    canvas.save(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("render")
    ap.add_argument("reference", nargs="?")
    ap.add_argument("--montage", help="write a centre-aligned render|reference montage")
    ap.add_argument("--orange-min-r", type=int, default=140)
    ap.add_argument("--orange-max-g", type=int, default=195)
    args = ap.parse_args()

    out = {"render": measure(args.render, args.orange_min_r, args.orange_max_g)}
    if args.reference:
        out["reference"] = measure(args.reference, args.orange_min_r, args.orange_max_g)
        if args.montage:
            montage(args.render, args.reference, args.montage)
            out["montage"] = args.montage
    json.dump(out, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
