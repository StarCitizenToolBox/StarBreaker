use super::*;
use crate::bb_atlas::AssetFetcher;
use crate::bb_scene::{BbCoordinateMethod, BbScene};
use crate::canvas::{CanvasRecord, ResolvedCanvas};
use crate::defaults::DefaultValueRegistry;
use crate::style::StyleLoader;
use crate::swf_assets::SwfAssetLibrary;
use std::collections::BTreeMap;

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
        b'F', b'W', b'S', 6, 21, 0, 0, 0, 0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    SwfAssetLibrary::new(minimal).expect("minimal SWF is valid")
}

struct NullFetcher;
impl AssetFetcher for NullFetcher {
    fn fetch_image_bytes(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

fn empty_bb_scene() -> BbScene {
    BbScene {
        coordinate_method: BbCoordinateMethod::UseRaw,
        canvas_size: (512.0, 256.0),
        roots: vec![],
        nodes: BTreeMap::new(),
        operations: vec![],
    }
}

#[test]
fn placeholder_is_predominantly_magenta() {
    let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let assets = empty_assets();
    let ctx = ComposeContext {
        style: &style,
        defaults: &defaults,
        assets: &assets,
    };
    let img = render_canvas(
        &empty_canvas(),
        &ctx,
        ComposeTarget {
            width: 128,
            height: 128,
        },
    )
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
    let ctx = ComposeContext {
        style: &style,
        defaults: &defaults,
        assets: &assets,
    };
    let err = render_canvas(
        &empty_canvas(),
        &ctx,
        ComposeTarget {
            width: 0,
            height: 64,
        },
    )
    .expect_err("should reject 0 width");
    assert!(matches!(err, UiError::RenderError(_)));
}

#[test]
fn render_bb_scene_empty_is_background_colour() {
    let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let assets = empty_assets();
    let ctx = ComposeContext {
        style: &style,
        defaults: &defaults,
        assets: &assets,
    };
    let fetcher = NullFetcher;
    let atlas = AtlasLibrary::new(&fetcher, Some("drak"));

    let img = render_bb_scene(
        &empty_bb_scene(),
        &ctx,
        &atlas,
        ComposeTarget {
            width: 64,
            height: 64,
        },
    )
    .expect("render empty bb scene");

    assert_eq!((img.width(), img.height()), (64, 64));

    let bg = style.background;
    for px in img.pixels() {
        assert_ne!(
            px.0,
            [255, 0, 255, 255],
            "empty scene must not produce magenta"
        );
    }
    let cx = img.get_pixel(32, 32);
    assert_eq!(
        [cx.0[0], cx.0[1], cx.0[2], cx.0[3]],
        [bg.r, bg.g, bg.b, bg.a],
        "center pixel of empty scene should equal background colour"
    );
}

#[test]
fn render_bb_scene_rejects_zero_size() {
    let style = StyleLoader::for_manufacturer("drak").drake_amber_fallback();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let assets = empty_assets();
    let ctx = ComposeContext {
        style: &style,
        defaults: &defaults,
        assets: &assets,
    };
    let fetcher = NullFetcher;
    let atlas = AtlasLibrary::new(&fetcher, Some("drak"));

    let err = render_bb_scene(
        &empty_bb_scene(),
        &ctx,
        &atlas,
        ComposeTarget {
            width: 0,
            height: 64,
        },
    )
    .expect_err("should reject 0 width");
    assert!(matches!(err, UiError::RenderError(_)));
}

#[test]
fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers() {
    let source = include_str!("mod.rs");
    let forbidden = [
        ["base_", "animatedelements"].concat(),
        ["BG", "Dots"].concat(),
        ["MainMenu", "Canvas"].concat(),
        ["s_", "bioc"].concat(),
        ["s_", "rsi"].concat(),
        ["s_", "aegs"].concat(),
    ];

    for marker in forbidden {
        assert!(
            !source.contains(marker.as_str()),
            "compose hardcoding marker reintroduced: {marker}"
        );
    }
}
