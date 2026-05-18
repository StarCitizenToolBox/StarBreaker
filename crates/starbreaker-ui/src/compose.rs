//! Canvas composer — **Phase 11 placeholder**.
//!
//! Phases 6–10 shipped a "compose" pass that walked a flat list of
//! `SceneItem`s and never matched real BuildingBlocks structure, producing
//! 54 effectively blank PNGs (see Phase 10.5 reality check in
//! `docs/ui-plan2.md`).  That code path is now removed.
//!
//! Until Phase 12 (layout engine) and Phase 13 (paint engine) land, every
//! call to [`render_canvas`] produces an unmistakable **bright magenta
//! grid** placeholder.  Any binding still routed through this placeholder
//! is visibly "not yet rendered" both in the generated PNG and in
//! `scene.blend`.
//!
//! Public API is unchanged so `pipeline.rs`, `ui_pipeline.rs` and
//! `decomposed.rs` continue to compile.

use image::{Rgba, RgbaImage};

use crate::canvas::ResolvedCanvas;
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::postprocess::PostProcessOptions;
use crate::style::ManufacturerStyle;
use crate::swf_assets::SwfAssetLibrary;

/// Shared references needed by the compositor.
pub struct ComposeContext<'a> {
    pub style: &'a ManufacturerStyle,
    pub defaults: &'a DefaultValueRegistry,
    pub assets: &'a SwfAssetLibrary,
}

/// Output canvas dimensions in pixels.
pub struct ComposeTarget {
    pub width: u32,
    pub height: u32,
}

/// Rasterise a resolved widget tree to RGBA.
///
/// **Placeholder behaviour (Phase 11):** paints a bright magenta field with
/// a white 64×64 grid overlay.  This makes any binding routed through this
/// path obvious to the eye in `scene.blend` and on disk.  No widget tree
/// walking is performed; the `canvas` argument is intentionally unused.
pub fn render_canvas(
    _canvas: &ResolvedCanvas,
    _ctx: &ComposeContext<'_>,
    target: ComposeTarget,
) -> Result<RgbaImage, UiError> {
    if target.width == 0 || target.height == 0 {
        return Err(UiError::RenderError(format!(
            "invalid target size {}×{}",
            target.width, target.height
        )));
    }

    let mut img = RgbaImage::from_pixel(target.width, target.height, Rgba([255, 0, 255, 255]));

    // White grid lines every 64 px so the placeholder reads as "not rendered"
    // even when sampled or scaled in Blender.
    for y in 0..target.height {
        for x in 0..target.width {
            if x % 64 == 0 || y % 64 == 0 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
    }
    // Solid white frame border (2 px) so the placeholder edges remain
    // visible when texture is sampled with clamp-to-edge.
    for x in 0..target.width {
        img.put_pixel(x, 0, Rgba([255, 255, 255, 255]));
        img.put_pixel(x, 1, Rgba([255, 255, 255, 255]));
        img.put_pixel(x, target.height - 1, Rgba([255, 255, 255, 255]));
        img.put_pixel(x, target.height - 2, Rgba([255, 255, 255, 255]));
    }
    for y in 0..target.height {
        img.put_pixel(0, y, Rgba([255, 255, 255, 255]));
        img.put_pixel(1, y, Rgba([255, 255, 255, 255]));
        img.put_pixel(target.width - 1, y, Rgba([255, 255, 255, 255]));
        img.put_pixel(target.width - 2, y, Rgba([255, 255, 255, 255]));
    }

    Ok(img)
}

/// Encode an [`RgbaImage`] to a PNG byte vector.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, UiError> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| UiError::RenderError(format!("PNG encode failed: {e}")))?;
    Ok(buf)
}

/// Rasterise + post-process.  **Placeholder:** runs [`render_canvas`] and
/// returns the magenta grid unmodified.  No post-process is applied yet —
/// applying the amber tint to a magenta-grid placeholder would only
/// obscure that the binding has not been ported.
pub fn render_canvas_with_postprocess(
    canvas: &ResolvedCanvas,
    ctx: &ComposeContext<'_>,
    target: ComposeTarget,
    _opts: &PostProcessOptions,
) -> Result<RgbaImage, UiError> {
    render_canvas(canvas, ctx, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{CanvasRecord, ResolvedCanvas};
    use crate::defaults::DefaultValueRegistry;
    use crate::style::StyleLoader;
    use crate::swf_assets::SwfAssetLibrary;

    fn empty_canvas() -> ResolvedCanvas {
        ResolvedCanvas {
            root: CanvasRecord {
                guid: String::from("00000000"),
                name: String::from("placeholder_test"),
                views: Vec::new(),
                scene: Vec::new(),
                operations: Vec::new(),
            },
            children: Default::default(),
        }
    }

    fn empty_assets() -> SwfAssetLibrary {
        let minimal: Vec<u8> = vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0, 0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        SwfAssetLibrary::new(minimal).expect("minimal SWF is valid")
    }

    #[test]
    fn placeholder_is_predominantly_magenta() {
        let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
        let img = render_canvas(&empty_canvas(), &ctx, ComposeTarget { width: 128, height: 128 })
            .expect("render placeholder");
        let mut magenta = 0usize;
        let mut white = 0usize;
        for px in img.pixels() {
            if px.0 == [255, 0, 255, 255] {
                magenta += 1;
            } else if px.0 == [255, 255, 255, 255] {
                white += 1;
            }
        }
        assert!(magenta > 0, "placeholder should contain magenta pixels");
        assert!(white > 0, "placeholder should contain white grid pixels");
        assert!(
            magenta > white,
            "magenta should dominate (got magenta={magenta}, white={white})"
        );
    }

    #[test]
    fn placeholder_rejects_zero_size() {
        let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = empty_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets };
        let err = render_canvas(&empty_canvas(), &ctx, ComposeTarget { width: 0, height: 64 })
            .expect_err("should reject 0 width");
        assert!(matches!(err, UiError::RenderError(_)));
    }
}
