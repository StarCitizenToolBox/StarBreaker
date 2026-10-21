//! Preview the radar-disc tilt-projection on a real decoded disc texture.
//!
//! `cargo run -p starbreaker-gfx --example radar_preview -- <disc_texture.png> [out.png] [tilt] [fit]`
use starbreaker_gfx::radar_plane::{project_radar_disc, RadarPlaneParams};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tex_path = args.get(1).map(String::as_str).unwrap_or("/tmp/radar_disc_tex.png");
    let out = args.get(2).map(String::as_str).unwrap_or("/tmp/radar_preview.png");
    let mut p = RadarPlaneParams::default();
    if let Some(v) = args.get(3).and_then(|s| s.parse().ok()) {
        p.tilt_deg = v;
    }
    if let Some(v) = args.get(4).and_then(|s| s.parse().ok()) {
        p.fit = v;
    }
    let tex = image::open(tex_path).expect("load texture").to_rgba8();
    let (w, h) = (1024u32, 834u32);
    let disc = project_radar_disc(w, h, &tex, None, None, &p);
    // Composite over a dark DRAK-vignette-ish background to mimic the screen.
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([34, 22, 10, 255]));
    for (x, y, px) in disc.enumerate_pixels() {
        let a = px.0[3] as f32 / 255.0;
        if a > 0.0 {
            let bg = img.get_pixel_mut(x, y);
            for i in 0..3 {
                bg.0[i] = (px.0[i] as f32 * a + bg.0[i] as f32 * (1.0 - a)).round() as u8;
            }
        }
    }
    img.save(out).expect("save");
    println!("wrote {out} (tilt={} fit={})", p.tilt_deg, p.fit);
}
