use std::fs::{self, File};
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=marmoset_logo_red.png");

    if let Err(err) = generate_shared_icon_assets() {
        panic!("failed to generate shared icon assets: {err}");
    }

    #[cfg(target_os = "windows")]
    {
        if let Err(err) = embed_windows_icon() {
            panic!("failed to embed Windows icon: {err}");
        }
    }
}

fn generate_shared_icon_assets() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let source_path = manifest_dir.join("marmoset_logo_red.png");
    let image = load_clean_icon(&source_path)?;

    let target_dir = cargo_target_dir()?;
    let output_dir = target_dir.join("generated-icons");
    fs::create_dir_all(&output_dir)?;

    for size in [64_u32, 128, 256, 512, 1024] {
        let resized = image::imageops::resize(
            &image,
            size,
            size,
            image::imageops::FilterType::Lanczos3,
        );
        resized.save(output_dir.join(format!("mviewer-{size}.png")))?;
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn embed_windows_icon() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let source_path = manifest_dir.join("marmoset_logo_red.png");
    let output_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let icon_path = output_dir.join("mviewer.ico");

    let image = load_clean_icon(&source_path)?;
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let resized = image::imageops::resize(
            &image,
            size,
            size,
            image::imageops::FilterType::Lanczos3,
        );
        let icon_image =
            ico::IconImage::from_rgba_data(size, size, resized.into_raw());
        icon_dir.add_entry(ico::IconDirEntry::encode(&icon_image)?);
    }

    let mut file = File::create(&icon_path)?;
    icon_dir.write(&mut file)?;

    winresource::WindowsResource::new()
        .set_icon(icon_path.to_string_lossy().as_ref())
        .compile()?;

    Ok(())
}

fn load_clean_icon(path: &Path) -> Result<image::RgbaImage, Box<dyn std::error::Error>> {
    let mut image = image::open(path)?.into_rgba8();
    for pixel in image.pixels_mut() {
        if pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245 {
            pixel[3] = 0;
        }
    }
    Ok(image)
}

fn cargo_target_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        return Ok(PathBuf::from(target_dir));
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let target_dir = out_dir
        .ancestors()
        .nth(4)
        .ok_or("failed to derive cargo target directory from OUT_DIR")?;
    Ok(target_dir.to_path_buf())
}
