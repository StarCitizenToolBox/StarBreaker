//! Text renderer using `rusttype` and bundled DejaVu fonts.
//!
//! Renders word-wrapped, aligned text onto a mutable [`RgbaImage`] using
//! sub-pixel-quality rasterisation. Font data is `include_bytes!` embedded at
//! compile time so no font files need to be present at runtime.

use image::RgbaImage;
use rusttype::{Font, Point, Scale, point};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::bb_layout::Rect;
use crate::swf_assets::FontGlyphSet;

static SANS_BYTES: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
static MONO_BYTES: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");
const SWF_TEXT_WIDTH_CALIBRATION: f32 = 1.0;

/// Which DejaVu font family to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontKind {
    Sans,
    Mono,
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Centre,
    Right,
}

impl TextAlign {
    /// Parse the string as it appears in BB `textAlignment`.
    pub fn from_bb_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "left" => Self::Left,
            "center" | "centre" => Self::Centre,
            "right" => Self::Right,
            _ => Self::Left,
        }
    }
}

/// Stateless text renderer. Holds loaded `Font` instances.
pub struct TextRenderer {
    sans: Font<'static>,
    mono: Font<'static>,
}

impl TextRenderer {
    /// Construct by loading fonts from the embedded byte arrays.
    ///
    /// Panics if the embedded font bytes are invalid (compile-time packaging bug).
    pub fn new() -> Self {
        let sans = Font::try_from_bytes(SANS_BYTES).expect("embedded DejaVuSans.ttf is invalid");
        let mono = Font::try_from_bytes(MONO_BYTES).expect("embedded DejaVuSansMono.ttf is invalid");
        Self { sans, mono }
    }

    fn font(&self, kind: FontKind) -> &Font<'static> {
        match kind {
            FontKind::Sans => &self.sans,
            FontKind::Mono => &self.mono,
        }
    }

    /// Return the pixel width and height of `text` at `size_px` without word wrapping.
    pub fn measure(&self, text: &str, kind: FontKind, size_px: f32) -> (f32, f32) {
        let font = self.font(kind);
        let scale = Scale::uniform(size_px);
        let v_metrics = font.v_metrics(scale);
        let h = (v_metrics.ascent - v_metrics.descent).ceil();
        let w = line_advance_width(font, text, scale);
        (w, h)
    }

    /// Draw `text` into `img` clipped to `rect`, with alignment and vertical centering.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        img: &mut RgbaImage,
        text: &str,
        rect: Rect,
        kind: FontKind,
        size_px: f32,
        colour: [u8; 4],
        align: TextAlign,
    ) {
        if text.is_empty() || rect.w < 1.0 || rect.h < 1.0 || size_px < 1.0 {
            return;
        }

        let font = self.font(kind);
        let scale = Scale::uniform(size_px);
        let v_metrics = font.v_metrics(scale);
        let line_h = (v_metrics.ascent - v_metrics.descent + v_metrics.line_gap).ceil();

        let lines = wrap_lines(font, text, scale, rect.w);
        let total_h = lines.len() as f32 * line_h;
        let start_baseline = rect.y + ((rect.h - total_h) * 0.5).max(0.0) + v_metrics.ascent;

        let img_w = img.width() as i32;
        let img_h = img.height() as i32;
        let clip_min_x = rect.x.floor().max(0.0) as i32;
        let clip_min_y = rect.y.floor().max(0.0) as i32;
        let clip_max_x = (rect.x + rect.w).ceil().min(img.width() as f32) as i32;
        let clip_max_y = (rect.y + rect.h).ceil().min(img.height() as f32) as i32;

        for (i, line) in lines.iter().enumerate() {
            let baseline_y = start_baseline + i as f32 * line_h;
            let line_w = line_advance_width(font, line, scale);

            let start_x = match align {
                TextAlign::Left => rect.x,
                TextAlign::Centre => rect.x + ((rect.w - line_w) * 0.5).max(0.0),
                TextAlign::Right => rect.x + (rect.w - line_w).max(0.0),
            };

            let origin: Point<f32> = point(start_x, baseline_y);
            for g in font.layout(line, scale, origin) {
                if let Some(bb) = g.pixel_bounding_box() {
                    g.draw(|gx, gy, coverage| {
                        if coverage < 1e-4 {
                            return;
                        }
                        let px = bb.min.x + gx as i32;
                        let py = bb.min.y + gy as i32;
                        if px < clip_min_x
                            || py < clip_min_y
                            || px >= clip_max_x
                            || py >= clip_max_y
                            || px >= img_w
                            || py >= img_h
                        {
                            return;
                        }
                        let pixel = img.get_pixel_mut(px as u32, py as u32);
                        let src_a = coverage * colour[3] as f32 / 255.0;
                        let inv = 1.0 - src_a;
                        pixel[0] = (pixel[0] as f32).mul_add(inv, colour[0] as f32 * src_a) as u8;
                        pixel[1] = (pixel[1] as f32).mul_add(inv, colour[1] as f32 * src_a) as u8;
                        pixel[2] = (pixel[2] as f32).mul_add(inv, colour[2] as f32 * src_a) as u8;
                        pixel[3] = (pixel[3] as f32
                            + (1.0 - pixel[3] as f32 / 255.0) * src_a * 255.0)
                            .min(255.0) as u8;
                    });
                }
            }
        }
    }

    /// Draw using extracted SWF vector glyphs (`DefineFont2/3`) when available.
    ///
    /// Returns `true` when the draw path succeeded; `false` means callers should
    /// fall back to regular TTF rendering.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_swf_font(
        &self,
        img: &mut RgbaImage,
        text: &str,
        rect: Rect,
        swf_font: &FontGlyphSet,
        size_px: f32,
        colour: [u8; 4],
        align: TextAlign,
    ) -> bool {
        if text.is_empty() || rect.w < 1.0 || rect.h < 1.0 || size_px < 1.0 {
            return false;
        }

        let ascent = swf_font.ascent.map(|v| v as f32).unwrap_or(820.0);
        let descent = swf_font.descent.map(|v| v as f32).unwrap_or(-204.0);
        let line_gap = swf_font.leading.map(|v| v as f32).unwrap_or(0.0);
        let units_per_em = (ascent - descent).abs().max(1.0);
        if units_per_em <= 0.0 {
            return false;
        }

        let nominal_line_h = (((ascent - descent + line_gap) / units_per_em) * size_px).max(1.0);
        let lines = swf_wrap_lines(swf_font, text, units_per_em, size_px, rect.w);

        struct GlyphRun {
            path: tiny_skia::Path,
            x_px: f32,
        }

        struct LineLayout {
            width_px: f32,
            runs: Vec<GlyphRun>,
            min_y_units: f32,
            height_px: f32,
        }

        let scale = size_px / units_per_em;
        let scale_x = scale * SWF_TEXT_WIDTH_CALIBRATION;
        let mut layouts = Vec::new();
        for line in &lines {
            let mut pen_x = 0.0f32;
            let mut runs = Vec::new();
            let mut min_y_units = f32::INFINITY;
            let mut max_y_units = f32::NEG_INFINITY;

            for ch in line.chars() {
                if ch == ' ' {
                    pen_x += (size_px * 0.33).max(1.0);
                    continue;
                }

                let Some((glyph_idx, glyph)) = swf_lookup_glyph(swf_font, ch) else {
                    pen_x += (size_px * 0.5).max(1.0);
                    continue;
                };

                let mut pb = PathBuilder::new();
                if swf_glyph_to_path(&glyph.shape_records, &mut pb)
                    && let Some(path) = pb.finish()
                {
                    let bounds = path.bounds();
                    min_y_units = min_y_units.min(bounds.top());
                    max_y_units = max_y_units.max(bounds.bottom());
                    runs.push(GlyphRun { path, x_px: pen_x });
                }

                let adv = swf_glyph_advance_px(swf_font, glyph_idx, units_per_em, size_px);
                pen_x += adv.max(1.0);
            }

            if !min_y_units.is_finite() || !max_y_units.is_finite() {
                min_y_units = 0.0;
                max_y_units = units_per_em;
            }
            let glyph_h_px = ((max_y_units - min_y_units).abs() * scale).max(size_px * 0.7);
            layouts.push(LineLayout {
                width_px: pen_x,
                runs,
                min_y_units,
                height_px: glyph_h_px.max(nominal_line_h),
            });
        }

        let interline_px = (size_px * 0.12).max(1.0);
        let total_h = layouts
            .iter()
            .map(|l| l.height_px)
            .sum::<f32>()
            + interline_px * (layouts.len().saturating_sub(1) as f32);
        let mut line_top = rect.y + ((rect.h - total_h) * 0.5).max(0.0);

        let mut pixmap = match Pixmap::new(img.width(), img.height()) {
            Some(pm) => pm,
            None => return false,
        };

        for layout in &layouts {
            let start_x = match align {
                TextAlign::Left => rect.x,
                TextAlign::Centre => rect.x + ((rect.w - layout.width_px) * 0.5).max(0.0),
                TextAlign::Right => rect.x + (rect.w - layout.width_px).max(0.0),
            };
            let y_offset = line_top - layout.min_y_units * scale;

            for run in &layout.runs {
                let transform = Transform::from_row(
                    scale_x,
                    0.0,
                    0.0,
                    scale,
                    start_x + run.x_px,
                    y_offset,
                );

                let mut paint = Paint::default();
                paint.set_color_rgba8(colour[0], colour[1], colour[2], colour[3]);
                pixmap.fill_path(&run.path, &paint, FillRule::Winding, transform, None);
            }

            line_top += layout.height_px + interline_px;
        }

        blend_pixmap_onto_image(&pixmap, img, rect);
        true
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

fn wrap_lines(font: &Font<'_>, text: &str, scale: Scale, max_w: f32) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in words {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && line_advance_width(font, &candidate, scale) > max_w {
                result.push(current);
                current = word.to_owned();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    result
}

fn line_advance_width(font: &Font<'_>, text: &str, scale: Scale) -> f32 {
    font.layout(text, scale, point(0.0, 0.0))
        .map(|g| g.unpositioned().h_metrics().advance_width)
        .sum()
}

fn swf_lookup_glyph(swf_font: &FontGlyphSet, ch: char) -> Option<(usize, &crate::swf_assets::FontGlyph)> {
    swf_font
        .glyphs
        .iter()
        .enumerate()
        .find(|(_, g)| g.code == Some(ch as u16))
}

fn swf_glyph_advance_px(swf_font: &FontGlyphSet, glyph_idx: usize, units_per_em: f32, size_px: f32) -> f32 {
    let units = swf_font
        .glyphs
        .get(glyph_idx)
        .and_then(|g| g.advance)
        .map(|v| v as f32)
        .unwrap_or(320.0);
    (units / units_per_em) * size_px * SWF_TEXT_WIDTH_CALIBRATION
}

fn swf_line_width(text: &str, swf_font: &FontGlyphSet, units_per_em: f32, size_px: f32) -> f32 {
    text.chars().fold(0.0, |acc, ch| {
        if ch == ' ' {
            acc + (size_px * 0.33).max(1.0)
        } else if let Some((idx, _)) = swf_lookup_glyph(swf_font, ch) {
            acc + swf_glyph_advance_px(swf_font, idx, units_per_em, size_px).max(1.0)
        } else {
            acc + (size_px * 0.5).max(1.0)
        }
    })
}

fn swf_wrap_lines(
    swf_font: &FontGlyphSet,
    text: &str,
    units_per_em: f32,
    size_px: f32,
    max_w: f32,
) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in words {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty()
                && swf_line_width(&candidate, swf_font, units_per_em, size_px) > max_w
            {
                result.push(current);
                current = word.to_owned();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    result
}

fn swf_glyph_to_path(records: &[swf::ShapeRecord], pb: &mut PathBuilder) -> bool {
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut started = false;

    for rec in records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if let Some(move_to) = sc.move_to {
                    x = move_to.x.get() as f32;
                    y = move_to.y.get() as f32;
                    pb.move_to(x, y);
                    started = true;
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                if !started {
                    pb.move_to(x, y);
                    started = true;
                }
                x += delta.dx.get() as f32;
                y += delta.dy.get() as f32;
                pb.line_to(x, y);
            }
            swf::ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                if !started {
                    pb.move_to(x, y);
                    started = true;
                }
                let cx = x + control_delta.dx.get() as f32;
                let cy = y + control_delta.dy.get() as f32;
                x = cx + anchor_delta.dx.get() as f32;
                y = cy + anchor_delta.dy.get() as f32;
                pb.quad_to(cx, cy, x, y);
            }
        }
    }

    started
}

fn blend_pixmap_onto_image(pixmap: &Pixmap, img: &mut RgbaImage, clip: Rect) {
    let img_w = img.width() as i32;
    let img_h = img.height() as i32;
    let clip_min_x = clip.x.floor().max(0.0) as i32;
    let clip_min_y = clip.y.floor().max(0.0) as i32;
    let clip_max_x = (clip.x + clip.w).ceil().min(img.width() as f32) as i32;
    let clip_max_y = (clip.y + clip.h).ceil().min(img.height() as f32) as i32;

    for py in clip_min_y..clip_max_y {
        for px in clip_min_x..clip_max_x {
            if px < 0 || py < 0 || px >= img_w || py >= img_h {
                continue;
            }
            let Some(src) = pixmap.pixel(px as u32, py as u32) else {
                continue;
            };
            if src.alpha() == 0 {
                continue;
            }
            let src_a = src.alpha() as f32 / 255.0;
            let dst = img.get_pixel_mut(px as u32, py as u32);
            let inv = 1.0 - src_a;
            // tiny-skia pixels are premultiplied; channels already include alpha.
            dst[0] = (src.red() as f32 + dst[0] as f32 * inv).min(255.0) as u8;
            dst[1] = (src.green() as f32 + dst[1] as f32 * inv).min(255.0) as u8;
            dst[2] = (src.blue() as f32 + dst[2] as f32 * inv).min(255.0) as u8;
            dst[3] = (dst[3] as f32 + (1.0 - dst[3] as f32 / 255.0) * src_a * 255.0).min(255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn make_img(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 255]))
    }

    #[test]
    fn measure_nonempty() {
        let r = TextRenderer::new();
        let (w, h) = r.measure("HELLO", FontKind::Sans, 14.0);
        assert!(w > 0.0, "expected positive width, got {w}");
        assert!(h > 0.0, "expected positive height, got {h}");
    }

    #[test]
    fn draw_leaves_nonblack_pixels() {
        let r = TextRenderer::new();
        let mut img = make_img(128, 32);
        let rect = Rect { x: 2.0, y: 2.0, w: 120.0, h: 28.0 };
        r.draw(&mut img, "HI", rect, FontKind::Sans, 14.0, [255, 255, 255, 255], TextAlign::Left);
        let changed = img.pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0);
        assert!(changed, "no pixels changed after draw");
    }

    #[test]
    fn draw_empty_string_does_nothing() {
        let r = TextRenderer::new();
        let mut img = make_img(64, 16);
        let before: Vec<_> = img.pixels().copied().collect();
        let rect = Rect { x: 0.0, y: 0.0, w: 64.0, h: 16.0 };
        r.draw(&mut img, "", rect, FontKind::Sans, 12.0, [255, 0, 0, 255], TextAlign::Left);
        let after: Vec<_> = img.pixels().copied().collect();
        assert_eq!(before, after, "draw of empty string mutated the image");
    }

    #[test]
    fn measure_mono_and_sans_return_positive_widths() {
        let r = TextRenderer::new();
        let (ws, _) = r.measure("TEST", FontKind::Sans, 12.0);
        let (wm, _) = r.measure("TEST", FontKind::Mono, 12.0);
        assert!(ws > 0.0 && wm > 0.0);
    }
}
