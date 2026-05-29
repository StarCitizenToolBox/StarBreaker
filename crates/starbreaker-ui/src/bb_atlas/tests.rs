use std::{collections::HashMap, sync::atomic::{AtomicUsize, Ordering}};

use image::{ImageFormat, Rgba, RgbaImage};

use super::{AssetFetcher, AtlasLibrary, canonicalise_path};

struct StubFetcher {
    files: HashMap<String, Vec<u8>>,
    fetches: AtomicUsize,
}

impl StubFetcher {
    fn new(files: HashMap<String, Vec<u8>>) -> Self {
        Self {
            files,
            fetches: AtomicUsize::new(0),
        }
    }
}

impl AssetFetcher for StubFetcher {
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.files
            .iter()
            .find(|(path, _)| path.eq_ignore_ascii_case(p4k_path))
            .map(|(_, bytes)| bytes.clone())
    }
}

fn red_svg() -> Vec<u8> {
    br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>"#.to_vec()
}

fn red_png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    img.write_to(&mut cursor, ImageFormat::Png)
        .expect("PNG encode should succeed");
    cursor.into_inner()
}

#[test]
fn resolves_svg_to_target_size() {
    let fetcher = StubFetcher::new(HashMap::from([("test/red.svg".to_string(), red_svg())]));
    let atlas = AtlasLibrary::new(&fetcher, None);

    let img = atlas
        .resolve("test/red.svg", 32, 32)
        .expect("SVG should resolve");

    assert_eq!((img.width(), img.height()), (32, 32));
    let px = img.get_pixel(16, 16).0;
    assert!(
        px[0] > 200 && px[1] < 50 && px[2] < 50 && px[3] > 200,
        "centre pixel was {px:?}"
    );
}

#[test]
fn resolves_png_and_scales_to_target_size() {
    let fetcher = StubFetcher::new(HashMap::from([("test/red.png".to_string(), red_png())]));
    let atlas = AtlasLibrary::new(&fetcher, None);

    let img = atlas
        .resolve("test/red.png", 8, 8)
        .expect("PNG should resolve");

    assert_eq!((img.width(), img.height()), (8, 8));
}

#[test]
fn applies_mfd_manufacturer_and_gen_fallbacks() {
    let fetcher = StubFetcher::new(HashMap::from([
        ("data/ui/textures/mfd/drak/foo.svg".to_string(), red_svg()),
        ("data/ui/textures/mfd/gen/foo.svg".to_string(), red_svg()),
    ]));

    let drak_atlas = AtlasLibrary::new(&fetcher, Some("drak"));
    let drak_img = drak_atlas
        .resolve("data/ui/textures/mfd/whatever/foo.svg", 4, 4)
        .expect("manufacturer fallback should resolve");
    assert_eq!((drak_img.width(), drak_img.height()), (4, 4));

    let anvl_atlas = AtlasLibrary::new(&fetcher, Some("anvl"));
    let gen_img = anvl_atlas
        .resolve("data/ui/textures/mfd/whatever/foo.svg", 4, 4)
        .expect("GEN fallback should resolve");
    assert_eq!((gen_img.width(), gen_img.height()), (4, 4));
}

#[test]
fn caches_successful_resolves() {
    let fetcher = StubFetcher::new(HashMap::from([("test/red.svg".to_string(), red_svg())]));
    let atlas = AtlasLibrary::new(&fetcher, None);

    assert!(atlas.resolve("test/red.svg", 16, 16).is_some());
    assert!(atlas.resolve("test/red.svg", 16, 16).is_some());

    assert_eq!(fetcher.fetches.load(Ordering::SeqCst), 1);
}

#[test]
fn canonicalises_common_path_variants() {
    assert_eq!(
        canonicalise_path(r#".\Data\Data\UI\foo.SVG"#),
        "data/ui/foo.svg"
    );
}
