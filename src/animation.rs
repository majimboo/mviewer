use std::io::Cursor;

use anyhow::{Context, Result, bail};
use byteorder::{LittleEndian, ReadBytesExt};

use crate::archive::Archive;
use crate::scene::{
    AnimData, AnimatedObjectDesc, AnimatedPropertyDesc, AnimationDesc, Scene, SkinningRigDesc,
};

#[derive(Debug)]
pub struct ParsedAnimationSet {
    pub scene_scale: f32,
    pub num_matrices: usize,
    pub matrix_table: MatrixTable,
    pub skinning_rigs: Vec<SkinningRig>,
    pub animations: Vec<ParsedAnimation>,
}

#[derive(Debug)]
pub struct MatrixTable {
    pub matrices: Vec<[f32; 16]>,
}

#[derive(Debug)]
pub struct SkinningRig {
    pub source_file: String,
    pub expected_num_clusters: usize,
    pub expected_num_vertices: usize,
    pub num_cluster_links: usize,
    pub original_object_index: usize,
    pub is_rigid_skin: bool,
    pub tangent_method: u32,
    pub clusters: Vec<SkinningCluster>,
    pub link_map_count: Vec<u8>,
    pub link_map_cluster_indices: Vec<u16>,
    pub link_map_weights: Vec<f32>,
}

#[derive(Debug)]
pub struct SkinningCluster {
    pub link_mode: u32,
    pub link_object_index: u32,
    pub associate_object_index: u32,
    pub default_cluster_world_transform_index: u32,
    pub default_cluster_base_transform_index: u32,
    pub default_associate_world_transform_index: u32,
    pub default_cluster_world_transform: [f32; 16],
    pub default_cluster_base_transform: [f32; 16],
    pub default_associate_world_transform: Option<[f32; 16]>,
}

#[derive(Debug)]
pub struct ParsedAnimation {
    pub desc: AnimationDesc,
    pub animated_objects: Vec<ParsedAnimatedObject>,
}

#[derive(Debug)]
pub struct ParsedAnimatedObject {
    pub desc: AnimatedObjectDesc,
    pub keyframes: Option<KeyframeFile>,
}

#[derive(Debug)]
pub struct KeyframeFile {
    pub shared_header_words: u32,
    pub properties: Vec<KeyframePropertyTrack>,
}

#[derive(Debug)]
pub struct KeyframePropertyTrack {
    pub name: String,
    pub packing_type: u8,
    pub interpolation_type: u8,
    pub num_keyframes: usize,
    pub data: KeyframeTrackData,
}

#[derive(Debug)]
pub enum KeyframeTrackData {
    Packed0(Vec<Packed0Keyframe>),
    Packed1(Vec<Packed1Keyframe>),
    Packed2(Vec<f32>),
}

#[derive(Debug)]
pub struct Packed0Keyframe {
    pub value: f32,
    pub weigh_in: f32,
    pub weigh_out: f32,
    pub frame_index: u16,
    pub interpolation: u16,
}

#[derive(Debug)]
pub struct Packed1Keyframe {
    pub value: f32,
    pub frame_index: u16,
    pub interpolation: u16,
}

impl ParsedAnimationSet {
    pub fn from_scene(archive: &Archive, scene: &Scene) -> Result<Option<Self>> {
        let Some(anim) = &scene.anim_data else {
            return Ok(None);
        };
        Self::from_anim_data(archive, anim)
    }

    pub fn from_anim_data(archive: &Archive, anim: &AnimData) -> Result<Option<Self>> {
        let matrix_table = parse_matrix_table(archive, anim.num_matrices)?;
        let mut skinning_rigs = Vec::with_capacity(anim.skinning_rigs.len());
        for rig in &anim.skinning_rigs {
            skinning_rigs.push(parse_skinning_rig(archive, rig, &matrix_table)?);
        }

        let mut animations = Vec::with_capacity(anim.animations.len());
        for animation in &anim.animations {
            animations.push(parse_animation(archive, animation)?);
        }

        Ok(Some(Self {
            scene_scale: anim.scene_scale,
            num_matrices: anim.num_matrices,
            matrix_table,
            skinning_rigs,
            animations,
        }))
    }
}

impl ParsedAnimation {
    pub fn find_object(&self, object_index: usize) -> Option<&ParsedAnimatedObject> {
        self.animated_objects.get(object_index)
    }

    pub fn get_object_animation_frame_percent(&self, object_index: usize, seconds: f32) -> f32 {
        let object = &self.animated_objects[object_index];
        let animation_length = object.animation_length();
        if self.desc.total_frames == 0 || animation_length <= f32::EPSILON {
            return 0.0;
        }
        if (object.desc.end_time - self.desc.length).abs() <= f32::EPSILON {
            return seconds * object.desc.model_part_fps;
        }

        let loops = (seconds / animation_length).floor();
        let local_seconds = seconds - animation_length * loops;
        let mut frame = local_seconds * object.desc.model_part_fps;
        let max_frame = object.desc.total_frames as f32;
        if frame >= max_frame + 1.0 {
            frame = max_frame;
        }
        frame
    }

    pub fn evaluate_model_part_transform_at_frame(
        &self,
        object_index: usize,
        frame: f32,
    ) -> [f32; 16] {
        let mut result = identity_matrix();
        let mut current_index = object_index;
        for _ in 0..100 {
            let object = &self.animated_objects[current_index];
            if current_index == object.desc.parent_index {
                break;
            }

            let mut local = object.evaluate_local_transform_at_frame_percent(frame, false);
            let pivot = object.pivot();
            local[12] += local[0] * pivot[0] + local[4] * pivot[1] + local[8] * pivot[2];
            local[13] += local[1] * pivot[0] + local[5] * pivot[1] + local[9] * pivot[2];
            local[14] += local[2] * pivot[0] + local[6] * pivot[1] + local[10] * pivot[2];
            result = mul_matrix4(&local, &result);
            if current_index == object.desc.parent_index {
                break;
            }
            current_index = object.desc.parent_index;
        }
        result
    }

    pub fn get_model_part_transform(&self, object_index: usize, seconds: f32) -> [f32; 16] {
        let object = &self.animated_objects[object_index];
        let blend = (seconds * object.desc.model_part_fps).fract();
        let frame0 = self
            .get_object_animation_frame_percent(object_index, seconds)
            .floor();
        let frame1 = frame0 + 1.0;
        let mat0 = self.evaluate_model_part_transform_at_frame(object_index, frame0);
        let mat1 = self.evaluate_model_part_transform_at_frame(object_index, frame1);
        lerp_matrix4(&mat0, &mat1, blend)
    }

    pub fn get_animated_local_transform(&self, object_index: usize, seconds: f32) -> [f32; 16] {
        let object = &self.animated_objects[object_index];
        let parent = &self.animated_objects[object.desc.parent_index];
        let parent_has_model_part = parent.desc.model_part_index != object.desc.parent_index;

        let model_part = self.get_model_part_transform(object.desc.model_part_index, seconds);
        if parent_has_model_part {
            let parent_model_part =
                self.get_model_part_transform(parent.desc.model_part_index, seconds);
            if let Some(parent_inverse) = invert_matrix4(&parent_model_part) {
                let mut local = mul_matrix4(&parent_inverse, &model_part);
                local[12] *= object.desc.model_part_scale;
                local[13] *= object.desc.model_part_scale;
                local[14] *= object.desc.model_part_scale;
                local
            } else {
                model_part
            }
        } else {
            model_part
        }
    }

    pub fn get_world_transform(
        &self,
        object_index: usize,
        seconds: f32,
        scene_scale: f32,
    ) -> [f32; 16] {
        let mut object = &self.animated_objects[object_index];
        let frame_percent = self.get_object_animation_frame_percent(object_index, seconds);
        let mut world = object.evaluate_local_transform_at_frame_percent(frame_percent, true);

        if object.desc.model_part_index != object_index {
            let animated_local = self.get_animated_local_transform(object_index, seconds);
            world = mul_matrix4(&animated_local, &world);
        }

        if object.desc.parent_index != object_index {
            let mut current_parent = object.desc.parent_index;
            for _ in 0..100 {
                object = &self.animated_objects[current_parent];
                let parent_frame = self.get_object_animation_frame_percent(current_parent, seconds);
                let local = object.evaluate_local_transform_at_frame_percent(parent_frame, true);
                if object.desc.model_part_index != current_parent {
                    let animated_local =
                        self.get_animated_local_transform(current_parent, seconds);
                    world = mul_matrix4(&animated_local, &mul_matrix4(&local, &world));
                } else {
                    world = mul_matrix4(&local, &world);
                }
                if current_parent == object.desc.parent_index {
                    break;
                }
                current_parent = object.desc.parent_index;
            }
        }

        world[12] *= scene_scale;
        world[13] *= scene_scale;
        world[14] *= scene_scale;
        world
    }

    pub fn is_visible_at_frame_percent(&self, object_index: usize, frame_percent: f32) -> bool {
        let mut current_index = object_index;
        for _ in 0..100 {
            let object = &self.animated_objects[current_index];
            if let Some(value) = object.sample_named_property("Visible", frame_percent, 1.0) {
                if value == 0.0 {
                    return false;
                }
            }
            if current_index == object.desc.parent_index {
                break;
            }
            current_index = object.desc.parent_index;
        }
        true
    }
}

impl ParsedAnimatedObject {
    pub fn sample_trs(&self, frame: f32) -> SampledTransform {
        let translation = [
            self.sample_property("Translation X", frame, 0.0),
            self.sample_property("Translation Y", frame, 0.0),
            self.sample_property("Translation Z", frame, 0.0),
        ];
        let rotation = [
            self.sample_property("Rotation X", frame, 0.0),
            self.sample_property("Rotation Y", frame, 0.0),
            self.sample_property("Rotation Z", frame, 0.0),
        ];
        let scale = [
            self.sample_property("Scale X", frame, 1.0),
            self.sample_property("Scale Y", frame, 1.0),
            self.sample_property("Scale Z", frame, 1.0),
        ];
        SampledTransform {
            translation,
            rotation_deg: rotation,
            scale,
        }
    }

    fn sample_property(&self, property_name: &str, frame: f32, default: f32) -> f32 {
        self.sample_named_property(property_name, frame, default)
            .unwrap_or(default)
    }

    pub fn sample_named_property(
        &self,
        property_name: &str,
        frame: f32,
        default: f32,
    ) -> Option<f32> {
        let Some(keyframes) = &self.keyframes else {
            return None;
        };
        let Some(track) = keyframes.properties.iter().find(|track| track.name == property_name) else {
            return None;
        };
        Some(track.sample(frame, default))
    }

    pub fn animation_length(&self) -> f32 {
        (self.desc.end_time - self.desc.start_time).max(0.0)
    }

    pub fn pivot(&self) -> [f32; 3] {
        [
            self.desc.pivotx.unwrap_or(0.0),
            self.desc.pivoty.unwrap_or(0.0),
            self.desc.pivotz.unwrap_or(0.0),
        ]
    }

    pub fn evaluate_local_transform_at_frame_percent(
        &self,
        frame_percent: f32,
        scene_space_order: bool,
    ) -> [f32; 16] {
        let sampled = self.sample_trs(frame_percent);
        let mut matrix = evaluate_trs_matrix(sampled, scene_space_order);
        let pivot = self.pivot();
        matrix[12] -= matrix[0] * pivot[0] + matrix[4] * pivot[1] + matrix[8] * pivot[2];
        matrix[13] -= matrix[1] * pivot[0] + matrix[5] * pivot[1] + matrix[9] * pivot[2];
        matrix[14] -= matrix[2] * pivot[0] + matrix[6] * pivot[1] + matrix[10] * pivot[2];
        matrix
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SampledTransform {
    pub translation: [f32; 3],
    pub rotation_deg: [f32; 3],
    pub scale: [f32; 3],
}

impl KeyframePropertyTrack {
    pub fn sample(&self, frame: f32, default: f32) -> f32 {
        match &self.data {
            KeyframeTrackData::Packed0(frames) => sample_packed0(frames, frame).unwrap_or(default),
            KeyframeTrackData::Packed1(frames) => sample_packed1(frames, frame).unwrap_or(default),
            KeyframeTrackData::Packed2(values) => sample_packed2(values, frame).unwrap_or(default),
        }
    }

    pub fn keyframe_times(&self) -> Vec<f32> {
        match &self.data {
            KeyframeTrackData::Packed0(frames) => frames
                .iter()
                .map(|frame| frame.frame_index as f32)
                .collect(),
            KeyframeTrackData::Packed1(frames) => frames
                .iter()
                .map(|frame| frame.frame_index as f32)
                .collect(),
            KeyframeTrackData::Packed2(values) => (0..values.len()).map(|i| i as f32).collect(),
        }
    }
}

fn parse_matrix_table(archive: &Archive, expected_count: usize) -> Result<MatrixTable> {
    let entry = archive
        .get("MatTable.bin")
        .context("AnimData references MatTable.bin but it is missing")?;
    let mut cursor = Cursor::new(entry.data.as_slice());
    let float_count = entry.data.len() / 4;
    if float_count % 16 != 0 {
        bail!("MatTable.bin length is not aligned to 16 floats");
    }

    let matrix_count = float_count / 16;
    let mut matrices = Vec::with_capacity(matrix_count);
    for _ in 0..matrix_count {
        let mut matrix = [0.0f32; 16];
        for component in &mut matrix {
            *component = cursor.read_f32::<LittleEndian>()?;
        }
        matrices.push(matrix);
    }

    if expected_count != 0 && expected_count != matrices.len() {
        bail!(
            "matrix table count mismatch: scene says {}, binary has {}",
            expected_count,
            matrices.len()
        );
    }

    Ok(MatrixTable { matrices })
}

fn sample_packed0(frames: &[Packed0Keyframe], frame: f32) -> Option<f32> {
    if frames.is_empty() {
        return None;
    }
    if frames.len() == 1 {
        return Some(frames[0].value);
    }
    if frame <= frames[0].frame_index as f32 {
        return Some(frames[0].value);
    }
    for window in frames.windows(2) {
        let start = &window[0];
        let end = &window[1];
        let start_frame = start.frame_index as f32;
        let end_frame = end.frame_index as f32;
        if frame <= end_frame {
            if end_frame <= start_frame {
                return Some(end.value);
            }
            return Some(sample_curve_segment_packed0(frames, window[0].frame_index, frame));
        }
    }
    frames.last().map(|frame| frame.value)
}

fn sample_packed1(frames: &[Packed1Keyframe], frame: f32) -> Option<f32> {
    if frames.is_empty() {
        return None;
    }
    if frames.len() == 1 {
        return Some(frames[0].value);
    }
    if frame <= frames[0].frame_index as f32 {
        return Some(frames[0].value);
    }
    for window in frames.windows(2) {
        let start = &window[0];
        let end = &window[1];
        let start_frame = start.frame_index as f32;
        let end_frame = end.frame_index as f32;
        if frame <= end_frame {
            if end_frame <= start_frame {
                return Some(end.value);
            }
            let interpolation = start.interpolation;
            if interpolation == 2 {
                return Some(if frame >= end_frame { end.value } else { start.value });
            }
            if interpolation == 0 {
                let t = ((frame - start_frame) / (end_frame - start_frame)).clamp(0.0, 1.0);
                return Some(start.value + (end.value - start.value) * t);
            }
            return Some(sample_curve_segment(
                frame,
                CurvePoint {
                    frame: start.frame_index as f32,
                    value: start.value,
                    weigh_in: 1.0,
                    weigh_out: 1.0,
                    interpolation: interpolation as u8,
                },
                CurvePoint {
                    frame: end.frame_index as f32,
                    value: end.value,
                    weigh_in: 1.0,
                    weigh_out: 1.0,
                    interpolation: end.interpolation as u8,
                },
                prev_curve_point_packed1(frames, window[0].frame_index),
                next_curve_point_packed1(frames, window[0].frame_index + 1),
            ));
        }
    }
    frames.last().map(|frame| frame.value)
}

fn sample_packed2(values: &[f32], frame: f32) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let lower = frame.floor().max(0.0) as usize;
    let upper = frame.ceil().max(0.0) as usize;
    if lower >= values.len() {
        return values.last().copied();
    }
    if lower == upper || upper >= values.len() {
        return values.get(lower).copied();
    }
    let t = (frame - lower as f32).clamp(0.0, 1.0);
    Some(values[lower] + (values[upper] - values[lower]) * t)
}

fn sample_curve_segment_packed0(
    frames: &[Packed0Keyframe],
    start_index: u16,
    frame: f32,
) -> f32 {
    let start_i = frames
        .iter()
        .position(|key| key.frame_index == start_index)
        .unwrap_or(0);
    let end_i = (start_i + 1).min(frames.len() - 1);
    let start = &frames[start_i];
    let end = &frames[end_i];
    let interpolation = start.interpolation;
    if interpolation == 2 {
        return if frame >= end.frame_index as f32 {
            end.value
        } else {
            start.value
        };
    }
    if interpolation == 0 {
        let t = ((frame - start.frame_index as f32)
            / (end.frame_index as f32 - start.frame_index as f32))
            .clamp(0.0, 1.0);
        return start.value + (end.value - start.value) * t;
    }
    sample_curve_segment(
        frame,
        CurvePoint {
            frame: start.frame_index as f32,
            value: start.value,
            weigh_in: start.weigh_in,
            weigh_out: start.weigh_out,
            interpolation: start.interpolation as u8,
        },
        CurvePoint {
            frame: end.frame_index as f32,
            value: end.value,
            weigh_in: end.weigh_in,
            weigh_out: end.weigh_out,
            interpolation: end.interpolation as u8,
        },
        prev_curve_point_packed0(frames, start_i),
        next_curve_point_packed0(frames, end_i),
    )
}

#[derive(Clone, Copy)]
struct CurvePoint {
    frame: f32,
    value: f32,
    weigh_in: f32,
    weigh_out: f32,
    interpolation: u8,
}

fn prev_curve_point_packed0(frames: &[Packed0Keyframe], start_i: usize) -> Option<CurvePoint> {
    start_i.checked_sub(1).and_then(|index| {
        frames.get(index).map(|frame| CurvePoint {
            frame: frame.frame_index as f32,
            value: frame.value,
            weigh_in: frame.weigh_in,
            weigh_out: frame.weigh_out,
            interpolation: frame.interpolation as u8,
        })
    })
}

fn next_curve_point_packed0(frames: &[Packed0Keyframe], end_i: usize) -> Option<CurvePoint> {
    frames.get(end_i + 1).map(|frame| CurvePoint {
        frame: frame.frame_index as f32,
        value: frame.value,
        weigh_in: frame.weigh_in,
        weigh_out: frame.weigh_out,
        interpolation: frame.interpolation as u8,
    })
}

fn prev_curve_point_packed1(frames: &[Packed1Keyframe], start_i: u16) -> Option<CurvePoint> {
    let index = frames
        .iter()
        .position(|frame| frame.frame_index == start_i)?
        .checked_sub(1)?;
    let frame = frames.get(index)?;
    Some(CurvePoint {
        frame: frame.frame_index as f32,
        value: frame.value,
        weigh_in: 1.0,
        weigh_out: 1.0,
        interpolation: frame.interpolation as u8,
    })
}

fn next_curve_point_packed1(frames: &[Packed1Keyframe], end_i: u16) -> Option<CurvePoint> {
    let index = frames.iter().position(|frame| frame.frame_index == end_i)?;
    let frame = frames.get(index + 1)?;
    Some(CurvePoint {
        frame: frame.frame_index as f32,
        value: frame.value,
        weigh_in: 1.0,
        weigh_out: 1.0,
        interpolation: frame.interpolation as u8,
    })
}

fn sample_curve_segment(
    frame: f32,
    start: CurvePoint,
    end: CurvePoint,
    prev: Option<CurvePoint>,
    next: Option<CurvePoint>,
) -> f32 {
    let mut kf0 = prev.unwrap_or(CurvePoint {
        frame: start.frame,
        value: start.value,
        weigh_in: 1.0,
        weigh_out: 1.0,
        interpolation: start.interpolation,
    });
    let mut kf1 = start;
    let mut kf2 = end;
    let mut kf3 = next.unwrap_or(CurvePoint {
        frame: end.frame,
        value: end.value,
        weigh_in: 1.0,
        weigh_out: 1.0,
        interpolation: end.interpolation,
    });

    if prev.is_none() || next.is_none() {
        kf0 = CurvePoint { frame: kf1.frame, value: kf1.value, ..kf0 };
        kf3 = CurvePoint { frame: kf2.frame, value: kf2.value, ..kf3 };
    }

    if prev.is_none() {
        kf1.frame += 1.0;
        kf2.frame += 1.0;
        kf3.frame += 1.0;
    }
    if next.is_none() {
        kf0.frame += 1.0;
        kf1.frame += 1.0;
        kf2.frame += 1.0;
    }

    evaluate_curve(frame, kf0, kf1, kf2, kf3)
}

fn evaluate_curve(frame: f32, kf0: CurvePoint, kf1: CurvePoint, kf2: CurvePoint, kf3: CurvePoint) -> f32 {
    let mut g = kf1.frame - (kf2.frame - kf0.frame);
    let mut h = kf2.frame - (kf1.frame - kf3.frame);
    let mut k = kf1.value - (kf2.value - kf0.value) * kf1.weigh_out;
    let mut n = kf2.value - (kf1.value - kf3.value) * kf2.weigh_in;
    if kf1.interpolation == 3 {
        g = kf1.frame - (kf2.frame - kf1.frame);
        k = kf1.value - kf1.weigh_out;
    }
    if kf2.interpolation == 3 {
        h = kf2.frame - (kf1.frame - kf2.frame);
        n = kf2.value + kf2.weigh_in;
    }
    let g = (frame - g) / (kf1.frame - g);
    let b = (frame - kf1.frame) / (kf2.frame - kf1.frame);
    let d = (frame - kf2.frame) / (h - kf2.frame);
    let hmid = kf1.value * (1.0 - b) + kf2.value * b;
    ((k * (1.0 - g) + kf1.value * g) * (1.0 - b) + hmid * b) * (1.0 - b)
        + ((kf2.value * (1.0 - d) + n * d) * b + hmid * (1.0 - b)) * b
}

fn evaluate_trs_matrix(sampled: SampledTransform, scene_space_order: bool) -> [f32; 16] {
    let rx = rotation_matrix(sampled.rotation_deg[0].to_radians(), 0);
    let ry = rotation_matrix(sampled.rotation_deg[1].to_radians(), 1);
    let rz = rotation_matrix(sampled.rotation_deg[2].to_radians(), 2);
    let mut matrix = if scene_space_order {
        mul_matrix4(&ry, &mul_matrix4(&rx, &rz))
    } else {
        mul_matrix4(&rz, &mul_matrix4(&ry, &rx))
    };

    matrix[12] = sampled.translation[0];
    matrix[13] = sampled.translation[1];
    matrix[14] = sampled.translation[2];

    matrix[0] *= sampled.scale[0];
    matrix[1] *= sampled.scale[0];
    matrix[2] *= sampled.scale[0];
    matrix[3] *= sampled.scale[0];
    matrix[4] *= sampled.scale[1];
    matrix[5] *= sampled.scale[1];
    matrix[6] *= sampled.scale[1];
    matrix[7] *= sampled.scale[1];
    matrix[8] *= sampled.scale[2];
    matrix[9] *= sampled.scale[2];
    matrix[10] *= sampled.scale[2];
    matrix[11] *= sampled.scale[2];
    matrix
}

fn rotation_matrix(angle: f32, axis: usize) -> [f32; 16] {
    let (s, c) = angle.sin_cos();
    match axis {
        0 => [1.0, 0.0, 0.0, 0.0, 0.0, c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0],
        1 => [c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0],
        _ => [c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    }
}

pub(crate) fn identity_matrix() -> [f32; 16] {
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}

pub(crate) fn mul_matrix4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
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

pub(crate) fn lerp_matrix4(a: &[f32; 16], b: &[f32; 16], t: f32) -> [f32; 16] {
    let mut result = [0.0; 16];
    for i in 0..16 {
        result[i] = a[i] * (1.0 - t) + b[i] * t;
    }
    result
}

fn invert_matrix4(matrix: &[f32; 16]) -> Option<[f32; 16]> {
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
    for value in &mut inv {
        *value *= inv_det;
    }
    Some(inv)
}

fn parse_skinning_rig(
    archive: &Archive,
    desc: &SkinningRigDesc,
    matrix_table: &MatrixTable,
) -> Result<SkinningRig> {
    let entry = archive
        .get(&desc.file)
        .with_context(|| format!("missing skinning rig {}", desc.file))?;
    let bytes = entry.data.as_slice();
    if bytes.len() < 24 {
        bail!("skinning rig {} is too short", desc.file);
    }

    let header_words = bytes.len() / 4;
    let mut words = Vec::with_capacity(header_words);
    let mut cursor = Cursor::new(bytes);
    for _ in 0..header_words {
        words.push(cursor.read_u32::<LittleEndian>()?);
    }

    let expected_num_clusters = words[0] as usize;
    let expected_num_vertices = words[1] as usize;
    let num_cluster_links = words[2] as usize;
    let original_object_index = words[3] as usize;
    let is_rigid_skin = words[4] != 0;
    let tangent_method = words[5];

    let expected_header_words = 6 + 7 * expected_num_clusters;
    if words.len() < expected_header_words {
        bail!("skinning rig {} truncated cluster header", desc.file);
    }

    let mut clusters = Vec::with_capacity(expected_num_clusters);
    for cluster_index in 0..expected_num_clusters {
        let base = 6 + 7 * cluster_index;
        let link_mode = words[base + 1];
        let link_object_index = words[base + 2];
        let associate_object_index = words[base + 3];
        let default_cluster_world_transform_index = words[base + 4];
        let default_cluster_base_transform_index = words[base + 5];
        let default_associate_world_transform_index = words[base + 6];

        let world = matrix_by_index(matrix_table, default_cluster_world_transform_index)
            .with_context(|| format!("invalid world matrix index in {}", desc.file))?;
        let base_transform = matrix_by_index(matrix_table, default_cluster_base_transform_index)
            .with_context(|| format!("invalid base matrix index in {}", desc.file))?;
        let associate = if link_mode == 1 {
            Some(
                matrix_by_index(matrix_table, default_associate_world_transform_index)
                    .with_context(|| format!("invalid associate matrix index in {}", desc.file))?,
            )
        } else {
            None
        };

        clusters.push(SkinningCluster {
            link_mode,
            link_object_index,
            associate_object_index,
            default_cluster_world_transform_index,
            default_cluster_base_transform_index,
            default_associate_world_transform_index,
            default_cluster_world_transform: world,
            default_cluster_base_transform: base_transform,
            default_associate_world_transform: associate,
        });
    }

    let header_bytes = expected_header_words * 4;
    let link_count_bytes = expected_num_vertices;
    let cluster_index_bytes = num_cluster_links * 2;
    let weight_bytes = num_cluster_links * 4;
    let expected_total = header_bytes + link_count_bytes + cluster_index_bytes + weight_bytes;
    if bytes.len() < expected_total {
        bail!("skinning rig {} truncated link map", desc.file);
    }

    let mut link_map_count = vec![0u8; expected_num_vertices];
    link_map_count.copy_from_slice(&bytes[header_bytes..header_bytes + link_count_bytes]);

    let cluster_indices_offset = header_bytes + link_count_bytes;
    let mut link_map_cluster_indices = Vec::with_capacity(num_cluster_links);
    let mut cur =
        Cursor::new(&bytes[cluster_indices_offset..cluster_indices_offset + cluster_index_bytes]);
    for _ in 0..num_cluster_links {
        link_map_cluster_indices.push(cur.read_u16::<LittleEndian>()?);
    }

    let weights_offset = cluster_indices_offset + cluster_index_bytes;
    let mut link_map_weights = Vec::with_capacity(num_cluster_links);
    let mut cur = Cursor::new(&bytes[weights_offset..weights_offset + weight_bytes]);
    for _ in 0..num_cluster_links {
        link_map_weights.push(cur.read_f32::<LittleEndian>()?);
    }

    Ok(SkinningRig {
        source_file: desc.file.clone(),
        expected_num_clusters,
        expected_num_vertices,
        num_cluster_links,
        original_object_index,
        is_rigid_skin,
        tangent_method,
        clusters,
        link_map_count,
        link_map_cluster_indices,
        link_map_weights,
    })
}

fn parse_animation(archive: &Archive, animation: &AnimationDesc) -> Result<ParsedAnimation> {
    let mut animated_objects = Vec::with_capacity(animation.animated_objects.len());
    for object in &animation.animated_objects {
        let keyframes = if object.file.is_empty() {
            None
        } else {
            let entry = archive
                .get(&object.file)
                .with_context(|| format!("missing keyframe file {}", object.file))?;
            Some(parse_keyframe_file(
                &entry.data,
                &object.animated_properties,
            )?)
        };
        animated_objects.push(ParsedAnimatedObject {
            desc: object.clone(),
            keyframes,
        });
    }
    Ok(ParsedAnimation {
        desc: animation.clone(),
        animated_objects,
    })
}

fn parse_keyframe_file(bytes: &[u8], properties: &[AnimatedPropertyDesc]) -> Result<KeyframeFile> {
    if bytes.len() < 4 {
        bail!("keyframe file too short");
    }

    let float_count = bytes.len() / 4;
    let u16_count = bytes.len() / 2;
    let mut float_cursor = Cursor::new(bytes);
    let mut floats = Vec::with_capacity(float_count);
    for _ in 0..float_count {
        floats.push(float_cursor.read_f32::<LittleEndian>()?);
    }

    let mut u32_cursor = Cursor::new(bytes);
    let mut u32s = Vec::with_capacity(float_count);
    for _ in 0..float_count {
        u32s.push(u32_cursor.read_u32::<LittleEndian>()?);
    }

    let mut u16_cursor = Cursor::new(bytes);
    let mut u16s = Vec::with_capacity(u16_count);
    for _ in 0..u16_count {
        u16s.push(u16_cursor.read_u16::<LittleEndian>()?);
    }

    let header_words = u32s[0];
    let mut next_float_index = 1usize
        .checked_add(header_words as usize)
        .context("keyframe header index overflow")?;

    let mut parsed_properties = Vec::with_capacity(properties.len());
    for (property_index, property) in properties.iter().enumerate() {
        let u16_base = 2 + 2 * property_index;
        if u16_base + 1 >= u16s.len() {
            bail!("keyframe property header out of bounds");
        }
        let byte_base = 2 * u16_base;
        if byte_base + 3 >= bytes.len() {
            bail!("keyframe property byte header out of bounds");
        }

        let num_keyframes = u16s[u16_base] as usize;
        let packing_type = bytes[byte_base + 2];
        let interpolation_type = bytes[byte_base + 3];

        let data = match packing_type {
            0 => {
                let mut frames = Vec::with_capacity(num_keyframes);
                for key_index in 0..num_keyframes {
                    let float_index = next_float_index + key_index * 4;
                    let u16_index = float_index * 2;
                    frames.push(Packed0Keyframe {
                        value: floats[float_index],
                        weigh_in: floats[float_index + 1],
                        weigh_out: floats[float_index + 2],
                        frame_index: u16s[u16_index + 6],
                        interpolation: u16s[u16_index + 7],
                    });
                }
                next_float_index += num_keyframes * 4;
                KeyframeTrackData::Packed0(frames)
            }
            1 => {
                let mut frames = Vec::with_capacity(num_keyframes);
                for key_index in 0..num_keyframes {
                    let float_index = next_float_index + key_index * 2;
                    let u16_index = float_index * 2;
                    frames.push(Packed1Keyframe {
                        value: floats[float_index],
                        frame_index: u16s[u16_index + 2],
                        interpolation: u16s[u16_index + 3],
                    });
                }
                next_float_index += num_keyframes * 2;
                KeyframeTrackData::Packed1(frames)
            }
            2 => {
                let end = next_float_index + num_keyframes;
                let values = floats[next_float_index..end].to_vec();
                next_float_index = end;
                KeyframeTrackData::Packed2(values)
            }
            other => bail!("unsupported keyframe packing type {}", other),
        };

        parsed_properties.push(KeyframePropertyTrack {
            name: property.name.clone(),
            packing_type,
            interpolation_type,
            num_keyframes,
            data,
        });
    }

    Ok(KeyframeFile {
        shared_header_words: header_words,
        properties: parsed_properties,
    })
}

fn matrix_by_index(matrix_table: &MatrixTable, index: u32) -> Result<[f32; 16]> {
    matrix_table
        .matrices
        .get(index as usize)
        .copied()
        .context("matrix index out of range")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::ParsedAnimationSet;
    use crate::archive::Archive;
    use crate::scene::Scene;

    #[test]
    fn parses_vivfox_animation_data() {
        let parsed = parse_fixture("test_data/vivfox.mview");
        assert_eq!(parsed.num_matrices, 72);
        assert_eq!(parsed.skinning_rigs.len(), 2);
        assert_eq!(parsed.animations.len(), 7);
    }

    #[test]
    fn parses_natnephilim_animation_data() {
        let parsed = parse_fixture("test_data/natnephilim.mview");
        assert_eq!(parsed.num_matrices, 380);
        assert_eq!(parsed.skinning_rigs.len(), 4);
        assert_eq!(parsed.animations.len(), 1);
    }

    fn parse_fixture(relative_path: &str) -> ParsedAnimationSet {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
        let bytes = fs::read(&path).expect("failed to read fixture");
        let archive = Archive::from_bytes(&bytes).expect("failed to parse archive");
        let scene_entry = archive
            .get("scene.json")
            .expect("scene.json missing from fixture");
        let scene = Scene::from_bytes(&scene_entry.data).expect("failed to parse scene");
        ParsedAnimationSet::from_scene(&archive, &scene)
            .expect("failed to parse animation data")
            .expect("fixture should contain animation data")
    }
}
