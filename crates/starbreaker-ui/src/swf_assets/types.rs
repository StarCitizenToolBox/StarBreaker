use swf::{CharacterId, ColorTransform, Depth, Matrix};

/// An extracted SWF shape character with style and edge records.
#[derive(Clone, Debug)]
pub struct ShapeRecord {
    pub id: CharacterId,
    pub shape_bounds: swf::Rectangle<swf::Twips>,
    pub fill_styles: Vec<swf::FillStyle>,
    pub line_styles: Vec<swf::LineStyle>,
    pub records: Vec<swf::ShapeRecord>,
}

/// A single glyph within a [`FontGlyphSet`].
#[derive(Clone, Debug)]
pub struct FontGlyph {
    pub code: Option<u16>,
    pub advance: Option<i16>,
    pub shape_records: Vec<swf::ShapeRecord>,
}

/// All glyphs for one extracted SWF font character.
#[derive(Clone, Debug)]
pub struct FontGlyphSet {
    pub id: CharacterId,
    pub name: String,
    pub is_bold: bool,
    pub is_italic: bool,
    pub ascent: Option<u16>,
    pub descent: Option<u16>,
    pub leading: Option<i16>,
    pub glyphs: Vec<FontGlyph>,
}

/// Source `DefineEditText` metrics for an imported/exported font symbol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwfEditTextMetrics {
    pub height_px: f32,
    pub bounds_y_min_px: f32,
    pub bounds_y_max_px: f32,
}

impl SwfEditTextMetrics {
    /// Scale source text-field overscan to the renderer's current font size.
    pub fn top_overscan_px(self, size_px: f32) -> f32 {
        if self.height_px <= 0.0 || !self.height_px.is_finite() || !size_px.is_finite() {
            return 0.0;
        }
        let above = (-self.bounds_y_min_px).max(0.0);
        let below = (self.bounds_y_max_px - self.height_px).max(0.0);
        ((above + below) * (size_px / self.height_px)).max(0.0)
    }
}

/// A single placed child from a sprite/stage frame display list.
#[derive(Clone, Debug)]
pub struct PlaceRecord {
    pub depth: Depth,
    pub character_id: CharacterId,
    pub matrix: Matrix,
    pub color_transform: Option<ColorTransform>,
    pub name: Option<String>,
}
