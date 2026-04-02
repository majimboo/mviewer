mod animated;
mod textures;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gltf_json::Root as MythGltfRoot;
use serde_json::json;

use animated::{MeshSkinData, export_animated_scene};
use crate::animation::ParsedAnimationSet;
use crate::archive::Archive;
use crate::js_export::JsExportScene;
use crate::mesh::{DecodedMesh, decode_mesh};
use crate::scene::{Lights, MainCamera, MaterialDesc, Scene, ViewDesc};
use textures::{
    merge_alpha_texture, merge_metallic_roughness_texture, merged_alpha_name,
    merged_metallic_roughness_name,
};

pub(super) fn export_static_scene(
    builder: &mut GltfBuilder,
    archive: &Archive,
    scene: &Scene,
    material_lookup: &HashMap<String, usize>,
    input_path: &Path,
    output_dir: &Path,
    progress: &mut dyn FnMut(u8, &str),
) -> Result<()> {
    let mut mesh_nodes = Vec::new();
    let total_meshes = scene.meshes.len().max(1);
    for (mesh_idx, mesh_desc) in scene.meshes.iter().enumerate() {
        let entry = archive
            .get(&mesh_desc.file)
            .with_context(|| format!("missing mesh payload {}", mesh_desc.file))?;
        let decoded = decode_mesh(&entry.data, mesh_desc)
            .with_context(|| format!("failed to decode {}", mesh_desc.file))?;
        let mesh_index = builder.add_mesh(mesh_desc, &decoded, material_lookup)?;
        let node_index = builder.add_node(mesh_desc.name.clone(), mesh_index, mesh_desc.transform);
        mesh_nodes.push(node_index);
        let p = 70 + ((mesh_idx + 1) * 25 / total_meshes);
        progress(p as u8, "Processing meshes");
    }

    let scene_name = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scene");
    progress(98, "Writing scene files");
    std::mem::take(builder).finish(scene_name, mesh_nodes, scene, output_dir)
}

pub fn export_scene(
    archive: &Archive,
    scene: &Scene,
    input_path: &Path,
    output_dir: &Path,
) -> Result<()> {
    export_scene_with_js_scene(archive, scene, input_path, output_dir, None)
}

pub fn export_scene_with_js_scene(
    archive: &Archive,
    scene: &Scene,
    input_path: &Path,
    output_dir: &Path,
    js_scene: Option<&JsExportScene>,
) -> Result<()> {
    let mut noop = |_progress: u8, _stage: &str| {};
    export_scene_with_js_scene_progress(
        archive,
        scene,
        input_path,
        output_dir,
        js_scene,
        &mut noop,
    )
}

pub fn export_scene_with_js_scene_progress(
    archive: &Archive,
    scene: &Scene,
    input_path: &Path,
    output_dir: &Path,
    js_scene: Option<&JsExportScene>,
    progress: &mut dyn FnMut(u8, &str),
) -> Result<()> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    progress(0, "Copying source textures");
    export_source_texture_files(archive, output_dir, progress)?;

    let mut builder = GltfBuilder::default();
    let mut material_lookup = HashMap::new();
    let mut texture_cache = HashMap::new();

    let total_materials = scene.materials.len().max(1);
    for (material_idx, material) in scene.materials.iter().enumerate() {
        let index = builder.add_material(archive, material, output_dir, &mut texture_cache)?;
        material_lookup.insert(material.name.clone(), index);
        let p = 20 + ((material_idx + 1) * 35 / total_materials);
        progress(p as u8, "Exporting materials");
    }

    if let Some(animations) = ParsedAnimationSet::from_scene(archive, scene)? {
        return export_animated_scene(
            &mut builder,
            archive,
            scene,
            &animations,
            &material_lookup,
            input_path,
            output_dir,
            js_scene,
            progress,
        );
    }

    export_static_scene(
        &mut builder,
        archive,
        scene,
        &material_lookup,
        input_path,
        output_dir,
        progress,
    )
}

#[derive(Default)]
pub(super) struct GltfBuilder {
    pub(super) buffer: Vec<u8>,
    pub(super) accessors: Vec<Accessor>,
    pub(super) buffer_views: Vec<BufferView>,
    pub(super) images: Vec<ImageDef>,
    pub(super) textures: Vec<TextureDef>,
    pub(super) materials: Vec<MaterialDef>,
    pub(super) meshes: Vec<MeshDef>,
    pub(super) nodes: Vec<NodeDef>,
    pub(super) cameras: Vec<CameraDef>,
    pub(super) skins: Vec<SkinDef>,
    pub(super) animations: Vec<AnimationDef>,
    pub(super) extensions_used: Vec<String>,
    pub(super) punctual_lights: Vec<LightDef>,
    pub(super) root_extras: Option<serde_json::Value>,
    pub(super) bound_camera_names: HashSet<String>,
    pub(super) bound_light_indices: HashSet<usize>,
}

impl GltfBuilder {
    fn add_material(
        &mut self,
        archive: &Archive,
        material: &MaterialDesc,
        output_dir: &Path,
        texture_cache: &mut HashMap<String, usize>,
    ) -> Result<usize> {
        let base_color_texture = self.ensure_base_color_texture(
            archive,
            &material.albedo_tex,
            material.alpha_tex.as_deref(),
            output_dir,
            texture_cache,
        )?;

        let normal_texture = if let Some(name) = &material.normal_tex {
            Some(self.ensure_texture_file(archive, name, output_dir, texture_cache)?)
        } else {
            None
        };
        let metallic_roughness_texture = match (
            material.reflectivity_tex.as_deref(),
            material.gloss_tex.as_deref(),
        ) {
            (Some(reflectivity_name), Some(gloss_name)) => Some(
                self.ensure_metallic_roughness_texture(
                    archive,
                    reflectivity_name,
                    gloss_name,
                    output_dir,
                    texture_cache,
                )?,
            ),
            _ => None,
        };

        let alpha_mode = match material.blend.as_deref() {
            Some("alpha") | Some("add") => Some("BLEND".to_string()),
            _ if material.alpha_tex.is_some() => Some("MASK".to_string()),
            _ => None,
        };

        let alpha_cutoff = if matches!(alpha_mode.as_deref(), Some("MASK")) {
            material.alpha_test.or(Some(0.5))
        } else {
            None
        };

        let material_index = self.materials.len();
        self.materials.push(MaterialDef {
            name: Some(material.name.clone()),
            pbr_metallic_roughness: Some(PbrMetallicRoughness {
                base_color_texture: Some(TextureRef {
                    index: base_color_texture,
                    tex_coord: None,
                }),
                metallic_factor: Some(1.0),
                roughness_factor: Some(1.0),
                metallic_roughness_texture: metallic_roughness_texture.map(|index| TextureRef {
                    index,
                    tex_coord: None,
                }),
            }),
            normal_texture: normal_texture.map(|index| NormalTextureRef { index, scale: None }),
            emissive_factor: Some(if material.emissive_intensity.unwrap_or(0.0) > 0.0 {
                [material.emissive_intensity.unwrap_or(1.0); 3]
            } else {
                [0.0, 0.0, 0.0]
            }),
            alpha_mode,
            alpha_cutoff,
            double_sided: Some(matches!(material.blend.as_deref(), Some("alpha") | Some("add"))),
            extras: Some(json!({
                "mviewer": {
                    "material": material
                }
            })),
        });

        Ok(material_index)
    }

    fn ensure_base_color_texture(
        &mut self,
        archive: &Archive,
        albedo_name: &str,
        alpha_name: Option<&str>,
        output_dir: &Path,
        texture_cache: &mut HashMap<String, usize>,
    ) -> Result<usize> {
        if let Some(alpha_name) = alpha_name {
            let merged_name = merged_alpha_name(albedo_name);
            if let Some(index) = texture_cache.get(&merged_name) {
                return Ok(*index);
            }

            let albedo = archive
                .get(albedo_name)
                .with_context(|| format!("missing texture {}", albedo_name))?;
            let alpha = archive
                .get(alpha_name)
                .with_context(|| format!("missing texture {}", alpha_name))?;

            let merged_path = output_dir.join(&merged_name);
            merge_alpha_texture(&albedo.data, &alpha.data, &merged_path)
                .with_context(|| format!("failed to merge {} and {}", albedo_name, alpha_name))?;

            let image_index = self.images.len();
            self.images.push(ImageDef {
                uri: merged_name.clone(),
                mime_type: None,
                name: Some(merged_name.clone()),
            });
            let texture_index = self.textures.len();
            self.textures.push(TextureDef {
                source: image_index,
                name: Some(merged_name.clone()),
            });
            texture_cache.insert(merged_name, texture_index);
            return Ok(texture_index);
        }

        self.ensure_texture_file(archive, albedo_name, output_dir, texture_cache)
    }

    fn ensure_texture_file(
        &mut self,
        archive: &Archive,
        name: &str,
        output_dir: &Path,
        texture_cache: &mut HashMap<String, usize>,
    ) -> Result<usize> {
        if let Some(index) = texture_cache.get(name) {
            return Ok(*index);
        }

        let entry = archive
            .get(name)
            .with_context(|| format!("missing texture {}", name))?;
        let output_path = output_dir.join(name);
        if !output_path.exists() {
            fs::write(&output_path, &entry.data)
                .with_context(|| format!("failed to write {}", output_path.display()))?;
        }

        let image_index = self.images.len();
        self.images.push(ImageDef {
            uri: name.to_string(),
            mime_type: None,
            name: Some(name.to_string()),
        });
        let texture_index = self.textures.len();
        self.textures.push(TextureDef {
            source: image_index,
            name: Some(name.to_string()),
        });
        texture_cache.insert(name.to_string(), texture_index);
        Ok(texture_index)
    }

    fn ensure_metallic_roughness_texture(
        &mut self,
        archive: &Archive,
        reflectivity_name: &str,
        gloss_name: &str,
        output_dir: &Path,
        texture_cache: &mut HashMap<String, usize>,
    ) -> Result<usize> {
        let merged_name = merged_metallic_roughness_name(reflectivity_name);
        if let Some(index) = texture_cache.get(&merged_name) {
            return Ok(*index);
        }

        let reflectivity = archive
            .get(reflectivity_name)
            .with_context(|| format!("missing texture {}", reflectivity_name))?;
        let gloss = archive
            .get(gloss_name)
            .with_context(|| format!("missing texture {}", gloss_name))?;

        let merged_path = output_dir.join(&merged_name);
        merge_metallic_roughness_texture(&reflectivity.data, &gloss.data, &merged_path)
            .with_context(|| format!("failed to merge {} and {}", reflectivity_name, gloss_name))?;

        let image_index = self.images.len();
        self.images.push(ImageDef {
            uri: merged_name.clone(),
            mime_type: None,
            name: Some(merged_name.clone()),
        });
        let texture_index = self.textures.len();
        self.textures.push(TextureDef {
            source: image_index,
            name: Some(merged_name.clone()),
        });
        texture_cache.insert(merged_name, texture_index);
        Ok(texture_index)
    }

    fn add_mesh(
        &mut self,
        mesh_desc: &crate::scene::MeshDesc,
        decoded: &DecodedMesh,
        material_lookup: &HashMap<String, usize>,
    ) -> Result<usize> {
        self.add_mesh_internal(mesh_desc, decoded, material_lookup, None)
    }

    fn add_node(&mut self, name: String, mesh_index: usize, transform: Option<[f32; 16]>) -> usize {
        let node_index = self.nodes.len();
        self.nodes.push(NodeDef {
            name: Some(name),
            mesh: Some(mesh_index),
            skin: None,
            matrix: transform,
            translation: None,
            rotation: None,
            scale: None,
            children: None,
            camera: None,
            extensions: None,
            extras: None,
        });
        node_index
    }

    pub(super) fn add_runtime_node(&mut self, node: NodeDef) -> usize {
        let node_index = self.nodes.len();
        self.nodes.push(node);
        node_index
    }

    pub(super) fn append_child(&mut self, parent: usize, child: usize) {
        if let Some(children) = &mut self.nodes[parent].children {
            children.push(child);
        } else {
            self.nodes[parent].children = Some(vec![child]);
        }
    }

    pub(super) fn add_runtime_mesh(
        &mut self,
        mesh_desc: &crate::scene::MeshDesc,
        decoded: &DecodedMesh,
        material_lookup: &HashMap<String, usize>,
        skin_data: Option<&MeshSkinData>,
    ) -> Result<usize> {
        self.add_mesh_internal(mesh_desc, decoded, material_lookup, skin_data)
    }

    pub(super) fn push_runtime_scalar_f32(&mut self, values: &[f32]) -> usize {
        self.push_scalar_f32(values, Target::ArrayBuffer)
    }

    pub(super) fn push_runtime_f32x3(&mut self, values: &[[f32; 3]]) -> usize {
        self.push_f32x3(values, Target::ArrayBuffer)
    }

    pub(super) fn push_runtime_f32x4(&mut self, values: &[[f32; 4]]) -> usize {
        self.push_f32x4(values, Target::ArrayBuffer)
    }

    pub(super) fn push_runtime_f32mat4(&mut self, values: &[[f32; 16]]) -> usize {
        self.push_f32mat4(values, Target::ArrayBuffer)
    }

    pub(super) fn node_local_matrix(&self, node_index: usize) -> [f32; 16] {
        let node = &self.nodes[node_index];
        if let Some(matrix) = node.matrix {
            matrix
        } else {
            compose_trs_matrix(
                node.translation.unwrap_or([0.0, 0.0, 0.0]),
                node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                node.scale.unwrap_or([1.0, 1.0, 1.0]),
            )
        }
    }

    fn add_mesh_internal(
        &mut self,
        mesh_desc: &crate::scene::MeshDesc,
        decoded: &DecodedMesh,
        material_lookup: &HashMap<String, usize>,
        skin_data: Option<&MeshSkinData>,
    ) -> Result<usize> {
        let position_accessor = self.push_f32x3(&decoded.positions, Target::ArrayBuffer);
        let normal_accessor = self.push_f32x3(&decoded.normals, Target::ArrayBuffer);
        let texcoord_accessor = self.push_f32x2(&decoded.texcoords, Target::ArrayBuffer);
        let color_accessor = decoded
            .colors
            .as_ref()
            .map(|colors| self.push_f32x4(colors, Target::ArrayBuffer));
        let joints_accessor = skin_data
            .map(|data| self.push_u16x4(&data.joints, Target::ArrayBuffer));
        let weights_accessor = skin_data
            .map(|data| self.push_f32x4(&data.weights, Target::ArrayBuffer));

        let mut primitives = Vec::new();
        for sub_mesh in &mesh_desc.sub_meshes {
            let end = sub_mesh.first_index + sub_mesh.index_count;
            let indices = decoded.indices[sub_mesh.first_index..end].to_vec();
            let index_accessor = self.push_u32(&indices, Target::ElementArrayBuffer);

            primitives.push(PrimitiveDef {
                attributes: Attributes {
                    position: position_accessor,
                    normal: Some(normal_accessor),
                    texcoord_0: Some(texcoord_accessor),
                    color_0: color_accessor,
                    joints_0: joints_accessor,
                    weights_0: weights_accessor,
                },
                indices: index_accessor,
                material: material_lookup.get(&sub_mesh.material).copied(),
                mode: Some(4),
            });
        }

        let mesh_index = self.meshes.len();
        self.meshes.push(MeshDef {
            name: Some(mesh_desc.name.clone()),
            primitives,
        });
        Ok(mesh_index)
    }

    fn push_scalar_f32(&mut self, values: &[f32], target: Target) -> usize {
        let start = self.push_aligned_bytes();
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for value in values {
            min = min.min(*value);
            max = max.max(*value);
            self.buffer.extend_from_slice(&value.to_le_bytes());
        }
        let view = self.push_buffer_view(start, self.buffer.len() - start, target);
        let accessor = self.accessors.len();
        self.accessors.push(Accessor {
            buffer_view: view,
            byte_offset: 0,
            component_type: 5126,
            count: values.len(),
            accessor_type: "SCALAR".to_string(),
            min: Some(vec![min]),
            max: Some(vec![max]),
        });
        accessor
    }

    fn finish(
        mut self,
        scene_name: &str,
        mut mesh_nodes: Vec<usize>,
        scene: &Scene,
        output_dir: &Path,
    ) -> Result<()> {
        if let Some(main_camera) = &scene.main_camera {
            if !self.bound_camera_names.contains("Main Camera") {
                if let Some(node_index) = self.add_camera_node("Main Camera", main_camera) {
                    mesh_nodes.push(node_index);
                }
            }
        }

        for (name, camera) in &scene.cameras {
            if name == "Main Camera" || self.bound_camera_names.contains(name) {
                continue;
            }
            if let Some(node_index) = self.add_camera_node(name, camera) {
                mesh_nodes.push(node_index);
            }
        }

        if let Some(lights) = &scene.lights {
            let count = lights.count.unwrap_or(0);
            if count > 0 && self.bound_light_indices.len() < count {
                self.ensure_punctual_lights_extension();
                for index in 0..count {
                    if self.bound_light_indices.contains(&index) {
                        continue;
                    }
                    let light_index = self.punctual_lights.len();
                    self.punctual_lights.push(build_light_def(lights, index));
                    let node_index = self.nodes.len();
                    self.nodes.push(NodeDef {
                        name: Some(format!("Light {}", index)),
                        mesh: None,
                        skin: None,
                        matrix: None,
                        translation: light_translation(lights, index),
                        rotation: light_rotation(lights, index),
                        scale: None,
                        children: None,
                        camera: None,
                        extensions: Some(NodeExtensions {
                            punctual_light: PunctualLightRef { light: light_index },
                        }),
                        extras: Some(json!({
                            "mviewer": {
                                "lightIndex": index
                            }
                        })),
                    });
                    mesh_nodes.push(node_index);
                }
            }
        }

        let bin_name = format!("{scene_name}.bin");
        let gltf_name = format!("{scene_name}.gltf");
        fs::write(output_dir.join(&bin_name), &self.buffer)
            .with_context(|| format!("failed to write {}", output_dir.join(&bin_name).display()))?;

        let root = GltfRoot {
            asset: AssetDef {
                generator: "mviewer".to_string(),
                version: "2.0".to_string(),
            },
            scene: 0,
            scenes: vec![SceneDef {
                name: Some(scene_name.to_string()),
                nodes: mesh_nodes,
            }],
            nodes: self.nodes,
            meshes: self.meshes,
            accessors: self.accessors,
            buffer_views: self.buffer_views,
            buffers: vec![BufferDef {
                byte_length: self.buffer.len(),
                uri: bin_name,
            }],
            materials: if self.materials.is_empty() {
                None
            } else {
                Some(self.materials)
            },
            textures: if self.textures.is_empty() {
                None
            } else {
                Some(self.textures)
            },
            images: if self.images.is_empty() {
                None
            } else {
                Some(self.images)
            },
            cameras: if self.cameras.is_empty() {
                None
            } else {
                Some(self.cameras)
            },
            skins: if self.skins.is_empty() {
                None
            } else {
                Some(self.skins)
            },
            animations: if self.animations.is_empty() {
                None
            } else {
                Some(self.animations)
            },
            extensions_used: if self.extensions_used.is_empty() {
                None
            } else {
                Some(self.extensions_used)
            },
            extensions: if self.punctual_lights.is_empty() {
                None
            } else {
                Some(RootExtensions {
                    punctual_lights: Some(PunctualLightsExtension {
                        lights: self.punctual_lights,
                    }),
                    mviewer_marmoset_runtime: None,
                })
            },
            extras: self.root_extras,
        };

        let gltf_path = output_dir.join(gltf_name);
        let json = serde_json::to_vec(&root)?;
        let _library_root =
            MythGltfRoot::from_slice(&json).context("generated glTF JSON failed myth-gltf-json validation parse")?;
        let json = serde_json::to_vec_pretty(&root)?;
        fs::write(&gltf_path, json)
            .with_context(|| format!("failed to write {}", gltf_path.display()))?;
        Ok(())
    }

    fn add_camera_node(&mut self, name: &str, camera: &MainCamera) -> Option<usize> {
        let view = camera.view.as_ref()?;
        let fov_deg = view.fov?;
        let camera_index = self.cameras.len();
        self.cameras.push(CameraDef {
            name: Some(name.to_string()),
            camera_type: "perspective".to_string(),
            perspective: PerspectiveDef {
                yfov: fov_deg.to_radians(),
                znear: 0.01,
                zfar: None,
            },
        });
        let (translation, rotation) = camera_transform_from_view(view);
        let node_index = self.nodes.len();
        self.nodes.push(NodeDef {
            name: Some(name.to_string()),
            mesh: None,
            skin: None,
            matrix: None,
            translation: Some(translation),
            rotation: Some(rotation),
            scale: None,
            children: None,
            camera: Some(camera_index),
            extensions: None,
            extras: Some(json!({
                "mviewer": {
                    "camera": camera
                }
            })),
        });
        self.bound_camera_names.insert(name.to_string());
        Some(node_index)
    }

    pub(super) fn attach_runtime_camera_node(
        &mut self,
        node_index: usize,
        name: &str,
        camera: &MainCamera,
    ) -> bool {
        let Some(view) = camera.view.as_ref() else {
            return false;
        };
        let Some(fov_deg) = view.fov else {
            return false;
        };
        let camera_index = self.cameras.len();
        self.cameras.push(CameraDef {
            name: Some(name.to_string()),
            camera_type: "perspective".to_string(),
            perspective: PerspectiveDef {
                yfov: fov_deg.to_radians(),
                znear: 0.01,
                zfar: None,
            },
        });
        self.nodes[node_index].camera = Some(camera_index);
        merge_optional_json(
            &mut self.nodes[node_index].extras,
            json!({
                "mviewer": {
                    "camera": camera
                }
            }),
        );
        self.bound_camera_names.insert(name.to_string());
        true
    }

    pub(super) fn attach_runtime_light_node(
        &mut self,
        node_index: usize,
        light_index_in_scene: usize,
        lights: &Lights,
    ) -> bool {
        let count = lights.count.unwrap_or(0);
        if light_index_in_scene >= count {
            return false;
        }
        self.ensure_punctual_lights_extension();
        let light_index = self.punctual_lights.len();
        self.punctual_lights
            .push(build_light_def(lights, light_index_in_scene));
        self.nodes[node_index].extensions = Some(NodeExtensions {
            punctual_light: PunctualLightRef { light: light_index },
        });
        merge_optional_json(
            &mut self.nodes[node_index].extras,
            json!({
                "mviewer": {
                    "lightIndex": light_index_in_scene
                }
            }),
        );
        self.bound_light_indices.insert(light_index_in_scene);
        true
    }

    fn ensure_punctual_lights_extension(&mut self) {
        if !self
            .extensions_used
            .iter()
            .any(|extension| extension == "KHR_lights_punctual")
        {
            self.extensions_used.push("KHR_lights_punctual".to_string());
        }
    }

    fn push_f32x3(&mut self, values: &[[f32; 3]], target: Target) -> usize {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let start = self.push_aligned_bytes();
        for value in values {
            for (i, component) in value.iter().enumerate() {
                min[i] = min[i].min(*component);
                max[i] = max[i].max(*component);
                self.buffer.extend_from_slice(&component.to_le_bytes());
            }
        }
        let view = self.push_buffer_view(start, self.buffer.len() - start, target);
        let accessor = self.accessors.len();
        self.accessors.push(Accessor {
            buffer_view: view,
            byte_offset: 0,
            component_type: 5126,
            count: values.len(),
            accessor_type: "VEC3".to_string(),
            min: Some(min.to_vec()),
            max: Some(max.to_vec()),
        });
        accessor
    }

    fn push_f32x2(&mut self, values: &[[f32; 2]], target: Target) -> usize {
        let start = self.push_aligned_bytes();
        for value in values {
            self.buffer.extend_from_slice(&value[0].to_le_bytes());
            self.buffer.extend_from_slice(&value[1].to_le_bytes());
        }
        let view = self.push_buffer_view(start, self.buffer.len() - start, target);
        let accessor = self.accessors.len();
        self.accessors.push(Accessor {
            buffer_view: view,
            byte_offset: 0,
            component_type: 5126,
            count: values.len(),
            accessor_type: "VEC2".to_string(),
            min: None,
            max: None,
        });
        accessor
    }

    fn push_f32x4(&mut self, values: &[[f32; 4]], target: Target) -> usize {
        let start = self.push_aligned_bytes();
        for value in values {
            for component in value {
                self.buffer.extend_from_slice(&component.to_le_bytes());
            }
        }
        let view = self.push_buffer_view(start, self.buffer.len() - start, target);
        let accessor = self.accessors.len();
        self.accessors.push(Accessor {
            buffer_view: view,
            byte_offset: 0,
            component_type: 5126,
            count: values.len(),
            accessor_type: "VEC4".to_string(),
            min: None,
            max: None,
        });
        accessor
    }

    fn push_u16x4(&mut self, values: &[[u16; 4]], target: Target) -> usize {
        let start = self.push_aligned_bytes();
        for value in values {
            for component in value {
                self.buffer.extend_from_slice(&component.to_le_bytes());
            }
        }
        let view = self.push_buffer_view(start, self.buffer.len() - start, target);
        let accessor = self.accessors.len();
        self.accessors.push(Accessor {
            buffer_view: view,
            byte_offset: 0,
            component_type: 5123,
            count: values.len(),
            accessor_type: "VEC4".to_string(),
            min: None,
            max: None,
        });
        accessor
    }

    fn push_f32mat4(&mut self, values: &[[f32; 16]], target: Target) -> usize {
        let start = self.push_aligned_bytes();
        for value in values {
            for component in value {
                self.buffer.extend_from_slice(&component.to_le_bytes());
            }
        }
        let view = self.push_buffer_view(start, self.buffer.len() - start, target);
        let accessor = self.accessors.len();
        self.accessors.push(Accessor {
            buffer_view: view,
            byte_offset: 0,
            component_type: 5126,
            count: values.len(),
            accessor_type: "MAT4".to_string(),
            min: None,
            max: None,
        });
        accessor
    }

    fn push_u32(&mut self, values: &[u32], target: Target) -> usize {
        let start = self.push_aligned_bytes();
        let mut min = u32::MAX;
        let mut max = 0u32;
        for value in values {
            min = min.min(*value);
            max = max.max(*value);
            self.buffer.extend_from_slice(&value.to_le_bytes());
        }
        let view = self.push_buffer_view(start, self.buffer.len() - start, target);
        let accessor = self.accessors.len();
        self.accessors.push(Accessor {
            buffer_view: view,
            byte_offset: 0,
            component_type: 5125,
            count: values.len(),
            accessor_type: "SCALAR".to_string(),
            min: Some(vec![min as f32]),
            max: Some(vec![max as f32]),
        });
        accessor
    }

    fn push_buffer_view(
        &mut self,
        byte_offset: usize,
        byte_length: usize,
        target: Target,
    ) -> usize {
        let index = self.buffer_views.len();
        self.buffer_views.push(BufferView {
            buffer: 0,
            byte_offset,
            byte_length,
            byte_stride: None,
            target: Some(target as u32),
        });
        index
    }

    fn push_aligned_bytes(&mut self) -> usize {
        while self.buffer.len() % 4 != 0 {
            self.buffer.push(0);
        }
        self.buffer.len()
    }
}

#[derive(Copy, Clone)]
enum Target {
    ArrayBuffer = 34962,
    ElementArrayBuffer = 34963,
}

pub(super) fn quaternion_from_euler_yxz(rotation_deg: [f32; 3]) -> [f32; 4] {
    let x = rotation_deg[0].to_radians() * 0.5;
    let y = rotation_deg[1].to_radians() * 0.5;
    let z = rotation_deg[2].to_radians() * 0.5;

    let qx = [x.sin(), 0.0, 0.0, x.cos()];
    let qy = [0.0, y.sin(), 0.0, y.cos()];
    let qz = [0.0, 0.0, z.sin(), z.cos()];
    normalize_quaternion(mul_quaternion(qy, mul_quaternion(qx, qz)))
}

pub(super) fn decompose_matrix_trs(matrix: [f32; 16]) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let translation = [matrix[12], matrix[13], matrix[14]];

    let mut scale = [
        (matrix[0] * matrix[0] + matrix[1] * matrix[1] + matrix[2] * matrix[2]).sqrt(),
        (matrix[4] * matrix[4] + matrix[5] * matrix[5] + matrix[6] * matrix[6]).sqrt(),
        (matrix[8] * matrix[8] + matrix[9] * matrix[9] + matrix[10] * matrix[10]).sqrt(),
    ];
    for value in &mut scale {
        if value.abs() <= f32::EPSILON {
            *value = 1.0;
        }
    }

    let rotation_matrix = [
        matrix[0] / scale[0],
        matrix[1] / scale[0],
        matrix[2] / scale[0],
        0.0,
        matrix[4] / scale[1],
        matrix[5] / scale[1],
        matrix[6] / scale[1],
        0.0,
        matrix[8] / scale[2],
        matrix[9] / scale[2],
        matrix[10] / scale[2],
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    let rotation = normalize_quaternion(quaternion_from_matrix(rotation_matrix));
    (translation, rotation, scale)
}

fn mul_quaternion(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn normalize_quaternion(quaternion: [f32; 4]) -> [f32; 4] {
    let length = (quaternion[0] * quaternion[0]
        + quaternion[1] * quaternion[1]
        + quaternion[2] * quaternion[2]
        + quaternion[3] * quaternion[3])
        .sqrt();
    if length <= f32::EPSILON {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [
            quaternion[0] / length,
            quaternion[1] / length,
            quaternion[2] / length,
            quaternion[3] / length,
        ]
    }
}

fn quaternion_from_matrix(matrix: [f32; 16]) -> [f32; 4] {
    let trace = matrix[0] + matrix[5] + matrix[10];
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (matrix[6] - matrix[9]) / s,
            (matrix[8] - matrix[2]) / s,
            (matrix[1] - matrix[4]) / s,
            0.25 * s,
        ]
    } else if matrix[0] > matrix[5] && matrix[0] > matrix[10] {
        let s = (1.0 + matrix[0] - matrix[5] - matrix[10]).sqrt() * 2.0;
        [
            0.25 * s,
            (matrix[4] + matrix[1]) / s,
            (matrix[8] + matrix[2]) / s,
            (matrix[6] - matrix[9]) / s,
        ]
    } else if matrix[5] > matrix[10] {
        let s = (1.0 + matrix[5] - matrix[0] - matrix[10]).sqrt() * 2.0;
        [
            (matrix[4] + matrix[1]) / s,
            0.25 * s,
            (matrix[9] + matrix[6]) / s,
            (matrix[8] - matrix[2]) / s,
        ]
    } else {
        let s = (1.0 + matrix[10] - matrix[0] - matrix[5]).sqrt() * 2.0;
        [
            (matrix[8] + matrix[2]) / s,
            (matrix[9] + matrix[6]) / s,
            0.25 * s,
            (matrix[1] - matrix[4]) / s,
        ]
    }
}

fn compose_trs_matrix(translation: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> [f32; 16] {
    let x = rotation[0];
    let y = rotation[1];
    let z = rotation[2];
    let w = rotation[3];
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;

    [
        (1.0 - (yy + zz)) * scale[0],
        (xy + wz) * scale[0],
        (xz - wy) * scale[0],
        0.0,
        (xy - wz) * scale[1],
        (1.0 - (xx + zz)) * scale[1],
        (yz + wx) * scale[1],
        0.0,
        (xz + wy) * scale[2],
        (yz - wx) * scale[2],
        (1.0 - (xx + yy)) * scale[2],
        0.0,
        translation[0],
        translation[1],
        translation[2],
        1.0,
    ]
}

pub(super) fn identity_matrix() -> [f32; 16] {
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}

pub(super) fn mul_matrix4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut result = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            result[column * 4 + row] = a[row] * b[column * 4]
                + a[4 + row] * b[column * 4 + 1]
                + a[8 + row] * b[column * 4 + 2]
                + a[12 + row] * b[column * 4 + 3];
        }
    }
    result
}

pub(super) fn invert_matrix4(matrix: &[f32; 16]) -> Option<[f32; 16]> {
    let mut inv = [0.0f32; 16];
    inv[0] = matrix[5] * matrix[10] * matrix[15] - matrix[5] * matrix[11] * matrix[14] - matrix[9] * matrix[6] * matrix[15]
        + matrix[9] * matrix[7] * matrix[14] + matrix[13] * matrix[6] * matrix[11] - matrix[13] * matrix[7] * matrix[10];
    inv[4] = -matrix[4] * matrix[10] * matrix[15] + matrix[4] * matrix[11] * matrix[14] + matrix[8] * matrix[6] * matrix[15]
        - matrix[8] * matrix[7] * matrix[14] - matrix[12] * matrix[6] * matrix[11] + matrix[12] * matrix[7] * matrix[10];
    inv[8] = matrix[4] * matrix[9] * matrix[15] - matrix[4] * matrix[11] * matrix[13] - matrix[8] * matrix[5] * matrix[15]
        + matrix[8] * matrix[7] * matrix[13] + matrix[12] * matrix[5] * matrix[11] - matrix[12] * matrix[7] * matrix[9];
    inv[12] = -matrix[4] * matrix[9] * matrix[14] + matrix[4] * matrix[10] * matrix[13] + matrix[8] * matrix[5] * matrix[14]
        - matrix[8] * matrix[6] * matrix[13] - matrix[12] * matrix[5] * matrix[10] + matrix[12] * matrix[6] * matrix[9];
    inv[1] = -matrix[1] * matrix[10] * matrix[15] + matrix[1] * matrix[11] * matrix[14] + matrix[9] * matrix[2] * matrix[15]
        - matrix[9] * matrix[3] * matrix[14] - matrix[13] * matrix[2] * matrix[11] + matrix[13] * matrix[3] * matrix[10];
    inv[5] = matrix[0] * matrix[10] * matrix[15] - matrix[0] * matrix[11] * matrix[14] - matrix[8] * matrix[2] * matrix[15]
        + matrix[8] * matrix[3] * matrix[14] + matrix[12] * matrix[2] * matrix[11] - matrix[12] * matrix[3] * matrix[10];
    inv[9] = -matrix[0] * matrix[9] * matrix[15] + matrix[0] * matrix[11] * matrix[13] + matrix[8] * matrix[1] * matrix[15]
        - matrix[8] * matrix[3] * matrix[13] - matrix[12] * matrix[1] * matrix[11] + matrix[12] * matrix[3] * matrix[9];
    inv[13] = matrix[0] * matrix[9] * matrix[14] - matrix[0] * matrix[10] * matrix[13] - matrix[8] * matrix[1] * matrix[14]
        + matrix[8] * matrix[2] * matrix[13] + matrix[12] * matrix[1] * matrix[10] - matrix[12] * matrix[2] * matrix[9];
    inv[2] = matrix[1] * matrix[6] * matrix[15] - matrix[1] * matrix[7] * matrix[14] - matrix[5] * matrix[2] * matrix[15]
        + matrix[5] * matrix[3] * matrix[14] + matrix[13] * matrix[2] * matrix[7] - matrix[13] * matrix[3] * matrix[6];
    inv[6] = -matrix[0] * matrix[6] * matrix[15] + matrix[0] * matrix[7] * matrix[14] + matrix[4] * matrix[2] * matrix[15]
        - matrix[4] * matrix[3] * matrix[14] - matrix[12] * matrix[2] * matrix[7] + matrix[12] * matrix[3] * matrix[6];
    inv[10] = matrix[0] * matrix[5] * matrix[15] - matrix[0] * matrix[7] * matrix[13] - matrix[4] * matrix[1] * matrix[15]
        + matrix[4] * matrix[3] * matrix[13] + matrix[12] * matrix[1] * matrix[7] - matrix[12] * matrix[3] * matrix[5];
    inv[14] = -matrix[0] * matrix[5] * matrix[14] + matrix[0] * matrix[6] * matrix[13] + matrix[4] * matrix[1] * matrix[14]
        - matrix[4] * matrix[2] * matrix[13] - matrix[12] * matrix[1] * matrix[6] + matrix[12] * matrix[2] * matrix[5];
    inv[3] = -matrix[1] * matrix[6] * matrix[11] + matrix[1] * matrix[7] * matrix[10] + matrix[5] * matrix[2] * matrix[11]
        - matrix[5] * matrix[3] * matrix[10] - matrix[9] * matrix[2] * matrix[7] + matrix[9] * matrix[3] * matrix[6];
    inv[7] = matrix[0] * matrix[6] * matrix[11] - matrix[0] * matrix[7] * matrix[10] - matrix[4] * matrix[2] * matrix[11]
        + matrix[4] * matrix[3] * matrix[10] + matrix[8] * matrix[2] * matrix[7] - matrix[8] * matrix[3] * matrix[6];
    inv[11] = -matrix[0] * matrix[5] * matrix[11] + matrix[0] * matrix[7] * matrix[9] + matrix[4] * matrix[1] * matrix[11]
        - matrix[4] * matrix[3] * matrix[9] - matrix[8] * matrix[1] * matrix[7] + matrix[8] * matrix[3] * matrix[5];
    inv[15] = matrix[0] * matrix[5] * matrix[10] - matrix[0] * matrix[6] * matrix[9] - matrix[4] * matrix[1] * matrix[10]
        + matrix[4] * matrix[2] * matrix[9] + matrix[8] * matrix[1] * matrix[6] - matrix[8] * matrix[2] * matrix[5];

    let determinant = matrix[0] * inv[0] + matrix[1] * inv[4] + matrix[2] * inv[8] + matrix[3] * inv[12];
    if determinant.abs() <= f32::EPSILON {
        return None;
    }
    let inv_det = 1.0 / determinant;
    for value in &mut inv { *value *= inv_det; }
    Some(inv)
}

fn camera_transform_from_view(view: &ViewDesc) -> ([f32; 3], [f32; 4]) {
    let angles = view.angles.unwrap_or([0.0, 0.0]);
    let radius = view.orbit_radius.unwrap_or(0.0);
    let pivot = view.pivot.unwrap_or([0.0, 0.0, 0.0]);
    let rx = rotation_matrix(angles[0].to_radians(), 0);
    let ry = rotation_matrix(angles[1].to_radians(), 1);
    let rotation_matrix = mul_matrix4(&ry, &rx);
    let forward = [rotation_matrix[8], rotation_matrix[9], rotation_matrix[10]];
    let translation = [
        pivot[0] + forward[0] * radius,
        pivot[1] + forward[1] * radius,
        pivot[2] + forward[2] * radius,
    ];
    let rotation = normalize_quaternion(quaternion_from_matrix(rotation_matrix));
    (translation, rotation)
}

fn build_light_def(lights: &Lights, index: usize) -> LightDef {
    let color = light_color(lights, index);
    let intensity = color
        .map(|c| c[0].max(c[1]).max(c[2]))
        .filter(|value| *value > 0.0)
        .unwrap_or(1.0);
    let normalized_color = color.map(|c| {
        if intensity > 0.0 {
            [c[0] / intensity, c[1] / intensity, c[2] / intensity]
        } else {
            c
        }
    });
    let outer_cone = lights
        .spot
        .as_ref()
        .and_then(|spot| spot.get(index * 3).copied())
        .unwrap_or(0.0)
        .to_radians()
        * 0.5;
    let has_spot = outer_cone > 0.0 && outer_cone < std::f32::consts::PI * 0.5;
    LightDef {
        light_type: if has_spot {
            "spot".to_string()
        } else if light_position_w(lights, index).unwrap_or(1.0) == 0.0 {
            "directional".to_string()
        } else {
            "point".to_string()
        },
        name: Some(format!("Light {}", index)),
        color: normalized_color,
        intensity: Some(intensity),
        range: light_range(lights, index),
        spot: has_spot.then_some(SpotDef {
            inner_cone_angle: Some(outer_cone * 0.5),
            outer_cone_angle: outer_cone,
        }),
    }
}

fn light_translation(lights: &Lights, index: usize) -> Option<[f32; 3]> {
    let positions = lights.positions.as_ref()?;
    let base = index * 4;
    Some([
        *positions.get(base)?,
        *positions.get(base + 1)?,
        *positions.get(base + 2)?,
    ])
}

fn light_position_w(lights: &Lights, index: usize) -> Option<f32> {
    let positions = lights.positions.as_ref()?;
    positions.get(index * 4 + 3).copied()
}

fn light_rotation(lights: &Lights, index: usize) -> Option<[f32; 4]> {
    let directions = lights.directions.as_ref()?;
    let base = index * 3;
    let dir = [
        *directions.get(base)?,
        *directions.get(base + 1)?,
        *directions.get(base + 2)?,
    ];
    Some(quaternion_from_direction(dir))
}

fn light_color(lights: &Lights, index: usize) -> Option<[f32; 3]> {
    let colors = lights.colors.as_ref()?;
    let base = index * 3;
    Some([
        *colors.get(base)?,
        *colors.get(base + 1)?,
        *colors.get(base + 2)?,
    ])
}

fn light_range(lights: &Lights, index: usize) -> Option<f32> {
    let parameters = lights.parameters.as_ref()?;
    let inv_distance = *parameters.get(index * 3 + 2)?;
    (inv_distance > 0.0).then_some(1.0 / inv_distance)
}

fn quaternion_from_direction(direction: [f32; 3]) -> [f32; 4] {
    let mut forward = normalize_vec3(direction);
    if vec3_len(forward) <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    forward = [-forward[0], -forward[1], -forward[2]];
    let up_hint = if forward[1].abs() > 0.99 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let right = normalize_vec3(cross(up_hint, forward));
    let up = normalize_vec3(cross(forward, right));
    normalize_quaternion(quaternion_from_matrix([
        right[0], right[1], right[2], 0.0, up[0], up[1], up[2], 0.0, forward[0], forward[1],
        forward[2], 0.0, 0.0, 0.0, 0.0, 1.0,
    ]))
}

fn export_source_texture_files(
    archive: &Archive,
    output_dir: &Path,
    progress: &mut dyn FnMut(u8, &str),
) -> Result<()> {
    let entries = archive.entries();
    let texture_entries: Vec<_> = entries
        .iter()
        .filter(|entry| is_source_texture_file(&entry.name))
        .collect();
    let total = texture_entries.len().max(1);
    for (index, entry) in texture_entries.into_iter().enumerate() {
        if !is_source_texture_file(&entry.name) {
            continue;
        }
        let relative_path = archive_relative_output_path(&entry.name);
        let output_path = output_dir.join(&relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if !output_path.exists() {
            fs::write(&output_path, &entry.data)
                .with_context(|| format!("failed to write {}", output_path.display()))?;
        }
        let p = ((index + 1) * 20 / total) as u8;
        progress(p, "Copying source textures");
    }
    Ok(())
}

fn is_source_texture_file(name: &str) -> bool {
    matches!(
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "tga" | "bmp" | "gif" | "dds")
    )
}

fn archive_relative_output_path(name: &str) -> PathBuf {
    let mut path = PathBuf::new();
    let mut had_component = false;
    for component in name.split(['/', '\\']) {
        if component.is_empty() || component == "." || component == ".." {
            continue;
        }
        path.push(sanitize_path_component(component));
        had_component = true;
    }
    if had_component {
        path
    } else {
        PathBuf::from("unnamed.bin")
    }
}

fn sanitize_path_component(component: &str) -> String {
    let sanitized: String = component
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => ch,
        })
        .collect();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn path_to_slash(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn merge_optional_json(target: &mut Option<serde_json::Value>, incoming: serde_json::Value) {
    match target {
        Some(existing) => merge_json(existing, incoming),
        None => *target = Some(incoming),
    }
}

fn merge_json(target: &mut serde_json::Value, incoming: serde_json::Value) {
    match (target, incoming) {
        (serde_json::Value::Object(target_map), serde_json::Value::Object(incoming_map)) => {
            for (key, value) in incoming_map {
                if let Some(existing) = target_map.get_mut(&key) {
                    merge_json(existing, value);
                } else {
                    target_map.insert(key, value);
                }
            }
        }
        (target_value, incoming_value) => *target_value = incoming_value,
    }
}

fn rotation_matrix(angle: f32, axis: usize) -> [f32; 16] {
    let (s, c) = angle.sin_cos();
    match axis {
        0 => [1.0, 0.0, 0.0, 0.0, 0.0, c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0],
        1 => [c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0],
        _ => [c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn vec3_len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn normalize_vec3(v: [f32; 3]) -> [f32; 3] {
    let len = vec3_len(v);
    if len <= f32::EPSILON {
        [0.0, 0.0, 0.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

#[derive(serde::Serialize)]
pub struct GltfRoot {
    pub asset: AssetDef,
    pub scene: usize,
    pub scenes: Vec<SceneDef>,
    pub nodes: Vec<NodeDef>,
    pub meshes: Vec<MeshDef>,
    pub accessors: Vec<Accessor>,
    #[serde(rename = "bufferViews")]
    pub buffer_views: Vec<BufferView>,
    pub buffers: Vec<BufferDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materials: Option<Vec<MaterialDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textures: Option<Vec<TextureDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cameras: Option<Vec<CameraDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skins: Option<Vec<SkinDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animations: Option<Vec<AnimationDef>>,
    #[serde(rename = "extensionsUsed", skip_serializing_if = "Option::is_none")]
    pub extensions_used: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<RootExtensions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
pub struct AssetDef {
    pub generator: String,
    pub version: String,
}

#[derive(serde::Serialize)]
pub struct SceneDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub nodes: Vec<usize>,
}

#[derive(serde::Serialize, Clone)]
pub struct NodeDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<[f32; 16]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<NodeExtensions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
pub struct MeshDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub primitives: Vec<PrimitiveDef>,
}

#[derive(serde::Serialize)]
pub struct PrimitiveDef {
    pub attributes: Attributes,
    pub indices: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

#[derive(serde::Serialize)]
pub struct Attributes {
    #[serde(rename = "POSITION")]
    pub position: usize,
    #[serde(rename = "NORMAL", skip_serializing_if = "Option::is_none")]
    pub normal: Option<usize>,
    #[serde(rename = "TEXCOORD_0", skip_serializing_if = "Option::is_none")]
    pub texcoord_0: Option<usize>,
    #[serde(rename = "COLOR_0", skip_serializing_if = "Option::is_none")]
    pub color_0: Option<usize>,
    #[serde(rename = "JOINTS_0", skip_serializing_if = "Option::is_none")]
    pub joints_0: Option<usize>,
    #[serde(rename = "WEIGHTS_0", skip_serializing_if = "Option::is_none")]
    pub weights_0: Option<usize>,
}

#[derive(serde::Serialize)]
pub struct Accessor {
    #[serde(rename = "bufferView")]
    pub buffer_view: usize,
    #[serde(rename = "byteOffset")]
    pub byte_offset: usize,
    #[serde(rename = "componentType")]
    pub component_type: u32,
    pub count: usize,
    #[serde(rename = "type")]
    pub accessor_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<Vec<f32>>,
}

#[derive(serde::Serialize)]
pub struct BufferView {
    pub buffer: usize,
    #[serde(rename = "byteOffset")]
    pub byte_offset: usize,
    #[serde(rename = "byteLength")]
    pub byte_length: usize,
    #[serde(rename = "byteStride", skip_serializing_if = "Option::is_none")]
    pub byte_stride: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<u32>,
}

#[derive(serde::Serialize)]
pub struct BufferDef {
    #[serde(rename = "byteLength")]
    pub byte_length: usize,
    pub uri: String,
}

#[derive(serde::Serialize)]
pub struct MaterialDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "pbrMetallicRoughness", skip_serializing_if = "Option::is_none")]
    pub pbr_metallic_roughness: Option<PbrMetallicRoughness>,
    #[serde(rename = "normalTexture", skip_serializing_if = "Option::is_none")]
    pub normal_texture: Option<NormalTextureRef>,
    #[serde(rename = "emissiveFactor", skip_serializing_if = "Option::is_none")]
    pub emissive_factor: Option<[f32; 3]>,
    #[serde(rename = "alphaMode", skip_serializing_if = "Option::is_none")]
    pub alpha_mode: Option<String>,
    #[serde(rename = "alphaCutoff", skip_serializing_if = "Option::is_none")]
    pub alpha_cutoff: Option<f32>,
    #[serde(rename = "doubleSided", skip_serializing_if = "Option::is_none")]
    pub double_sided: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
pub struct PbrMetallicRoughness {
    #[serde(rename = "baseColorTexture", skip_serializing_if = "Option::is_none")]
    pub base_color_texture: Option<TextureRef>,
    #[serde(rename = "metallicFactor", skip_serializing_if = "Option::is_none")]
    pub metallic_factor: Option<f32>,
    #[serde(rename = "roughnessFactor", skip_serializing_if = "Option::is_none")]
    pub roughness_factor: Option<f32>,
    #[serde(rename = "metallicRoughnessTexture", skip_serializing_if = "Option::is_none")]
    pub metallic_roughness_texture: Option<TextureRef>,
}

#[derive(serde::Serialize)]
pub struct TextureRef {
    pub index: usize,
    #[serde(rename = "texCoord", skip_serializing_if = "Option::is_none")]
    pub tex_coord: Option<u32>,
}

#[derive(serde::Serialize)]
pub struct NormalTextureRef {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
}

#[derive(serde::Serialize)]
pub struct ImageDef {
    pub uri: String,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(serde::Serialize)]
pub struct TextureDef {
    pub source: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(serde::Serialize)]
pub struct CameraDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub camera_type: String,
    pub perspective: PerspectiveDef,
}

#[derive(serde::Serialize)]
pub struct PerspectiveDef {
    pub yfov: f32,
    pub znear: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zfar: Option<f32>,
}

#[derive(serde::Serialize)]
pub struct RootExtensions {
    #[serde(rename = "KHR_lights_punctual", skip_serializing_if = "Option::is_none")]
    pub punctual_lights: Option<PunctualLightsExtension>,
    #[serde(rename = "MVIEWER_marmoset_runtime", skip_serializing_if = "Option::is_none")]
    pub mviewer_marmoset_runtime: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
pub struct PunctualLightsExtension {
    pub lights: Vec<LightDef>,
}

#[derive(serde::Serialize)]
pub struct LightDef {
    #[serde(rename = "type")]
    pub light_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spot: Option<SpotDef>,
}

#[derive(serde::Serialize)]
pub struct SpotDef {
    #[serde(rename = "innerConeAngle", skip_serializing_if = "Option::is_none")]
    pub inner_cone_angle: Option<f32>,
    #[serde(rename = "outerConeAngle")]
    pub outer_cone_angle: f32,
}

#[derive(serde::Serialize, Clone)]
pub struct NodeExtensions {
    #[serde(rename = "KHR_lights_punctual")]
    pub punctual_light: PunctualLightRef,
}

#[derive(serde::Serialize, Clone)]
pub struct PunctualLightRef {
    pub light: usize,
}

#[derive(serde::Serialize, Clone)]
pub struct SkinDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "inverseBindMatrices", skip_serializing_if = "Option::is_none")]
    pub inverse_bind_matrices: Option<usize>,
    pub joints: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skeleton: Option<usize>,
}

#[derive(serde::Serialize, Clone)]
pub struct AnimationDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub channels: Vec<AnimationChannelDef>,
    pub samplers: Vec<AnimationSamplerDef>,
}

#[derive(serde::Serialize, Clone)]
pub struct AnimationChannelDef {
    pub sampler: usize,
    pub target: AnimationChannelTargetDef,
}

#[derive(serde::Serialize, Clone)]
pub struct AnimationChannelTargetDef {
    pub node: usize,
    pub path: String,
}

#[derive(serde::Serialize, Clone)]
pub struct AnimationSamplerDef {
    pub input: usize,
    pub output: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpolation: Option<String>,
}
