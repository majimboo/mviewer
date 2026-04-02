use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{DynamicImage, imageops::FilterType};

pub fn merged_alpha_name(albedo_name: &str) -> String {
    let path = PathBuf::from(albedo_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("basecolor");
    format!("{stem}_rgba.png")
}

pub fn merge_alpha_texture(
    albedo_bytes: &[u8],
    alpha_bytes: &[u8],
    output_path: &Path,
) -> Result<()> {
    let albedo = image::load_from_memory(albedo_bytes).context("failed to decode albedo image")?;
    let alpha = image::load_from_memory(alpha_bytes).context("failed to decode alpha image")?;

    let mut rgba = albedo.to_rgba8();
    let alpha_rgba = match alpha {
        DynamicImage::ImageLuma8(img) => DynamicImage::ImageLuma8(img).to_luma8(),
        DynamicImage::ImageLuma16(img) => DynamicImage::ImageLuma16(img).to_luma8(),
        other => other.to_luma8(),
    };

    let alpha_rgba = if rgba.dimensions() != alpha_rgba.dimensions() {
        image::imageops::resize(
            &alpha_rgba,
            rgba.width(),
            rgba.height(),
            FilterType::Triangle,
        )
    } else {
        alpha_rgba
    };

    for y in 0..rgba.height() {
        for x in 0..rgba.width() {
            let alpha_value = alpha_rgba.get_pixel(x, y)[0];
            rgba.get_pixel_mut(x, y)[3] = alpha_value;
        }
    }

    rgba.save(output_path)
        .with_context(|| format!("failed to save {}", output_path.display()))
}

pub fn merged_metallic_roughness_name(reflectivity_name: &str) -> String {
    let path = PathBuf::from(reflectivity_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("metalrough");
    format!("{stem}_metalrough.png")
}

pub fn merge_metallic_roughness_texture(
    reflectivity_bytes: &[u8],
    gloss_bytes: &[u8],
    output_path: &Path,
) -> Result<()> {
    let reflectivity = image::load_from_memory(reflectivity_bytes)
        .context("failed to decode reflectivity image")?
        .to_luma8();
    let gloss = image::load_from_memory(gloss_bytes)
        .context("failed to decode gloss image")?
        .to_luma8();

    let gloss = if reflectivity.dimensions() != gloss.dimensions() {
        image::imageops::resize(
            &gloss,
            reflectivity.width(),
            reflectivity.height(),
            FilterType::Triangle,
        )
    } else {
        gloss
    };

    let mut rgba = image::RgbaImage::new(reflectivity.width(), reflectivity.height());
    for y in 0..rgba.height() {
        for x in 0..rgba.width() {
            let metal = reflectivity.get_pixel(x, y)[0];
            let roughness = 255u8.saturating_sub(gloss.get_pixel(x, y)[0]);
            rgba.put_pixel(x, y, image::Rgba([0, roughness, metal, 255]));
        }
    }

    rgba.save(output_path)
        .with_context(|| format!("failed to save {}", output_path.display()))
}
