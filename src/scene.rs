use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Scene {
    #[serde(rename = "metaData")]
    pub meta_data: Option<MetaData>,
    #[serde(rename = "mainCamera")]
    pub main_camera: Option<MainCamera>,
    #[serde(rename = "Cameras", default)]
    pub cameras: HashMap<String, MainCamera>,
    pub lights: Option<Lights>,
    pub meshes: Vec<MeshDesc>,
    pub materials: Vec<MaterialDesc>,
    pub fog: Option<FogDesc>,
    pub sky: Option<SkyDesc>,
    #[serde(rename = "shadowFloor")]
    pub shadow_floor: Option<ShadowFloorDesc>,
    #[serde(rename = "AnimData")]
    pub anim_data: Option<AnimData>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MetaData {
    #[serde(rename = "tbVersion")]
    pub tb_version: Option<u32>,
    pub title: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MainCamera {
    pub view: Option<ViewDesc>,
    pub post: Option<PostDesc>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ViewDesc {
    pub angles: Option<[f32; 2]>,
    pub fov: Option<f32>,
    #[serde(rename = "orbitRadius")]
    pub orbit_radius: Option<f32>,
    pub pivot: Option<[f32; 3]>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PostDesc {
    #[serde(rename = "bloomColor")]
    pub bloom_color: Option<[f32; 4]>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Lights {
    pub count: Option<usize>,
    #[serde(rename = "shadowCount")]
    pub shadow_count: Option<usize>,
    pub rotation: Option<f32>,
    pub positions: Option<Vec<f32>>,
    pub directions: Option<Vec<f32>>,
    pub colors: Option<Vec<f32>>,
    pub parameters: Option<Vec<f32>>,
    pub spot: Option<Vec<f32>>,
    #[serde(rename = "matrixWeights")]
    pub matrix_weights: Option<Vec<u32>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MeshDesc {
    pub name: String,
    #[serde(rename = "indexCount")]
    pub index_count: usize,
    #[serde(rename = "indexTypeSize")]
    pub index_type_size: usize,
    #[serde(rename = "wireCount")]
    pub wire_count: usize,
    #[serde(rename = "vertexCount")]
    pub vertex_count: usize,
    #[serde(rename = "secondaryTexCoord")]
    pub secondary_tex_coord: Option<u32>,
    #[serde(rename = "vertexColor")]
    pub vertex_color: Option<u32>,
    #[serde(rename = "isDynamicMesh")]
    pub is_dynamic_mesh: Option<bool>,
    pub transform: Option<[f32; 16]>,
    pub file: String,
    #[serde(rename = "subMeshes")]
    pub sub_meshes: Vec<SubMeshDesc>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SubMeshDesc {
    pub material: String,
    #[serde(rename = "firstIndex")]
    pub first_index: usize,
    #[serde(rename = "indexCount")]
    pub index_count: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MaterialDesc {
    pub name: String,
    #[serde(rename = "albedoTex")]
    pub albedo_tex: String,
    #[serde(rename = "alphaTex")]
    pub alpha_tex: Option<String>,
    #[serde(rename = "normalTex")]
    pub normal_tex: Option<String>,
    #[serde(rename = "reflectivityTex")]
    pub reflectivity_tex: Option<String>,
    #[serde(rename = "glossTex")]
    pub gloss_tex: Option<String>,
    #[serde(rename = "extrasTex")]
    pub extras_tex: Option<String>,
    #[serde(rename = "extrasTexA")]
    pub extras_tex_a: Option<String>,
    pub blend: Option<String>,
    #[serde(rename = "alphaTest")]
    pub alpha_test: Option<f32>,
    #[serde(rename = "useSkin")]
    pub use_skin: Option<bool>,
    pub fresnel: Option<[f32; 3]>,
    #[serde(rename = "horizonOcclude")]
    pub horizon_occlude: Option<f32>,
    #[serde(rename = "horizonSmoothing")]
    pub horizon_smoothing: Option<f32>,
    pub aniso: Option<bool>,
    pub microfiber: Option<bool>,
    pub refraction: Option<bool>,
    #[serde(rename = "emissiveIntensity")]
    pub emissive_intensity: Option<f32>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FogDesc {
    pub opacity: Option<f32>,
    pub distance: Option<f32>,
    pub dispersion: Option<f32>,
    #[serde(rename = "skyIllum")]
    pub sky_illum: Option<f32>,
    #[serde(rename = "lightIllum")]
    pub light_illum: Option<f32>,
    pub color: Option<[f32; 3]>,
    #[serde(rename = "type")]
    pub fog_type: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkyDesc {
    #[serde(rename = "imageURL")]
    pub image_url: Option<String>,
    #[serde(rename = "backgroundBrightness")]
    pub background_brightness: Option<f32>,
    #[serde(rename = "backgroundColor")]
    pub background_color: Option<Vec<f32>>,
    #[serde(rename = "backgroundMode")]
    pub background_mode: Option<u32>,
    #[serde(rename = "diffuseCoefficients")]
    pub diffuse_coefficients: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShadowFloorDesc {
    pub simple: Option<bool>,
    pub alpha: Option<f32>,
    #[serde(rename = "edgeFade")]
    pub edge_fade: Option<bool>,
    pub transform: Option<[f32; 16]>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AnimData {
    #[serde(rename = "hasAnimData")]
    pub has_anim_data: Option<bool>,
    #[serde(rename = "numAnimations")]
    pub num_animations: Option<usize>,
    #[serde(rename = "numMatrices")]
    pub num_matrices: usize,
    #[serde(rename = "numSkinningRigs")]
    pub num_skinning_rigs: Option<usize>,
    #[serde(rename = "selectedAnimation")]
    pub selected_animation: Option<usize>,
    #[serde(rename = "selectedCamera")]
    pub selected_camera: Option<usize>,
    #[serde(rename = "sceneScale")]
    pub scene_scale: f32,
    #[serde(rename = "showPlayControls")]
    pub show_play_controls: Option<bool>,
    #[serde(rename = "autoPlayAnims")]
    pub auto_play_anims: Option<bool>,
    #[serde(rename = "meshIDs", default)]
    pub mesh_ids: Vec<PartIndexRef>,
    #[serde(rename = "lightIDs", default)]
    pub light_ids: Vec<PartIndexRef>,
    #[serde(rename = "materialIDs", default)]
    pub material_ids: Vec<PartIndexRef>,
    #[serde(rename = "skinningRigs", default)]
    pub skinning_rigs: Vec<SkinningRigDesc>,
    #[serde(default)]
    pub animations: Vec<AnimationDesc>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PartIndexRef {
    #[serde(rename = "partIndex")]
    pub part_index: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkinningRigDesc {
    pub file: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AnimationDesc {
    pub name: String,
    pub length: f32,
    #[serde(rename = "originalFPS")]
    pub original_fps: f32,
    #[serde(rename = "totalFrames")]
    pub total_frames: usize,
    #[serde(rename = "numAnimatedObjects")]
    pub num_animated_objects: usize,
    #[serde(rename = "animatedObjects", default)]
    pub animated_objects: Vec<AnimatedObjectDesc>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AnimatedObjectDesc {
    #[serde(rename = "partName")]
    pub part_name: String,
    #[serde(rename = "sceneObjectType")]
    pub scene_object_type: String,
    #[serde(rename = "skinningRigIndex")]
    pub skinning_rig_index: isize,
    #[serde(rename = "modelPartIndex")]
    pub model_part_index: usize,
    #[serde(rename = "modelPartFPS")]
    pub model_part_fps: f32,
    #[serde(rename = "modelPartScale")]
    pub model_part_scale: f32,
    #[serde(rename = "parentIndex")]
    pub parent_index: usize,
    #[serde(rename = "startTime")]
    pub start_time: f32,
    #[serde(rename = "endTime")]
    pub end_time: f32,
    #[serde(rename = "totalFrames")]
    pub total_frames: usize,
    pub file: String,
    #[serde(rename = "numAnimatedProperties")]
    pub num_animated_properties: Option<usize>,
    #[serde(rename = "animatedProperties", default)]
    pub animated_properties: Vec<AnimatedPropertyDesc>,
    pub pivotx: Option<f32>,
    pub pivoty: Option<f32>,
    pub pivotz: Option<f32>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AnimatedPropertyDesc {
    pub name: String,
}

impl Scene {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        match serde_json::from_slice(bytes) {
            Ok(scene) => Ok(scene),
            Err(parse_error) => {
                let lossy = String::from_utf8_lossy(bytes);
                serde_json::from_str(&lossy)
                    .with_context(|| format!("failed to parse scene.json: {parse_error}"))
            }
        }
    }
}
