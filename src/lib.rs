pub mod animation;
pub mod archive;
pub mod gltf;
pub mod gui;
pub mod mesh;
pub mod runtime;
pub mod scene;
pub mod viewer;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::animation::ParsedAnimationSet;
use crate::archive::Archive;
use crate::runtime::RuntimeScene;
use crate::scene::Scene;

#[derive(Debug, Clone)]
pub struct ProjectSummary {
    pub title: Option<String>,
    pub author: Option<String>,
    pub mesh_count: usize,
    pub material_count: usize,
    pub camera_count: usize,
    pub light_count: usize,
    pub animation_count: usize,
    pub skinning_rig_count: usize,
}

#[derive(Debug)]
pub struct ProjectDocument {
    pub input_path: PathBuf,
    pub archive: Archive,
    pub scene: Scene,
    pub animations: Option<ParsedAnimationSet>,
    pub runtime: RuntimeScene,
    pub summary: ProjectSummary,
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub included_meshes: BTreeSet<usize>,
    pub include_cameras: bool,
    pub include_lights: bool,
    pub include_animations: bool,
}

#[derive(Debug, Clone)]
pub struct ExportReport {
    pub output_dir: PathBuf,
    pub exported_meshes: usize,
    pub total_meshes: usize,
}

impl ExportOptions {
    pub fn include_all(scene: &Scene) -> Self {
        Self {
            included_meshes: (0..scene.meshes.len()).collect(),
            include_cameras: true,
            include_lights: true,
            include_animations: true,
        }
    }
}

pub fn load_project(path: &Path) -> Result<ProjectDocument> {
    let archive = Archive::from_path(path)?;
    let scene_entry = archive
        .get("scene.json")
        .context("scene.json not found in archive")?;
    let scene = Scene::from_bytes(&scene_entry.data)?;
    let animations = ParsedAnimationSet::from_scene(&archive, &scene)?;
    let runtime = RuntimeScene::from_project(&archive, &scene, animations.as_ref())?;
    let summary = ProjectSummary {
        title: scene.meta_data.as_ref().and_then(|meta| meta.title.clone()),
        author: scene.meta_data.as_ref().and_then(|meta| meta.author.clone()),
        mesh_count: scene.meshes.len(),
        material_count: scene.materials.len(),
        camera_count: scene.cameras.len() + usize::from(scene.main_camera.is_some()),
        light_count: scene.lights.as_ref().and_then(|lights| lights.count).unwrap_or(0),
        animation_count: animations
            .as_ref()
            .map(|parsed| parsed.animations.len())
            .unwrap_or(0),
        skinning_rig_count: animations
            .as_ref()
            .map(|parsed| parsed.skinning_rigs.len())
            .unwrap_or(0),
    };

    Ok(ProjectDocument {
        input_path: path.to_path_buf(),
        archive,
        scene,
        animations,
        runtime,
        summary,
    })
}

pub fn export_project(
    project: &ProjectDocument,
    output_dir: &Path,
    options: &ExportOptions,
) -> Result<ExportReport> {
    let filtered_scene = filtered_scene(&project.scene, options);
    gltf::export_scene(&project.archive, &filtered_scene, &project.input_path, output_dir)?;
    let scene_name = project
        .input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scene");
    viewer::write_viewer(output_dir, &format!("{scene_name}.gltf"), "mviewer.runtime.json")?;

    Ok(ExportReport {
        output_dir: output_dir.to_path_buf(),
        exported_meshes: filtered_scene.meshes.len(),
        total_meshes: project.scene.meshes.len(),
    })
}

pub fn default_output_dir(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scene");
    input.parent().unwrap_or_else(|| Path::new(".")).join(format!("{stem}_gltf"))
}

fn filtered_scene(scene: &Scene, options: &ExportOptions) -> Scene {
    let mut filtered = scene.clone();
    filtered.meshes = scene
        .meshes
        .iter()
        .enumerate()
        .filter(|(index, _)| options.included_meshes.contains(index))
        .map(|(_, mesh)| mesh.clone())
        .collect();

    if let Some(anim_data) = &scene.anim_data {
        let mut filtered_anim = anim_data.clone();
        filtered_anim.mesh_ids = anim_data
            .mesh_ids
            .iter()
            .enumerate()
            .filter(|(index, _)| options.included_meshes.contains(index))
            .map(|(_, mesh_id)| mesh_id.clone())
            .collect();
        filtered.anim_data = options.include_animations.then_some(filtered_anim);
    }

    if !options.include_cameras {
        filtered.main_camera = None;
        filtered.cameras.clear();
        if let Some(anim_data) = &mut filtered.anim_data {
            anim_data.selected_camera = None;
        }
    }

    if !options.include_lights {
        filtered.lights = None;
    }

    filtered
}
