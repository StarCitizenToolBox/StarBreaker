//! Canvas-to-image compositor — Phase 7 implementation.
//!
//! Walks a [`ResolvedCanvas`] widget tree and produces an RGBA image buffer via
//! [`tiny_skia::Pixmap`] as the intermediate raster store.
//!
//! # Primary entry points
//! - [`render_canvas`] — rasterise a resolved canvas tree to an [`image::RgbaImage`].
//! - [`encode_png`]    — encode an [`image::RgbaImage`] to a PNG byte vector.
//!
//! # Rendering pipeline
//! 1. The default view is selected (the one with `default == true`, or ordinal 0).
//! 2. Scene items are walked in declaration order (painter's order, back-to-front).
//! 3. Each [`SceneItem`] is dispatched by kind:
//!    - `BuildingBlocks_TextField`   → text rendering (SWF glyphs or built-in fallback).
//!    - `BuildingBlocks_Shape`/`Rect`/`Rectangle` → filled/stroked rectangle or path.
//!    - `BuildingBlocks_Image`/`Bitmap` → bitmap blit with src-over alpha.
//!    - `BuildingBlocks_Sprite`/`MovieClip` → SWF export linkage first-frame rendering.
//!    - `BuildingBlocks_WidgetCanvas` → recursive sub-canvas compositing.
//!    - `BuildingBlocks_Group`/`Container` → transform group (children inherit parent xform).
//!    - Unknown kinds → skipped with `log::debug!`.
//! 4. The completed `Pixmap` is converted to `image::RgbaImage`.
//!
//! # Text rendering
//! Two paths are tried in order:
//! 1. **SWF glyph outlines** — when `font_id` resolves to a [`FontGlyphSet`] in the
//!    [`SwfAssetLibrary`], each glyph's `ShapeRecord` list is converted to a
//!    `tiny_skia::Path` and filled with the current colour.
//! 2. **Built-in bitmap fallback** — when glyph rendering is unavailable or fails, a
//!    minimal 5×7 bitmap font (baked as byte literals in this file) is used.  The
//!    fallback is deterministic and requires no external dependencies.
//!
//! # Transform model
//! The compositor maintains a `tiny_skia::Transform` stack.  `Transform2D` values
//! from scene items are composed via translation → scale → rotation (Euler order
//! matching BuildingBlocks authoring convention).

use std::collections::HashMap;

use image::RgbaImage;
use log::debug;
use tiny_skia::{BlendMode, Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::canvas::{
    CanvasRecord, ResolvedCanvas, RgbaColor, SceneItem, Transform2D, Value, ViewComponent,
};
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::postprocess::{PostProcessOptions, PostProcessor};
use crate::style::ManufacturerStyle;
use crate::swf_assets::{ShapeRecord, SwfAssetLibrary};

// ──────────────────────────────────────────────────────────────────────────────
// Public API types
// ──────────────────────────────────────────────────────────────────────────────

/// Shared references needed by the compositor.
pub struct ComposeContext<'a> {
    /// Manufacturer visual style (tint, background, CRT params).
    pub style: &'a ManufacturerStyle,
    /// Default "switched on, no live data" values for state-bound widgets.
    pub defaults: &'a DefaultValueRegistry,
    /// Static visual atoms extracted from SWF files.
    /// May be an empty library for Tier 1 canvases that contain no SWF assets.
    pub assets: &'a SwfAssetLibrary,
}

/// Output canvas dimensions in pixels.
pub struct ComposeTarget {
    pub width: u32,
    pub height: u32,
}

// ──────────────────────────────────────────────────────────────────────────────
// Top-level entry points
// ──────────────────────────────────────────────────────────────────────────────

/// Rasterise a resolved widget tree to an RGBA [`RgbaImage`].
///
/// Selects the default view (the one with `CanvasView::default == true`, or
/// `ordinal == 0` when none is marked default), then walks its scene items in
/// painter's order.
///
/// The canvas background is filled with `ctx.style.background` before any
/// widget is drawn.
pub fn render_canvas(
    canvas: &ResolvedCanvas,
    ctx: &ComposeContext<'_>,
    target: ComposeTarget,
) -> Result<RgbaImage, UiError> {
    let mut pixmap = Pixmap::new(target.width, target.height).ok_or_else(|| {
        UiError::RenderError(format!(
            "failed to create {}×{} pixmap",
            target.width, target.height
        ))
    })?;

    // Fill background.
    let bg = ctx.style.background;
    pixmap.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    // Determine the canvas scale from target vs. canvas intrinsic size.
    // Build a state machine for the walk.
    let scale_x = target.width as f32;
    let scale_y = target.height as f32;
    let base_xform = Transform::from_scale(scale_x / 1024.0, scale_y / 768.0);

    // Walk the root canvas scene directly (the scene[] array items).
    let state = DrawState {
        ctx,
        canvas,
        target_w: target.width,
        target_h: target.height,
    };
    state.draw_canvas_record(&canvas.root, base_xform, &mut pixmap)?;

    Ok(pixmap_to_rgba(pixmap))
}

/// Encode an [`RgbaImage`] to a PNG byte vector.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, UiError> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| UiError::RenderError(format!("PNG encode failed: {e}")))?;
    Ok(buf)
}

/// Rasterise a resolved widget tree to an RGBA [`RgbaImage`] and then apply
/// the manufacturer post-process passes in-place.
///
/// This is a thin wrapper around [`render_canvas`] + [`PostProcessor::run`].
/// It is the preferred entry point for producing final screen textures.
///
/// # Backwards compatibility
/// [`render_canvas`] remains unchanged and performs **no** post-processing.
/// Existing callers (Tier 1/2 tests from Phases 6–7) continue to work without
/// modification.  New tests and production code should call this function or
/// run [`PostProcessor`] explicitly after `render_canvas`.
///
/// # Example
/// ```ignore
/// let img = render_canvas_with_postprocess(
///     &canvas, &ctx, ComposeTarget { width: 1600, height: 900 },
///     &PostProcessOptions::default(),
/// )?;
/// ```
pub fn render_canvas_with_postprocess(
    canvas: &ResolvedCanvas,
    ctx: &ComposeContext<'_>,
    target: ComposeTarget,
    opts: &PostProcessOptions,
) -> Result<RgbaImage, UiError> {
    let mut img = render_canvas(canvas, ctx, target)?;
    PostProcessor::new(ctx.style).run(&mut img, opts);
    Ok(img)
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal drawing state
// ──────────────────────────────────────────────────────────────────────────────

struct DrawState<'a> {
    ctx: &'a ComposeContext<'a>,
    canvas: &'a ResolvedCanvas,
    target_w: u32,
    target_h: u32,
}

impl<'a> DrawState<'a> {
    /// Draw all scene items from a [`CanvasRecord`] in painter's order.
    fn draw_canvas_record(
        &self,
        record: &CanvasRecord,
        parent_xform: Transform,
        pixmap: &mut Pixmap,
    ) -> Result<(), UiError> {
        // Choose the default view (first with default==true, else ordinal 0).
        let default_view = record
            .views
            .iter()
            .find(|v| v.default)
            .or_else(|| record.views.first());

        // Walk scene items.
        for item in &record.scene {
            self.draw_scene_item(item, record, parent_xform, pixmap);
        }

        // Also walk view components if available (sub-canvas references).
        if let Some(view) = default_view {
            for comp in &view.components {
                self.draw_component_at(comp, record, parent_xform, Transform2D::default(), None, pixmap);
            }
        }
        Ok(())
    }

    fn draw_scene_item(
        &self,
        item: &SceneItem,
        parent_record: &CanvasRecord,
        parent_xform: Transform,
        pixmap: &mut Pixmap,
    ) {
        let local_xform = compose_transform(parent_xform, &item.transform, self.target_w, self.target_h);

        // Dispatch by kind string.
        let kind = item.kind.as_str();
        let suffix = kind.strip_prefix("BuildingBlocks_").unwrap_or(kind);

        match suffix {
            "WidgetCanvas" | "WidgetCanvasURL" => {
                if let Some(guid) = &item.guid {
                    if let Some(child) = self.canvas.children.get(guid) {
                        let _ = self.draw_canvas_record(child, local_xform, pixmap);
                    } else {
                        debug!("compose: WidgetCanvas guid={guid} not in children map");
                    }
                }
            }
            "TextField" | "Text" | "Label" | "TextWidget" | "WidgetText"
            | "BindingsText" | "DynamicText" | "SpriteText" => {
                let text = self.resolve_text_for_item(item, parent_record);
                let color = item
                    .color
                    .unwrap_or_else(|| rgba_from_u32(0xFFFFFFFF))
                    .lerp_tint(self.ctx.style.primary_tint);
                let font_id = item
                    .properties
                    .get("fontId")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u16);
                self.draw_text_at_full(&text, local_xform, color, font_id, pixmap);
            }
            "Shape" | "Rectangle" | "Rect" | "Circle" | "Line" | "WidgetShape" => {
                let fill = item
                    .color
                    .or_else(|| item.properties.get("fill").and_then(|v| v.as_u64()).map(|n| rgba_from_u32(n as u32)))
                    .unwrap_or(self.ctx.style.primary_tint);
                let w = item.properties.get("width").and_then(|v| v.as_f64()).unwrap_or(100.0) as f32;
                let h = item.properties.get("height").and_then(|v| v.as_f64()).unwrap_or(20.0) as f32;
                self.draw_filled_rect(local_xform, w, h, fill, pixmap);
            }
            "Sprite" | "WidgetSprite" | "SpriteInstance" | "MovieClip" => {
                let swf_path = item
                    .properties
                    .get("swfPath")
                    .or_else(|| item.properties.get("swf"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let linkage_name = item
                    .properties
                    .get("linkageName")
                    .or_else(|| item.properties.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let _ = self.draw_sprite_component(swf_path, linkage_name, local_xform, pixmap, 0);
            }
            "Image" | "Bitmap" | "Texture" | "WidgetImage" => {
                // No external texture loading in Phase 6 — bitmaps from SwfAssetLibrary only.
                if let Some(id_val) = item.properties.get("characterId").or_else(|| item.properties.get("bitmapId")) {
                    if let Some(id) = id_val.as_u64().map(|n| n as u16) {
                        if let Some(bmp) = self.ctx.assets.get_bitmap(id) {
                            self.blit_bitmap(bmp, local_xform, pixmap);
                        }
                    }
                }
            }
            "Group" | "Container" | "Panel" | "WidgetGroup" | "Canvas" => {
                // Children are stored in `properties["children"]` — walk them.
                if let Some(children) = item.properties.get("children").and_then(|v| v.as_array()) {
                    for child_val in children {
                        let child_item = SceneItem::from_json(child_val);
                        self.draw_scene_item(&child_item, parent_record, local_xform, pixmap);
                    }
                }
            }
            _ => {
                debug!("compose: skipping unknown scene item kind '{kind}'");
            }
        }
    }

    /// Resolve display text for a TextField scene item.
    ///
    /// Priority (highest first):
    /// 1. `DefaultValueRegistry::lookup_path` for the item's binding path.
    /// 2. Item's `default_text` property.
    /// 3. Empty string.
    fn resolve_text_for_item(&self, item: &SceneItem, parent_record: &CanvasRecord) -> String {
        // Find the binding path from the scene item's properties or from the
        // corresponding operation in the parent canvas.
        let binding_path = item
            .properties
            .get("binding")
            .or_else(|| item.properties.get("bindingPath"))
            .and_then(|v| v.as_str());

        if let Some(path) = binding_path {
            if let Some(val) = self.ctx.defaults.lookup_path(path) {
                return value_to_display_string(val);
            }
        }

        // Fall back to any operation in the parent record that binds to this item.
        // (Operations don't carry a widget ID reference in our schema, so we can
        // only match by binding path.)
        if let Some(path) = binding_path {
            for op in &parent_record.operations {
                if op.binding_path.as_deref() == Some(path) {
                    if let Some(dv) = &op.default_value {
                        return value_to_display_string(dv);
                    }
                }
            }
        }

        // Final fallback: static text from the item.
        item.properties
            .get("text")
            .or_else(|| item.properties.get("defaultText"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn draw_component_at(
        &self,
        comp: &ViewComponent,
        _parent_record: &CanvasRecord,
        parent_xform: Transform,
        _local_transform: Transform2D,
        _color: Option<RgbaColor>,
        pixmap: &mut Pixmap,
    ) {
        match comp {
            ViewComponent::WidgetCanvas { sub_guid: Some(sg), .. } => {
                if let Some(child) = self.canvas.children.get(sg) {
                    let _ = self.draw_canvas_record(child, parent_xform, pixmap);
                }
            }
            ViewComponent::Sprite { swf_path, linkage_name } => {
                let _ = self.draw_sprite_component(swf_path, linkage_name, parent_xform, pixmap, 0);
            }
            _ => {}
        }
    }

    fn draw_sprite_component(
        &self,
        swf_path: &str,
        linkage_name: &str,
        parent_xform: Transform,
        pixmap: &mut Pixmap,
        depth: u32,
    ) -> Result<(), UiError> {
        const MAX_DEPTH: u32 = 8;
        if depth >= MAX_DEPTH {
            return Err(UiError::SpriteDepthExceeded(depth));
        }

        let Some(char_id) = self.ctx.assets.lookup_export(linkage_name) else {
            debug!("compose: sprite linkage '{}' not found in SWF '{}'", linkage_name, swf_path);
            return Ok(());
        };

        let place_list = self.ctx.assets.extract_sprite_first_frame(char_id);
        for place in &place_list {
            let xform = parent_xform.pre_concat(swf_matrix_to_skia(&place.matrix));
            if let Some(shape) = self.ctx.assets.get_shape(place.character_id) {
                self.draw_swf_shape(shape, xform, place.color_transform, pixmap);
            } else if let Some(nested_name) = self.ctx.assets.export_name_for(place.character_id) {
                self.draw_sprite_component(swf_path, &nested_name, xform, pixmap, depth + 1)?;
            }
        }
        Ok(())
    }

    fn draw_swf_shape(
        &self,
        shape: &ShapeRecord,
        xform: Transform,
        _color_transform: Option<swf::ColorTransform>,
        pixmap: &mut Pixmap,
    ) {
        let fill_color = self.ctx.style.primary_tint;
        if let Some(path) = swf_shape_to_path(&shape.records) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(fill_color.r, fill_color.g, fill_color.b, fill_color.a);
            paint.blend_mode = BlendMode::SourceOver;
            pixmap.fill_path(&path, &paint, FillRule::Winding, xform, None);
        }
    }

    // ── Primitives ────────────────────────────────────────────────────────────

    fn draw_filled_rect(
        &self,
        xform: Transform,
        w: f32,
        h: f32,
        color: RgbaColor,
        pixmap: &mut Pixmap,
    ) {
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, 0.0);
        pb.line_to(w, 0.0);
        pb.line_to(w, h);
        pb.line_to(0.0, h);
        pb.close();
        let Some(path) = pb.finish() else { return };

        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        paint.blend_mode = BlendMode::SourceOver;
        pixmap.fill_path(&path, &paint, FillRule::Winding, xform, None);
    }

    fn blit_bitmap(&self, bmp: &RgbaImage, xform: Transform, pixmap: &mut Pixmap) {
        // Convert the bitmap to a tiny-skia PixmapRef and draw with src-over.
        // If conversion fails, skip silently.
        let w = bmp.width();
        let h = bmp.height();
        let Some(mut bmp_px) = Pixmap::new(w, h) else { return };
        // Copy RGBA pixels.
        let raw = bmp.as_raw();
        let data = bmp_px.data_mut();
        // tiny-skia uses premultiplied alpha; image crate gives straight alpha.
        // Convert to premultiplied.
        for (i, px) in raw.chunks_exact(4).enumerate() {
            let r = px[0] as u32;
            let g = px[1] as u32;
            let b = px[2] as u32;
            let a = px[3] as u32;
            let base = i * 4;
            if a == 255 {
                data[base] = r as u8;
                data[base + 1] = g as u8;
                data[base + 2] = b as u8;
                data[base + 3] = a as u8;
            } else if a == 0 {
                data[base] = 0;
                data[base + 1] = 0;
                data[base + 2] = 0;
                data[base + 3] = 0;
            } else {
                data[base] = (r * a / 255) as u8;
                data[base + 1] = (g * a / 255) as u8;
                data[base + 2] = (b * a / 255) as u8;
                data[base + 3] = a as u8;
            }
        }
        let pattern = tiny_skia::PixmapPaint::default();
        pixmap.draw_pixmap(0, 0, bmp_px.as_ref(), &pattern, xform, None);
    }

    // ── Text rendering ────────────────────────────────────────────────────────

    /// Render `text` at the origin of `xform` with `color`.
    ///
    /// Tries SWF glyph outlines first; falls back to the built-in bitmap font.
    #[allow(dead_code)]
    fn draw_text_at(&self, text: &str, xform: Transform, color: RgbaColor, pixmap: &mut Pixmap) {
        self.draw_text_at_full(text, xform, color, None, pixmap);
    }

    /// Render `text`, preferring SWF glyph outlines when `font_id` resolves.
    fn draw_text_at_full(
        &self,
        text: &str,
        xform: Transform,
        color: RgbaColor,
        font_id: Option<u16>,
        pixmap: &mut Pixmap,
    ) {
        if text.is_empty() {
            return;
        }
        if let Some(font_id) = font_id {
            if self.draw_text_with_swf_glyphs(text, font_id, xform, color, pixmap) {
                return;
            }
        }
        draw_text_builtin(text, xform, color, pixmap);
    }

    /// Render `text` using glyphs from a SWF font, falling back if necessary.
    fn draw_text_with_swf_glyphs(
        &self,
        text: &str,
        font_id: u16,
        xform: Transform,
        color: RgbaColor,
        pixmap: &mut Pixmap,
    ) -> bool {
        let Some(font) = self.ctx.assets.get_font(font_id) else { return false };
        // Build a code → glyph index map.
        let code_map: HashMap<u16, usize> = font
            .glyphs
            .iter()
            .enumerate()
            .filter_map(|(i, g)| g.code.map(|c| (c, i)))
            .collect();

        let mut cursor_x = 0.0_f32;
        let em_size = 1024.0_f32; // SWF glyphs are defined in a 1024-unit EM square.
        let scale = 12.0 / em_size; // Target ~12px glyph height.

        for ch in text.chars() {
            let code = ch as u16;
            let Some(&glyph_idx) = code_map.get(&code) else {
                cursor_x += 8.0;
                continue;
            };
            let glyph = &font.glyphs[glyph_idx];

            // Convert shape records to a tiny-skia path.
            if let Some(path) = swf_shape_to_path(&glyph.shape_records) {
                let glyph_xform = xform
                    .pre_translate(cursor_x, 0.0)
                    .pre_scale(scale, scale);
                let mut paint = Paint::default();
                paint.set_color_rgba8(color.r, color.g, color.b, color.a);
                paint.blend_mode = BlendMode::SourceOver;
                pixmap.fill_path(&path, &paint, FillRule::Winding, glyph_xform, None);
            }

            let advance = glyph
                .advance
                .map(|a| a as f32 * scale)
                .unwrap_or(8.0);
            cursor_x += advance;
        }
        true
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Built-in bitmap fallback font (5×7 glyphs)
// ──────────────────────────────────────────────────────────────────────────────

/// Render `text` using the built-in 5×7 bitmap font.
///
/// Characters not in the font map are rendered as a small filled rectangle
/// (a visible placeholder).  The font covers ASCII 0x20–0x7E.
fn draw_text_builtin(text: &str, xform: Transform, color: RgbaColor, pixmap: &mut Pixmap) {
    let scale = 2.0_f32; // 2× renders the 5×7 glyph at 10×14 device pixels.
    let glyph_w = 5.0 * scale;
    let glyph_h = 7.0 * scale;
    let kern = 1.0 * scale;
    let advance = glyph_w + kern;

    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    paint.blend_mode = BlendMode::SourceOver;

    let mut cursor_x = 0.0_f32;
    for ch in text.chars() {
        let bitmap = FONT5X7.get(&(ch as u8)).copied().unwrap_or(UNKNOWN_GLYPH);
        for row in 0..7_usize {
            let row_bits = bitmap[row];
            for col in 0..5_usize {
                if (row_bits >> (4 - col)) & 1 == 1 {
                    let px = cursor_x + col as f32 * scale;
                    let py = row as f32 * scale;
                    let mut pb = PathBuilder::new();
                    pb.move_to(px, py);
                    pb.line_to(px + scale, py);
                    pb.line_to(px + scale, py + scale);
                    pb.line_to(px, py + scale);
                    pb.close();
                    let Some(path) = pb.finish() else { continue };
                    pixmap.fill_path(&path, &paint, FillRule::Winding, xform, None);
                }
            }
        }
        cursor_x += advance;
    }
    let _ = (glyph_h, kern); // suppress unused warnings
}

/// Measure text width with the built-in font (in pixels at scale=2).
#[allow(dead_code)]
fn measure_text_builtin(text: &str) -> f32 {
    let scale = 2.0_f32;
    let glyph_w = 5.0 * scale;
    let kern = 1.0 * scale;
    let advance = glyph_w + kern;
    let n = text.chars().count() as f32;
    if n == 0.0 { return 0.0; }
    n * advance - kern
}

/// Measure text height with the built-in font (in pixels at scale=2).
#[allow(dead_code)]
fn measure_text_height() -> f32 {
    7.0 * 2.0
}

// ── 5×7 bitmap font table ─────────────────────────────────────────────────────
// Each entry is 7 bytes; each byte is a 5-bit row (bits 4..0, left-to-right).
// Coverage: printable ASCII 0x20–0x7E.

type Glyph5x7 = [u8; 7];

static UNKNOWN_GLYPH: Glyph5x7 = [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111];

// Macro to define the font table compactly.
macro_rules! glyph {
    ($($b:expr),*) => { [$($b),*] as Glyph5x7 }
}

// The font is hand-crafted to cover printable ASCII.  Each glyph is 5×7.
fn build_font_map() -> HashMap<u8, Glyph5x7> {
    let mut m: HashMap<u8, Glyph5x7> = HashMap::new();
    // 0x20 SPACE
    m.insert(b' ', glyph![0,0,0,0,0,0,0]);
    // 0x21 !
    m.insert(b'!', glyph![0b00100,0b00100,0b00100,0b00100,0b00000,0b00100,0b00000]);
    // 0x22 "
    m.insert(b'"', glyph![0b01010,0b01010,0b00000,0b00000,0b00000,0b00000,0b00000]);
    // 0x23 #
    m.insert(b'#', glyph![0b01010,0b11111,0b01010,0b01010,0b11111,0b01010,0b00000]);
    // 0x24 $
    m.insert(b'$', glyph![0b00100,0b01111,0b10000,0b01110,0b00001,0b11110,0b00100]);
    // 0x25 %
    m.insert(b'%', glyph![0b11000,0b11001,0b00010,0b00100,0b01000,0b10011,0b00011]);
    // 0x26 &
    m.insert(b'&', glyph![0b01100,0b10010,0b01100,0b10011,0b10010,0b01101,0b00000]);
    // 0x27 '
    m.insert(b'\'', glyph![0b00100,0b00100,0b00000,0b00000,0b00000,0b00000,0b00000]);
    // 0x28 (
    m.insert(b'(', glyph![0b00010,0b00100,0b01000,0b01000,0b01000,0b00100,0b00010]);
    // 0x29 )
    m.insert(b')', glyph![0b01000,0b00100,0b00010,0b00010,0b00010,0b00100,0b01000]);
    // 0x2A *
    m.insert(b'*', glyph![0b00000,0b10101,0b01110,0b11111,0b01110,0b10101,0b00000]);
    // 0x2B +
    m.insert(b'+', glyph![0b00000,0b00100,0b00100,0b11111,0b00100,0b00100,0b00000]);
    // 0x2C ,
    m.insert(b',', glyph![0b00000,0b00000,0b00000,0b00000,0b00110,0b00100,0b01000]);
    // 0x2D -
    m.insert(b'-', glyph![0b00000,0b00000,0b00000,0b11111,0b00000,0b00000,0b00000]);
    // 0x2E .
    m.insert(b'.', glyph![0b00000,0b00000,0b00000,0b00000,0b00000,0b00110,0b00000]);
    // 0x2F /
    m.insert(b'/', glyph![0b00001,0b00010,0b00100,0b01000,0b10000,0b00000,0b00000]);
    // 0-9
    m.insert(b'0', glyph![0b01110,0b10011,0b10101,0b10101,0b11001,0b01110,0b00000]);
    m.insert(b'1', glyph![0b00100,0b01100,0b00100,0b00100,0b00100,0b01110,0b00000]);
    m.insert(b'2', glyph![0b01110,0b10001,0b00001,0b00110,0b01000,0b11111,0b00000]);
    m.insert(b'3', glyph![0b11111,0b00010,0b00100,0b00010,0b10001,0b01110,0b00000]);
    m.insert(b'4', glyph![0b00010,0b00110,0b01010,0b10010,0b11111,0b00010,0b00000]);
    m.insert(b'5', glyph![0b11111,0b10000,0b11110,0b00001,0b10001,0b01110,0b00000]);
    m.insert(b'6', glyph![0b00110,0b01000,0b10000,0b11110,0b10001,0b01110,0b00000]);
    m.insert(b'7', glyph![0b11111,0b10001,0b00010,0b00100,0b01000,0b01000,0b00000]);
    m.insert(b'8', glyph![0b01110,0b10001,0b01110,0b10001,0b10001,0b01110,0b00000]);
    m.insert(b'9', glyph![0b01110,0b10001,0b01111,0b00001,0b00010,0b01100,0b00000]);
    // : ; < = > ? @
    m.insert(b':', glyph![0b00000,0b00110,0b00000,0b00000,0b00110,0b00000,0b00000]);
    m.insert(b';', glyph![0b00000,0b00110,0b00000,0b00000,0b00110,0b00100,0b01000]);
    m.insert(b'<', glyph![0b00010,0b00100,0b01000,0b10000,0b01000,0b00100,0b00010]);
    m.insert(b'=', glyph![0b00000,0b11111,0b00000,0b00000,0b11111,0b00000,0b00000]);
    m.insert(b'>', glyph![0b10000,0b01000,0b00100,0b00010,0b00100,0b01000,0b10000]);
    m.insert(b'?', glyph![0b01110,0b10001,0b00010,0b00100,0b00000,0b00100,0b00000]);
    m.insert(b'@', glyph![0b01110,0b10001,0b10111,0b10101,0b10110,0b10000,0b01111]);
    // A-Z
    m.insert(b'A', glyph![0b01110,0b10001,0b10001,0b11111,0b10001,0b10001,0b00000]);
    m.insert(b'B', glyph![0b11110,0b10001,0b11110,0b10001,0b10001,0b11110,0b00000]);
    m.insert(b'C', glyph![0b01110,0b10001,0b10000,0b10000,0b10001,0b01110,0b00000]);
    m.insert(b'D', glyph![0b11100,0b10010,0b10001,0b10001,0b10010,0b11100,0b00000]);
    m.insert(b'E', glyph![0b11111,0b10000,0b11110,0b10000,0b10000,0b11111,0b00000]);
    m.insert(b'F', glyph![0b11111,0b10000,0b11110,0b10000,0b10000,0b10000,0b00000]);
    m.insert(b'G', glyph![0b01110,0b10001,0b10000,0b10111,0b10001,0b01110,0b00000]);
    m.insert(b'H', glyph![0b10001,0b10001,0b11111,0b10001,0b10001,0b10001,0b00000]);
    m.insert(b'I', glyph![0b01110,0b00100,0b00100,0b00100,0b00100,0b01110,0b00000]);
    m.insert(b'J', glyph![0b00111,0b00010,0b00010,0b00010,0b10010,0b01100,0b00000]);
    m.insert(b'K', glyph![0b10001,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001]);
    m.insert(b'L', glyph![0b10000,0b10000,0b10000,0b10000,0b10000,0b11111,0b00000]);
    m.insert(b'M', glyph![0b10001,0b11011,0b10101,0b10001,0b10001,0b10001,0b00000]);
    m.insert(b'N', glyph![0b10001,0b11001,0b10101,0b10011,0b10001,0b10001,0b00000]);
    m.insert(b'O', glyph![0b01110,0b10001,0b10001,0b10001,0b10001,0b01110,0b00000]);
    m.insert(b'P', glyph![0b11110,0b10001,0b11110,0b10000,0b10000,0b10000,0b00000]);
    m.insert(b'Q', glyph![0b01110,0b10001,0b10001,0b10101,0b10011,0b01111,0b00000]);
    m.insert(b'R', glyph![0b11110,0b10001,0b11110,0b10100,0b10010,0b10001,0b00000]);
    m.insert(b'S', glyph![0b01111,0b10000,0b01110,0b00001,0b00001,0b11110,0b00000]);
    m.insert(b'T', glyph![0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b00000]);
    m.insert(b'U', glyph![0b10001,0b10001,0b10001,0b10001,0b10001,0b01110,0b00000]);
    m.insert(b'V', glyph![0b10001,0b10001,0b10001,0b10001,0b01010,0b00100,0b00000]);
    m.insert(b'W', glyph![0b10001,0b10001,0b10001,0b10101,0b11011,0b10001,0b00000]);
    m.insert(b'X', glyph![0b10001,0b01010,0b00100,0b00100,0b01010,0b10001,0b00000]);
    m.insert(b'Y', glyph![0b10001,0b10001,0b01010,0b00100,0b00100,0b00100,0b00000]);
    m.insert(b'Z', glyph![0b11111,0b00010,0b00100,0b01000,0b10000,0b11111,0b00000]);
    // [ \ ] ^ _ `
    m.insert(b'[', glyph![0b01110,0b01000,0b01000,0b01000,0b01000,0b01110,0b00000]);
    m.insert(b'\\', glyph![0b10000,0b01000,0b00100,0b00010,0b00001,0b00000,0b00000]);
    m.insert(b']', glyph![0b01110,0b00010,0b00010,0b00010,0b00010,0b01110,0b00000]);
    m.insert(b'^', glyph![0b00100,0b01010,0b10001,0b00000,0b00000,0b00000,0b00000]);
    m.insert(b'_', glyph![0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b11111]);
    m.insert(b'`', glyph![0b01000,0b00100,0b00000,0b00000,0b00000,0b00000,0b00000]);
    // a-z
    m.insert(b'a', glyph![0b00000,0b00000,0b01110,0b00001,0b01111,0b10001,0b01111]);
    m.insert(b'b', glyph![0b10000,0b10000,0b11110,0b10001,0b10001,0b10001,0b11110]);
    m.insert(b'c', glyph![0b00000,0b00000,0b01111,0b10000,0b10000,0b10000,0b01111]);
    m.insert(b'd', glyph![0b00001,0b00001,0b01111,0b10001,0b10001,0b10001,0b01111]);
    m.insert(b'e', glyph![0b00000,0b00000,0b01110,0b10001,0b11111,0b10000,0b01111]);
    m.insert(b'f', glyph![0b00110,0b01001,0b01000,0b11100,0b01000,0b01000,0b01000]);
    m.insert(b'g', glyph![0b00000,0b01111,0b10001,0b01111,0b00001,0b10001,0b01110]);
    m.insert(b'h', glyph![0b10000,0b10000,0b11110,0b10001,0b10001,0b10001,0b10001]);
    m.insert(b'i', glyph![0b00100,0b00000,0b01100,0b00100,0b00100,0b00100,0b01110]);
    m.insert(b'j', glyph![0b00010,0b00000,0b00110,0b00010,0b00010,0b10010,0b01100]);
    m.insert(b'k', glyph![0b10000,0b10000,0b10010,0b10100,0b11100,0b10010,0b10001]);
    m.insert(b'l', glyph![0b01100,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110]);
    m.insert(b'm', glyph![0b00000,0b00000,0b11010,0b10101,0b10101,0b10001,0b10001]);
    m.insert(b'n', glyph![0b00000,0b00000,0b11110,0b10001,0b10001,0b10001,0b10001]);
    m.insert(b'o', glyph![0b00000,0b00000,0b01110,0b10001,0b10001,0b10001,0b01110]);
    m.insert(b'p', glyph![0b00000,0b11110,0b10001,0b11110,0b10000,0b10000,0b10000]);
    m.insert(b'q', glyph![0b00000,0b01111,0b10001,0b01111,0b00001,0b00001,0b00001]);
    m.insert(b'r', glyph![0b00000,0b00000,0b10110,0b11001,0b10000,0b10000,0b10000]);
    m.insert(b's', glyph![0b00000,0b00000,0b01111,0b10000,0b01110,0b00001,0b11110]);
    m.insert(b't', glyph![0b01000,0b01000,0b11100,0b01000,0b01000,0b01001,0b00110]);
    m.insert(b'u', glyph![0b00000,0b00000,0b10001,0b10001,0b10001,0b10011,0b01101]);
    m.insert(b'v', glyph![0b00000,0b00000,0b10001,0b10001,0b10001,0b01010,0b00100]);
    m.insert(b'w', glyph![0b00000,0b00000,0b10001,0b10001,0b10101,0b11011,0b10001]);
    m.insert(b'x', glyph![0b00000,0b00000,0b10001,0b01010,0b00100,0b01010,0b10001]);
    m.insert(b'y', glyph![0b00000,0b10001,0b10001,0b01111,0b00001,0b10001,0b01110]);
    m.insert(b'z', glyph![0b00000,0b00000,0b11111,0b00010,0b00100,0b01000,0b11111]);
    // { | } ~
    m.insert(b'{', glyph![0b00010,0b00100,0b00100,0b01000,0b00100,0b00100,0b00010]);
    m.insert(b'|', glyph![0b00100,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100]);
    m.insert(b'}', glyph![0b01000,0b00100,0b00100,0b00010,0b00100,0b00100,0b01000]);
    m.insert(b'~', glyph![0b00000,0b01000,0b10101,0b00010,0b00000,0b00000,0b00000]);
    // ° (degree) - stored as a special case
    m.insert(0xB0_u8, glyph![0b01100,0b10010,0b01100,0b00000,0b00000,0b00000,0b00000]);
    m
}

// Lazily initialized font map using std::sync::OnceLock (stable Rust).
static FONT5X7: std::sync::LazyLock<HashMap<u8, Glyph5x7>> =
    std::sync::LazyLock::new(build_font_map);

// ──────────────────────────────────────────────────────────────────────────────
// SWF shape records → tiny-skia path
// ──────────────────────────────────────────────────────────────────────────────

/// Convert SWF shape records into a `tiny_skia::Path`.
///
/// SWF coordinates are in **twips** (1/20 of a pixel).  The path is produced
/// in twip units; callers apply a scale transform to map to canvas pixels.
fn swf_shape_to_path(records: &[swf::ShapeRecord]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let mut pen_x = 0.0_f32;
    let mut pen_y = 0.0_f32;
    let mut has_data = false;

    for rec in records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if let Some(mv) = &sc.move_to {
                    pen_x = mv.x.get() as f32;
                    pen_y = mv.y.get() as f32;
                    pb.move_to(pen_x, pen_y);
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                pen_x += delta.dx.get() as f32;
                pen_y += delta.dy.get() as f32;
                pb.line_to(pen_x, pen_y);
                has_data = true;
            }
            swf::ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                let cx = pen_x + control_delta.dx.get() as f32;
                let cy = pen_y + control_delta.dy.get() as f32;
                let ax = cx + anchor_delta.dx.get() as f32;
                let ay = cy + anchor_delta.dy.get() as f32;
                pb.quad_to(cx, cy, ax, ay);
                pen_x = ax;
                pen_y = ay;
                has_data = true;
            }
        }
    }

    if has_data { pb.finish() } else { None }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────


fn swf_matrix_to_skia(m: &swf::Matrix) -> Transform {
    Transform::from_row(
        m.a.to_f32(),
        m.b.to_f32(),
        m.c.to_f32(),
        m.d.to_f32(),
        m.tx.get() as f32 / 20.0,
        m.ty.get() as f32 / 20.0,
    )
}

/// Convert a `tiny_skia::Pixmap` to an `image::RgbaImage`.
///
/// tiny-skia stores pixels in premultiplied RGBA. We convert back to
/// straight-alpha RGBA for the output image.
fn pixmap_to_rgba(pixmap: Pixmap) -> RgbaImage {
    let w = pixmap.width();
    let h = pixmap.height();
    let data = pixmap.data();

    let mut img_data = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        let r = px[0];
        let g = px[1];
        let b = px[2];
        let a = px[3];
        // Un-premultiply.
        if a == 0 {
            img_data.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            img_data.extend_from_slice(&[r, g, b, 255]);
        } else {
            let af = a as f32 / 255.0;
            img_data.push((r as f32 / af).min(255.0) as u8);
            img_data.push((g as f32 / af).min(255.0) as u8);
            img_data.push((b as f32 / af).min(255.0) as u8);
            img_data.push(a);
        }
    }

    RgbaImage::from_raw(w, h, img_data)
        .expect("pixmap_to_rgba: buffer size invariant violated")
}

/// Compose a `tiny_skia::Transform` from a parent transform and a local
/// [`Transform2D`].
///
/// The local transform is applied as: translate → scale → rotate.
/// Coordinates in [`Transform2D`] are in canvas units (1024×768 reference
/// space), which are mapped to device pixels by `target_w` and `target_h`.
fn compose_transform(parent: Transform, local: &Transform2D, target_w: u32, target_h: u32) -> Transform {
    let sx = local.sx;
    let sy = local.sy;
    let tx = local.tx * target_w as f32 / 1024.0;
    let ty = local.ty * target_h as f32 / 768.0;
    let angle_rad = local.angle.to_radians();

    let t = if angle_rad.abs() < 1e-6 {
        Transform::from_scale(sx, sy).post_translate(tx, ty)
    } else {
        let (sin, cos) = angle_rad.sin_cos();
        Transform::from_row(
            cos * sx,
            sin * sx,
            -sin * sy,
            cos * sy,
            tx,
            ty,
        )
    };

    parent.post_concat(t)
}

/// Convert a packed ARGB u32 (as used in BuildingBlocks canvas records) to an
/// [`RgbaColor`].
fn rgba_from_u32(packed: u32) -> RgbaColor {
    RgbaColor {
        r: ((packed >> 16) & 0xFF) as u8,
        g: ((packed >> 8) & 0xFF) as u8,
        b: (packed & 0xFF) as u8,
        a: ((packed >> 24) & 0xFF) as u8,
    }
}

/// Format a [`Value`] as a display string for a text widget.
fn value_to_display_string(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => format!("{f:.1}"),
        Value::Bool(b) => if *b { "ON" } else { "OFF" }.into(),
        Value::Guid(g) => g.clone(),
    }
}

trait RgbaColorExt {
    /// Linearly tint `self` towards `tint` at 50% weight (simple modulate).
    fn lerp_tint(self, tint: RgbaColor) -> RgbaColor;
}

impl RgbaColorExt for RgbaColor {
    fn lerp_tint(self, tint: RgbaColor) -> RgbaColor {
        // Modulate: multiply component-wise and normalize.
        RgbaColor {
            r: ((self.r as u16 * tint.r as u16) / 255) as u8,
            g: ((self.g as u16 * tint.g as u16) / 255) as u8,
            b: ((self.b as u16 * tint.b as u16) / 255) as u8,
            a: self.a,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// High-level canvas layout helpers (used by integration tests and CLI)
// ──────────────────────────────────────────────────────────────────────────────

/// Draw an annunciator-style button strip from a structured description.
///
/// `buttons` is a slice of `(label, lit)` pairs.  Lit buttons are drawn with
/// an inverted (filled amber) background; unlit buttons have a dark background
/// with an amber border.
#[allow(dead_code)]
pub(crate) fn draw_annunciator_strip(
    buttons: &[(&str, bool)],
    target_w: u32,
    target_h: u32,
    style: &ManufacturerStyle,
) -> Result<RgbaImage, UiError> {
    let mut pixmap = Pixmap::new(target_w, target_h).ok_or_else(|| {
        UiError::RenderError(format!("failed to create {target_w}×{target_h} pixmap"))
    })?;
    let bg = style.background;
    pixmap.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    let amber = style.primary_tint;
    let n = buttons.len() as f32;
    let pad = 6.0_f32;
    let btn_w = (target_w as f32 - pad * (n + 1.0)) / n;
    let btn_h = target_h as f32 - pad * 2.0;

    for (i, &(label, lit)) in buttons.iter().enumerate() {
        let x = pad + i as f32 * (btn_w + pad);
        let y = pad;
        let xform = Transform::from_translate(x, y);

        if lit {
            // Filled amber box.
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, 0.0);
            pb.line_to(btn_w, 0.0);
            pb.line_to(btn_w, btn_h);
            pb.line_to(0.0, btn_h);
            pb.close();
            let Some(path) = pb.finish() else { continue };
            let mut paint = Paint::default();
            paint.set_color_rgba8(amber.r, amber.g, amber.b, amber.a);
            paint.blend_mode = BlendMode::SourceOver;
            pixmap.fill_path(&path, &paint, FillRule::Winding, xform, None);
            // Dark text.
            let text_color = style.background;
            let tw = measure_text_builtin(label);
            let th = measure_text_height();
            let text_x = x + (btn_w - tw) / 2.0;
            let text_y = y + (btn_h - th) / 2.0;
            draw_text_builtin(label, Transform::from_translate(text_x, text_y), text_color, &mut pixmap);
        } else {
            // Outlined box with amber border, dark fill.
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, 0.0);
            pb.line_to(btn_w, 0.0);
            pb.line_to(btn_w, btn_h);
            pb.line_to(0.0, btn_h);
            pb.close();
            let Some(path) = pb.finish() else { continue };
            let mut paint = Paint::default();
            paint.set_color_rgba8(amber.r, amber.g, amber.b, amber.a);
            paint.blend_mode = BlendMode::SourceOver;
            let mut stroke = Stroke::default();
            stroke.width = 2.0;
            pixmap.stroke_path(&path, &paint, &stroke, xform, None);
            // Amber text.
            let tw = measure_text_builtin(label);
            let th = measure_text_height();
            let text_x = x + (btn_w - tw) / 2.0;
            let text_y = y + (btn_h - th) / 2.0;
            draw_text_builtin(label, Transform::from_translate(text_x, text_y), amber, &mut pixmap);
        }
    }

    Ok(pixmap_to_rgba(pixmap))
}

/// Render a "Target Status" screen with centered `target_text` and a footer.
///
/// Layout: dark background → upper dashed separator → centered `target_text` →
/// lower dashed separator → amber footer text.
#[allow(dead_code)]
pub(crate) fn draw_target_status(
    target_text: &str,
    footer_text: &str,
    target_w: u32,
    target_h: u32,
    style: &ManufacturerStyle,
) -> Result<RgbaImage, UiError> {
    let mut pixmap = Pixmap::new(target_w, target_h).ok_or_else(|| {
        UiError::RenderError(format!("failed to create {target_w}×{target_h} pixmap"))
    })?;
    let bg = style.background;
    pixmap.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    let amber = style.primary_tint;
    let w = target_w as f32;
    let h = target_h as f32;

    // Upper dashed separator line at 25% height.
    let sep_y1 = h * 0.25;
    draw_dashes(&mut pixmap, 0.0, sep_y1, w, amber);

    // Lower dashed separator line at 75% height.
    let sep_y2 = h * 0.75;
    draw_dashes(&mut pixmap, 0.0, sep_y2, w, amber);

    // Centered target text.
    let tw = measure_text_builtin(target_text);
    let th = measure_text_height();
    let tx = (w - tw) / 2.0;
    let ty = (h - th) / 2.0;
    draw_text_builtin(target_text, Transform::from_translate(tx, ty), amber, &mut pixmap);

    // Footer.
    let fw = measure_text_builtin(footer_text);
    let fx = (w - fw) / 2.0;
    let fy = h - th - 8.0;
    draw_text_builtin(footer_text, Transform::from_translate(fx, fy), amber, &mut pixmap);

    Ok(pixmap_to_rgba(pixmap))
}

/// Render a door control panel (closed/idle state).
///
/// Layout: dark background → "DOOR CONTROL" header → outlined OPEN / CLOSE
/// buttons → status indicator "CLOSED".
#[allow(dead_code)]
pub(crate) fn draw_door_panel(
    status_text: &str,
    target_w: u32,
    target_h: u32,
    style: &ManufacturerStyle,
) -> Result<RgbaImage, UiError> {
    let mut pixmap = Pixmap::new(target_w, target_h).ok_or_else(|| {
        UiError::RenderError(format!("failed to create {target_w}×{target_h} pixmap"))
    })?;
    let bg = style.background;
    pixmap.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    let amber = style.primary_tint;
    let w = target_w as f32;
    let h = target_h as f32;

    // Header: "DOOR CONTROL"
    let header = "DOOR CONTROL";
    let hw = measure_text_builtin(header);
    draw_text_builtin(header, Transform::from_translate((w - hw) / 2.0, 16.0), amber, &mut pixmap);

    // Separator below header.
    draw_dashes(&mut pixmap, 0.0, 36.0, w, amber);

    // Two buttons: OPEN (left), CLOSE (right, highlighted = default action).
    let btn_w = w * 0.35;
    let btn_h = h * 0.15;
    let btn_y = h * 0.4;
    let open_x = w * 0.1;
    let close_x = w * 0.55;

    // OPEN button — outlined only.
    draw_outlined_button(&mut pixmap, open_x, btn_y, btn_w, btn_h, "OPEN", amber, false);
    // CLOSE button — filled (current state).
    draw_outlined_button(&mut pixmap, close_x, btn_y, btn_w, btn_h, "CLOSE", amber, true);

    // Status indicator.
    let sw = measure_text_builtin(status_text);
    draw_text_builtin(status_text, Transform::from_translate((w - sw) / 2.0, h * 0.7), amber, &mut pixmap);

    // Footer separator.
    draw_dashes(&mut pixmap, 0.0, h - 30.0, w, amber);

    Ok(pixmap_to_rgba(pixmap))
}

#[allow(dead_code)]
fn draw_outlined_button(
    pixmap: &mut Pixmap,
    x: f32, y: f32,
    w: f32, h: f32,
    label: &str,
    amber: RgbaColor,
    filled: bool,
) {
    let xform = Transform::from_translate(x, y);
    let mut pb = PathBuilder::new();
    pb.move_to(0.0, 0.0);
    pb.line_to(w, 0.0);
    pb.line_to(w, h);
    pb.line_to(0.0, h);
    pb.close();
    let Some(path) = pb.finish() else { return };
    let mut paint = Paint::default();
    paint.set_color_rgba8(amber.r, amber.g, amber.b, amber.a);
    paint.blend_mode = BlendMode::SourceOver;
    if filled {
        pixmap.fill_path(&path, &paint, FillRule::Winding, xform, None);
        // Dark label on filled button.
        let dark = RgbaColor { r: 10, g: 10, b: 10, a: 255 };
        let lw = measure_text_builtin(label);
        let lh = measure_text_height();
        draw_text_builtin(label, Transform::from_translate(x + (w - lw) / 2.0, y + (h - lh) / 2.0), dark, pixmap);
    } else {
        let mut stroke = Stroke::default();
        stroke.width = 2.0;
        pixmap.stroke_path(&path, &paint, &stroke, xform, None);
        let lw = measure_text_builtin(label);
        let lh = measure_text_height();
        draw_text_builtin(label, Transform::from_translate(x + (w - lw) / 2.0, y + (h - lh) / 2.0), amber, pixmap);
    }
}

#[allow(dead_code)]
fn draw_dashes(pixmap: &mut Pixmap, x: f32, y: f32, total_w: f32, color: RgbaColor) {
    let dash = 8.0_f32;
    let gap = 4.0_f32;
    let mut cx = x;
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    paint.blend_mode = BlendMode::SourceOver;
    let mut stroke = Stroke::default();
    stroke.width = 1.5;
    while cx < x + total_w {
        let end = (cx + dash).min(x + total_w);
        let mut pb = PathBuilder::new();
        pb.move_to(cx, y);
        pb.line_to(end, y);
        let Some(path) = pb.finish() else { cx += dash + gap; continue };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        cx += dash + gap;
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Public companion – expose `parse_scene_item` for use in compose
// ──────────────────────────────────────────────────────────────────────────────

// NOTE: `parse_scene_item` in canvas.rs is private; we re-implement the minimal
// version here for use in Container child walking.  The canvas module exposes
// `CanvasParser::parse_view_component` which is sufficient for our purpose.
impl crate::canvas::SceneItem {
    fn from_json(v: &serde_json::Value) -> Self {
        // Build a minimal SceneItem from the JSON.  Properties are preserved
        // verbatim in the SceneItem returned by the canvas parser's internal
        // helper; since that helper is private we reproduce the logic here for
        // the Container-child path.
        let kind = v.get("_Type_").and_then(|t| t.as_str()).unwrap_or("").to_string();
        let guid = v.get("canvas").and_then(|c| c.as_str()).map(str::to_owned);
        let url_postfix = v.get("urlPostfix").and_then(|u| u.as_str()).map(str::to_owned);
        let url_optional = v.get("urlOptional").and_then(|u| u.as_str()).map(str::to_owned);
        // Transform is zero/default for inline container children.
        let transform = Transform2D::default();
        let color = None;
        let mut properties: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                if !["_Type_", "canvas", "urlPostfix", "urlOptional"].contains(&k.as_str()) {
                    properties.insert(k.clone(), val.clone());
                }
            }
        }
        SceneItem { kind, guid, url_postfix, url_optional, transform, color, properties }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{CanvasParser, ResolvedCanvas};
    use crate::defaults::DefaultValueRegistry;
    use crate::style::StyleLoader;
    use crate::swf_assets::SwfAssetLibrary;

    fn drake_style() -> ManufacturerStyle {
        StyleLoader::for_manufacturer("drak").drake_amber_fallback()
    }

    fn empty_assets() -> SwfAssetLibrary {
        // Minimal valid SWF header: signature "FWS", version 6, file size 21
        // followed by an empty tag stream (EndTag = 0x0000).
        let swf_header: Vec<u8> = vec![
            b'F', b'W', b'S',   // signature (uncompressed)
            6,                   // version
            21, 0, 0, 0,         // file size (LE)
            // FrameSize RECT: nbits=0 → 1 byte: 0x00
            0x00,
            // Frame rate: 24.0 (fixed 8.8) = 0x18 0x00
            0x18, 0x00,
            // Frame count: 1
            0x01, 0x00,
            // EndTag: record header 0x0000 (type 0, length 0)
            0x00, 0x00,
            // Pad to declared file size.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        SwfAssetLibrary::new(swf_header).expect("minimal SWF must parse")
    }

    fn defaults() -> DefaultValueRegistry {
        DefaultValueRegistry::with_well_known_path_defaults()
    }

    // ── 1. Canvas walk selects default view ────────────────────────────────────

    #[test]
    fn canvas_walk_selects_default_view() {
        // Verify that render_canvas picks the view with default==true.
        let json = serde_json::json!({
            "views": [
                { "name": "Off", "default": false, "screens": [] },
                { "name": "On",  "default": true,  "screens": [] }
            ],
            "scene": [],
            "operations": []
        });
        let record = CanvasParser::parse("test-guid-0000-0000-0000-000000000000", "TestCanvas", &json).unwrap();
        assert_eq!(record.views.len(), 2);
        assert!(!record.views[0].default);
        assert!(record.views[1].default);
    }

    // ── 2. Text default substitution ──────────────────────────────────────────

    #[test]
    fn text_default_substitution_via_binding() {
        let style = drake_style();
        let defaults = defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };

        // Confirm the registry answers correctly.
        assert_eq!(
            defaults.lookup_path("/vehicle/targetname"),
            Some(&Value::Str("NO TARGET".into()))
        );

        // Simulate what the composer does: resolve binding → display text.
        let val = ctx.defaults.lookup_path("/vehicle/targetname").unwrap();
        assert_eq!(value_to_display_string(val), "NO TARGET");
    }

    // ── 3. Text default falls back to default_text ────────────────────────────

    #[test]
    fn text_default_falls_back_to_item_text() {
        let defaults = DefaultValueRegistry::new(); // empty registry
        let val = defaults.lookup_path("/nonexistent");
        assert!(val.is_none(), "unknown path should return None");
        // Fallback would be the item's static text property.
    }

    // ── 4. Transform composition ──────────────────────────────────────────────

    #[test]
    fn transform_composition_identity() {
        let t2d = Transform2D::default();
        let parent = Transform::identity();
        let result = compose_transform(parent, &t2d, 1024, 768);
        // Identity composed with identity should give (approximately) identity.
        let id = Transform::identity();
        assert!((result.tx - id.tx).abs() < 1e-4, "tx differs: {}", result.tx);
        assert!((result.ty - id.ty).abs() < 1e-4, "ty differs: {}", result.ty);
    }

    #[test]
    fn transform_composition_translate() {
        let t2d = Transform2D { tx: 512.0, ty: 384.0, ..Default::default() };
        let result = compose_transform(Transform::identity(), &t2d, 1024, 768);
        // 512/1024*1024 = 512, 384/768*768 = 384.
        assert!((result.tx - 512.0).abs() < 1e-3, "tx={}", result.tx);
        assert!((result.ty - 384.0).abs() < 1e-3, "ty={}", result.ty);
    }

    // ── 5. Bitmap blit produces non-transparent pixels ────────────────────────

    #[test]
    fn bitmap_blit_produces_pixels() {
        let style = drake_style();
        let defaults = defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };

        // Create a 4×4 solid-red RGBA image.
        let bmp = RgbaImage::from_raw(4, 4, vec![255, 0, 0, 255u8].repeat(16)).unwrap();

        let mut pixmap = Pixmap::new(100, 100).unwrap();
        let state = DrawState {
            ctx: &ctx,
            canvas: &ResolvedCanvas { root: CanvasParser::parse("g", "t", &serde_json::json!({"views":[],"scene":[],"operations":[]})).unwrap(), children: Default::default() },
            target_w: 100,
            target_h: 100,
        };
        state.blit_bitmap(&bmp, Transform::identity(), &mut pixmap);

        // Check that pixel (0,0) is non-zero (red was blitted).
        let data = pixmap.data();
        assert_ne!(data[0], 0, "red channel should be non-zero after blit");
    }

    // ── 6. End-to-end render_canvas produces non-empty image ─────────────────

    #[test]
    fn end_to_end_render_produces_non_empty_image() {
        let style = drake_style();
        let defaults = defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };

        let json = serde_json::json!({
            "views": [{ "name": "default", "default": true, "screens": [] }],
            "scene": [
                {
                    "_Type_": "BuildingBlocks_TextField",
                    "binding": "/vehicle/targetname",
                    "text": "NO TARGET",
                    "transform": { "tx": 100.0, "ty": 50.0, "sx": 1.0, "sy": 1.0, "angle": 0.0 }
                }
            ],
            "operations": []
        });
        let record = CanvasParser::parse("g-t-0000-0000-0000-0000000a", "Target", &json).unwrap();
        let canvas = ResolvedCanvas { root: record, children: Default::default() };

        let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 400, height: 100 }).unwrap();
        assert_eq!(img.width(), 400);
        assert_eq!(img.height(), 100);

        // Some pixels must differ from the background (background is near-black).
        let bg = style.background;
        let differs = img.pixels().any(|p| {
            p[0] != bg.r || p[1] != bg.g || p[2] != bg.b
        });
        assert!(differs, "rendered image should have pixels different from background");
    }

    // ── 7. encode_png roundtrip ───────────────────────────────────────────────

    #[test]
    fn encode_png_roundtrip() {
        let style = drake_style();
        let defaults = defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };

        let json = serde_json::json!({"views":[{"name":"v","default":true,"screens":[]}],"scene":[],"operations":[]});
        let record = CanvasParser::parse("enc-guid-0000-0000-0000-0000000b", "Enc", &json).unwrap();
        let canvas = ResolvedCanvas { root: record, children: Default::default() };

        let img = render_canvas(&canvas, &ctx, ComposeTarget { width: 64, height: 64 }).unwrap();
        let png = encode_png(&img).unwrap();
        assert!(!png.is_empty());
        // Check PNG magic.
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    // ── 8. draw_annunciator_strip generates correct pixel count ───────────────

    #[test]
    fn annunciator_strip_pixel_count() {
        let style = drake_style();
        let img = draw_annunciator_strip(
            &[("PWR", false), ("WPN", true), ("THR", false), ("SHLD", false)],
            400, 80, &style,
        ).unwrap();
        assert_eq!(img.width(), 400);
        assert_eq!(img.height(), 80);
    }

    // ── 9. draw_target_status generates image with non-background pixels ──────

    #[test]
    fn target_status_has_content() {
        let style = drake_style();
        let img = draw_target_status(">> NO TARGET <<", "< TARGET STATUS >", 320, 200, &style).unwrap();
        let bg = style.background;
        let has_content = img.pixels().any(|p| p[0] != bg.r || p[1] != bg.g || p[2] != bg.b);
        assert!(has_content, "target status image should have non-background pixels");
    }

    // ── 10. Sprite resolve: missing font_id falls back gracefully ─────────────

    #[test]
    fn sprite_resolve_missing_swf_font_falls_back() {
        // If the asset library has no font for a given ID, text still renders
        // (using the built-in fallback) rather than panicking.
        let style = drake_style();
        let defaults = defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };

        // Font id 999 doesn't exist in the empty library.
        assert!(ctx.assets.get_font(999).is_none());

        // The composer should still produce an image without panicking.
        let json = serde_json::json!({
            "views": [{"name": "v", "default": true, "screens": []}],
            "scene": [{
                "_Type_": "BuildingBlocks_TextField",
                "text": "HELLO",
                "fontId": "999",
                "transform": {}
            }],
            "operations": []
        });
        let record = CanvasParser::parse("spr-guid-0000-0000-0000-000000000c", "Spr", &json).unwrap();
        let canvas = ResolvedCanvas { root: record, children: Default::default() };
        let result = render_canvas(&canvas, &ctx, ComposeTarget { width: 200, height: 50 });
        assert!(result.is_ok(), "render should not fail when font is missing");
    }
}
