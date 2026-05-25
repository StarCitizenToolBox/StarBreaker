//! Hybrid UI IR renderer that composes IR-backed BB content with SWF overlays.
//!
//! Phase 2 uses this as the deterministic bridge for `swf` and `hybrid`
//! renderer hints: the IR renderer owns the BB/content pass, then SWF exports
//! are composited from the IR-selected SWF source.

use image::RgbaImage;
use tiny_skia::Color;

use crate::bb_atlas::AtlasLibrary;
use crate::compose::ComposeContext;
use crate::error::UiError;
use crate::ir_compose::render_ui_ir_document;
use crate::swf_assets::SwfAssetLibrary;
use crate::swf_render;
use crate::ui_ir::{UiIrDocument, UiRendererHint};

/// Render a UI IR document and apply a SWF visual-exports overlay when required.
pub fn render_ui_ir_with_swf_overlay(
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    swf_assets: Option<&SwfAssetLibrary>,
) -> Result<RgbaImage, UiError> {
    let mut img = render_ui_ir_document(document, ctx, atlas)?;

    if matches!(document.renderer_hint, UiRendererHint::Swf | UiRendererHint::Hybrid) {
        let assets = swf_assets.ok_or_else(|| {
            UiError::RenderError(
                "IR requested SWF/hybrid rendering but no selected SWF source was provided"
                    .to_string(),
            )
        })?;
        let pt = &ctx.style.primary_tint;
        let tint = Color::from_rgba8(pt.r, pt.g, pt.b, pt.a);
        let _ = swf_render::draw_swf_visual_exports_rgba(&mut img, assets, tint, 1.0);
    }

    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::bb_atlas::AtlasLibrary;
    use crate::compose::ComposeContext;
    use crate::defaults::DefaultValueRegistry;
    use crate::style::{CrtParams, ManufacturerStyle};
    use crate::swf_assets::SwfAssetLibrary;
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiIrDocument};
    use crate::canvas::RgbaColor;

    struct EmptyFetcher;

    impl crate::bb_atlas::AssetFetcher for EmptyFetcher {
        fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
            None
        }
    }

    fn stub_style() -> ManufacturerStyle {
        ManufacturerStyle {
            name: "drak".to_string(),
            primary_tint: RgbaColor { r: 240, g: 168, b: 104, a: 255 },
            secondary_tint: None,
            colour_slots: vec![RgbaColor { r: 240, g: 168, b: 104, a: 255 }],
            background: RgbaColor { r: 48, g: 32, b: 16, a: 255 },
            backlight: RgbaColor { r: 102, g: 214, b: 255, a: 255 },
            font_family_hints: Vec::new(),
            crt: CrtParams::default(),
        }
    }

    #[test]
    fn hybrid_renderer_requires_swf_source_for_hybrid_hint() {
        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "hybrid-guid".to_string(),
            canvas_name: Some("Hybrid".to_string()),
            target_width: 64,
            target_height: 64,
            selected_style_source: None,
            selected_swf_source: Some("test.swf".to_string()),
            renderer_hint: UiRendererHint::Hybrid,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: Vec::new(),
        };

        let fetcher = EmptyFetcher;
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
        };

        let err = render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, None)
            .expect_err("hybrid IR without SWF assets should fail loudly");
        assert!(err.to_string().contains("no selected SWF source"));
    }
}