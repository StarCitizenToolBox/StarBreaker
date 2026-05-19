//! Text renderer using `rusttype` and bundled DejaVu fonts.
//!
//! Renders word-wrapped, aligned text onto a mutable [`RgbaImage`] using
//! sub-pixel-quality rasterisation. Font data is `include_bytes!` embedded at
//! compile time so no font files need to be present at runtime.

use image::RgbaImage;
use rusttype::{Font, Point, Scale, point};

use crate::bb_layout::Rect;

static SANS_BYTES: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
static MONO_BYTES: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");

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
            let mut n_pixels = 0u32;
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
                        n_pixels += 1;
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
