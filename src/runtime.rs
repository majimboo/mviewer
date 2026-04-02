use std::collections::HashMap;

use anyhow::{Context, Result};
use image::GenericImageView as _;

use crate::animation::ParsedAnimationSet;
use crate::archive::Archive;
use crate::mesh::{DecodedMesh, decode_mesh};
use crate::scene::{MaterialDesc, MeshDesc, Scene};

#[derive(Debug, Clone)]
pub struct RuntimeScene {
    pub meshes: Vec<RuntimeMesh>,
    pub materials: Vec<RuntimeMaterial>,
    pub material_lookup: HashMap<String, usize>,
    pub texture_usage: Vec<RuntimeTextureUsage>,
    pub animation_binding: Option<RuntimeAnimationBinding>,
}

#[derive(Debug, Clone)]
pub struct RuntimeMesh {
    pub index: usize,
    pub desc: MeshDesc,
    pub decoded: DecodedMesh,
    pub material_indices: Vec<usize>,
    pub animated_object_index: Option<usize>,
    pub skinning_rig_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct RuntimeMaterial {
    pub index: usize,
    pub desc: MaterialDesc,
    pub textures: Vec<RuntimeTextureRef>,
    pub preview_color: [f32; 3],
    pub animated_object_index: Option<usize>,
    pub preview_metallic: f32,
    pub preview_roughness: f32,
    pub preview_emissive: f32,
}

#[derive(Debug, Clone)]
pub struct RuntimeTextureRef {
    pub slot: &'static str,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeTextureUsage {
    pub name: String,
    pub used_by_materials: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct RuntimeAnimationBinding {
    pub mesh_object_indices: Vec<Option<usize>>,
    pub selected_animation: Option<usize>,
    pub selected_camera: Option<usize>,
    pub scene_scale: f32,
    pub total_clips: usize,
}

impl RuntimeScene {
    pub fn from_project(
        archive: &Archive,
        scene: &Scene,
        animations: Option<&ParsedAnimationSet>,
    ) -> Result<Self> {
        let materials: Vec<_> = scene
            .materials
            .iter()
            .enumerate()
            .map(|(index, material)| RuntimeMaterial {
                index,
                desc: material.clone(),
                textures: collect_material_textures(material),
                preview_color: derive_preview_color(archive, material),
                animated_object_index: scene
                    .anim_data
                    .as_ref()
                    .and_then(|anim| anim.material_ids.get(index))
                    .map(|part| part.part_index),
                preview_metallic: derive_preview_metallic(archive, material),
                preview_roughness: derive_preview_roughness(archive, material),
                preview_emissive: material.emissive_intensity.unwrap_or(0.0),
            })
            .collect();

        let material_lookup: HashMap<_, _> = materials
            .iter()
            .map(|material| (material.desc.name.clone(), material.index))
            .collect();

        let meshes: Vec<_> = scene
            .meshes
            .iter()
            .enumerate()
            .map(|(index, mesh)| {
                let entry = archive
                    .get(&mesh.file)
                    .with_context(|| format!("missing mesh payload {}", mesh.file))?;
                let decoded = decode_mesh(&entry.data, mesh)
                    .with_context(|| format!("failed to decode {}", mesh.file))?;
                let material_indices = mesh
                    .sub_meshes
                    .iter()
                    .filter_map(|sub_mesh| material_lookup.get(&sub_mesh.material).copied())
                    .collect();
                let animated_object_index = scene
                    .anim_data
                    .as_ref()
                    .and_then(|anim| anim.mesh_ids.get(index))
                    .map(|part| part.part_index);
                let skinning_rig_index = animated_object_index.and_then(|object_index| {
                    animations
                        .and_then(|parsed| {
                            let clip_index = scene
                                .anim_data
                                .as_ref()
                                .and_then(|anim| anim.selected_animation)
                                .unwrap_or(0);
                            parsed.animations.get(clip_index).or_else(|| parsed.animations.first())
                        })
                        .and_then(|clip| clip.find_object(object_index))
                        .and_then(|object| usize::try_from(object.desc.skinning_rig_index).ok())
                });

                Ok(RuntimeMesh {
                    index,
                    desc: mesh.clone(),
                    decoded,
                    material_indices,
                    animated_object_index,
                    skinning_rig_index,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let texture_usage = build_texture_usage(&materials);
        let animation_binding = scene.anim_data.as_ref().map(|anim| RuntimeAnimationBinding {
            mesh_object_indices: scene
                .meshes
                .iter()
                .enumerate()
                .map(|(mesh_index, _)| anim.mesh_ids.get(mesh_index).map(|part| part.part_index))
                .collect(),
            selected_animation: anim.selected_animation,
            selected_camera: anim.selected_camera,
            scene_scale: anim.scene_scale,
            total_clips: animations.map(|parsed| parsed.animations.len()).unwrap_or(0),
        });

        Ok(Self {
            meshes,
            materials,
            material_lookup,
            texture_usage,
            animation_binding,
        })
    }
}

fn collect_material_textures(material: &MaterialDesc) -> Vec<RuntimeTextureRef> {
    let mut textures = Vec::new();
    textures.push(RuntimeTextureRef {
        slot: "albedo",
        name: material.albedo_tex.clone(),
    });
    if let Some(name) = &material.alpha_tex {
        textures.push(RuntimeTextureRef {
            slot: "alpha",
            name: name.clone(),
        });
    }
    if let Some(name) = &material.normal_tex {
        textures.push(RuntimeTextureRef {
            slot: "normal",
            name: name.clone(),
        });
    }
    if let Some(name) = &material.reflectivity_tex {
        textures.push(RuntimeTextureRef {
            slot: "reflectivity",
            name: name.clone(),
        });
    }
    if let Some(name) = &material.gloss_tex {
        textures.push(RuntimeTextureRef {
            slot: "gloss",
            name: name.clone(),
        });
    }
    if let Some(name) = &material.extras_tex {
        textures.push(RuntimeTextureRef {
            slot: "extras",
            name: name.clone(),
        });
    }
    if let Some(name) = &material.extras_tex_a {
        textures.push(RuntimeTextureRef {
            slot: "extrasA",
            name: name.clone(),
        });
    }
    textures
}

fn build_texture_usage(materials: &[RuntimeMaterial]) -> Vec<RuntimeTextureUsage> {
    let mut usage: HashMap<String, Vec<usize>> = HashMap::new();
    for material in materials {
        for texture in &material.textures {
            usage.entry(texture.name.clone()).or_default().push(material.index);
        }
    }
    let mut collected: Vec<_> = usage
        .into_iter()
        .map(|(name, used_by_materials)| RuntimeTextureUsage {
            name,
            used_by_materials,
        })
        .collect();
    collected.sort_by(|a, b| a.name.cmp(&b.name));
    collected
}

fn derive_preview_color(archive: &Archive, material: &MaterialDesc) -> [f32; 3] {
    let Some(entry) = archive.get(&material.albedo_tex) else {
        return [0.72, 0.54, 0.42];
    };
    let Ok(image) = image::load_from_memory(&entry.data) else {
        return [0.72, 0.54, 0.42];
    };
    let rgba = image.to_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return [0.72, 0.54, 0.42];
    }

    let mut rgb = [0.0f32; 3];
    let mut total_alpha = 0.0f32;
    for pixel in rgba.pixels() {
        let alpha = pixel[3] as f32 / 255.0;
        rgb[0] += (pixel[0] as f32 / 255.0) * alpha;
        rgb[1] += (pixel[1] as f32 / 255.0) * alpha;
        rgb[2] += (pixel[2] as f32 / 255.0) * alpha;
        total_alpha += alpha;
    }

    if total_alpha <= f32::EPSILON {
        [0.72, 0.54, 0.42]
    } else {
        [rgb[0] / total_alpha, rgb[1] / total_alpha, rgb[2] / total_alpha]
    }
}

fn derive_preview_metallic(archive: &Archive, material: &MaterialDesc) -> f32 {
    material
        .reflectivity_tex
        .as_deref()
        .and_then(|name| archive.get(name))
        .and_then(|entry| image::load_from_memory(&entry.data).ok())
        .map(|image| average_luma(&image.to_luma8()))
        .unwrap_or(0.0)
}

fn derive_preview_roughness(archive: &Archive, material: &MaterialDesc) -> f32 {
    material
        .gloss_tex
        .as_deref()
        .and_then(|name| archive.get(name))
        .and_then(|entry| image::load_from_memory(&entry.data).ok())
        .map(|image| 1.0 - average_luma(&image.to_luma8()))
        .unwrap_or(0.85)
}

fn average_luma(image: &image::GrayImage) -> f32 {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return 0.0;
    }
    let sum: f32 = image.pixels().map(|pixel| pixel[0] as f32 / 255.0).sum();
    sum / (width * height) as f32
}
