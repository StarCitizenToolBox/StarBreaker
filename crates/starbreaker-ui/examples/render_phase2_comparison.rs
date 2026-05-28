use std::fs;
use std::path::PathBuf;

use starbreaker_ui::bb_atlas::{AssetFetcher, AtlasLibrary};
use starbreaker_ui::canvas::RgbaColor;
use starbreaker_ui::compose::ComposeContext;
use starbreaker_ui::defaults::DefaultValueRegistry;
use starbreaker_ui::hybrid_compose::render_ui_ir_with_swf_overlay;
use starbreaker_ui::ir_compose::render_ui_ir_document;
use starbreaker_ui::style::{CrtParams, ManufacturerStyle};
use starbreaker_ui::swf_assets::SwfAssetLibrary;
use starbreaker_ui::ui_ir::{
    UI_IR_SCHEMA_VERSION, UiIrBorder, UiIrBorderSide, UiIrDocument, UiIrNode, UiIrRect,
    UiIrTextStyle, UiIrValue, UiRendererHint,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = PathBuf::from("../docs/StarBreaker/ui-rework-artifacts/phase-2/comparison");
    fs::create_dir_all(&output_dir)?;

    let document = synthetic_hybrid_document();
    let fetcher = EmptyFetcher;
    let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
    let style = stub_style();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let swf_assets = SwfAssetLibrary::new(make_exported_rect_swf())?;
    let ctx = ComposeContext {
        style: &style,
        defaults: &defaults,
        assets: &swf_assets,
    };

    let before = render_ui_ir_document(&document, &ctx, &atlas)?;
    let after = render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, Some(&swf_assets))?;

    before.save(output_dir.join("synthetic-hybrid-before.png"))?;
    after.save(output_dir.join("synthetic-hybrid-after.png"))?;

    println!("wrote {}", output_dir.display());
    Ok(())
}

struct EmptyFetcher;

impl AssetFetcher for EmptyFetcher {
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

fn synthetic_hybrid_document() -> UiIrDocument {
    UiIrDocument {
        schema_version: UI_IR_SCHEMA_VERSION,
        canvas_guid: "phase2-synthetic-hybrid".to_string(),
        canvas_name: Some("BuildingBlocks_Canvas.SyntheticHybrid".to_string()),
        target_width: 120,
        target_height: 72,
        selected_style_source: Some("synthetic-style".to_string()),
        selected_swf_source: Some("synthetic-red-rect.swf".to_string()),
        renderer_hint: UiRendererHint::Hybrid,
        confidence: 100,
        warnings: Vec::new(),
        unresolved_references: Vec::new(),
        resolved_asset_refs: Vec::new(),
        missing_asset_refs: Vec::new(),
        nodes: vec![UiIrNode {
            id: 1,
            parent_id: None,
            children: Vec::new(),
            node_type: "widget_canvas".to_string(),
            name: "panel".to_string(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            authored_position: [12.0, 10.0],
            authored_size: [
                UiIrValue::Fixed { value: 96.0 },
                UiIrValue::Fixed { value: 44.0 },
            ],
            padding: [0.0, 0.0, 0.0, 0.0],
            margin: [0.0, 0.0, 0.0, 0.0],
            overflow_mode: None,
            computed_rect: UiIrRect { x: 12.0, y: 10.0, w: 96.0, h: 44.0 },
            background_fill_colour: Some([0.0, 0.0, 1.0, 0.65]),
            corner_radius: None,
            background_fill_alpha: None,
            background_fill_colour_token: Some("Accent2".to_string()),
            segmented_fill: None,
            border: Some(UiIrBorder {
                top: UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: Some("Accent1".to_string()) },
                right: UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: Some("Accent1".to_string()) },
                bottom: UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: Some("Accent1".to_string()) },
                left: UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: Some("Accent1".to_string()) },
            }),
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: None::<UiIrTextStyle>,
            asset_ref: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: Vec::new(),
            resolved_style_tags: Vec::new(),
        }],
    }
}

fn make_exported_rect_swf() -> Vec<u8> {
    use swf::Color;
    use swf::Compression;
    use swf::ExportedAsset;
    use swf::FillStyle;
    use swf::Fixed8;
    use swf::Header;
    use swf::Point;
    use swf::PointDelta;
    use swf::Rectangle;
    use swf::Shape;
    use swf::ShapeFlag;
    use swf::ShapeRecord;
    use swf::ShapeStyles;
    use swf::StyleChangeData;
    use swf::SwfStr;
    use swf::Tag;
    use swf::Twips;

    let header = Header {
        compression: Compression::None,
        version: 6,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(60.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let shape = Shape {
        version: 1,
        id: 1,
        shape_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(60.0),
        },
        edge_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(60.0),
        },
        flags: ShapeFlag::empty(),
        styles: ShapeStyles {
            fill_styles: vec![FillStyle::Color(Color {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            })],
            line_styles: vec![],
        },
        shape: vec![
            ShapeRecord::StyleChange(Box::new(StyleChangeData {
                move_to: Some(Point::new(Twips::ZERO, Twips::ZERO)),
                fill_style_0: None,
                fill_style_1: Some(1),
                line_style: None,
                new_styles: None,
            })),
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::from_pixels(100.0), Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(60.0)),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::from_pixels(-100.0), Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(-60.0)),
            },
        ],
    };

    let export: Vec<ExportedAsset<'_>> = vec![ExportedAsset {
        id: 1,
        name: SwfStr::from_utf8_str("synthetic_rect"),
    }];

    let tags = [Tag::DefineShape(shape), Tag::ExportAssets(export), Tag::ShowFrame];
    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
    buf
}