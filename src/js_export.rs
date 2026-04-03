use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::archive::Archive;
use crate::scene::{Lights, MainCamera, MaterialDesc, Scene};
use crate::{ExportOptions, ExportReport};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsExportScene {
    pub source: String,
    pub version: u32,
    #[serde(rename = "archiveBase64", default)]
    pub archive_base64: Option<String>,
    #[serde(rename = "selectedAnimationIndex")]
    pub selected_animation_index: i32,
    #[serde(rename = "selectedCameraIndex", default)]
    pub selected_camera_index: i32,
    #[serde(rename = "animationProgress")]
    pub animation_progress: Option<f32>,
    #[serde(rename = "totalSeconds")]
    pub total_seconds: Option<f32>,
    pub scene: Option<JsSceneSummary>,
    #[serde(default)]
    pub cameras: Vec<JsCamera>,
    #[serde(default)]
    pub lights: Vec<JsLight>,
    #[serde(default)]
    pub meshes: Vec<JsMesh>,
    #[serde(rename = "sampledAnimation")]
    pub sampled_animation: Option<JsSampledAnimation>,
    #[serde(default)]
    pub animations: Vec<JsAnimation>,
    #[serde(default)]
    pub mesh_bindings: Vec<JsMeshBinding>,
    #[serde(default)]
    pub materials: Vec<JsMaterial>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsSceneSummary {
    pub title: Option<String>,
    pub author: Option<String>,
    #[serde(rename = "meshCount")]
    pub mesh_count: usize,
    #[serde(rename = "materialCount")]
    pub material_count: usize,
    #[serde(rename = "cameraCount")]
    pub camera_count: usize,
    #[serde(rename = "lightCount")]
    pub light_count: usize,
    #[serde(rename = "animationCount")]
    pub animation_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsCamera {
    pub index: usize,
    pub name: String,
    pub fov: Option<f32>,
    pub near: Option<f32>,
    pub far: Option<f32>,
    pub transform: Option<[f32; 16]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsLight {
    pub index: usize,
    pub color: Option<[f32; 3]>,
    pub position: Option<[f32; 4]>,
    pub direction: Option<[f32; 3]>,
    pub parameters: Option<[f32; 3]>,
    pub spot: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsMesh {
    pub index: usize,
    pub name: String,
    #[serde(rename = "vertexCount")]
    pub vertex_count: Option<usize>,
    #[serde(rename = "indexCount")]
    pub index_count: Option<usize>,
    #[serde(rename = "displayMatrix")]
    pub display_matrix: Option<[f32; 16]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsSampledAnimation {
    pub index: usize,
    pub name: String,
    #[serde(default)]
    pub samples: Vec<JsAnimationSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsAnimationSample {
    #[serde(rename = "sampleIndex")]
    pub sample_index: usize,
    pub frame: f32,
    pub seconds: f32,
    #[serde(default)]
    pub objects: Vec<JsAnimationSampleObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsAnimationSampleObject {
    pub id: usize,
    #[serde(rename = "worldMatrix")]
    pub world_matrix: [f32; 16],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsAnimation {
    pub index: usize,
    pub name: String,
    #[serde(rename = "totalSeconds")]
    pub total_seconds: Option<f32>,
    #[serde(rename = "totalFrames")]
    pub total_frames: Option<u32>,
    #[serde(rename = "originalFPS")]
    pub original_fps: Option<f32>,
    #[serde(default)]
    pub animated_objects: Vec<JsAnimatedObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsAnimatedObject {
    pub id: usize,
    pub name: String,
    #[serde(rename = "parentIndex")]
    pub parent_index: usize,
    #[serde(rename = "sceneObjectType")]
    pub scene_object_type: Option<String>,
    #[serde(rename = "modelPartIndex")]
    pub model_part_index: Option<usize>,
    #[serde(rename = "modelPartFPS")]
    pub model_part_fps: Option<f32>,
    #[serde(rename = "modelPartScale")]
    pub model_part_scale: Option<f32>,
    #[serde(rename = "animationLength")]
    pub animation_length: Option<f32>,
    #[serde(rename = "totalFrames")]
    pub total_frames: Option<u32>,
    #[serde(rename = "startTime")]
    pub start_time: Option<f32>,
    #[serde(rename = "endTime")]
    pub end_time: Option<f32>,
    #[serde(rename = "meshIndex")]
    pub mesh_index: Option<i32>,
    #[serde(rename = "materialIndex")]
    pub material_index: Option<i32>,
    #[serde(rename = "lightIndex")]
    pub light_index: Option<i32>,
    #[serde(rename = "skinningRigIndex")]
    pub skinning_rig_index: Option<i32>,
    pub pivot: Option<JsPivot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsPivot {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsMeshBinding {
    #[serde(rename = "animatedObjectId")]
    pub animated_object_id: usize,
    pub name: String,
    #[serde(rename = "meshIndex")]
    pub mesh_index: Option<i32>,
    #[serde(rename = "materialIndex")]
    pub material_index: Option<i32>,
    #[serde(rename = "modelPartIndex")]
    pub model_part_index: Option<usize>,
    #[serde(rename = "skinningRigIndex")]
    pub skinning_rig_index: Option<i32>,
    #[serde(rename = "displayMatrix")]
    pub display_matrix: Option<[f32; 16]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsMaterial {
    pub index: usize,
    pub name: String,
    pub desc: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuiExportFormat {
    Gltf,
    Glb,
    Obj,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiExportOptions {
    pub format: GuiExportFormat,
    #[serde(default)]
    pub included_meshes: Vec<usize>,
    #[serde(default = "default_true")]
    pub include_textures: bool,
    #[serde(default = "default_true")]
    pub include_animations: bool,
    #[serde(default = "default_true")]
    pub include_cameras: bool,
    #[serde(default = "default_true")]
    pub include_lights: bool,
}

fn default_true() -> bool {
    true
}

pub fn export_from_js_scene_path(
    input_path: &Path,
    output_dir: &Path,
    js_scene: &JsExportScene,
) -> Result<ExportReport> {
    export_from_js_scene_path_with_options_and_progress(
        input_path,
        output_dir,
        js_scene,
        &GuiExportOptions {
            format: GuiExportFormat::Gltf,
            included_meshes: Vec::new(),
            include_textures: true,
            include_animations: true,
            include_cameras: true,
            include_lights: true,
        },
        |_progress, _stage| {},
    )
}

pub fn export_from_js_scene_path_with_progress<F>(
    input_path: &Path,
    output_dir: &Path,
    js_scene: &JsExportScene,
    mut progress: F,
) -> Result<ExportReport>
where
    F: FnMut(u8, &str),
{
    export_from_js_scene_path_with_options_and_progress(
        input_path,
        output_dir,
        js_scene,
        &GuiExportOptions {
            format: GuiExportFormat::Gltf,
            included_meshes: Vec::new(),
            include_textures: true,
            include_animations: true,
            include_cameras: true,
            include_lights: true,
        },
        &mut progress,
    )
}

pub fn export_from_js_scene_path_with_options_and_progress<F>(
    input_path: &Path,
    output_dir: &Path,
    js_scene: &JsExportScene,
    export_options: &GuiExportOptions,
    mut progress: F,
) -> Result<ExportReport>
where
    F: FnMut(u8, &str),
{
    progress(5, "Reading scene data");
    let archive = if let Some(encoded) = &js_scene.archive_base64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .context("failed to decode archiveBase64 from JS export scene")?;
        Archive::from_bytes(&bytes)?
    } else {
        Archive::from_path(input_path)?
    };
    progress(20, "Parsing scene");
    let scene_entry = archive
        .get("scene.json")
        .context("scene.json not found in archive")?;
    let base_scene = Scene::from_bytes(&scene_entry.data)?;
    progress(35, "Merging Marmoset runtime data");
    let mut merged_scene = merge_js_export_scene(&base_scene, js_scene);
    if !export_options.include_textures {
        strip_scene_textures(&mut merged_scene);
    }
    let options = build_export_options(&merged_scene, export_options);
    let filtered_scene = filter_scene_for_export(&merged_scene, &options);

    match export_options.format {
        GuiExportFormat::Gltf => {
            progress(55, "Starting glTF export");
            crate::gltf::export_scene_with_js_scene_format_progress(
                &archive,
                &filtered_scene,
                input_path,
                output_dir,
                Some(js_scene),
                crate::gltf::GltfOutputFormat::Gltf,
                &mut |local_progress, stage| {
                    let mapped = 55 + (u16::from(local_progress) * 40 / 100) as u8;
                    progress(mapped, stage);
                },
            )?;
            progress(95, "Finalizing export");
        }
        GuiExportFormat::Glb => {
            progress(55, "Starting GLB export");
            crate::gltf::export_scene_with_js_scene_format_progress(
                &archive,
                &filtered_scene,
                input_path,
                output_dir,
                Some(js_scene),
                crate::gltf::GltfOutputFormat::Glb,
                &mut |local_progress, stage| {
                    let mapped = 55 + (u16::from(local_progress) * 40 / 100) as u8;
                    progress(mapped, stage);
                },
            )?;
            progress(95, "Finalizing export");
        }
        GuiExportFormat::Obj => {
            progress(55, "Starting OBJ export");
            crate::obj::export_scene(
                &archive,
                &filtered_scene,
                input_path,
                output_dir,
                &crate::obj::ObjExportOptions {
                    included_meshes: options.included_meshes.clone(),
                    include_textures: export_options.include_textures,
                },
                &mut |local_progress, stage| {
                    let mapped = 55 + (u16::from(local_progress) * 40 / 100) as u8;
                    progress(mapped, stage);
                },
            )?;
            progress(95, "Finalizing export");
        }
    }

    Ok(ExportReport {
        output_dir: output_dir.to_path_buf(),
        exported_meshes: filtered_scene.meshes.len(),
        total_meshes: merged_scene.meshes.len(),
    })
}

pub fn merge_js_export_scene(base_scene: &Scene, js_scene: &JsExportScene) -> Scene {
    let mut merged = base_scene.clone();

    if let Some(summary) = &js_scene.scene {
        let meta = merged.meta_data.get_or_insert_with(|| crate::scene::MetaData {
            tb_version: None,
            title: None,
            author: None,
        });
        if summary.title.is_some() {
            meta.title = summary.title.clone();
        }
        if summary.author.is_some() {
            meta.author = summary.author.clone();
        }
    }

    if !js_scene.materials.is_empty() {
        for js_material in &js_scene.materials {
            if let Some(desc_value) = &js_material.desc {
                if let Ok(desc) = serde_json::from_value::<MaterialDesc>(desc_value.clone()) {
                    if let Some(existing) = merged
                        .materials
                        .iter_mut()
                        .find(|material| material.name == js_material.name)
                    {
                        *existing = desc;
                    } else if let Some(existing) = merged.materials.get_mut(js_material.index) {
                        *existing = desc;
                    }
                }
            }
        }
    }

    if let Some(anim_data) = &mut merged.anim_data {
        if js_scene.selected_animation_index >= 0 {
            anim_data.selected_animation = Some(js_scene.selected_animation_index as usize);
        }
        if js_scene.selected_camera_index >= 0 {
            anim_data.selected_camera = Some(js_scene.selected_camera_index as usize);
        }
    }

    if !js_scene.cameras.is_empty() {
        let mut cameras = merged.cameras.clone();
        for camera in &js_scene.cameras {
            cameras.insert(
                camera.name.clone(),
                MainCamera {
                    view: None,
                    post: None,
                },
            );
        }
        merged.cameras = cameras;
    }

    if !js_scene.lights.is_empty() {
        merged.lights = Some(build_lights_from_js(&js_scene.lights, merged.lights.as_ref()));
    }

    merged
}

fn build_export_options(scene: &Scene, options: &GuiExportOptions) -> ExportOptions {
    let mut export_options = ExportOptions::include_all(scene);
    if !options.included_meshes.is_empty() {
        export_options.included_meshes = options.included_meshes.iter().copied().collect();
    }
    export_options.include_animations = options.include_animations;
    export_options.include_cameras = options.include_cameras;
    export_options.include_lights = options.include_lights;
    export_options
}

fn build_lights_from_js(js_lights: &[JsLight], fallback: Option<&Lights>) -> Lights {
    let count = js_lights.len();
    let mut positions = Vec::with_capacity(count * 4);
    let mut directions = Vec::with_capacity(count * 3);
    let mut colors = Vec::with_capacity(count * 3);
    let mut parameters = Vec::with_capacity(count * 3);
    let mut spot = Vec::with_capacity(count * 3);
    let mut matrix_weights = Vec::with_capacity(count);

    for light in js_lights {
        positions.extend_from_slice(&light.position.unwrap_or([0.0, 0.0, 0.0, 1.0]));
        directions.extend_from_slice(&light.direction.unwrap_or([0.0, -1.0, 0.0]));
        colors.extend_from_slice(&light.color.unwrap_or([1.0, 1.0, 1.0]));
        parameters.extend_from_slice(&light.parameters.unwrap_or([1.0, 0.0, 10.0]));
        spot.extend_from_slice(&light.spot.unwrap_or([0.0, 1.0, 0.0]));
        matrix_weights.push(0);
    }

    Lights {
        count: Some(count),
        shadow_count: fallback.and_then(|lights| lights.shadow_count),
        use_new_attenuation: fallback.and_then(|lights| lights.use_new_attenuation),
        rotation: fallback.and_then(|lights| lights.rotation),
        positions: Some(positions),
        directions: Some(directions),
        colors: Some(colors),
        parameters: Some(parameters),
        spot: Some(spot),
        matrix_weights: Some(matrix_weights),
    }
}

fn filter_scene_for_export(scene: &Scene, options: &ExportOptions) -> Scene {
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

fn strip_scene_textures(scene: &mut Scene) {
    for material in &mut scene.materials {
        material.albedo_tex.clear();
        material.alpha_tex = None;
        material.normal_tex = None;
        material.reflectivity_tex = None;
        material.gloss_tex = None;
        material.extras_tex = None;
        material.extras_tex_a = None;
    }
}
