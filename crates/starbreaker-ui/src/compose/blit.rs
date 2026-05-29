//! Pixel blit and pixmap conversion helpers.

use image::{Rgba, RgbaImage};
use tiny_skia::{IntSize, Pixmap, PixmapPaint, Transform};

use crate::error::UiError;

pub(crate) fn blit_atlas_image(pixmap: &mut Pixmap, img: &RgbaImage, dx: i32, dy: i32, alpha: f32) {
    blit_atlas_image_tinted(pixmap, img, dx, dy, [1.0, 1.0, 1.0, 1.0], alpha);
}

pub(crate) fn blit_atlas_image_tinted(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
) {
    let w = img.width();
    let h = img.height();

    let mut premul: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
    for chunk in img.as_raw().chunks_exact(4) {
        let r = chunk[0] as f32 / 255.0 * tint[0];
        let g = chunk[1] as f32 / 255.0 * tint[1];
        let b = chunk[2] as f32 / 255.0 * tint[2];
        let a = chunk[3] as f32 / 255.0 * tint[3];
        premul.push((r * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((g * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((b * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((a * 255.0).clamp(0.0, 255.0) as u8);
    }

    let Some(size) = IntSize::from_wh(w, h) else {
        return;
    };
    let Some(src_pixmap) = Pixmap::from_vec(premul, size) else {
        return;
    };

    let mut paint = PixmapPaint::default();
    paint.opacity = alpha.clamp(0.0, 1.0);
    pixmap
        .as_mut()
        .draw_pixmap(dx, dy, src_pixmap.as_ref(), &paint, Transform::identity(), None);
}

pub(crate) fn blit_atlas_image_alpha_mask_tinted(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
) {
    let w = img.width();
    let h = img.height();
    let mut premul: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
    for chunk in img.as_raw().chunks_exact(4) {
        let src_a = chunk[3] as f32 / 255.0;
        let a = (src_a * tint[3]).clamp(0.0, 1.0);
        let r = tint[0].clamp(0.0, 1.0);
        let g = tint[1].clamp(0.0, 1.0);
        let b = tint[2].clamp(0.0, 1.0);
        premul.push((r * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((g * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((b * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((a * 255.0).clamp(0.0, 255.0) as u8);
    }
    let Some(size) = IntSize::from_wh(w, h) else {
        return;
    };
    let Some(src_pixmap) = Pixmap::from_vec(premul, size) else {
        return;
    };
    let mut paint = PixmapPaint::default();
    paint.opacity = alpha.clamp(0.0, 1.0);
    pixmap
        .as_mut()
        .draw_pixmap(dx, dy, src_pixmap.as_ref(), &paint, Transform::identity(), None);
}

pub(crate) fn pixmap_to_rgba_image(pixmap: Pixmap) -> Result<RgbaImage, UiError> {
    let w = pixmap.width();
    let h = pixmap.height();
    let mut data = pixmap.take();

    for pixel in data.chunks_exact_mut(4) {
        let a = pixel[3] as u32;
        if a > 0 && a < 255 {
            pixel[0] = (pixel[0] as u32 * 255 / a).min(255) as u8;
            pixel[1] = (pixel[1] as u32 * 255 / a).min(255) as u8;
            pixel[2] = (pixel[2] as u32 * 255 / a).min(255) as u8;
        }
    }

    RgbaImage::from_raw(w, h, data)
        .ok_or_else(|| UiError::RenderError("pixmap->RgbaImage conversion failed".into()))
}

pub(crate) fn magenta_placeholder(w: u32, h: u32) -> Result<RgbaImage, UiError> {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([255, 0, 255, 255]));
    for y in 0..h {
        for x in 0..w {
            if x % 64 == 0 || y % 64 == 0 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
    }
    Ok(img)
}
