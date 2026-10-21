use std::io::Read as _;

use flate2::read::ZlibDecoder;
use image::{ImageFormat, RgbaImage};

use crate::error::UiError;

pub(super) fn decode_lossless(bmp: &swf::DefineBitsLossless<'_>) -> Result<RgbaImage, UiError> {
    let width = u32::from(bmp.width);
    let height = u32::from(bmp.height);

    let mut raw: Vec<u8> = Vec::new();
    ZlibDecoder::new(bmp.data.as_ref()).read_to_end(&mut raw)?;

    match bmp.format {
        swf::BitmapFormat::Rgb32 => {
            let expected = (width * height * 4) as usize;
            if raw.len() < expected {
                return Err(UiError::SwfParse(format!(
                    "DefineBitsLossless id={}: Rgb32 decompressed size {} < expected {expected}",
                    bmp.id,
                    raw.len()
                )));
            }
            let mut img = RgbaImage::new(width, height);
            for (i, pixel) in img.pixels_mut().enumerate() {
                let base = i * 4;
                let a = if bmp.version == 2 { raw[base] } else { 255 };
                let r = raw[base + 1];
                let g = raw[base + 2];
                let b = raw[base + 3];
                *pixel = image::Rgba([r, g, b, a]);
            }
            Ok(img)
        }
        swf::BitmapFormat::ColorMap8 { num_colors } => {
            let palette_entries = usize::from(num_colors) + 1;
            let bytes_per_entry: usize = if bmp.version == 2 { 4 } else { 3 };
            let palette_bytes = palette_entries * bytes_per_entry;

            if raw.len() < palette_bytes {
                return Err(UiError::SwfParse(format!(
                    "DefineBitsLossless id={}: ColorMap8 palette truncated",
                    bmp.id
                )));
            }

            let row_stride = (width as usize + 3) & !3;
            let pixel_data = &raw[palette_bytes..];

            let mut img = RgbaImage::new(width, height);
            for row in 0..height as usize {
                for col in 0..width as usize {
                    let idx = pixel_data.get(row * row_stride + col).copied().unwrap_or(0) as usize;
                    let pal_off = idx * bytes_per_entry;
                    let (r, g, b, a) = if bmp.version == 2 {
                        (
                            raw.get(pal_off).copied().unwrap_or(0),
                            raw.get(pal_off + 1).copied().unwrap_or(0),
                            raw.get(pal_off + 2).copied().unwrap_or(0),
                            raw.get(pal_off + 3).copied().unwrap_or(255),
                        )
                    } else {
                        (
                            raw.get(pal_off).copied().unwrap_or(0),
                            raw.get(pal_off + 1).copied().unwrap_or(0),
                            raw.get(pal_off + 2).copied().unwrap_or(0),
                            255,
                        )
                    };
                    img.put_pixel(col as u32, row as u32, image::Rgba([r, g, b, a]));
                }
            }
            Ok(img)
        }
        swf::BitmapFormat::Rgb15 => {
            let expected = (width * height * 2) as usize;
            if raw.len() < expected {
                return Err(UiError::SwfParse(format!(
                    "DefineBitsLossless id={}: Rgb15 decompressed size {} < expected {expected}",
                    bmp.id,
                    raw.len()
                )));
            }
            let mut img = RgbaImage::new(width, height);
            for (i, pixel) in img.pixels_mut().enumerate() {
                let lo = raw[i * 2] as u16;
                let hi = raw[i * 2 + 1] as u16;
                let word = (hi << 8) | lo;
                let r = (((word >> 10) & 0x1F) * 255 / 31) as u8;
                let g = (((word >> 5) & 0x1F) * 255 / 31) as u8;
                let b = ((word & 0x1F) * 255 / 31) as u8;
                *pixel = image::Rgba([r, g, b, 255]);
            }
            Ok(img)
        }
    }
}

pub(super) fn decode_jpeg3(jpeg3: &swf::DefineBitsJpeg3<'_>) -> Result<RgbaImage, UiError> {
    let mut img = image::load_from_memory_with_format(jpeg3.data, ImageFormat::Jpeg)
        .map(|d| d.to_rgba8())?;

    if !jpeg3.alpha_data.is_empty() {
        let mut alpha: Vec<u8> = Vec::new();
        ZlibDecoder::new(jpeg3.alpha_data).read_to_end(&mut alpha)?;

        let pixel_count = (img.width() * img.height()) as usize;
        if alpha.len() >= pixel_count {
            for (i, pixel) in img.pixels_mut().enumerate() {
                pixel.0[3] = alpha[i];
            }
        } else {
            log::warn!(
                "DefineBitsJpeg3 id={}: alpha channel length {} < pixel count {pixel_count}",
                jpeg3.id,
                alpha.len()
            );
        }
    }

    Ok(img)
}
