//! Render-diff verifier: rasterize one glyph two ways and pixel-diff.
//!
//! Path A — load the SWF, find the glyph by Unicode codepoint, render its
//! shape records directly to a pixmap via tiny-skia. This is the "ground
//! truth" — exactly what Scaleform/Ruffle would render, because they
//! operate on the same shape records.
//!
//! Path B — load the extracted TTF, fetch the same glyph, render its
//! outline via ttf-parser+tiny-skia. This is the "test" — exercises every
//! step of our SWF→TTF conversion.
//!
//! Both render to the same canvas size with the same baseline and scale,
//! so a pixel-perfect (or near-perfect) match proves our conversion is
//! mathematically faithful.
//!
//! Usage: `cargo run -p starbreaker-swf --example render_diff -- \
//!   <swf> <ttf> <char> <output-dir>`
//! produces: swf.png, ttf.png, diff.png, plus a printed summary.

use std::path::PathBuf;
use swf::{Tag, decompress_swf, parse_swf};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

const CANVAS: u32 = 512;
const GLYPH_EM_RATIO: f32 = 0.7; // how much of canvas the em-box fills
const X_OFFSET_RATIO: f32 = 0.15; // padding-left fraction
const Y_BASELINE_RATIO: f32 = 0.8; // baseline y as fraction of canvas height

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let swf_path: PathBuf = args.next().expect("usage: render_diff <swf> <ttf> <char> <outdir>").into();
    let ttf_path: PathBuf = args.next().expect("missing ttf").into();
    let test_char: char = args.next().expect("missing char").chars().next().unwrap();
    let outdir: PathBuf = args.next().expect("missing outdir").into();
    std::fs::create_dir_all(&outdir)?;

    let swf_bytes = std::fs::read(&swf_path)?;
    let ttf_bytes = std::fs::read(&ttf_path)?;

    // ─── locate the SWF font whose family matches the TTF's family ──────
    let ttf_face = ttf_parser::Face::parse(&ttf_bytes, 0)?;
    let ttf_family = ttf_face
        .names()
        .into_iter()
        .find(|n| n.name_id == 1)
        .and_then(|n| n.to_string())
        .unwrap_or_default();
    let ttf_subfamily = ttf_face
        .names()
        .into_iter()
        .find(|n| n.name_id == 2)
        .and_then(|n| n.to_string())
        .unwrap_or_default();
    let want_bold = ttf_subfamily.contains("Bold");
    let want_italic = ttf_subfamily.contains("Italic");
    println!("TTF: {ttf_family} ({ttf_subfamily})");

    let buf = decompress_swf(&swf_bytes[..])?;
    let swf_data = parse_swf(&buf)?;
    let mut chosen_font: Option<&swf::Font> = None;
    for tag in &swf_data.tags {
        if let Tag::DefineFont2(f) = tag {
            let name = f.name.to_string_lossy(swf::UTF_8);
            if name == ttf_family
                && f.flags.contains(swf::FontFlag::IS_BOLD) == want_bold
                && f.flags.contains(swf::FontFlag::IS_ITALIC) == want_italic
            {
                chosen_font = Some(f);
                break;
            }
        }
    }
    let font = chosen_font.ok_or_else(|| anyhow::anyhow!("no matching font in SWF for {ttf_family}"))?;

    let target_glyph = font
        .glyphs
        .iter()
        .find(|g| g.code == test_char as u16)
        .ok_or_else(|| anyhow::anyhow!("no glyph for {test_char:?} in SWF font"))?;
    println!("SWF: glyph for {test_char:?}, advance={}", target_glyph.advance);

    // ─── Path A: render SWF shape directly ──────────────────────────────
    // SWF DefineFont3 em = 20480 twips
    const SWF_EM: f32 = 20480.0;
    let scale_a = (CANVAS as f32 * GLYPH_EM_RATIO) / SWF_EM;
    let baseline_y = CANVAS as f32 * Y_BASELINE_RATIO;
    let x_left = CANVAS as f32 * X_OFFSET_RATIO;

    let path_a = build_path_from_swf(&target_glyph.shape_records, scale_a, x_left, baseline_y);
    let pixmap_a = rasterize(&path_a, CANVAS);

    // ─── Path B: render TTF glyph via ttf-parser ────────────────────────
    let gid = ttf_face
        .glyph_index(test_char)
        .ok_or_else(|| anyhow::anyhow!("no glyph for {test_char:?} in TTF"))?;
    let upem = ttf_face.units_per_em() as f32;
    let scale_b = (CANVAS as f32 * GLYPH_EM_RATIO) / upem;
    let mut tsb = TinySkiaBuilder::new(scale_b, x_left, baseline_y);
    let _ = ttf_face.outline_glyph(gid, &mut tsb);
    let path_b = tsb.finish();
    let pixmap_b = rasterize(&path_b, CANVAS);

    // ─── Diff ──────────────────────────────────────────────────────────
    let pa = pixmap_a.data();
    let pb = pixmap_b.data();
    let mut diff = Pixmap::new(CANVAS, CANVAS).unwrap();
    let pd = diff.data_mut();
    let mut max_delta: u32 = 0;
    let mut sum_delta: u64 = 0;
    let mut differ_pixels: u32 = 0;
    let n_pixels = (CANVAS * CANVAS) as usize;

    for i in 0..n_pixels {
        let off = i * 4;
        // Black-on-white render: alpha channel is the signal. Compare alpha.
        let a = pa[off + 3];
        let b = pb[off + 3];
        let delta = (a as i32 - b as i32).unsigned_abs();
        sum_delta += delta as u64;
        if delta > max_delta as u32 {
            max_delta = delta as u32;
        }
        if delta > 1 {
            differ_pixels += 1;
        }
        // Diff visual: red where A has coverage missing from B, blue inverse, grey overlap
        let r = (a as i32 - b as i32).max(0) as u8; // A but not B
        let bl = (b as i32 - a as i32).max(0) as u8; // B but not A
        let overlap = a.min(b) / 2;
        pd[off] = (r as u16 + overlap as u16).min(255) as u8;
        pd[off + 1] = overlap;
        pd[off + 2] = (bl as u16 + overlap as u16).min(255) as u8;
        pd[off + 3] = 255;
    }

    let swf_out = outdir.join(format!("swf-{test_char}.png"));
    let ttf_out = outdir.join(format!("ttf-{test_char}.png"));
    let diff_out = outdir.join(format!("diff-{test_char}.png"));
    pixmap_a.save_png(&swf_out)?;
    pixmap_b.save_png(&ttf_out)?;
    diff.save_png(&diff_out)?;

    let avg_delta = sum_delta as f64 / n_pixels as f64;
    let differ_pct = differ_pixels as f64 / n_pixels as f64 * 100.0;
    println!("\n=== Render diff for {test_char:?} ===");
    println!("  Canvas:        {CANVAS}x{CANVAS}");
    println!("  Max α delta:   {} / 255", max_delta);
    println!("  Avg α delta:   {:.3} / 255", avg_delta);
    println!("  Pixels δ > 1:  {} ({:.3}%)", differ_pixels, differ_pct);
    println!("  swf:  {}", swf_out.display());
    println!("  ttf:  {}", ttf_out.display());
    println!("  diff: {} (red=SWF only, blue=TTF only, grey=overlap)", diff_out.display());

    if max_delta <= 2 {
        println!("\n  ✓ Identical (sub-pixel rounding noise only)");
    } else if avg_delta < 1.0 && differ_pct < 0.5 {
        println!("\n  ~ Visually identical (minor anti-alias differences at edges)");
    } else {
        println!("\n  ✗ NOTABLE DIFFERENCES — inspect diff.png");
    }

    Ok(())
}

/// Build a tiny-skia Path from SWF shape records, mapping SWF coords (twips,
/// Y-down) → pixmap coords (pixels, Y-down). Y is NOT flipped because SWF
/// and pixmap both use Y-down; we just scale and offset.
fn build_path_from_swf(
    records: &[swf::ShapeRecord],
    scale: f32,
    x_offset: f32,
    y_baseline: f32,
) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;
    let mut have_subpath = false;
    // SWF uses Y-positive-DOWN. Glyph baseline is at y_swf=0; ascender
    // tops have NEGATIVE y_swf (above baseline visually). The pixmap is
    // also Y-down, so we just add: pixmap_y = baseline + y_swf * scale.
    let map = |x: f32, y: f32| (x * scale + x_offset, y_baseline + y * scale);

    for rec in records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if let Some(mv) = sc.move_to {
                    if have_subpath {
                        pb.close();
                    }
                    pen_x = mv.x.get() as f32;
                    pen_y = mv.y.get() as f32;
                    let (mx, my) = map(pen_x, pen_y);
                    pb.move_to(mx, my);
                    have_subpath = true;
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                if !have_subpath {
                    continue;
                }
                pen_x += delta.dx.get() as f32;
                pen_y += delta.dy.get() as f32;
                let (lx, ly) = map(pen_x, pen_y);
                pb.line_to(lx, ly);
            }
            swf::ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                if !have_subpath {
                    continue;
                }
                let cx = pen_x + control_delta.dx.get() as f32;
                let cy = pen_y + control_delta.dy.get() as f32;
                pen_x = cx + anchor_delta.dx.get() as f32;
                pen_y = cy + anchor_delta.dy.get() as f32;
                let (cxp, cyp) = map(cx, cy);
                let (axp, ayp) = map(pen_x, pen_y);
                pb.quad_to(cxp, cyp, axp, ayp);
            }
        }
    }
    if have_subpath {
        pb.close();
    }
    pb.finish().expect("non-empty path")
}

/// ttf-parser OutlineBuilder that writes into a tiny-skia PathBuilder.
/// TTF coords are Y-up; we flip to Y-down (pixmap convention) here.
struct TinySkiaBuilder {
    pb: PathBuilder,
    scale: f32,
    x_off: f32,
    y_baseline: f32,
    has_path: bool,
}
impl TinySkiaBuilder {
    fn new(scale: f32, x_off: f32, y_baseline: f32) -> Self {
        Self {
            pb: PathBuilder::new(),
            scale,
            x_off,
            y_baseline,
            has_path: false,
        }
    }
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + self.x_off, self.y_baseline - y * self.scale)
    }
    fn finish(self) -> tiny_skia::Path {
        self.pb.finish().expect("non-empty path")
    }
}
impl ttf_parser::OutlineBuilder for TinySkiaBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        let (mx, my) = self.map(x, y);
        self.pb.move_to(mx, my);
        self.has_path = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (lx, ly) = self.map(x, y);
        self.pb.line_to(lx, ly);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (cxp, cyp) = self.map(cx, cy);
        let (axp, ayp) = self.map(x, y);
        self.pb.quad_to(cxp, cyp, axp, ayp);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        // TTF SimpleGlyph never emits cubics; we don't expect this.
    }
    fn close(&mut self) {
        if self.has_path {
            self.pb.close();
        }
    }
}

fn rasterize(path: &tiny_skia::Path, size: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(size, size).expect("pixmap alloc");
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    paint.anti_alias = true;
    pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
    pixmap
}
