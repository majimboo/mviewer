use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use mviewer::{ExportOptions, default_output_dir, export_project, load_project};

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
    let project = load_project(&cli.input)?;
    let report = export_project(&project, &output_dir, &ExportOptions::include_all(&project.scene))?;

    if let Some(animations) = &project.animations {
        println!(
            "parsed animation data: {} matrices, {} skinning rigs, {} animations",
            animations.num_matrices,
            animations.skinning_rigs.len(),
            animations.animations.len()
        );
    }
    println!(
        "wrote glTF scene to {} ({} of {} meshes)",
        report.output_dir.display(),
        report.exported_meshes,
        report.total_meshes
    );
    Ok(())
}
