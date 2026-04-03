#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use mviewer::{ExportOptions, default_output_dir, export_project, load_project};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliFormat {
    Gltf,
    Glb,
}

#[derive(Debug, Parser)]
#[command(name = "mviewer")]
#[command(about = "Export Marmoset .mview scenes to glTF or GLB")]
struct Cli {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    #[arg(value_name = "OUTPUT_DIR")]
    output_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "gltf")]
    format: CliFormat,
}

fn main() -> Result<()> {
    if std::env::args_os().len() == 1 {
        return mviewer::gui::app::run();
    }

    run_cli()
}

fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    let output_dir = cli
        .output_dir
        .unwrap_or_else(|| default_output_dir(&cli.input));
    let project = load_project(&cli.input)?;
    let report = match cli.format {
        CliFormat::Gltf => {
            export_project(&project, &output_dir, &ExportOptions::include_all(&project.scene))?
        }
        CliFormat::Glb => {
            let filtered_scene = project.scene.clone();
            mviewer::gltf::export_scene_with_js_scene_format_progress(
                &project.archive,
                &filtered_scene,
                &project.input_path,
                &output_dir,
                None,
                mviewer::gltf::GltfOutputFormat::Glb,
                &mut |_progress, _stage| {},
            )?;
            mviewer::ExportReport {
                output_dir: output_dir.clone(),
                exported_meshes: filtered_scene.meshes.len(),
                total_meshes: project.scene.meshes.len(),
            }
        }
    };

    if let Some(animations) = &project.animations {
        println!(
            "parsed animation data: {} matrices, {} skinning rigs, {} animations",
            animations.num_matrices,
            animations.skinning_rigs.len(),
            animations.animations.len()
        );
    }
    println!(
        "wrote {} scene to {} ({} of {} meshes)",
        match cli.format {
            CliFormat::Gltf => "glTF",
            CliFormat::Glb => "GLB",
        },
        report.output_dir.display(),
        report.exported_meshes,
        report.total_meshes
    );
    Ok(())
}
