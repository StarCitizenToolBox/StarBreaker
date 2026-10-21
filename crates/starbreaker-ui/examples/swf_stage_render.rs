//! Throwaway: render a SWF main-timeline stage (frame 0) to a PNG and print the
//! horizontal extent of its orange content, to compare against the reference.
//! Usage: `cargo run -p starbreaker-ui --example swf_stage_render -- '<p4k\path.swf>' [w] [h]`

use image::RgbaImage;
use starbreaker_p4k::MappedP4k;
use starbreaker_ui::swf_assets::SwfAssetLibrary;
use starbreaker_ui::swf_render::state_select::compute_sample_data_export_ids;
use starbreaker_ui::swf_render::{draw_swf_stage_rgba, draw_swf_stage_rgba_in_rect};
use tiny_skia::{Color, Rect as TskRect};

fn main() {
    let p4k_path = std::env::var("SC_DATA_P4K").expect("SC_DATA_P4K not set");
    let p4k = MappedP4k::open(std::path::Path::new(&p4k_path)).expect("open p4k");
    let mut args = std::env::args().skip(1);
    let target = args.next().expect("usage: <swf path> [w] [h]");
    let w: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(898);

    let entry = p4k
        .entries()
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(&target))
        .unwrap_or_else(|| panic!("missing: {target}"));
    let bytes = p4k.read(entry).expect("read swf");
    let assets = SwfAssetLibrary::new(bytes).expect("parse swf");
    println!("stage_size={:?}", assets.stage_size());

    let suppress = std::env::var("SUPPRESS").as_deref() == Ok("1");
    let mut img = RgbaImage::from_pixel(w, h, image::Rgba([10, 7, 3, 255]));
    let tint = Color::from_rgba8(255, 158, 57, 255);
    let drew = if suppress {
        let ids = compute_sample_data_export_ids(&assets);
        println!("suppressed_export_ids={ids:?}");
        let dest = TskRect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
        draw_swf_stage_rgba_in_rect(&mut img, &assets, dest, tint, 1.0, &ids, &|_| None)
    } else {
        draw_swf_stage_rgba(&mut img, &assets, tint, 1.0)
    };
    println!("drew_any={drew}");

    // Per-column: any non-background pixel → measure horizontal extent.
    let mut lo = w;
    let mut hi = 0u32;
    for x in 0..w {
        for y in 0..h {
            let p = img.get_pixel(x, y).0;
            if p[0].max(p[1]).max(p[2]) > 40 && (p[0] as i32 - 10).abs() + (p[1] as i32 - 7).abs() > 30 {
                lo = lo.min(x);
                hi = hi.max(x);
                break;
            }
        }
    }
    if lo <= hi {
        println!(
            "content x={lo}-{hi} ({:.1}%-{:.1}%) width={:.1}%",
            100.0 * lo as f32 / w as f32,
            100.0 * hi as f32 / w as f32,
            100.0 * (hi - lo) as f32 / w as f32
        );
    } else {
        println!("no content drawn");
    }
    let out = "/tmp/swf_stage.png";
    img.save(out).expect("save");
    println!("wrote {out}");
}
