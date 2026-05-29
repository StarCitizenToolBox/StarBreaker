//! Text renderer using `rusttype` and bundled DejaVu fonts.

use rusttype::Font;

mod swf_draw;
mod ttf_draw;
#[cfg(test)]
mod tests;

pub use ttf_draw::TextRenderer;

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
