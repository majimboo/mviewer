use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use super::GltfOutputFormat;
use crate::animation::{
    ParsedAnimation, ParsedAnimationSet, ParsedAnimatedObject, identity_matrix, lerp_matrix4,
    mul_matrix4 as animation_mul_matrix4,
};
use crate::archive::Archive;
use crate::js_export::{JsAnimationSample, JsExportScene};
use crate::mesh::decode_mesh;
use crate::scene::Scene;

use super::{
    AnimationChannelDef, AnimationChannelTargetDef, AnimationDef, AnimationSamplerDef, GltfBuilder,
    NodeDef, SkinDef, decompose_matrix_trs, mul_matrix4,
};

pub fn export_animated_scene(
    builder: &mut GltfBuilder,
    archive: &Archive,
    scene: &Scene,
    animations: &ParsedAnimationSet,
    material_lookup: &HashMap<String, usize>,
    input_path: &Path,
    output_dir: &Path,
    js_scene: Option<&JsExportScene>,
    output_format: GltfOutputFormat,
    progress: &mut dyn FnMut(u8, &str),
) -> Result<()> {
    let Some(primary_animation) = selected_primary_animation(scene, animations) else {
        return super::export_static_scene(
            builder,
            archive,
            scene,
            material_lookup,
            input_path,
            output_dir,
            output_format,
            progress,
        );
    };

    let mut object_to_node = HashMap::new();
    let mut object_nodes = Vec::new();
    let mesh_object_bindings = build_object_binding_map(
        scene
            .anim_data
            .as_ref()
            .map(|anim| anim.mesh_ids.iter().map(|entry| entry.part_index).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    let light_object_bindings = build_object_binding_map(
        scene
            .anim_data
            .as_ref()
            .map(|anim| anim.light_ids.iter().map(|entry| entry.part_index).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    let material_object_bindings = build_object_binding_map(
        scene
            .anim_data
            .as_ref()
            .map(|anim| {
                anim.material_ids
                    .iter()
                    .map(|entry| entry.part_index)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );

    for (object_index, object) in primary_animation.animated_objects.iter().enumerate() {
        let local_matrix = compute_local_matrix(
            primary_animation,
            object_index,
            0.0,
            animations.scene_scale,
            &object_to_node,
        );
        let (translation, rotation, scale) = decompose_matrix_trs(local_matrix);
        let node_index = builder.add_runtime_node(NodeDef {
            name: Some(object_name(object)),
            mesh: None,
            skin: None,
            matrix: None,
            translation: Some(translation),
            rotation: Some(rotation),
            scale: Some(scale),
            children: Some(Vec::new()),
            camera: None,
            extensions: None,
            extras: Some(build_object_extras(
                object,
                mesh_object_bindings.get(&object_index).copied(),
                light_object_bindings.get(&object_index).copied(),
                material_object_bindings.get(&object_index).copied(),
            )),
        });
        object_to_node.insert(object_index, node_index);
        object_nodes.push(object_index);
    }

    attach_runtime_scene_bindings(
        builder,
        scene,
        primary_animation,
        &object_to_node,
        &light_object_bindings,
    );

    let mut scene_roots = Vec::new();
    for &object_index in &object_nodes {
        let object = &primary_animation.animated_objects[object_index];
        let node_index = object_to_node[&object_index];
        let parent_index = object.desc.parent_index;
        if parent_index != object_index && object_to_node.contains_key(&parent_index) {
            builder.append_child(object_to_node[&parent_index], node_index);
        } else {
            scene_roots.push(node_index);
        }
    }

    let mesh_part_indices: Vec<Option<usize>> = scene
        .anim_data
        .as_ref()
        .map(|anim| anim.mesh_ids.iter().map(|entry| Some(entry.part_index)).collect())
        .unwrap_or_else(|| vec![None; scene.meshes.len()]);

    let mut render_nodes = Vec::new();
    let total_meshes = scene.meshes.len().max(1);
    for (mesh_scene_index, mesh_desc) in scene.meshes.iter().enumerate() {
        let entry = archive
            .get(&mesh_desc.file)
            .with_context(|| format!("missing mesh payload {}", mesh_desc.file))?;
        let decoded = decode_mesh(&entry.data, mesh_desc)
            .with_context(|| format!("failed to decode {}", mesh_desc.file))?;

        let mapped_object_index = mesh_part_indices
            .get(mesh_scene_index)
            .and_then(|value| *value)
            .filter(|part_index| object_to_node.contains_key(part_index));
        let skin_binding = mapped_object_index
            .and_then(|object_index| primary_animation.find_object(object_index))
            .and_then(|object| usize::try_from(object.desc.skinning_rig_index).ok());
        let skin_binding = skin_binding
            .and_then(|rig_index| animations.skinning_rigs.get(rig_index))
            .map(|rig| {
                build_skin_binding(
                    builder,
                    rig,
                    mesh_desc.vertex_count,
                    primary_animation,
                    &object_to_node,
                    mapped_object_index.expect("mapped mesh object required for skin"),
                )
            })
            .transpose()?;

        let mesh_index = builder.add_runtime_mesh(
            mesh_desc,
            &decoded,
            material_lookup,
            skin_binding.as_ref().map(|binding| &binding.skin_data),
        )?;
        let mesh_node_matrix = if mapped_object_index.is_some() {
            None
        } else {
            mesh_desc.transform
        };
        let render_node_index = builder.add_runtime_node(NodeDef {
            name: Some(mesh_desc.name.clone()),
            mesh: Some(mesh_index),
            skin: None,
            matrix: mesh_node_matrix,
            translation: None,
            rotation: None,
            scale: None,
            children: None,
            camera: None,
            extensions: None,
            extras: Some(json!({
                "mviewer": {
                    "mesh": mesh_desc
                }
            })),
        });

        if let Some(object_index) = mapped_object_index {
            builder.append_child(object_to_node[&object_index], render_node_index);
        } else {
            scene_roots.push(render_node_index);
        }

        render_nodes.push(RenderNodeBinding {
            render_node_index,
            animated_object_index: mapped_object_index,
            skin_binding,
        });
        let p = 70 + ((mesh_scene_index + 1) * 25 / total_meshes);
        progress(p as u8, "Processing meshes");
    }

    attach_skins(builder, primary_animation, &render_nodes);
    attach_animations(builder, scene, animations, &object_to_node, &render_nodes, js_scene);

    let scene_name = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scene");
    progress(98, "Writing scene files");
    std::mem::take(builder).finish(scene_name, scene_roots, scene, output_dir, output_format)
}

fn selected_primary_animation<'a>(
    scene: &Scene,
    animations: &'a ParsedAnimationSet,
) -> Option<&'a ParsedAnimation> {
    let selected = scene
        .anim_data
        .as_ref()
        .and_then(|anim| anim.selected_animation)
        .unwrap_or(0);
    animations
        .animations
        .get(selected)
        .or_else(|| animations.animations.first())
}

fn object_name(object: &ParsedAnimatedObject) -> String {
    format!("{} {}", object.desc.scene_object_type, object.desc.part_name)
}

fn build_object_binding_map(part_indices: Vec<usize>) -> HashMap<usize, usize> {
    let mut map = HashMap::new();
    for (scene_index, object_index) in part_indices.into_iter().enumerate() {
        map.insert(object_index, scene_index);
    }
    map
}

fn build_object_extras(
    object: &ParsedAnimatedObject,
    mesh_index: Option<usize>,
    light_index: Option<usize>,
    material_index: Option<usize>,
) -> serde_json::Value {
    let property_names: Vec<_> = object
        .desc
        .animated_properties
        .iter()
        .map(|property| property.name.clone())
        .collect();
    json!({
        "mviewer": {
            "animatedObject": object.desc,
            "runtimeBinding": {
                "meshIndex": mesh_index,
                "lightIndex": light_index,
                "materialIndex": material_index,
            },
            "animatedProperties": property_names,
            "hasVisibilityAnimation": property_names.iter().any(|name| name == "Visible"),
            "hasMaterialAnimation": property_names.iter().any(|name| {
                matches!(name.as_str(), "OffsetU" | "OffsetV" | "EmissiveIntensity")
            }),
        }
    })
}

fn attach_runtime_scene_bindings(
    builder: &mut GltfBuilder,
    scene: &Scene,
    animation: &ParsedAnimation,
    object_to_node: &HashMap<usize, usize>,
    light_object_bindings: &HashMap<usize, usize>,
) {
    if let Some(lights) = &scene.lights {
        for (&object_index, &light_index) in light_object_bindings {
            if let Some(&node_index) = object_to_node.get(&object_index) {
                builder.attach_runtime_light_node(node_index, light_index, lights);
            }
        }
    }

    for (object_index, object) in animation.animated_objects.iter().enumerate() {
        let Some(&node_index) = object_to_node.get(&object_index) else {
            continue;
        };
        if object.desc.part_name == "Main Camera" {
            if let Some(main_camera) = &scene.main_camera {
                builder.attach_runtime_camera_node(node_index, "Main Camera", main_camera);
            }
            continue;
        }
        if let Some(camera) = scene.cameras.get(&object.desc.part_name) {
            builder.attach_runtime_camera_node(node_index, &object.desc.part_name, camera);
        }
    }
}

#[allow(dead_code)]
fn build_runtime_metadata(
    scene: &Scene,
    animations: &ParsedAnimationSet,
    primary_animation: &ParsedAnimation,
    object_to_node: &HashMap<usize, usize>,
    mesh_object_bindings: &HashMap<usize, usize>,
    light_object_bindings: &HashMap<usize, usize>,
    material_object_bindings: &HashMap<usize, usize>,
    render_nodes: &[RenderNodeBinding],
) -> serde_json::Value {
    let mesh_runtime_bindings: Vec<_> = render_nodes
        .iter()
        .enumerate()
        .map(|(mesh_index, binding)| {
            json!({
                "meshIndex": mesh_index,
                "renderNode": binding.render_node_index,
                "animatedObjectIndex": binding.animated_object_index,
                "skinningRigIndex": binding
                    .skin_binding
                    .as_ref()
                    .map(|_| binding.animated_object_index)
                    .flatten(),
            })
        })
        .collect();

    let animated_objects: Vec<_> = primary_animation
        .animated_objects
        .iter()
        .enumerate()
        .map(|(object_index, object)| {
            let property_names: Vec<_> = object
                .desc
                .animated_properties
                .iter()
                .map(|property| property.name.clone())
                .collect();
            json!({
                "objectIndex": object_index,
                "nodeIndex": object_to_node.get(&object_index),
                "sceneObjectType": object.desc.scene_object_type,
                "partName": object.desc.part_name,
                "meshIndex": mesh_object_bindings.get(&object_index),
                "lightIndex": light_object_bindings.get(&object_index),
                "materialIndex": material_object_bindings.get(&object_index),
                "visibleAtStart": primary_animation.is_visible_at_frame_percent(object_index, 0.0),
                "visibilityAnimated": property_names.iter().any(|name| name == "Visible"),
                "materialAnimated": property_names.iter().any(|name| {
                    matches!(name.as_str(), "OffsetU" | "OffsetV" | "EmissiveIntensity")
                }),
                "properties": property_names,
            })
        })
        .collect();

    json!({
        "selectedAnimation": scene.anim_data.as_ref().and_then(|anim| anim.selected_animation),
        "selectedCamera": scene.anim_data.as_ref().and_then(|anim| anim.selected_camera),
        "meshBindings": mesh_runtime_bindings,
        "lightBindings": map_bindings(light_object_bindings, "lightIndex"),
        "materialBindings": map_bindings(material_object_bindings, "materialIndex"),
        "animatedObjects": animated_objects,
        "clips": animations
            .animations
            .iter()
            .map(|animation| {
                build_clip_runtime_metadata(
                    animation,
                    animations.scene_scale,
                    object_to_node,
                )
            })
            .collect::<Vec<_>>(),
    })
}

#[allow(dead_code)]
fn build_clip_runtime_metadata(
    animation: &ParsedAnimation,
    scene_scale: f32,
    object_to_node: &HashMap<usize, usize>,
) -> serde_json::Value {
    let objects: Vec<_> = animation
        .animated_objects
        .iter()
        .enumerate()
        .filter_map(|(object_index, object)| {
            let property_names: Vec<_> = object
                .desc
                .animated_properties
                .iter()
                .map(|property| property.name.clone())
                .collect();
            let non_transform_properties: Vec<_> = property_names
                .iter()
                .filter(|name| {
                    !matches!(
                        name.as_str(),
                        "Translation X"
                            | "Translation Y"
                            | "Translation Z"
                            | "Rotation X"
                            | "Rotation Y"
                            | "Rotation Z"
                            | "Scale X"
                            | "Scale Y"
                            | "Scale Z"
                    )
                })
                .cloned()
                .collect();
            if non_transform_properties.is_empty() {
                return None;
            }

            let sample_times = collect_property_times(
                object,
                &non_transform_properties
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>(),
            );
            let tracks = build_runtime_property_tracks(
                animation,
                object_index,
                object,
                &non_transform_properties,
                &sample_times,
            );
            Some(json!({
                "objectIndex": object_index,
                "sceneObjectType": object.desc.scene_object_type,
                "partName": object.desc.part_name,
                "nonTransformProperties": non_transform_properties,
                "sampleTimes": sample_times,
                "visibleAtStart": animation.is_visible_at_frame_percent(object_index, 0.0),
                "tracks": tracks,
            }))
        })
        .collect();

    json!({
        "name": animation.desc.name,
        "length": animation.desc.length,
        "originalFPS": animation.desc.original_fps,
        "totalFrames": animation.desc.total_frames,
        "objectsWithRuntimeProperties": objects,
        "sampledFrames": build_clip_frame_samples(animation, scene_scale, object_to_node),
    })
}

#[allow(dead_code)]
fn build_clip_frame_samples(
    animation: &ParsedAnimation,
    scene_scale: f32,
    object_to_node: &HashMap<usize, usize>,
) -> Vec<serde_json::Value> {
    let total_frames = animation.desc.total_frames.max(1);
    let fps = if animation.desc.original_fps.abs() > f32::EPSILON {
        animation.desc.original_fps
    } else {
        30.0
    };
    (0..total_frames)
        .map(|frame_index| {
            let seconds = frame_index as f32 / fps;
            let objects: Vec<_> = animation
                .animated_objects
                .iter()
                .enumerate()
                .map(|(object_index, object)| {
                    let frame_percent =
                        animation.get_object_animation_frame_percent(object_index, seconds);
                    let local = compute_local_matrix(
                        animation,
                        object_index,
                        seconds,
                        scene_scale,
                        object_to_node,
                    );
                    let world = animation.get_world_transform(object_index, seconds, scene_scale);
                    let mut runtime_properties = serde_json::Map::new();
                    for property in &object.desc.animated_properties {
                        if is_transform_property(&property.name) {
                            continue;
                        }
                        let default = runtime_property_default(&property.name);
                        let value = object
                            .sample_named_property(&property.name, frame_percent, default)
                            .unwrap_or(default);
                        runtime_properties.insert(property.name.clone(), json!(value));
                    }
                    runtime_properties.insert(
                        "VisibleInherited".to_string(),
                        json!(animation.is_visible_at_frame_percent(object_index, frame_percent)),
                    );
                    json!({
                        "objectIndex": object_index,
                        "nodeIndex": object_to_node.get(&object_index),
                        "localMatrix": local,
                        "worldMatrix": world,
                        "runtimeProperties": runtime_properties,
                    })
                })
                .collect();
            json!({
                "frame": frame_index,
                "time": seconds,
                "objects": objects,
            })
        })
        .collect()
}

#[allow(dead_code)]
fn is_transform_property(name: &str) -> bool {
    matches!(
        name,
        "Translation X"
            | "Translation Y"
            | "Translation Z"
            | "Rotation X"
            | "Rotation Y"
            | "Rotation Z"
            | "Scale X"
            | "Scale Y"
            | "Scale Z"
    )
}

#[allow(dead_code)]
fn build_runtime_property_tracks(
    animation: &ParsedAnimation,
    object_index: usize,
    object: &ParsedAnimatedObject,
    property_names: &[String],
    sample_times: &[f32],
) -> Vec<serde_json::Value> {
    property_names
        .iter()
        .map(|property_name| {
            let default = runtime_property_default(property_name);
            let values: Vec<_> = sample_times
                .iter()
                .map(|frame| {
                    object
                        .sample_named_property(property_name, *frame, default)
                        .unwrap_or(default)
                })
                .collect();
            let active = has_nontrivial_scalar_animation(&values);
            let semantic = runtime_property_semantic(property_name, &object.desc.scene_object_type);
            let visible_samples = if property_name == "Visible" {
                Some(
                    sample_times
                        .iter()
                        .map(|frame| animation.is_visible_at_frame_percent(object_index, *frame))
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            };
            json!({
                "name": property_name,
                "semantic": semantic,
                "defaultValue": default,
                "active": active,
                "values": values,
                "visibleSamples": visible_samples,
            })
        })
        .collect()
}

#[allow(dead_code)]
fn runtime_property_default(property_name: &str) -> f32 {
    match property_name {
        "Visible" => 1.0,
        "Red" | "Green" | "Blue" => 1.0,
        "Field Of View" => 45.0,
        "PlaybackSpeed" => 1.0,
        "Brightness" | "Light Brightness" | "Background Brightness" | "EmissiveIntensity" => 1.0,
        _ => 0.0,
    }
}

#[allow(dead_code)]
fn runtime_property_semantic(property_name: &str, scene_object_type: &str) -> &'static str {
    match property_name {
        "Visible" => "visibility",
        "OffsetU" => "material.uvOffset.u",
        "OffsetV" => "material.uvOffset.v",
        "EmissiveIntensity" => "material.emissiveIntensity",
        "Red" => {
            if scene_object_type.contains("Light") || scene_object_type == "FogSO" {
                "color.r"
            } else {
                "generic.r"
            }
        }
        "Green" => {
            if scene_object_type.contains("Light") || scene_object_type == "FogSO" {
                "color.g"
            } else {
                "generic.g"
            }
        }
        "Blue" => {
            if scene_object_type.contains("Light") || scene_object_type == "FogSO" {
                "color.b"
            } else {
                "generic.b"
            }
        }
        "Brightness" => {
            if scene_object_type.contains("Light") {
                "light.intensity"
            } else if scene_object_type == "SkyBoxSO" {
                "sky.brightness"
            } else {
                "generic.brightness"
            }
        }
        "Distance" => {
            if scene_object_type.contains("Light") {
                "light.range"
            } else if scene_object_type == "FogSO" {
                "fog.distance"
            } else {
                "generic.distance"
            }
        }
        "Spot Angle" => "light.spotAngle",
        "Spot Sharpness" => "light.spotSharpness",
        "Attenuation" => "light.attenuation",
        "Field Of View" => "camera.fov",
        "Focus Distance" => "camera.focusDistance",
        "Front Scale" => "camera.frontScale",
        "Back Scale" => "camera.backScale",
        "Max Bokeh" => "camera.maxBokeh",
        "Rotation Degrees" => "camera.rotationDegrees",
        "Swirl Vignette" => "camera.swirlVignette",
        "Sharpen" => "camera.post.sharpen",
        "Sharpen Limit" => "camera.post.sharpenLimit",
        "Bloom Size" => "camera.post.bloomSize",
        "Vignette Curve" => "camera.post.vignetteCurve",
        "Grain" => "camera.post.grain",
        "Grain Sharpness" => "camera.post.grainSharpness",
        "Barrel" => "camera.post.barrel",
        "Lens Flare Strength" => "camera.post.lensFlareStrength",
        "Lens Flare Contrast" => "camera.post.lensFlareContrast",
        "Lens Flare Scale" => "camera.post.lensFlareScale",
        "Opacity" => "fog.opacity",
        "Dispersion" => "fog.dispersion",
        "CurrentAnimation" => "playback.currentAnimation",
        "AnimationProgress" => "playback.progress",
        "PlaybackSpeed" => "playback.speed",
        _ => "generic",
    }
}

#[allow(dead_code)]
fn has_nontrivial_scalar_animation(values: &[f32]) -> bool {
    values
        .windows(2)
        .any(|window| (window[0] - window[1]).abs() > 1e-5)
}

#[allow(dead_code)]
fn map_bindings(bindings: &HashMap<usize, usize>, key: &str) -> Vec<serde_json::Value> {
    let mut pairs: Vec<_> = bindings.iter().collect();
    pairs.sort_by_key(|(object_index, _)| **object_index);
    pairs.into_iter()
        .map(|(object_index, scene_index)| {
            json!({
                "objectIndex": object_index,
                key: scene_index,
            })
        })
        .collect()
}

struct RenderNodeBinding {
    render_node_index: usize,
    animated_object_index: Option<usize>,
    skin_binding: Option<MeshSkinBinding>,
}

#[derive(Debug)]
pub struct MeshSkinData {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

struct MeshSkinBinding {
    skin_data: MeshSkinData,
    joint_nodes: Vec<usize>,
    inverse_bind_matrices: Vec<[f32; 16]>,
    skeleton_root: Option<usize>,
    cluster_bindings: Vec<ClusterJointBinding>,
}

#[allow(dead_code)]
struct ClusterJointBinding {
    node_index: usize,
    mesh_object_index: usize,
    mesh_model_part_index: usize,
    link_mode: u32,
    link_object_index: usize,
    associate_object_index: usize,
    default_cluster_world_transform: [f32; 16],
    default_cluster_base_transform: [f32; 16],
    default_associate_world_transform: Option<[f32; 16]>,
}

fn attach_skins(builder: &mut GltfBuilder, _primary_animation: &ParsedAnimation, render_nodes: &[RenderNodeBinding]) {
    for binding in render_nodes {
        let Some(_animated_object_index) = binding.animated_object_index else {
            continue;
        };
        let Some(skin_binding) = &binding.skin_binding else {
            continue;
        };

        let inverse_bind_accessor =
            builder.push_runtime_f32mat4(&skin_binding.inverse_bind_matrices);
        let skin_index = builder.skins.len();
        builder.skins.push(SkinDef {
            name: Some(format!("Skin {}", skin_index)),
            inverse_bind_matrices: Some(inverse_bind_accessor),
            joints: skin_binding.joint_nodes.clone(),
            skeleton: skin_binding.skeleton_root,
        });
        builder.nodes[binding.render_node_index].skin = Some(skin_index);
    }
}

fn attach_animations(
    builder: &mut GltfBuilder,
    scene: &Scene,
    animations: &ParsedAnimationSet,
    object_to_node: &HashMap<usize, usize>,
    render_nodes: &[RenderNodeBinding],
    js_scene: Option<&JsExportScene>,
) {
    let selected_index = scene
        .anim_data
        .as_ref()
        .and_then(|anim| anim.selected_animation);
    for (animation_index, animation) in animations.animations.iter().enumerate() {
        if let Some(clip) = build_animation_clip(
            builder,
            animation,
            animations.scene_scale,
            object_to_node,
            render_nodes,
            if selected_index == Some(animation_index) {
                js_scene.and_then(|scene| scene.sampled_animation.as_ref())
            } else {
                None
            },
        )
        {
            builder.animations.push(clip);
        }
    }
}

fn build_animation_clip(
    builder: &mut GltfBuilder,
    animation: &ParsedAnimation,
    scene_scale: f32,
    object_to_node: &HashMap<usize, usize>,
    render_nodes: &[RenderNodeBinding],
    sampled_animation: Option<&crate::js_export::JsSampledAnimation>,
) -> Option<AnimationDef> {
    let mut channels = Vec::new();
    let mut samplers = Vec::new();

    let used_sampled_animation = if let Some(sampled) = sampled_animation {
        build_animation_channels_from_samples(
            builder,
            animation,
            object_to_node,
            sampled,
            &mut channels,
            &mut samplers,
        )
    } else {
        false
    };

    if !used_sampled_animation {
        for object_index in 0..animation.animated_objects.len() {
            let Some(&node_index) = object_to_node.get(&object_index) else {
                continue;
            };

            let sample_times = collect_object_sample_times(animation, object_index);
            if sample_times.len() <= 1 {
                continue;
            }

            let mut translations = Vec::with_capacity(sample_times.len());
            let mut rotations = Vec::with_capacity(sample_times.len());
            let mut scales = Vec::with_capacity(sample_times.len());
            for seconds in sample_times.iter().copied() {
                let local = compute_local_matrix(animation, object_index, seconds, scene_scale, object_to_node);
                let (translation, rotation, scale) = decompose_matrix_trs(local);
                translations.push(translation);
                rotations.push(rotation);
                scales.push(scale);
            }

            if has_nontrivial_vec3_animation(&translations) {
                let sampler_index = samplers.len();
                samplers.push(AnimationSamplerDef {
                    input: builder.push_runtime_scalar_f32(&sample_times),
                    output: builder.push_runtime_f32x3(&translations),
                    interpolation: Some("LINEAR".to_string()),
                });
                channels.push(AnimationChannelDef {
                    sampler: sampler_index,
                    target: AnimationChannelTargetDef {
                        node: node_index,
                        path: "translation".to_string(),
                    },
                });
            }

            if has_nontrivial_quat_animation(&rotations) {
                let sampler_index = samplers.len();
                samplers.push(AnimationSamplerDef {
                    input: builder.push_runtime_scalar_f32(&sample_times),
                    output: builder.push_runtime_f32x4(&rotations),
                    interpolation: Some("LINEAR".to_string()),
                });
                channels.push(AnimationChannelDef {
                    sampler: sampler_index,
                    target: AnimationChannelTargetDef {
                        node: node_index,
                        path: "rotation".to_string(),
                    },
                });
            }

            if has_nontrivial_vec3_animation(&scales) {
                let sampler_index = samplers.len();
                samplers.push(AnimationSamplerDef {
                    input: builder.push_runtime_scalar_f32(&sample_times),
                    output: builder.push_runtime_f32x3(&scales),
                    interpolation: Some("LINEAR".to_string()),
                });
                channels.push(AnimationChannelDef {
                    sampler: sampler_index,
                    target: AnimationChannelTargetDef {
                        node: node_index,
                        path: "scale".to_string(),
                    },
                });
            }
        }
    }

    for binding in render_nodes {
        let Some(skin_binding) = &binding.skin_binding else {
            continue;
        };
        if skin_binding.cluster_bindings.is_empty() {
            continue;
        }

        let sample_times: Vec<f32> = (0..=animation.desc.total_frames)
            .map(|frame| frame as f32 / animation.desc.original_fps)
            .collect();
        for cluster_binding in &skin_binding.cluster_bindings {
            let mut translations = Vec::with_capacity(sample_times.len());
            let mut rotations = Vec::with_capacity(sample_times.len());
            let mut scales = Vec::with_capacity(sample_times.len());
            for seconds in sample_times.iter().copied() {
                let matrix = sample_cluster_matrix(
                    animation,
                    cluster_binding,
                    seconds,
                );
                let (translation, rotation, scale) = decompose_matrix_trs(matrix);
                translations.push(translation);
                rotations.push(rotation);
                scales.push(scale);
            }

            if has_nontrivial_vec3_animation(&translations) {
                let sampler_index = samplers.len();
                samplers.push(AnimationSamplerDef {
                    input: builder.push_runtime_scalar_f32(&sample_times),
                    output: builder.push_runtime_f32x3(&translations),
                    interpolation: Some("LINEAR".to_string()),
                });
                channels.push(AnimationChannelDef {
                    sampler: sampler_index,
                    target: AnimationChannelTargetDef {
                        node: cluster_binding.node_index,
                        path: "translation".to_string(),
                    },
                });
            }

            if has_nontrivial_quat_animation(&rotations) {
                let sampler_index = samplers.len();
                samplers.push(AnimationSamplerDef {
                    input: builder.push_runtime_scalar_f32(&sample_times),
                    output: builder.push_runtime_f32x4(&rotations),
                    interpolation: Some("LINEAR".to_string()),
                });
                channels.push(AnimationChannelDef {
                    sampler: sampler_index,
                    target: AnimationChannelTargetDef {
                        node: cluster_binding.node_index,
                        path: "rotation".to_string(),
                    },
                });
            }

            if has_nontrivial_vec3_animation(&scales) {
                let sampler_index = samplers.len();
                samplers.push(AnimationSamplerDef {
                    input: builder.push_runtime_scalar_f32(&sample_times),
                    output: builder.push_runtime_f32x3(&scales),
                    interpolation: Some("LINEAR".to_string()),
                });
                channels.push(AnimationChannelDef {
                    sampler: sampler_index,
                    target: AnimationChannelTargetDef {
                        node: cluster_binding.node_index,
                        path: "scale".to_string(),
                    },
                });
            }
        }
    }

    if channels.is_empty() {
        None
    } else {
        Some(AnimationDef {
            name: Some(animation.desc.name.clone()),
            channels,
            samplers,
        })
    }
}

fn collect_property_times(object: &ParsedAnimatedObject, property_names: &[&str]) -> Vec<f32> {
    let Some(keyframes) = &object.keyframes else {
        return Vec::new();
    };
    let mut times = BTreeSet::new();
    for track in &keyframes.properties {
        if property_names.iter().any(|property_name| *property_name == track.name) {
            for time in track.keyframe_times() {
                times.insert((time * 1000.0).round() as i32);
            }
        }
    }
    times.into_iter().map(|time| time as f32 / 1000.0).collect()
}

fn collect_object_sample_times(animation: &ParsedAnimation, object_index: usize) -> Vec<f32> {
    let safe_fps = if animation.desc.original_fps.abs() < f32::EPSILON {
        30.0
    } else {
        animation.desc.original_fps
    };
    let mut frame_times = BTreeSet::new();
    collect_related_frames(animation, object_index, &mut frame_times, 0);
    frame_times
        .into_iter()
        .map(|time| (time as f32 / 1000.0) / safe_fps)
        .collect()
}

fn collect_related_frames(
    animation: &ParsedAnimation,
    object_index: usize,
    frame_times: &mut BTreeSet<i32>,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    let object = &animation.animated_objects[object_index];
    let times = collect_property_times(
        object,
        &[
            "Translation X",
            "Translation Y",
            "Translation Z",
            "Rotation X",
            "Rotation Y",
            "Rotation Z",
            "Scale X",
            "Scale Y",
            "Scale Z",
        ],
    );
    for time in times {
        frame_times.insert((time * 1000.0).round() as i32);
    }
    if object.desc.model_part_index != object_index {
        collect_related_frames(animation, object.desc.model_part_index, frame_times, depth + 1);
    }
    if object.desc.parent_index != object_index {
        let parent = &animation.animated_objects[object.desc.parent_index];
        if parent.desc.model_part_index != parent.desc.parent_index {
            collect_related_frames(animation, parent.desc.model_part_index, frame_times, depth + 1);
        }
    }
    if frame_times.is_empty() {
        frame_times.insert(0);
    }
}

fn compute_local_matrix(
    animation: &ParsedAnimation,
    object_index: usize,
    seconds: f32,
    _scene_scale: f32,
    _object_to_node: &HashMap<usize, usize>,
) -> [f32; 16] {
    let object = &animation.animated_objects[object_index];
    let frame_percent = animation.get_object_animation_frame_percent(object_index, seconds);
    let local = object.evaluate_local_transform_at_frame_percent(frame_percent, true);
    if object.desc.model_part_index != object_index {
        let animated_local = animation.get_animated_local_transform(object_index, seconds);
        mul_matrix4(&animated_local, &local)
    } else {
        local
    }
}

fn build_animation_channels_from_samples(
    builder: &mut GltfBuilder,
    animation: &ParsedAnimation,
    object_to_node: &HashMap<usize, usize>,
    sampled_animation: &crate::js_export::JsSampledAnimation,
    channels: &mut Vec<AnimationChannelDef>,
    samplers: &mut Vec<AnimationSamplerDef>,
) -> bool {
    if sampled_animation.samples.len() <= 1 {
        return false;
    }

    let mut added_any = false;
    for object_index in 0..animation.animated_objects.len() {
        let Some(&node_index) = object_to_node.get(&object_index) else {
            continue;
        };

        let mut sample_times = Vec::with_capacity(sampled_animation.samples.len());
        let mut translations = Vec::with_capacity(sampled_animation.samples.len());
        let mut rotations = Vec::with_capacity(sampled_animation.samples.len());
        let mut scales = Vec::with_capacity(sampled_animation.samples.len());

        for sample in &sampled_animation.samples {
            let Some(local_matrix) =
                sampled_local_matrix_for_object(animation, object_index, sample, sampled_animation)
            else {
                continue;
            };
            sample_times.push(sample.seconds);
            let (translation, rotation, scale) = decompose_matrix_trs(local_matrix);
            translations.push(translation);
            rotations.push(rotation);
            scales.push(scale);
        }

        if sample_times.len() <= 1 {
            continue;
        }

        if has_nontrivial_vec3_animation(&translations) {
            let sampler_index = samplers.len();
            samplers.push(AnimationSamplerDef {
                input: builder.push_runtime_scalar_f32(&sample_times),
                output: builder.push_runtime_f32x3(&translations),
                interpolation: Some("LINEAR".to_string()),
            });
            channels.push(AnimationChannelDef {
                sampler: sampler_index,
                target: AnimationChannelTargetDef {
                    node: node_index,
                    path: "translation".to_string(),
                },
            });
            added_any = true;
        }

        if has_nontrivial_quat_animation(&rotations) {
            let sampler_index = samplers.len();
            samplers.push(AnimationSamplerDef {
                input: builder.push_runtime_scalar_f32(&sample_times),
                output: builder.push_runtime_f32x4(&rotations),
                interpolation: Some("LINEAR".to_string()),
            });
            channels.push(AnimationChannelDef {
                sampler: sampler_index,
                target: AnimationChannelTargetDef {
                    node: node_index,
                    path: "rotation".to_string(),
                },
            });
            added_any = true;
        }

        if has_nontrivial_vec3_animation(&scales) {
            let sampler_index = samplers.len();
            samplers.push(AnimationSamplerDef {
                input: builder.push_runtime_scalar_f32(&sample_times),
                output: builder.push_runtime_f32x3(&scales),
                interpolation: Some("LINEAR".to_string()),
            });
            channels.push(AnimationChannelDef {
                sampler: sampler_index,
                target: AnimationChannelTargetDef {
                    node: node_index,
                    path: "scale".to_string(),
                },
            });
            added_any = true;
        }
    }

    added_any
}

fn sampled_local_matrix_for_object(
    animation: &ParsedAnimation,
    object_index: usize,
    sample: &JsAnimationSample,
    sampled_animation: &crate::js_export::JsSampledAnimation,
) -> Option<[f32; 16]> {
    let object = &animation.animated_objects[object_index];
    let world = sample
        .objects
        .iter()
        .find(|entry| entry.id == object_index)?
        .world_matrix;

    if object.desc.parent_index == object_index {
        return Some(world);
    }

    let parent_world = sample
        .objects
        .iter()
        .find(|entry| entry.id == object.desc.parent_index)
        .map(|entry| entry.world_matrix)
        .or_else(|| {
            sampled_animation
                .samples
                .first()
                .and_then(|first| {
                    first
                        .objects
                        .iter()
                        .find(|entry| entry.id == object.desc.parent_index)
                        .map(|entry| entry.world_matrix)
                })
        })?;

    let parent_inverse = crate::gltf::invert_matrix4(&parent_world)?;
    Some(mul_matrix4(&parent_inverse, &world))
}

fn sample_cluster_matrix(
    animation: &ParsedAnimation,
    cluster_binding: &ClusterJointBinding,
    seconds: f32,
) -> [f32; 16] {
    let mesh_model_part = &animation.animated_objects[cluster_binding.mesh_model_part_index];
    let frame_blend = (seconds * mesh_model_part.desc.model_part_fps).fract();
    let frame0 = animation
        .get_object_animation_frame_percent(cluster_binding.mesh_model_part_index, seconds)
        .floor();
    let frame1 = frame0 + 1.0;

    let matrix0 = solve_cluster_matrix_at_frame(animation, cluster_binding, frame0);
    let matrix1 = solve_cluster_matrix_at_frame(animation, cluster_binding, frame1);
    lerp_matrix4(&matrix0, &matrix1, frame_blend)
}

fn solve_cluster_matrix_at_frame(
    animation: &ParsedAnimation,
    cluster_binding: &ClusterJointBinding,
    frame: f32,
) -> [f32; 16] {
    if cluster_binding.link_mode == 1 {
        let link =
            animation.evaluate_model_part_transform_at_frame(cluster_binding.link_object_index, frame);
        let link_base = animation_mul_matrix4(&link, &cluster_binding.default_cluster_base_transform);
        let Some(default_associate_world_transform) =
            cluster_binding.default_associate_world_transform
        else {
            return identity_matrix();
        };
        let Some(associate_inverse) =
            crate::gltf::invert_matrix4(&default_associate_world_transform)
        else {
            return identity_matrix();
        };
        let tmp = animation_mul_matrix4(&associate_inverse, &link_base);
        let tmp = animation_mul_matrix4(&associate_inverse, &tmp);
        let Some(cluster_world_inverse) =
            crate::gltf::invert_matrix4(&cluster_binding.default_cluster_world_transform)
        else {
            return identity_matrix();
        };
        animation_mul_matrix4(&cluster_world_inverse, &tmp)
    } else {
        let link =
            animation.evaluate_model_part_transform_at_frame(cluster_binding.link_object_index, frame);
        let mesh = animation.evaluate_model_part_transform_at_frame(cluster_binding.mesh_model_part_index, frame);
        let Some(mesh_inverse) = crate::gltf::invert_matrix4(&mesh) else {
            return identity_matrix();
        };
        let delta = animation_mul_matrix4(&mesh_inverse, &link);
        animation_mul_matrix4(&delta, &cluster_binding.default_cluster_base_transform)
    }
}

fn has_nontrivial_vec3_animation(values: &[[f32; 3]]) -> bool {
    values.windows(2).any(|window| !approx_eq3(window[0], window[1]))
}

fn has_nontrivial_quat_animation(values: &[[f32; 4]]) -> bool {
    values.windows(2).any(|window| !approx_eq4(window[0], window[1]))
}

fn approx_eq3(a: [f32; 3], b: [f32; 3]) -> bool {
    (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5 && (a[2] - b[2]).abs() < 1e-5
}

fn approx_eq4(a: [f32; 4], b: [f32; 4]) -> bool {
    (a[0] - b[0]).abs() < 1e-5
        && (a[1] - b[1]).abs() < 1e-5
        && (a[2] - b[2]).abs() < 1e-5
        && (a[3] - b[3]).abs() < 1e-5
}

fn build_skin_binding(
    builder: &mut GltfBuilder,
    rig: &crate::animation::SkinningRig,
    vertex_count: usize,
    animation: &ParsedAnimation,
    object_to_node: &HashMap<usize, usize>,
    mesh_object_index: usize,
) -> Result<MeshSkinBinding> {
    let mut joints = vec![[0u16; 4]; vertex_count];
    let mut weights = vec![[0.0f32; 4]; vertex_count];

    let mesh_object = animation
        .find_object(mesh_object_index)
        .context("mesh animated object missing for skin binding")?;
    let mesh_parent_node = object_to_node
        .get(&mesh_object_index)
        .copied()
        .context("mesh animated object missing glTF node")?;

    let mut cluster_to_joint = HashMap::new();
    let mut joint_nodes = Vec::new();
    let mut inverse_bind_matrices = Vec::new();
    let mut joint_object_indices = Vec::new();
    let mut cluster_bindings = Vec::new();
    for (cluster_index, cluster) in rig.clusters.iter().enumerate() {
        let joint_index =
            u16::try_from(joint_nodes.len()).context("too many skin joints for glTF skin")?;
        let joint_node = builder.add_runtime_node(NodeDef {
            name: Some(format!("cluster_joint_{}_{}", mesh_object.desc.part_name, cluster_index)),
            mesh: None,
            skin: None,
            matrix: Some(identity_matrix()),
            translation: None,
            rotation: None,
            scale: None,
            children: None,
            camera: None,
            extensions: None,
            extras: None,
        });
        builder.append_child(mesh_parent_node, joint_node);
        cluster_to_joint.insert(cluster_index, joint_index);
        joint_nodes.push(joint_node);
        joint_object_indices.push(cluster.link_object_index as usize);
        inverse_bind_matrices.push(identity_matrix());
        cluster_bindings.push(ClusterJointBinding {
            node_index: joint_node,
            mesh_object_index,
            mesh_model_part_index: mesh_object.desc.model_part_index,
            link_mode: cluster.link_mode,
            link_object_index: cluster.link_object_index as usize,
            associate_object_index: cluster.associate_object_index as usize,
            default_cluster_world_transform: cluster.default_cluster_world_transform,
            default_cluster_base_transform: cluster.default_cluster_base_transform,
            default_associate_world_transform: cluster.default_associate_world_transform,
        });
    }
    if joint_nodes.is_empty() {
        anyhow::bail!("skinning rig produced no usable joint nodes");
    }

    let mut link_cursor = 0usize;
    for vertex_index in 0..vertex_count {
        let count = rig.link_map_count.get(vertex_index).copied().unwrap_or(0) as usize;
        let mut influences = Vec::with_capacity(count);
        for offset in 0..count {
            let cluster_index = *rig
                .link_map_cluster_indices
                .get(link_cursor + offset)
                .context("cluster index out of bounds while building skin data")? as usize;
            let weight = *rig
                .link_map_weights
                .get(link_cursor + offset)
                .context("cluster weight out of bounds while building skin data")?;
            let Some(&joint_index) = cluster_to_joint.get(&cluster_index) else {
                continue;
            };
            influences.push((joint_index, weight));
        }
        link_cursor += count;

        influences.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let retained = influences.len().min(4);
        let total_weight = influences
            .iter()
            .take(retained)
            .map(|(_, weight)| *weight)
            .sum::<f32>();

        for (slot, (joint_index, weight)) in influences.into_iter().take(retained).enumerate() {
            joints[vertex_index][slot] = joint_index;
            weights[vertex_index][slot] = if total_weight > 0.0 {
                weight / total_weight
            } else {
                0.0
            };
        }
    }

    Ok(MeshSkinBinding {
        skin_data: MeshSkinData { joints, weights },
        joint_nodes,
        inverse_bind_matrices,
        skeleton_root: object_to_node.get(&mesh_object_index).copied(),
        cluster_bindings,
    })
}

#[allow(dead_code)]
fn find_skeleton_root(
    joints: &[usize],
    animation: &ParsedAnimation,
    object_to_node: &HashMap<usize, usize>,
) -> Option<usize> {
    let first_joint = *joints.first()?;
    let mut current = first_joint;
    loop {
        let object = &animation.animated_objects[current];
        if object.desc.parent_index == current || !object_to_node.contains_key(&object.desc.parent_index)
        {
            return object_to_node.get(&current).copied();
        }
        current = object.desc.parent_index;
    }
}
