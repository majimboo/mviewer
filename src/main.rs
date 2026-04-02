mod animation;
mod archive;
mod gltf;
mod mesh;
mod scene;
mod viewer;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use crate::animation::ParsedAnimationSet;
use crate::archive::Archive;
use crate::scene::Scene;

#[derive(Debug, Parser)]
#[command(name = "mviewer")]
#[command(about = "Export Marmoset .mview scenes to glTF")]
struct Cli {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    #[arg(value_name = "OUTPUT_DIR")]
    output_dir: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let output_dir = cli
        .output_dir
        .unwrap_or_else(|| default_output_dir(&cli.input));

    let archive = Archive::from_path(&cli.input)?;
    let scene_entry = archive
        .get("scene.json")
        .context("scene.json not found in archive")?;
    let scene = Scene::from_bytes(&scene_entry.data)?;
    let animations = ParsedAnimationSet::from_scene(&archive, &scene)?;

    gltf::export_scene(&archive, &scene, &cli.input, &output_dir)?;
    let scene_name = cli
        .input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scene");
    viewer::write_viewer(&output_dir, &format!("{scene_name}.gltf"), "mviewer.runtime.json")?;

    if let Some(animations) = animations {
        println!(
            "parsed animation data: {} matrices, {} skinning rigs, {} animations",
            animations.num_matrices,
            animations.skinning_rigs.len(),
            animations.animations.len()
        );
    }
    println!("wrote glTF scene to {}", output_dir.display());
    Ok(())
}

fn default_output_dir(input: &std::path::Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scene");
    input
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("{stem}_gltf"))
}
