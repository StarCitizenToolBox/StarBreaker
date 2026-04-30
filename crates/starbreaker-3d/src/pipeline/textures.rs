use starbreaker_dds::DdsFile;
use starbreaker_p4k::MappedP4k;

use crate::mtl;
use crate::types::{MaterialTextures, TextureTransformInfo};

use super::{datacore_path_to_p4k, try_load_mtl, P4kSiblingReader};

pub(crate) type PngCache = std::collections::HashMap<String, Option<Vec<u8>>>;

pub(super) fn empty_material_textures(len: usize) -> MaterialTextures {
    MaterialTextures {
        diffuse: Vec::with_capacity(len),
        normal: Vec::with_capacity(len),
        roughness: Vec::with_capacity(len),
        emissive: Vec::with_capacity(len),
        occlusion: Vec::with_capacity(len),
        diffuse_transform: Vec::with_capacity(len),
        normal_transform: Vec::with_capacity(len),
        roughness_transform: Vec::with_capacity(len),
        emissive_transform: Vec::with_capacity(len),
        occlusion_transform: Vec::with_capacity(len),
        bundled_fallbacks: Vec::with_capacity(len),
    }
}

pub(super) fn push_fallback_tag(tags: &mut Vec<String>, tag: &str) {
    if !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_string());
    }
}

pub(super) fn make_texture_transform(scale: [f32; 2], tex_coord: u32) -> Option<TextureTransformInfo> {
    if tex_coord == 0
        && (scale[0] - 1.0).abs() <= 1e-4
        && (scale[1] - 1.0).abs() <= 1e-4
    {
        None
    } else {
        Some(TextureTransformInfo { scale, tex_coord })
    }
}

pub(super) fn material_uses_secondary_uv(material: &mtl::SubMaterial) -> bool {
    material.public_param_f32(&["UseUV2ForStencil"]).is_some_and(|value| value > 0.0)
        || material.string_gen_mask.contains("SECOND_UVS")
        || material.string_gen_mask.contains("EMISSIVE_SECOND_UVS")
}

pub(super) fn uniform_scale_transform(material: &mtl::SubMaterial, names: &[&str]) -> Option<[f32; 2]> {
    material
        .public_param_f32(names)
        .map(|value| value.abs())
        .filter(|value| *value > f32::EPSILON)
        .map(|value| [value, value])
}

pub(super) fn simple_texture_transform(
    material: &mtl::SubMaterial,
    role: Option<mtl::TextureSemanticRole>,
) -> Option<TextureTransformInfo> {
    use mtl::TextureSemanticRole;

    let tex_coord = if material_uses_secondary_uv(material) { 1 } else { 0 };
    let scale = match role {
        Some(TextureSemanticRole::ScreenPixelLayout) => {
            let sx = material
                .public_param_f32(&["PixelGridTilingX"])
                .unwrap_or(1.0)
                .abs();
            let sy = material
                .public_param_f32(&["PixelGridTilingY"])
                .unwrap_or(1.0)
                .abs();
            [sx.max(1.0), sy.max(1.0)]
        }
        Some(TextureSemanticRole::Breakup) => uniform_scale_transform(
            material,
            &["StencilBreakupTiling", "BreakupTiling", "Tiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        Some(TextureSemanticRole::BlendMask) => uniform_scale_transform(
            material,
            &["BlendMaskTiling", "Tiling", "LayerTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        Some(TextureSemanticRole::ScreenMask)
        | Some(TextureSemanticRole::WearGloss)
        | Some(TextureSemanticRole::Dirt)
        | Some(TextureSemanticRole::PatternMask) => uniform_scale_transform(
            material,
            &["GlassTiling", "Tiling", "LayerTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        Some(TextureSemanticRole::WearMask)
        | Some(TextureSemanticRole::HalControl) => uniform_scale_transform(
            material,
            &["Tiling", "LayerTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        _ => uniform_scale_transform(
            material,
            &["StencilTiling", "GlassTiling", "Tiling", "LayerTiling", "MacroTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
    };

    make_texture_transform(scale, tex_coord)
}

pub(super) fn decode_png(bytes: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()
        .map(|image| image.to_rgba8())
}

pub(super) fn encode_png(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut png_buf = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut png_buf),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(png_buf)
}

pub(super) fn make_solid_image(width: u32, height: u32, color: [f32; 3], alpha: u8) -> image::RgbaImage {
    let red = (color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let green = (color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let blue = (color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    image::RgbaImage::from_pixel(width.max(1), height.max(1), image::Rgba([red, green, blue, alpha]))
}

pub(super) fn sample_pixel(
    image: &image::RgbaImage,
    x: u32,
    y: u32,
    target_width: u32,
    target_height: u32,
) -> image::Rgba<u8> {
    let src_x = if target_width <= 1 || image.width() <= 1 {
        0
    } else {
        x.saturating_mul(image.width().saturating_sub(1))
            / target_width.saturating_sub(1)
    };
    let src_y = if target_height <= 1 || image.height() <= 1 {
        0
    } else {
        y.saturating_mul(image.height().saturating_sub(1))
            / target_height.saturating_sub(1)
    };
    *image.get_pixel(
        src_x.min(image.width().saturating_sub(1)),
        src_y.min(image.height().saturating_sub(1)),
    )
}

pub(super) fn sample_luma(
    image: &image::RgbaImage,
    x: u32,
    y: u32,
    target_width: u32,
    target_height: u32,
) -> f32 {
    let pixel = sample_pixel(image, x, y, target_width, target_height);
    (f32::from(pixel[0]) + f32::from(pixel[1]) + f32::from(pixel[2])) / (255.0 * 3.0)
}

pub(super) fn tint_image(image: &image::RgbaImage, color: [f32; 3]) -> image::RgbaImage {
    let mut tinted = image.clone();
    for pixel in tinted.pixels_mut() {
        pixel[0] = (f32::from(pixel[0]) * color[0].clamp(0.0, 1.0)).round() as u8;
        pixel[1] = (f32::from(pixel[1]) * color[1].clamp(0.0, 1.0)).round() as u8;
        pixel[2] = (f32::from(pixel[2]) * color[2].clamp(0.0, 1.0)).round() as u8;
    }
    tinted
}

pub(super) fn first_role_path(
    material: &mtl::SubMaterial,
    roles: &[mtl::TextureSemanticRole],
) -> Option<String> {
    roles
        .iter()
        .find_map(|role| material.first_texture_path_for_role(*role))
}

pub(super) fn load_texture_png(
    p4k: &MappedP4k,
    path: &str,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    cached_load(p4k, path, mip, png_cache, load_diffuse_texture)
}

pub(super) fn load_semantic_texture_png(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    roles: &[mtl::TextureSemanticRole],
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let path = first_role_path(material, roles)?;
    load_texture_png(p4k, &path, mip, png_cache)
}

pub(super) fn load_layer_diffuse_png(
    p4k: &MappedP4k,
    layer: &mtl::MatLayer,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    if layer.path.is_empty() {
        return None;
    }

    let p4k_path = datacore_path_to_p4k(&layer.path);
    let layer_mtl = try_load_mtl(p4k, &p4k_path)?;
    let layer_material = mtl::resolve_layer_submaterial(&layer_mtl, &layer.sub_material)?;
    let texture_path = layer_material.diffuse_tex.as_ref()?;
    load_texture_png(p4k, texture_path, mip, png_cache)
}

pub(super) fn build_layer_source_image(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    layer: &mtl::MatLayer,
    palette: Option<&mtl::TintPalette>,
    mip: u32,
    png_cache: &mut PngCache,
    canvas_size: Option<(u32, u32)>,
) -> Option<image::RgbaImage> {
    let color = material.resolved_layer_color(layer, palette);
    if let Some(layer_png) = load_layer_diffuse_png(p4k, layer, mip, png_cache) {
        return decode_png(&layer_png).map(|image| tint_image(&image, color));
    }

    let (width, height) = canvas_size.unwrap_or((64, 64));
    Some(make_solid_image(width, height, color, 255))
}

pub(super) fn build_layered_base_color_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    palette: Option<&mtl::TintPalette>,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let base_layer = material.layers.first()?;
    let base_image = build_layer_source_image(p4k, material, base_layer, palette, mip, png_cache, None)?;
    let mut output = base_image.clone();

    if let Some(overlay_layer) = material.layers.get(1) {
        let overlay_image = build_layer_source_image(
            p4k,
            material,
            overlay_layer,
            palette,
            mip,
            png_cache,
            Some((output.width(), output.height())),
        )?;
        let blend_mask = load_semantic_texture_png(
            p4k,
            material,
            &[
                mtl::TextureSemanticRole::BlendMask,
                mtl::TextureSemanticRole::WearMask,
                mtl::TextureSemanticRole::Breakup,
                mtl::TextureSemanticRole::Dirt,
            ],
            mip,
            png_cache,
        )
        .and_then(|png| decode_png(&png));
        let blend_factor = material
            .public_param_f32(&["BlendFactor", "WearBlendBase"])
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);

        for y in 0..output.height() {
            for x in 0..output.width() {
                let base_pixel = *output.get_pixel(x, y);
                let overlay_pixel = sample_pixel(&overlay_image, x, y, output.width(), output.height());
                let mask = blend_mask
                    .as_ref()
                    .map(|image| sample_luma(image, x, y, output.width(), output.height()))
                    .unwrap_or(blend_factor)
                    .clamp(0.0, 1.0);
                let inv = 1.0 - mask;
                output.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        (f32::from(base_pixel[0]) * inv + f32::from(overlay_pixel[0]) * mask)
                            .round() as u8,
                        (f32::from(base_pixel[1]) * inv + f32::from(overlay_pixel[1]) * mask)
                            .round() as u8,
                        (f32::from(base_pixel[2]) * inv + f32::from(overlay_pixel[2]) * mask)
                            .round() as u8,
                        255,
                    ]),
                );
            }
        }
    }

    encode_png(&output)
}

pub(super) fn build_illum_blend_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    base_color_png: Option<&Vec<u8>>,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let base_image = base_color_png.and_then(|png| decode_png(png))?;
    let alternate_path = first_role_path(
        material,
        &[
            mtl::TextureSemanticRole::AlternateBaseColor,
            mtl::TextureSemanticRole::DecalSheet,
        ],
    )?;
    let alternate_png = load_texture_png(p4k, &alternate_path, mip, png_cache)?;
    let alternate_image = decode_png(&alternate_png)?;
    let blend_mask = load_semantic_texture_png(
        p4k,
        material,
        &[mtl::TextureSemanticRole::BlendMask],
        mip,
        png_cache,
    )
    .and_then(|png| decode_png(&png));
    let blend_factor = material
        .public_param_f32(&["BlendFactor"])
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);

    let mut output = base_image.clone();
    for y in 0..output.height() {
        for x in 0..output.width() {
            let base_pixel = sample_pixel(&base_image, x, y, output.width(), output.height());
            let alternate_pixel = sample_pixel(&alternate_image, x, y, output.width(), output.height());
            let mask = blend_mask
                .as_ref()
                .map(|image| sample_luma(image, x, y, output.width(), output.height()))
                .unwrap_or(blend_factor)
                .clamp(0.0, 1.0);
            let inv = 1.0 - mask;
            output.put_pixel(
                x,
                y,
                image::Rgba([
                    (f32::from(base_pixel[0]) * inv + f32::from(alternate_pixel[0]) * mask)
                        .round() as u8,
                    (f32::from(base_pixel[1]) * inv + f32::from(alternate_pixel[1]) * mask)
                        .round() as u8,
                    (f32::from(base_pixel[2]) * inv + f32::from(alternate_pixel[2]) * mask)
                        .round() as u8,
                    255,
                ]),
            );
        }
    }

    encode_png(&output)
}

pub(super) fn build_stencil_fallback_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    palette: Option<&mtl::TintPalette>,
    base_color_png: Option<&Vec<u8>>,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let decoded = material.decoded_string_gen_mask();
    if !decoded.has_stencil_map && !material.has_virtual_input("$TintPaletteDecal") {
        return None;
    }

    let base_image = base_color_png.and_then(|png| decode_png(png));
    let stencil_image = load_semantic_texture_png(
        p4k,
        material,
        &[
            mtl::TextureSemanticRole::Stencil,
            mtl::TextureSemanticRole::PatternMask,
        ],
        mip,
        png_cache,
    )
    .and_then(|png| decode_png(&png));
    let breakup_image = load_semantic_texture_png(
        p4k,
        material,
        &[
            mtl::TextureSemanticRole::Breakup,
            mtl::TextureSemanticRole::Dirt,
        ],
        mip,
        png_cache,
    )
    .and_then(|png| decode_png(&png));

    let (width, height) = base_image
        .as_ref()
        .map(|image| (image.width(), image.height()))
        .or_else(|| stencil_image.as_ref().map(|image| (image.width(), image.height())))
        .or_else(|| breakup_image.as_ref().map(|image| (image.width(), image.height())))
        .unwrap_or((64, 64));

    let mut output = base_image.unwrap_or_else(|| make_solid_image(width, height, [0.0, 0.0, 0.0], 0));
    let stencil_color = material
        .public_param_rgb(&[
            "StencilDiffuseColor1",
            "StencilDiffuse1",
            "StencilTintColor",
            "TintColor",
        ])
        .or_else(|| material.resolved_palette_color(palette))
        .unwrap_or(material.diffuse);
    let opacity = material
        .public_param_f32(&["StencilOpacity", "DecalDiffuseOpacity", "DecalAlphaMult"])
        .unwrap_or(if material.is_decal() { 0.85 } else { 0.5 })
        .clamp(0.0, 1.0);
    let color = [
        (stencil_color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (stencil_color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (stencil_color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    ];

    for y in 0..height {
        for x in 0..width {
            let mask = stencil_image
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let breakup = breakup_image
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let blend = (opacity * mask * (0.35 + 0.65 * breakup)).clamp(0.0, 1.0);
            let mut base_pixel = *output.get_pixel(x, y);

            if material.is_decal() && base_color_png.is_none() {
                base_pixel = image::Rgba([
                    color[0],
                    color[1],
                    color[2],
                    (blend * 255.0).round() as u8,
                ]);
            } else {
                let inv = 1.0 - blend;
                base_pixel = image::Rgba([
                    (f32::from(base_pixel[0]) * inv + f32::from(color[0]) * blend).round() as u8,
                    (f32::from(base_pixel[1]) * inv + f32::from(color[1]) * blend).round() as u8,
                    (f32::from(base_pixel[2]) * inv + f32::from(color[2]) * blend).round() as u8,
                    if material.is_decal() {
                        (blend * 255.0).round() as u8
                    } else {
                        255
                    },
                ]);
            }

            output.put_pixel(x, y, base_pixel);
        }
    }

    encode_png(&output)
}

pub(super) fn build_screen_placeholder_textures(
    material: &mtl::SubMaterial,
    support_mask_png: Option<Vec<u8>>,
    pixel_layout_png: Option<Vec<u8>>,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let family = material.shader_family();
    if !matches!(family, mtl::ShaderFamily::DisplayScreen | mtl::ShaderFamily::UiPlane)
        && !material.has_virtual_input("$RenderToTexture")
    {
        return None;
    }

    let support_mask = support_mask_png.as_deref().and_then(decode_png);
    let pixel_layout = pixel_layout_png.as_deref().and_then(decode_png);
    let (width, height) = support_mask
        .as_ref()
        .map(|image| (image.width(), image.height()))
        .or_else(|| pixel_layout.as_ref().map(|image| (image.width(), image.height())))
        .unwrap_or((96, 64));

    let back_color = material
        .public_param_rgb(&["BackColour"])
        .or_else(|| {
            let emissive = material.emissive_factor();
            if emissive == [0.0, 0.0, 0.0] {
                None
            } else {
                Some(emissive)
            }
        })
        .unwrap_or([0.08, 0.22, 0.35]);
    let accent_color = [
        (back_color[0] * 1.6 + 0.15).clamp(0.0, 1.0),
        (back_color[1] * 1.5 + 0.15).clamp(0.0, 1.0),
        (back_color[2] * 1.8 + 0.20).clamp(0.0, 1.0),
    ];
    let grid_x = material
        .public_param_f32(&["PixelGridTilingX"])
        .unwrap_or(8.0)
        .abs()
        .max(1.0);
    let grid_y = material
        .public_param_f32(&["PixelGridTilingY"])
        .unwrap_or(6.0)
        .abs()
        .max(1.0);

    let mut diffuse = image::RgbaImage::new(width, height);
    let mut emissive = image::RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let u = if width <= 1 {
                0.0
            } else {
                x as f32 / (width - 1) as f32
            };
            let v = if height <= 1 {
                0.0
            } else {
                y as f32 / (height - 1) as f32
            };

            let stripe = if (u * grid_x * 4.0).fract() < 0.08 || (v * grid_y * 4.0).fract() < 0.08 {
                1.0
            } else {
                0.0
            };
            let scanline = if ((v * height.max(2) as f32 / 2.0).fract()) < 0.5 {
                0.92
            } else {
                0.74
            };
            let support = support_mask
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let pixel_grid = pixel_layout
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(stripe)
                .clamp(0.0, 1.0);

            let base_mix = (0.12 + 0.24 * support + 0.10 * pixel_grid).clamp(0.0, 1.0);
            let emissive_mix = (0.35 + 0.65 * pixel_grid.max(stripe)) * support * scanline;
            diffuse.put_pixel(
                x,
                y,
                image::Rgba([
                    (back_color[0] * base_mix * 255.0 + accent_color[0] * stripe * 28.0)
                        .clamp(0.0, 255.0)
                        .round() as u8,
                    (back_color[1] * base_mix * 255.0 + accent_color[1] * stripe * 28.0)
                        .clamp(0.0, 255.0)
                        .round() as u8,
                    (back_color[2] * base_mix * 255.0 + accent_color[2] * stripe * 28.0)
                        .clamp(0.0, 255.0)
                        .round() as u8,
                    255,
                ]),
            );
            emissive.put_pixel(
                x,
                y,
                image::Rgba([
                    (accent_color[0] * emissive_mix * 255.0).clamp(0.0, 255.0).round() as u8,
                    (accent_color[1] * emissive_mix * 255.0).clamp(0.0, 255.0).round() as u8,
                    (accent_color[2] * emissive_mix * 255.0).clamp(0.0, 255.0).round() as u8,
                    255,
                ]),
            );
        }
    }

    Some((encode_png(&diffuse)?, encode_png(&emissive)?))
}

pub(super) fn build_emissive_texture(
    material: &mtl::SubMaterial,
    base_color_png: Option<&Vec<u8>>,
    screen_emissive_png: Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if let Some(emissive_png) = screen_emissive_png {
        return Some(emissive_png);
    }

    let emissive = material.emissive_factor();
    if emissive == [0.0, 0.0, 0.0] {
        return None;
    }

    let mut image = base_color_png
        .and_then(|png| decode_png(png))
        .unwrap_or_else(|| make_solid_image(64, 64, material.diffuse, 255));
    let scale = [
        emissive[0].clamp(0.0, 1.0),
        emissive[1].clamp(0.0, 1.0),
        emissive[2].clamp(0.0, 1.0),
    ];
    for pixel in image.pixels_mut() {
        pixel[0] = (f32::from(pixel[0]) * scale[0]).round() as u8;
        pixel[1] = (f32::from(pixel[1]) * scale[1]).round() as u8;
        pixel[2] = (f32::from(pixel[2]) * scale[2]).round() as u8;
    }
    encode_png(&image)
}

pub(super) fn convert_png_to_occlusion(png_bytes: &[u8], invert: bool) -> Option<Vec<u8>> {
    let source = decode_png(png_bytes)?;
    let mut image = image::RgbaImage::new(source.width(), source.height());
    for (x, y, pixel) in source.enumerate_pixels() {
        let luminance = ((u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2])) / 3) as u8;
        let occlusion = if invert { 255u8.saturating_sub(luminance) } else { luminance };
        image.put_pixel(x, y, image::Rgba([occlusion, occlusion, occlusion, 255]));
    }
    encode_png(&image)
}

pub(super) fn build_occlusion_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<(Vec<u8>, &'static str)> {
    let height_path = first_role_path(material, &[mtl::TextureSemanticRole::Height]);
    if let Some(path) = height_path {
        let png = load_texture_png(p4k, &path, mip, png_cache)?;
        return convert_png_to_occlusion(&png, true).map(|bytes| (bytes, "height"));
    }

    let mask_path = first_role_path(
        material,
        &[
            mtl::TextureSemanticRole::Dirt,
            mtl::TextureSemanticRole::WearMask,
            mtl::TextureSemanticRole::BlendMask,
            mtl::TextureSemanticRole::Breakup,
            mtl::TextureSemanticRole::PatternMask,
            mtl::TextureSemanticRole::ScreenMask,
            mtl::TextureSemanticRole::WearGloss,
        ],
    );
    let path = mask_path?;
    let png = load_texture_png(p4k, &path, mip, png_cache)?;
    convert_png_to_occlusion(&png, false).map(|bytes| (bytes, "mask"))
}

pub(super) fn load_material_textures(
    p4k: &MappedP4k,
    mtl: &mtl::MtlFile,
    palette: Option<&mtl::TintPalette>,
    mip: u32,
    png_cache: &mut PngCache,
    include_normals: bool,
    experimental_textures: bool,
) -> MaterialTextures {
    let mut textures = empty_material_textures(mtl.materials.len());

    for material in &mtl.materials {
        let mut fallback_tags = Vec::new();

        let screen_mask = load_semantic_texture_png(
            p4k,
            material,
            &[
                mtl::TextureSemanticRole::ScreenMask,
                mtl::TextureSemanticRole::PatternMask,
            ],
            mip,
            png_cache,
        );
        let pixel_layout = load_semantic_texture_png(
            p4k,
            material,
            &[mtl::TextureSemanticRole::ScreenPixelLayout],
            mip,
            png_cache,
        );
        let screen_placeholder = build_screen_placeholder_textures(material, screen_mask, pixel_layout);

        let direct_diffuse = material
            .diffuse_tex
            .as_ref()
            .and_then(|path| load_texture_png(p4k, path, mip, png_cache));
        let prefer_layered_base = !material.layers.is_empty()
            && matches!(
                material.shader_family(),
                mtl::ShaderFamily::HardSurface
                    | mtl::ShaderFamily::LayerBlendV2
                    | mtl::ShaderFamily::Layer
                    | mtl::ShaderFamily::Illum
            );
        let layered_base = if prefer_layered_base {
            build_layered_base_color_texture(p4k, material, palette, mip, png_cache)
        } else {
            None
        };
        let used_layered_base = layered_base.is_some();
        let mut diffuse = if used_layered_base {
            layered_base.clone().or(direct_diffuse.clone())
        } else {
            direct_diffuse.clone().or(layered_base.clone())
        };
        if used_layered_base {
            push_fallback_tag(&mut fallback_tags, "layered_base_color");
        }

        if matches!(material.shader_family(), mtl::ShaderFamily::Illum) {
            if let Some(blended) = build_illum_blend_texture(p4k, material, diffuse.as_ref(), mip, png_cache) {
                diffuse = Some(blended);
                push_fallback_tag(&mut fallback_tags, "illum_blend_fallback");
            }
        }

        if let Some(stencil) = build_stencil_fallback_texture(p4k, material, palette, diffuse.as_ref(), mip, png_cache) {
            diffuse = Some(stencil);
            push_fallback_tag(&mut fallback_tags, "stencil_fallback");
        }

        if diffuse.is_none() {
            if let Some((placeholder_diffuse, _)) = screen_placeholder.as_ref() {
                diffuse = Some(placeholder_diffuse.clone());
                push_fallback_tag(&mut fallback_tags, "rtt_placeholder");
            }
        }

        let normal_path = if let Some(path) = &material.normal_tex {
            Some(path.clone())
        } else {
            material.layers.first().and_then(|layer| {
                let p4k_path = datacore_path_to_p4k(&layer.path);
                try_load_mtl(p4k, &p4k_path).and_then(|layer_mtl| {
                    layer_mtl
                        .materials
                        .first()
                        .and_then(|layer_material| layer_material.normal_tex.clone())
                })
            })
        };

        let normal = if !include_normals {
            None
        } else if let Some(path) = normal_path.as_ref() {
            if !experimental_textures {
                if let Some(diffuse_path) = material.diffuse_tex.as_ref() {
                    if !textures_share_uv_space(diffuse_path, path) {
                        log::debug!("  skipping mismatched normal: diffuse={diffuse_path}, normal={path}");
                        None
                    } else {
                        cached_load(p4k, path, mip, png_cache, load_normal_texture)
                    }
                } else {
                    cached_load(p4k, path, mip, png_cache, load_normal_texture)
                }
            } else {
                cached_load(p4k, path, mip, png_cache, load_normal_texture)
            }
        } else {
            None
        };

        let roughness = if !include_normals {
            None
        } else if let Some(path) = normal_path.as_ref() {
            if !path.contains("_ddna") {
                None
            } else {
                if !experimental_textures {
                    if let Some(diffuse_path) = material.diffuse_tex.as_ref() {
                        if !textures_share_uv_space(diffuse_path, path) {
                            None
                        } else {
                            let cache_key = format!("{path}@roughness_mip{mip}");
                            if let Some(cached) = png_cache.get(&cache_key) {
                                cached.clone()
                            } else {
                                let result = load_roughness_texture(p4k, path, mip);
                                png_cache.insert(cache_key, result.clone());
                                result
                            }
                        }
                    } else {
                        let cache_key = format!("{path}@roughness_mip{mip}");
                        if let Some(cached) = png_cache.get(&cache_key) {
                            cached.clone()
                        } else {
                            let result = load_roughness_texture(p4k, path, mip);
                            png_cache.insert(cache_key, result.clone());
                            result
                        }
                    }
                } else {
                    let cache_key = format!("{path}@roughness_mip{mip}");
                    if let Some(cached) = png_cache.get(&cache_key) {
                        cached.clone()
                    } else {
                        let result = load_roughness_texture(p4k, path, mip);
                        png_cache.insert(cache_key, result.clone());
                        result
                    }
                }
            }
        } else {
            None
        };

        let emissive = build_emissive_texture(
            material,
            diffuse.as_ref(),
            screen_placeholder.as_ref().map(|(_, emissive)| emissive.clone()),
        );
        if emissive.is_some() {
            push_fallback_tag(&mut fallback_tags, if material.has_virtual_input("$RenderToTexture") {
                "screen_emissive_placeholder"
            } else {
                "emissive_texture"
            });
        }

        let occlusion = build_occlusion_texture(p4k, material, mip, png_cache).map(|(bytes, source)| {
            push_fallback_tag(
                &mut fallback_tags,
                if source == "height" {
                    "occlusion_from_height"
                } else {
                    "occlusion_from_mask"
                },
            );
            bytes
        });

        textures.diffuse.push(diffuse.clone());
        textures.normal.push(normal.clone());
        textures.roughness.push(roughness.clone());
        textures.emissive.push(emissive.clone());
        textures.occlusion.push(occlusion.clone());
        textures.diffuse_transform.push(
            diffuse
                .as_ref()
                .and_then(|_| simple_texture_transform(material, Some(mtl::TextureSemanticRole::BaseColor))),
        );
        textures.normal_transform.push(
            normal
                .as_ref()
                .and_then(|_| simple_texture_transform(material, Some(mtl::TextureSemanticRole::NormalGloss))),
        );
        textures.roughness_transform.push(
            roughness
                .as_ref()
                .and_then(|_| simple_texture_transform(material, Some(mtl::TextureSemanticRole::Height))),
        );
        textures.emissive_transform.push(
            emissive
                .as_ref()
                .and_then(|_| {
                    if matches!(material.shader_family(), mtl::ShaderFamily::DisplayScreen | mtl::ShaderFamily::UiPlane)
                        || material.has_virtual_input("$RenderToTexture")
                    {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::ScreenPixelLayout))
                            .or_else(|| simple_texture_transform(material, Some(mtl::TextureSemanticRole::ScreenMask)))
                    } else {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::BaseColor))
                    }
                }),
        );
        textures.occlusion_transform.push(
            occlusion
                .as_ref()
                .and_then(|_| {
                    if material.decoded_string_gen_mask().has_parallax_occlusion_mapping {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::Height))
                    } else {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::BlendMask))
                            .or_else(|| simple_texture_transform(material, Some(mtl::TextureSemanticRole::Dirt)))
                    }
                }),
        );
        textures.bundled_fallbacks.push(fallback_tags);
    }

    textures
}

/// Check if a diffuse and normal texture are from the same texture set (same UV space).
///
/// CryEngine materials can pair atlas diffuse textures (unique UV layout per mesh) with
/// tileable normal maps (designed to repeat). These use different UV mappings but we only
/// support one texCoord in glTF. When they don't match, the normal/roughness creates noise.
///
/// Heuristic: extract the filename stem (strip path + suffixes like `_diff`, `_ddna`) and
/// check if they share a common base. E.g., `cockpit_diff.tif` + `cockpit_ddna.tif` → match.
/// `leather_atlas_a_diff.tif` + `leather_base_tilable_ddna.dds` → no match.
pub(super) fn textures_share_uv_space(diffuse_path: &str, normal_path: &str) -> bool {
    fn stem(p: &str) -> &str {
        let filename = p.rsplit(&['/', '\\']).next().unwrap_or(p);
        let base = filename.split('.').next().unwrap_or(filename);
        let base = base.strip_suffix("_diff").unwrap_or(base);
        let base = base.strip_suffix("_ddna").unwrap_or(base);
        let base = base.strip_suffix("_ddn").unwrap_or(base);
        let base = base.strip_suffix("_spec").unwrap_or(base);
        base
    }
    let d = stem(diffuse_path);
    let n = stem(normal_path);
    d == n || d.starts_with(n) || n.starts_with(d)
}

/// Load a texture with caching by path — prevents redundant DDS decode + PNG encode.
pub(crate) fn cached_load(
    p4k: &MappedP4k,
    path: &str,
    mip: u32,
    cache: &mut PngCache,
    loader: fn(&MappedP4k, &str, u32) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let key = format!("{path}@mip{mip}");
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let result = loader(p4k, path, mip);
    cache.insert(key, result.clone());
    result
}

pub(super) fn encode_png_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png_buf = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_buf),
        image::ImageFormat::Png,
    )
    .ok()?;
    Some(png_buf)
}

pub(crate) fn load_diffuse_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
    if tif_path.starts_with('$') {
        return None;
    }

    let dds_path = tif_path
        .strip_suffix(".tif")
        .map(|base| format!("{base}.dds"))
        .unwrap_or_else(|| tif_path.to_string());

    let p4k_dds_path = datacore_path_to_p4k(&dds_path);
    let base_entry = p4k.entry_case_insensitive(&p4k_dds_path)?;
    let base_bytes = p4k.read(base_entry).ok()?;

    let sibling_reader = P4kSiblingReader {
        p4k,
        base_path: p4k_dds_path,
    };
    let dds = DdsFile::from_split(&base_bytes, &sibling_reader).ok()?;

    // Use requested mip level, clamped to available levels
    let mip = (mip_level as usize).min(dds.mip_count().saturating_sub(1));
    let (w, h) = dds.dimensions(mip);
    let rgba = dds.decode_rgba(mip).ok()?;

    encode_png_rgba(w, h, rgba)
}

/// Load a normal-gloss texture as a PNG while preserving DDNA smoothness in alpha.
///
/// The RGB channels come from the decoded normal texture. When sibling alpha mips
/// are present, their smoothness values are copied into the PNG alpha channel so
/// downstream consumers can derive roughness without Rust-side reinterpretation.
pub(crate) fn load_normal_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
    if tif_path.starts_with('$') {
        return None;
    }

    // Only load actual normal maps (_ddna/_ddn), not specular/other textures
    // that happen to be in TexSlot2.
    let lower = tif_path.to_lowercase();
    if !lower.contains("_ddna") && !lower.contains("_ddn.") && !lower.contains("_ddn_") {
        log::debug!("  skipping non-normal in TexSlot2: {tif_path}");
        return None;
    }

    let dds_path = tif_path
        .strip_suffix(".tif")
        .map(|base| format!("{base}.dds"))
        .unwrap_or_else(|| tif_path.to_string());

    let p4k_dds_path = datacore_path_to_p4k(&dds_path);
    let base_entry = p4k.entry_case_insensitive(&p4k_dds_path)?;
    let base_bytes = p4k.read(base_entry).ok()?;

    let sibling_reader = P4kSiblingReader {
        p4k,
        base_path: p4k_dds_path,
    };
    let dds = DdsFile::from_split(&base_bytes, &sibling_reader).ok()?;

    let format = starbreaker_dds::resolve_format(
        &dds.header.pixel_format,
        dds.dxt10_header.as_ref(),
    );
    let (dw, dh) = ({ dds.header.width }, { dds.header.height });
    log::debug!("  normal: {tif_path} → {format:?}, {dw}x{dh}");

    let mip = (mip_level as usize).min(dds.mip_count().saturating_sub(1));
    let (w, h) = dds.dimensions(mip);
    let mut rgba = dds.decode_rgba(mip).ok()?;

    if dds.has_alpha_mips()
        && let Ok(smoothness) = dds.decode_alpha_mip(mip)
        && smoothness.len() * 4 == rgba.len()
    {
        for (index, value) in smoothness.iter().enumerate() {
            rgba[index * 4 + 3] = *value;
        }
    }

    encode_png_rgba(w, h, rgba)
}

/// Extract per-pixel roughness from the alpha mips of a _ddna normal map DDS.
///
/// CryEngine stores smoothness in separate sibling files (.7a, .6a, ...) as BC4 compressed.
/// We convert smoothness → roughness (1-smoothness) and pack into a glTF metallicRoughness
/// texture: R=0, G=roughness, B=metallic(0), A=255.
pub(crate) fn load_roughness_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
    let dds_path = tif_path
        .strip_suffix(".tif")
        .map(|base| format!("{base}.dds"))
        .unwrap_or_else(|| tif_path.to_string());

    let p4k_path = datacore_path_to_p4k(&dds_path);
    let entry = p4k.entry_case_insensitive(&p4k_path)?;
    let base_bytes = p4k.read(entry).ok()?;
    let sibling_reader = P4kSiblingReader {
        p4k,
        base_path: p4k_path,
    };
    let dds = DdsFile::from_split(&base_bytes, &sibling_reader).ok()?;

    if !dds.has_alpha_mips() {
        return None;
    }

    let mip = (mip_level as usize).min(dds.alpha_mip_data.len().saturating_sub(1));
    let (w, h) = dds.dimensions(mip);

    let smoothness = dds.decode_alpha_mip(mip).ok()?;
    let pixel_count = (w * h) as usize;
    if smoothness.len() != pixel_count {
        return None;
    }

    // Pack into glTF metallicRoughness format: R=0, G=roughness, B=metallic(0), A=255
    let mut rgba = vec![0u8; pixel_count * 4];
    for i in 0..pixel_count {
        let roughness = 255 - smoothness[i]; // roughness = 1 - smoothness
        rgba[i * 4] = 0;           // R: unused
        rgba[i * 4 + 1] = roughness; // G: roughness
        rgba[i * 4 + 2] = 0;       // B: metallic = 0
        rgba[i * 4 + 3] = 255;     // A: unused
    }

    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut png_buf = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_buf),
        image::ImageFormat::Png,
    )
    .ok()?;

    Some(png_buf)
}

