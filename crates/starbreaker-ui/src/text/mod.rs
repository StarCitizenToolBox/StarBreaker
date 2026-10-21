//! Text renderer using `rusttype` and bundled DejaVu fonts.

use rusttype::Font;

mod swf_draw;
mod ttf_draw;
#[cfg(test)]
mod tests;

pub use ttf_draw::TextRenderer;

use std::cell::RefCell;

thread_local! {
    /// Per-call (canvas, node) context for the optional `SB_UI_FONT_DUMP` font-size
    /// audit. Set by the IR text drawer before each `draw_swf_font` call so the dump
    /// can key each rendered text element to its source node.
    ///
    /// Diagnostic tooling only: it has no effect on rendering and emits nothing
    /// unless the `SB_UI_FONT_DUMP` environment variable is set. Used with
    /// `font_size_baseline.tsv` / `font_size_check.py` (workspace root) to verify
    /// per-element rendered font sizes against the frozen platinum/gold baseline.
    pub(crate) static FONT_DUMP_CTX: RefCell<(String, String)> =
        const { RefCell::new((String::new(), String::new())) };
}

/// Record the (canvas, node) for the next `draw_swf_font` font-size dump line.
/// No-op for rendering; only consumed by the `SB_UI_FONT_DUMP` audit emitter.
pub(crate) fn set_font_dump_ctx(canvas: &str, node: &str) {
    FONT_DUMP_CTX.with(|ctx| {
        let mut c = ctx.borrow_mut();
        c.0.clear();
        c.0.push_str(canvas);
        c.1.clear();
        c.1.push_str(node);
    });
}

static SANS_BYTES: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");
static MONO_BYTES: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");
pub(super) const SWF_TEXT_WIDTH_CALIBRATION: f32 = 1.0;

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

/// Vertical text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Top,
    Centre,
    Bottom,
    /// GFx line-box placement for anchor-centred labels: the full em sits
    /// ABOVE the baseline and the box is centred on the rect centre, so the
    /// baseline lands at `centre + em/2` (one descent below cap-centring).
    /// Verified against the Clipper target screen's NO TARGET heading.
    EmBaseline,
}

impl VerticalAlign {
    /// Parse the string as it appears in BB `verticalTextAlignment`.
    pub fn from_bb_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            "center" | "centre" => Self::Centre,
            _ => Self::Centre,
        }
    }
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
pub struct FontStore {
    pub(super) sans: Font<'static>,
    pub(super) mono: Font<'static>,
}

impl FontStore {
    pub(super) fn new() -> Self {
        let sans = Font::try_from_bytes(SANS_BYTES).expect("embedded DejaVuSans.ttf is invalid");
        let mono = Font::try_from_bytes(MONO_BYTES).expect("embedded DejaVuSansMono.ttf is invalid");
        Self { sans, mono }
    }

    pub(super) fn font(&self, kind: FontKind) -> &Font<'static> {
        match kind {
            FontKind::Sans => &self.sans,
            FontKind::Mono => &self.mono,
        }
    }
}
