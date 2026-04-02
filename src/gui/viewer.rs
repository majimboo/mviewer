use std::sync::Mutex;

use bytemuck::{Pod, Zeroable};
use eframe::{egui, egui_wgpu, wgpu};
use image::imageops::FilterType;
use wgpu::util::DeviceExt as _;

use crate::animation::identity_matrix;
use crate::ProjectDocument;

const MAX_VIEWER_LIGHTS: usize = 4;

pub struct RuntimeViewer {
    render_state: Option<egui_wgpu::RenderState>,
    project_key: Option<String>,
    yaw_offset: f32,
    pitch_offset: f32,
    distance_offset: f32,
    fov_y_radians: f32,
    show_wireframe: bool,
    smooth_shading: bool,
    atlas_cache: Option<SceneTextureAtlas>,
}

impl RuntimeViewer {
    pub fn new(render_state: Option<egui_wgpu::RenderState>) -> Self {
        Self {
            render_state,
            project_key: None,
            yaw_offset: 0.0,
            pitch_offset: 0.0,
            distance_offset: 0.0,
            fov_y_radians: 45.0f32.to_radians(),
            show_wireframe: false,
            smooth_shading: true,
            atlas_cache: None,
        }
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        project: &ProjectDocument,
        visible_meshes: &[bool],
        selected_clip: Option<usize>,
        time_seconds: f32,
    ) {
        self.sync_project_defaults(project);
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.heading("Runtime Viewer");
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Meshes: {}", project.runtime.meshes.len()));
                    ui.separator();
                    ui.label(format!("Materials: {}", project.runtime.materials.len()));
                    ui.separator();
                    ui.label(format!("Textures: {}", project.runtime.texture_usage.len()));
                    ui.separator();
                    ui.checkbox(&mut self.smooth_shading, "Smooth");
                    ui.checkbox(&mut self.show_wireframe, "Wire");
                    if ui.button("Reset View").clicked() {
                        self.reset_camera_offsets();
                    }
                });

                ui.add_space(8.0);
                let available = ui.available_size_before_wrap();
                let max_width_from_height = (available.y.max(180.0) * (16.0 / 9.0)).max(320.0);
                let viewport_width = available.x.min(max_width_from_height).max(320.0);
                let viewport_height = (viewport_width * 9.0 / 16.0).max(180.0);

                ui.horizontal(|ui| {
                    let side_space = ((ui.available_width() - viewport_width) * 0.5).max(0.0);
                    if side_space > 0.0 {
                        ui.add_space(side_space);
                    }

                    let desired = egui::vec2(viewport_width, viewport_height);
                    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::drag());
                    if response.dragged() {
                        let delta = response.drag_delta();
                        self.yaw_offset += delta.x * std::f32::consts::TAU / rect.width().max(1.0);
                        self.pitch_offset = (self.pitch_offset
                            - delta.y * std::f32::consts::PI / rect.height().max(1.0))
                            .clamp(-1.35, 1.35);
                    }
                    if response.hovered() {
                        let scroll = ui.input(|input| input.raw_scroll_delta.y);
                        if scroll.abs() > f32::EPSILON {
                            self.distance_offset += -scroll * 0.01;
                        }
                    }

                    ui.painter().rect_filled(rect, 12.0, background_color(project));
                    draw_grid(ui.painter(), rect);

                    let yaw_offset = self.yaw_offset;
                    let pitch_offset = self.pitch_offset;
                    let distance_offset = self.distance_offset;
                    let fov_y_radians = self.fov_y_radians;
                    let smooth_shading = self.smooth_shading;
                    let atlas = self.ensure_texture_atlas(project).clone();
                    let scene = build_scene_mesh(
                        project,
                        &atlas,
                        visible_meshes,
                        rect,
                        yaw_offset,
                        pitch_offset,
                        distance_offset,
                        fov_y_radians,
                        smooth_shading,
                        selected_clip,
                        time_seconds,
                    );
                    if let Some(render_state) = &self.render_state {
                        if !scene.indices.is_empty() {
                            let callback = egui_wgpu::Callback::new_paint_callback(
                                rect,
                                SceneCallback::new(scene.clone(), render_state.target_format),
                            );
                            ui.painter().add(callback);
                        }
                    } else {
                        draw_scene_fallback(ui.painter(), &scene);
                    }

                    if self.show_wireframe {
                        draw_wireframe(ui.painter(), &scene);
                    }
                });
            });
    }
}

#[derive(Clone)]
struct SceneGpuMesh {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    wire_segments: Vec<[egui::Pos2; 2]>,
    atlas: TextureAtlasData,
    lights: Vec<LightPreview>,
    environment: SceneEnvironment,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 2],
    uv0: [f32; 2],
    color: [f32; 4],
    view_pos: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    material: [f32; 4],
    uv1: [f32; 2],
    tile_rect: [f32; 4],
    ao_range: [f32; 4],
    emissive_range: [f32; 4],
    flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniforms {
    light_dirs: [[f32; 4]; MAX_VIEWER_LIGHTS],
    light_colors: [[f32; 4]; MAX_VIEWER_LIGHTS],
    light_count: [f32; 4],
    ambient_color: [f32; 4],
    fog_color: [f32; 4],
    fog_params: [f32; 4],
    post_brightness: [f32; 4],
    post_contrast: [f32; 4],
    post_saturation: [f32; 4],
    post_vignette: [f32; 4],
    post_misc: [f32; 4],
}

#[derive(Clone, Copy)]
struct LightPreview {
    direction: [f32; 3],
    color: [f32; 3],
}

#[derive(Clone, Copy)]
struct SceneEnvironment {
    ambient_color: [f32; 3],
    ambient_strength: f32,
    fog_color: [f32; 3],
    fog_opacity: f32,
    fog_inv_distance: f32,
    fog_dispersion: f32,
    fog_type: f32,
    post_brightness: [f32; 4],
    post_contrast: [f32; 4],
    post_saturation: [f32; 4],
    post_vignette: [f32; 4],
    post_misc: [f32; 4],
}

fn build_scene_uniforms(environment: SceneEnvironment, lights: &[LightPreview]) -> SceneUniforms {
    let mut uniforms = SceneUniforms {
        light_dirs: [[0.0, 0.0, -1.0, 0.0]; MAX_VIEWER_LIGHTS],
        light_colors: [[0.0, 0.0, 0.0, 0.0]; MAX_VIEWER_LIGHTS],
        light_count: [lights.len().min(MAX_VIEWER_LIGHTS) as f32, 0.0, 0.0, 0.0],
        ambient_color: [0.12, 0.12, 0.12, 0.12],
        fog_color: [0.0, 0.0, 0.0, 0.0],
        fog_params: [0.0, 0.0, 0.0, 0.0],
        post_brightness: [0.0, 0.0, 0.0, 0.0],
        post_contrast: [1.0, 1.0, 1.0, 1.0],
        post_saturation: [1.0, 1.0, 1.0, 1.0],
        post_vignette: [0.0, 0.0, 0.0, 0.0],
        post_misc: [0.0, 0.0, 0.0, 0.0],
    };
    for (index, light) in lights.iter().take(MAX_VIEWER_LIGHTS).enumerate() {
        uniforms.light_dirs[index] = [light.direction[0], light.direction[1], light.direction[2], 0.0];
        uniforms.light_colors[index] = [light.color[0], light.color[1], light.color[2], 0.0];
    }
    uniforms.fog_color = [
        environment.fog_color[0],
        environment.fog_color[1],
        environment.fog_color[2],
        environment.fog_opacity,
    ];
    uniforms.ambient_color = [
        environment.ambient_color[0],
        environment.ambient_color[1],
        environment.ambient_color[2],
        environment.ambient_strength,
    ];
    uniforms.fog_params = [
        environment.fog_inv_distance,
        environment.fog_dispersion,
        environment.fog_type,
        0.0,
    ];
    uniforms.post_brightness = environment.post_brightness;
    uniforms.post_contrast = environment.post_contrast;
    uniforms.post_saturation = environment.post_saturation;
    uniforms.post_vignette = environment.post_vignette;
    uniforms.post_misc = environment.post_misc;
    uniforms
}

struct SceneGpuBuffers {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct ScenePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

struct SceneCallback {
    mesh: SceneGpuMesh,
    target_format: wgpu::TextureFormat,
    buffers: Mutex<Option<SceneGpuBuffers>>,
}

impl SceneCallback {
    fn new(mesh: SceneGpuMesh, target_format: wgpu::TextureFormat) -> Self {
        Self {
            mesh,
            target_format,
            buffers: Mutex::new(None),
        }
    }
}

impl egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        callback_resources
            .entry::<ScenePipeline>()
            .or_insert_with(|| {
                let bind_group_layout = create_bind_group_layout(device);
                ScenePipeline {
                    pipeline: create_pipeline(device, self.target_format, &bind_group_layout),
                    bind_group_layout,
                }
            });
        let resources = callback_resources
            .get::<ScenePipeline>()
            .expect("scene pipeline resource missing");

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mviewer_runtime_scene_vertices"),
            contents: bytemuck::cast_slice(&self.mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mviewer_runtime_scene_indices"),
            contents: bytemuck::cast_slice(&self.mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let size = wgpu::Extent3d {
            width: self.mesh.atlas.width,
            height: self.mesh.atlas.height,
            depth_or_array_layers: 1,
        };
        let create_texture = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let albedo_texture = create_texture("mviewer_runtime_scene_albedo_atlas");
        let normal_texture = create_texture("mviewer_runtime_scene_normal_atlas");
        let reflectivity_texture = create_texture("mviewer_runtime_scene_reflectivity_atlas");
        let extras_texture = create_texture("mviewer_runtime_scene_extras_atlas");
        let upload = |texture: &wgpu::Texture, bytes: &[u8]| {
            _queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * self.mesh.atlas.width),
                    rows_per_image: Some(self.mesh.atlas.height),
                },
                size,
            );
        };
        upload(&albedo_texture, &self.mesh.atlas.albedo_rgba);
        upload(&normal_texture, &self.mesh.atlas.normal_rgba);
        upload(&reflectivity_texture, &self.mesh.atlas.reflectivity_rgba);
        upload(&extras_texture, &self.mesh.atlas.extras_rgba);
        let albedo_view = albedo_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let normal_view = normal_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let reflectivity_view =
            reflectivity_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let extras_view = extras_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mviewer_runtime_scene_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mviewer_runtime_scene_uniforms"),
            contents: bytemuck::bytes_of(&build_scene_uniforms(self.mesh.environment, &self.mesh.lights)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mviewer_runtime_scene_bind_group"),
            layout: &resources.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&reflectivity_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&extras_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        *self.buffers.lock().expect("scene viewer mutex poisoned") = Some(SceneGpuBuffers {
            vertex_buffer,
            index_buffer,
            index_count: self.mesh.indices.len() as u32,
            uniform_buffer,
            bind_group,
        });
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<ScenePipeline>() else {
            return;
        };
        let guard = self.buffers.lock().expect("scene viewer mutex poisoned");
        let Some(buffers) = guard.as_ref() else {
            return;
        };
        let clip = info.clip_rect_in_pixels();
        render_pass.set_scissor_rect(
            clip.left_px.max(0) as u32,
            clip.top_px.max(0) as u32,
            clip.width_px.max(0) as u32,
            clip.height_px.max(0) as u32,
        );
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &buffers.bind_group, &[]);
        render_pass.set_vertex_buffer(0, buffers.vertex_buffer.slice(..));
        render_pass.set_index_buffer(buffers.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..buffers.index_count, 0, 0..1);
    }
}

#[derive(Clone)]
struct TextureAtlasData {
    albedo_rgba: Vec<u8>,
    normal_rgba: Vec<u8>,
    reflectivity_rgba: Vec<u8>,
    extras_rgba: Vec<u8>,
    width: u32,
    height: u32,
    material_rects: Vec<[f32; 4]>,
    background_rect: Option<[f32; 4]>,
}

struct SceneTextureAtlas {
    project_key: String,
    data: TextureAtlasData,
}

impl RuntimeViewer {
    fn sync_project_defaults(&mut self, project: &ProjectDocument) {
        let key = project.input_path.display().to_string();
        if self.project_key.as_deref() == Some(&key) {
            return;
        }
        self.project_key = Some(key);
        self.atlas_cache = None;
        self.reset_camera_offsets();
        if let Some(view) = project.scene.main_camera.as_ref().and_then(|camera| camera.view.as_ref()) {
            if let Some(fov) = view.fov {
                self.fov_y_radians = fov.to_radians().clamp(15.0f32.to_radians(), 100.0f32.to_radians());
            }
        }
    }

    fn reset_camera_offsets(&mut self) {
        self.yaw_offset = 0.0;
        self.pitch_offset = 0.0;
        self.distance_offset = 0.0;
    }

    fn ensure_texture_atlas(&mut self, project: &ProjectDocument) -> &TextureAtlasData {
        let key = format!(
            "{}:{}:{}",
            project.input_path.display(),
            project.runtime.materials.len(),
            project.runtime.texture_usage.len()
        );
        let refresh = self
            .atlas_cache
            .as_ref()
            .map(|cached| cached.project_key != key)
            .unwrap_or(true);
        if refresh {
            self.atlas_cache = Some(SceneTextureAtlas {
                project_key: key,
                data: build_texture_atlas(project),
            });
        }
        &self.atlas_cache.as_ref().expect("atlas cache just set").data
    }
}

fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("mviewer_runtime_viewer_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mviewer_runtime_viewer_shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv0: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) view_pos: vec3<f32>,
    @location(3) normal: vec3<f32>,
    @location(4) tangent: vec3<f32>,
    @location(5) bitangent: vec3<f32>,
    @location(6) material: vec4<f32>,
    @location(7) uv1: vec2<f32>,
    @location(8) tile_rect: vec4<f32>,
    @location(9) ao_range: vec4<f32>,
    @location(10) emissive_range: vec4<f32>,
    @location(11) flags: vec4<f32>,
    @location(12) screen_uv: vec2<f32>,
};

struct SceneUniforms {
    light_dirs: array<vec4<f32>, 4>,
    light_colors: array<vec4<f32>, 4>,
    light_count: vec4<f32>,
    ambient_color: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    post_brightness: vec4<f32>,
    post_contrast: vec4<f32>,
    post_saturation: vec4<f32>,
    post_vignette: vec4<f32>,
    post_misc: vec4<f32>,
};

@group(0) @binding(0) var albedo_texture: texture_2d<f32>;
@group(0) @binding(1) var normal_texture: texture_2d<f32>;
@group(0) @binding(2) var reflectivity_texture: texture_2d<f32>;
@group(0) @binding(3) var extras_texture: texture_2d<f32>;
@group(0) @binding(4) var atlas_sampler: sampler;
@group(0) @binding(5) var<uniform> scene_uniforms: SceneUniforms;

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) view_pos: vec3<f32>,
    @location(4) normal: vec3<f32>,
    @location(5) tangent: vec3<f32>,
    @location(6) bitangent: vec3<f32>,
    @location(7) material: vec4<f32>,
    @location(8) uv1: vec2<f32>,
    @location(9) tile_rect: vec4<f32>,
    @location(10) ao_range: vec4<f32>,
    @location(11) emissive_range: vec4<f32>,
    @location(12) flags: vec4<f32>
) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv0 = uv0;
    out.color = color;
    out.view_pos = view_pos;
    out.normal = normal;
    out.tangent = tangent;
    out.bitangent = bitangent;
    out.material = material;
    out.uv1 = uv1;
    out.tile_rect = tile_rect;
    out.ao_range = ao_range;
    out.emissive_range = emissive_range;
    out.flags = flags;
    out.screen_uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return out;
}

fn atlas_uv(uv: vec2<f32>, tile_rect: vec4<f32>) -> vec2<f32> {
    let wrapped = fract(uv);
    return tile_rect.xy + wrapped * tile_rect.zw;
}

fn extras_uv(uv: vec2<f32>, range: vec4<f32>, tile_rect: vec4<f32>) -> vec2<f32> {
    let wrapped = fract(uv);
    let ranged = fract(wrapped) * range.xy + range.zw;
    return tile_rect.xy + ranged * tile_rect.zw;
}

fn safe_normalize3(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let len2 = dot(v, v);
    if (len2 <= 1.0e-8) {
        return fallback;
    }
    return v * inverseSqrt(len2);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo_uv = atlas_uv(in.uv0, in.tile_rect);
    let albedo_texel = textureSample(albedo_texture, atlas_sampler, albedo_uv);
    let normal_texel = textureSample(normal_texture, atlas_sampler, albedo_uv).xyz;
    let reflectivity_texel = textureSample(reflectivity_texture, atlas_sampler, albedo_uv);

    let albedo = albedo_texel.rgb * albedo_texel.rgb;
    let reflectivity = reflectivity_texel.rgb * reflectivity_texel.rgb;
    let gloss = reflectivity_texel.a;
    let use_secondary_ao = in.flags.x > 0.5;
    let use_secondary_emissive = in.flags.y > 0.5;
    let shading_mode = in.flags.z;
    let use_vertex_color = in.flags.w >= 10.0;
    let alpha_test = select(in.flags.w, in.flags.w - 10.0, use_vertex_color);
    let ao_source_uv = select(in.uv0, in.uv1, vec2<bool>(use_secondary_ao, use_secondary_ao));
    let emissive_source_uv = select(in.uv0, in.uv1, vec2<bool>(use_secondary_emissive, use_secondary_emissive));
    let ao_texel = textureSample(extras_texture, atlas_sampler, extras_uv(ao_source_uv, in.ao_range, in.tile_rect));
    let emissive_texel = textureSample(extras_texture, atlas_sampler, extras_uv(emissive_source_uv, in.emissive_range, in.tile_rect));
    let occlusion = ao_texel.r * ao_texel.r;
    let emissive = emissive_texel.rgb * emissive_texel.rgb * in.material.z;
    let mapped = safe_normalize3(normal_texel * 2.0 - vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(0.0, 0.0, 1.0));
    let n = safe_normalize3(in.normal, vec3<f32>(0.0, 0.0, 1.0));
    let tangent_seed = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.y) > 0.9);
    let generated_tangent = safe_normalize3(cross(tangent_seed, n), vec3<f32>(1.0, 0.0, 0.0));
    let t = safe_normalize3(in.tangent, generated_tangent);
    let generated_bitangent = safe_normalize3(cross(n, t), vec3<f32>(0.0, 1.0, 0.0));
    let b = safe_normalize3(in.bitangent, generated_bitangent);
    let shading_normal = normalize(t * mapped.x + b * mapped.y + n * mapped.z);

    if (shading_mode >= 1.5 && shading_mode < 2.5) {
        var background = albedo_texel.rgb * in.color.rgb;
        let tone_mode = i32(scene_uniforms.post_misc.z);
        if (tone_mode > 0) {
            background = background / (background + vec3<f32>(1.0, 1.0, 1.0));
        }
        background = pow(max(background, vec3<f32>(0.0, 0.0, 0.0)), vec3<f32>(1.0 / 2.2));
        return vec4<f32>(background, albedo_texel.a * in.color.a);
    }

    let view_dir = normalize(-in.view_pos);
    let ambient = scene_uniforms.ambient_color.a;
    let roughness = clamp(in.material.y, 0.04, 1.0);
    let safe_gloss = max(gloss, 0.001);
    let shininess = 10.0 / log2(safe_gloss * 0.968 + 0.03);
    let specular_scale = min(shininess * (1.0 / (8.0 * 3.1415926)) + (4.0 / (8.0 * 3.1415926)), 1.0e3);
    let fresnel_factor = pow(1.0 - max(dot(view_dir, shading_normal), 0.0), 5.0);
    let fresnel = reflectivity + (vec3<f32>(1.0, 1.0, 1.0) - reflectivity) * fresnel_factor;
    let env_reflection = max(dot(reflect(-view_dir, shading_normal), vec3<f32>(0.0, 1.0, 0.0)), 0.0);
    let env_specular = scene_uniforms.ambient_color.rgb * fresnel * mix(0.25, 1.0, pow(env_reflection, 4.0));

    if (albedo_texel.a < alpha_test) {
        discard;
    }

    var diffuse_accum = scene_uniforms.ambient_color.rgb * (ambient * occlusion);
    var specular_accum = vec3<f32>(0.0, 0.0, 0.0);
    let light_count = i32(scene_uniforms.light_count.x);
    for (var i = 0; i < 4; i = i + 1) {
        if (i >= light_count) {
            break;
        }
        let light_dir = normalize(-scene_uniforms.light_dirs[i].xyz);
        let light_color = scene_uniforms.light_colors[i].rgb;
        let half_vec = normalize(light_dir + view_dir);
        let ndotl = max(dot(shading_normal, light_dir), 0.0);
        let ndoth = max(dot(shading_normal, half_vec), 0.0);
        diffuse_accum += vec3<f32>(ndotl * occlusion, ndotl * occlusion, ndotl * occlusion) * light_color;
        let specular_strength = pow(ndoth, shininess) * specular_scale;
        specular_accum += fresnel * specular_strength * light_color;
    }

    let unlit_diffuse = shading_mode >= 0.5 && shading_mode < 1.5;
    let diffuse_color = select(albedo * diffuse_accum, albedo, vec3<bool>(unlit_diffuse, unlit_diffuse, unlit_diffuse));
    var lit = diffuse_color + specular_accum + env_specular + emissive;
    let fog_inv_distance = scene_uniforms.fog_params.x;
    let fog_dispersion = scene_uniforms.fog_params.y;
    let fog_type = i32(scene_uniforms.fog_params.z);
    let fog_opacity = scene_uniforms.fog_color.w;
    if (fog_opacity > 0.0) {
        let fog_distance = length(in.view_pos);
        let b = fog_distance * fog_inv_distance;
        let linear_term = min(b, 1.0);
        let quadratic_term = 1.0 - 1.0 / (1.0 + 16.0 * b * b);
        let exponential_term = 1.0 - exp(-3.0 * b);
        var fog_amount = 0.0;
        if (fog_type == 0) {
            fog_amount = linear_term;
        } else if (fog_type == 1) {
            fog_amount = quadratic_term;
        } else {
            fog_amount = exponential_term;
        }
        fog_amount *= fog_opacity;
        if (light_count > 0) {
            let primary_light_dir = normalize(-scene_uniforms.light_dirs[0].xyz);
            let view_dir_for_fog = normalize(-in.view_pos);
            var directional_term = 0.5 + 0.5 * dot(view_dir_for_fog, primary_light_dir);
            directional_term = 1.0 + fog_dispersion * (2.0 * directional_term * directional_term - 1.0);
            fog_amount *= directional_term;
        }
        lit = mix(lit, scene_uniforms.fog_color.rgb, clamp(fog_amount, 0.0, 1.0));
    }

    let luminance = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
    let saturation = scene_uniforms.post_saturation.rgb;
    lit = vec3<f32>(luminance, luminance, luminance) + (lit - vec3<f32>(luminance, luminance, luminance)) * saturation;
    lit = (lit - vec3<f32>(0.5, 0.5, 0.5)) * scene_uniforms.post_contrast.rgb + vec3<f32>(0.5, 0.5, 0.5);
    lit = lit + scene_uniforms.post_brightness.rgb + scene_uniforms.post_misc.xxx;

    let vignette_strength = scene_uniforms.post_vignette.w;
    if (vignette_strength > 0.0) {
        let centered = in.screen_uv * 2.0 - vec2<f32>(1.0, 1.0);
        let dist = length(centered);
        let curve = max(scene_uniforms.post_misc.y, 0.001);
        let vignette = clamp(1.0 - pow(dist, curve) * vignette_strength, 0.0, 1.0);
        lit = mix(scene_uniforms.post_vignette.rgb, lit, vignette);
    }

    let tone_mode = i32(scene_uniforms.post_misc.z);
    if (tone_mode > 0) {
        lit = lit / (lit + vec3<f32>(1.0, 1.0, 1.0));
    }
    lit = clamp(lit, vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(8.0, 8.0, 8.0));

    if (shading_mode >= 2.5) {
        let floor_color = mix(vec3<f32>(1.0, 1.0, 1.0), scene_uniforms.fog_color.rgb, 0.35);
        let encoded_floor = pow(max(floor_color, vec3<f32>(0.0, 0.0, 0.0)), vec3<f32>(1.0 / 2.2));
        return vec4<f32>(encoded_floor, albedo_texel.a * in.material.w * in.color.a);
    }

    let alpha = albedo_texel.a * in.material.w;
    let color_mod = select(vec3<f32>(1.0, 1.0, 1.0), in.color.rgb, vec3<bool>(use_vertex_color, use_vertex_color, use_vertex_color));
    let alpha_mod = select(1.0, in.color.a, use_vertex_color);
    let encoded = pow(max(lit * color_mod, vec3<f32>(0.0, 0.0, 0.0)), vec3<f32>(1.0 / 2.2));
    return vec4<f32>(encoded, alpha * alpha_mod);
}
"#
            .into(),
        ),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mviewer_runtime_viewer_layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mviewer_runtime_viewer_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GpuVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: std::mem::size_of::<[f32; 2]>() as u64,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: (std::mem::size_of::<[f32; 2]>() * 2) as u64,
                        shader_location: 2,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: (std::mem::size_of::<[f32; 2]>() * 2
                            + std::mem::size_of::<[f32; 4]>()) as u64,
                        shader_location: 3,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: (std::mem::size_of::<[f32; 2]>() * 2
                            + std::mem::size_of::<[f32; 4]>()
                            + std::mem::size_of::<[f32; 3]>()) as u64,
                        shader_location: 4,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: (std::mem::size_of::<[f32; 2]>() * 2
                            + std::mem::size_of::<[f32; 4]>()
                            + std::mem::size_of::<[f32; 3]>() * 2) as u64,
                        shader_location: 5,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: (std::mem::size_of::<[f32; 2]>() * 2
                            + std::mem::size_of::<[f32; 4]>()
                            + std::mem::size_of::<[f32; 3]>() * 3) as u64,
                        shader_location: 6,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: (std::mem::size_of::<[f32; 2]>() * 2
                            + std::mem::size_of::<[f32; 4]>()
                            + std::mem::size_of::<[f32; 3]>() * 4) as u64,
                        shader_location: 7,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: (std::mem::size_of::<[f32; 2]>() * 2
                            + std::mem::size_of::<[f32; 4]>()
                            + std::mem::size_of::<[f32; 3]>() * 4
                            + std::mem::size_of::<[f32; 4]>()) as u64,
                        shader_location: 8,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: (std::mem::size_of::<[f32; 2]>() * 3
                            + std::mem::size_of::<[f32; 4]>()
                            + std::mem::size_of::<[f32; 3]>() * 4
                            + std::mem::size_of::<[f32; 4]>()) as u64,
                        shader_location: 9,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: (std::mem::size_of::<[f32; 2]>() * 3
                            + std::mem::size_of::<[f32; 4]>() * 2
                            + std::mem::size_of::<[f32; 3]>() * 4) as u64,
                        shader_location: 10,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: (std::mem::size_of::<[f32; 2]>() * 3
                            + std::mem::size_of::<[f32; 4]>() * 3
                            + std::mem::size_of::<[f32; 3]>() * 4) as u64,
                        shader_location: 11,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: (std::mem::size_of::<[f32; 2]>() * 3
                            + std::mem::size_of::<[f32; 4]>() * 4
                            + std::mem::size_of::<[f32; 3]>() * 4) as u64,
                        shader_location: 12,
                    },
                ],
            }],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

fn build_scene_mesh(
    project: &ProjectDocument,
    atlas: &TextureAtlasData,
    visible_meshes: &[bool],
    rect: egui::Rect,
    yaw_offset: f32,
    pitch_offset: f32,
    distance_offset: f32,
    fov_y_radians: f32,
    smooth_shading: bool,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> SceneGpuMesh {
    let mut scene_vertices = Vec::new();
    let mut scene_indices = Vec::new();
    let mut wire_segments = Vec::new();

    let bounds = scene_bounds(project, visible_meshes);
    let bounds_center = [
        (bounds.0[0] + bounds.1[0]) * 0.5,
        (bounds.0[1] + bounds.1[1]) * 0.5,
        (bounds.0[2] + bounds.1[2]) * 0.5,
    ];
    let mut radius = 0.001f32;
    for mesh in project
        .runtime
        .meshes
        .iter()
        .enumerate()
        .filter_map(|(index, mesh)| visible_meshes.get(index).copied().unwrap_or(false).then_some(mesh))
    {
        let mesh_transform = mesh_world_transform(project, mesh.index, selected_clip, time_seconds);
        for position in &mesh.decoded.positions {
            let world = apply_transform(Some(mesh_transform), *position);
            radius = radius.max(distance3(world, bounds_center));
        }
    }

    let camera = resolve_scene_camera(
        project,
        bounds_center,
        radius,
        yaw_offset,
        pitch_offset,
        distance_offset,
        fov_y_radians,
        selected_clip,
        time_seconds,
    );
    let center = camera.pivot;
    let focal = (rect.height() * 0.5) / (camera.fov_y_radians * 0.5).tan().max(0.01);
    let camera_distance = camera.distance;
    let yaw = camera.yaw;
    let pitch = camera.pitch;
    let lights = scene_lights(project, yaw, pitch);
    let environment = scene_environment(project);

    append_sky_background(&mut scene_vertices, &mut scene_indices, project, rect, atlas);

    for (mesh_index, mesh) in project.runtime.meshes.iter().enumerate() {
        if !visible_meshes.get(mesh_index).copied().unwrap_or(false) {
            continue;
        }
        if !mesh_is_visible(project, mesh, selected_clip, time_seconds) {
            continue;
        }
        let mesh_transform = mesh_world_transform(project, mesh_index, selected_clip, time_seconds);
        let deformed_positions = mesh_deformed_positions(project, mesh, selected_clip, time_seconds);
        let deformed_normals = mesh_deformed_normals(project, mesh, selected_clip, time_seconds);
        let deformed_tangents = mesh_deformed_tangents(project, mesh, selected_clip, time_seconds);
        let deformed_bitangents = mesh_deformed_bitangents(project, mesh, selected_clip, time_seconds);
        let mut projected = Vec::with_capacity(mesh.decoded.positions.len());
        let mut view_positions = Vec::with_capacity(mesh.decoded.positions.len());
        let mut normals = Vec::with_capacity(mesh.decoded.normals.len());
        let mut tangents = Vec::with_capacity(mesh.decoded.tangents.len());
        let mut bitangents = Vec::with_capacity(mesh.decoded.bitangents.len());
        for (((&position, &normal), &tangent), &bitangent) in deformed_positions
            .iter()
            .zip(&deformed_normals)
            .zip(&deformed_tangents)
            .zip(&deformed_bitangents)
        {
            let world = apply_transform(Some(mesh_transform), position);
            let camera = orbit_point(world, center, yaw, pitch, camera_distance);
            view_positions.push(camera);
            let rotated_normal = orbit_direction(apply_direction(Some(mesh_transform), normal), yaw, pitch);
            let rotated_tangent =
                orbit_direction(apply_direction(Some(mesh_transform), tangent), yaw, pitch);
            let rotated_bitangent =
                orbit_direction(apply_direction(Some(mesh_transform), bitangent), yaw, pitch);
            normals.push(rotated_normal);
            tangents.push(rotated_tangent);
            bitangents.push(rotated_bitangent);
            projected.push(project_camera_vertex(camera, focal, rect.center()));
        }

        for sub_mesh in &mesh.desc.sub_meshes {
            let Some(&material_index) = project.runtime.material_lookup.get(&sub_mesh.material) else {
                continue;
            };
            let Some(material) = project.runtime.materials.get(material_index) else {
                continue;
            };
            let material_preview = material_preview_for(material);
            let tile_rect = material_tile_rect(atlas, material_index);
            let uv_offset = material_uv_offset(project, material, selected_clip, time_seconds);
            let ao_range = material_extras_range(material, "aoTex");
            let emissive_range = material_extras_range(material, "emissiveTex");
            let flags = material_flags(material);
            let start = sub_mesh.first_index.min(mesh.decoded.indices.len());
            let end = (sub_mesh.first_index + sub_mesh.index_count).min(mesh.decoded.indices.len());
            let index_slice = &mesh.decoded.indices[start..end];
            for triangle in index_slice.chunks_exact(3) {
            let ia = triangle[0] as usize;
            let ib = triangle[1] as usize;
            let ic = triangle[2] as usize;
            let (Some(a), Some(b), Some(c)) = (projected[ia], projected[ib], projected[ic]) else {
                continue;
            };
            let cross_z = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            if cross_z >= 0.0 {
                continue;
            }
            wire_segments.push([a, b]);
            wire_segments.push([b, c]);
            wire_segments.push([c, a]);
            let base = scene_vertices.len() as u32;
            let face_normal = if smooth_shading {
                None
            } else {
                let aw = apply_transform(Some(mesh_transform), deformed_positions[ia]);
                let bw = apply_transform(Some(mesh_transform), deformed_positions[ib]);
                let cw = apply_transform(Some(mesh_transform), deformed_positions[ic]);
                Some(normalize3(cross3(sub3(bw, aw), sub3(cw, aw))))
            };
            for index in [ia, ib, ic] {
                let screen = projected[index].expect("projected vertex");
                let mut uv0 = mesh.decoded.texcoords.get(index).copied().unwrap_or([0.5, 0.5]);
                uv0[0] += uv_offset[0];
                uv0[1] += uv_offset[1];
                let uv1 = mesh
                    .decoded
                    .secondary_texcoords
                    .as_ref()
                    .and_then(|uvs| uvs.get(index).copied())
                    .unwrap_or(uv0);
                let color = mesh
                    .decoded
                    .colors
                    .as_ref()
                    .and_then(|colors| colors.get(index).copied())
                    .unwrap_or([1.0, 1.0, 1.0, 1.0]);
                let normal = face_normal.unwrap_or(normals[index]);
                scene_vertices.push(GpuVertex {
                    position: screen_to_ndc(screen, rect),
                    uv0,
                    color,
                    view_pos: view_positions[index],
                    normal,
                    tangent: tangents[index],
                    bitangent: bitangents[index],
                    material: [
                        material_preview.metallic,
                        material_preview.roughness,
                        material_preview.emissive,
                        material_preview.alpha,
                    ],
                    uv1,
                    tile_rect,
                    ao_range,
                    emissive_range,
                    flags,
                });
            }
            scene_indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
        }
    }

    append_shadow_floor(
        &mut scene_vertices,
        &mut scene_indices,
        project,
        rect,
        center,
        yaw,
        pitch,
        camera_distance,
        atlas,
    );

    SceneGpuMesh {
        vertices: scene_vertices,
        indices: scene_indices,
        wire_segments,
        atlas: atlas.clone(),
        lights,
        environment,
    }
}

fn draw_scene_fallback(painter: &egui::Painter, scene: &SceneGpuMesh) {
    let mut mesh = egui::Mesh::default();
    for vertex in &scene.vertices {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(vertex.position[0], vertex.position[1]),
            uv: egui::pos2(vertex.uv0[0], vertex.uv0[1]),
            color: egui::Color32::from_rgba_unmultiplied(
                (vertex.color[0] * 255.0) as u8,
                (vertex.color[1] * 255.0) as u8,
                (vertex.color[2] * 255.0) as u8,
                255,
            ),
        });
    }
    mesh.indices = scene.indices.clone();
    painter.add(egui::Shape::mesh(mesh));
}

fn draw_wireframe(painter: &egui::Painter, scene: &SceneGpuMesh) {
    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(240, 122, 56));
    for segment in &scene.wire_segments {
        painter.line_segment(*segment, stroke);
    }
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect) {
    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(44, 50, 58));
    for row in 1..6 {
        let y = egui::lerp(rect.top()..=rect.bottom(), row as f32 / 6.0);
        painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], stroke);
    }
    for col in 1..10 {
        let x = egui::lerp(rect.left()..=rect.right(), col as f32 / 10.0);
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], stroke);
    }
}

#[derive(Clone, Copy)]
struct SceneCameraState {
    pivot: [f32; 3],
    yaw: f32,
    pitch: f32,
    distance: f32,
    fov_y_radians: f32,
}

fn resolve_scene_camera(
    project: &ProjectDocument,
    bounds_center: [f32; 3],
    bounds_radius: f32,
    yaw_offset: f32,
    pitch_offset: f32,
    distance_offset: f32,
    fallback_fov_y_radians: f32,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> SceneCameraState {
    let mut pivot = project
        .scene
        .main_camera
        .as_ref()
        .and_then(|camera| camera.view.as_ref())
        .and_then(|view| view.pivot)
        .unwrap_or(bounds_center);
    let mut yaw = project
        .scene
        .main_camera
        .as_ref()
        .and_then(|camera| camera.view.as_ref())
        .and_then(|view| view.angles)
        .map(|angles| angles[0].to_radians())
        .unwrap_or(0.5);
    let mut pitch = project
        .scene
        .main_camera
        .as_ref()
        .and_then(|camera| camera.view.as_ref())
        .and_then(|view| view.angles)
        .map(|angles| angles[1].to_radians())
        .unwrap_or(0.3);
    let mut distance = project
        .scene
        .main_camera
        .as_ref()
        .and_then(|camera| camera.view.as_ref())
        .and_then(|view| view.orbit_radius)
        .unwrap_or((bounds_radius * 2.8).max(0.1));
    let mut fov_y_radians = project
        .scene
        .main_camera
        .as_ref()
        .and_then(|camera| camera.view.as_ref())
        .and_then(|view| view.fov)
        .map(|fov| fov.to_radians())
        .unwrap_or(fallback_fov_y_radians);

    if let Some(animated) =
        resolve_selected_animated_camera(project, selected_clip, time_seconds, distance, fov_y_radians)
    {
        pivot = animated.pivot;
        yaw = animated.yaw;
        pitch = animated.pitch;
        distance = animated.distance;
        fov_y_radians = animated.fov_y_radians;
    }

    SceneCameraState {
        pivot,
        yaw: yaw + yaw_offset,
        pitch: (pitch + pitch_offset).clamp(-1.35, 1.35),
        distance: (distance + distance_offset * distance.max(0.25)).max(0.05),
        fov_y_radians,
    }
}

fn resolve_selected_animated_camera(
    project: &ProjectDocument,
    selected_clip: Option<usize>,
    time_seconds: f32,
    fallback_distance: f32,
    fallback_fov_y_radians: f32,
) -> Option<SceneCameraState> {
    let animations = project.animations.as_ref()?;
    let clip_index = selected_clip
        .or_else(|| project.runtime.animation_binding.as_ref().and_then(|binding| binding.selected_animation))
        .unwrap_or(0);
    let animation = animations.animations.get(clip_index)?;
    let object_index = selected_camera_object_index(project, animation)?;
    let object = animation.animated_objects.get(object_index)?;
    let world = animation.get_world_transform(object_index, time_seconds, animations.scene_scale);
    let position = [world[12], world[13], world[14]];
    let forward = normalize3([-world[8], -world[9], -world[10]]);
    let distance = camera_orbit_radius(project).unwrap_or(fallback_distance).max(0.05);
    let pivot = [
        position[0] + forward[0] * distance,
        position[1] + forward[1] * distance,
        position[2] + forward[2] * distance,
    ];
    let yaw = forward[0].atan2(-forward[2]);
    let pitch = (-forward[1]).asin().clamp(-1.35, 1.35);
    let frame = animation.get_object_animation_frame_percent(object_index, time_seconds);
    let fov_y_radians = sample_camera_fov(object, frame)
        .unwrap_or(fallback_fov_y_radians)
        .clamp(15.0f32.to_radians(), 100.0f32.to_radians());
    Some(SceneCameraState {
        pivot,
        yaw,
        pitch,
        distance,
        fov_y_radians,
    })
}

fn camera_orbit_radius(project: &ProjectDocument) -> Option<f32> {
    project
        .scene
        .main_camera
        .as_ref()
        .and_then(|camera| camera.view.as_ref())
        .and_then(|view| view.orbit_radius)
}

fn selected_camera_object_index(
    project: &ProjectDocument,
    animation: &crate::animation::ParsedAnimation,
) -> Option<usize> {
    let desired = project
        .runtime
        .animation_binding
        .as_ref()
        .and_then(|binding| binding.selected_camera);
    let camera_objects: Vec<usize> = animation
        .animated_objects
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            object
                .desc
                .scene_object_type
                .to_ascii_lowercase()
                .contains("camera")
                .then_some(index)
        })
        .collect();
    if camera_objects.is_empty() {
        return None;
    }
    if let Some(desired) = desired {
        if let Some(&exact) = camera_objects.iter().find(|&&index| index == desired) {
            return Some(exact);
        }
        if let Some(&model_part_match) = camera_objects.iter().find(|&&index| {
            animation
                .animated_objects
                .get(index)
                .map(|object| object.desc.model_part_index == desired)
                .unwrap_or(false)
        }) {
            return Some(model_part_match);
        }
        if let Some(index) = camera_objects.get(desired).copied() {
            return Some(index);
        }
    }
    camera_objects.first().copied()
}

fn sample_camera_fov(object: &crate::animation::ParsedAnimatedObject, frame: f32) -> Option<f32> {
    const NAMES: [&str; 4] = ["FOV", "Field Of View", "Field of View", "Camera FOV"];
    for name in NAMES {
        if let Some(value) = object.sample_named_property(name, frame, 0.0) {
            if value > 0.0 {
                return Some(value.to_radians());
            }
        }
    }
    None
}

fn append_shadow_floor(
    scene_vertices: &mut Vec<GpuVertex>,
    scene_indices: &mut Vec<u32>,
    project: &ProjectDocument,
    rect: egui::Rect,
    center: [f32; 3],
    yaw: f32,
    pitch: f32,
    camera_distance: f32,
    atlas: &TextureAtlasData,
) {
    let Some(shadow_floor) = &project.scene.shadow_floor else {
        return;
    };
    let transform = shadow_floor.transform.unwrap_or(identity_matrix());
    let alpha = shadow_floor.alpha.unwrap_or(0.5).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let focal = rect.width().min(rect.height()) * 0.5;
    let quad = [
        [-1.0f32, 0.0, -1.0],
        [-1.0f32, 0.0, 1.0],
        [1.0f32, 0.0, 1.0],
        [1.0f32, 0.0, -1.0],
    ];
    let quad_uv = [[0.0f32, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    let floor_normal = orbit_direction(
        apply_direction(Some(transform), [0.0, 1.0, 0.0]),
        yaw,
        pitch,
    );
    let mut projected = Vec::with_capacity(4);
    let mut view_positions = Vec::with_capacity(4);
    for position in quad {
        let world = apply_transform(Some(transform), position);
        let camera = orbit_point(world, center, yaw, pitch, camera_distance);
        let Some(screen) = project_camera_vertex(camera, focal, rect.center()) else {
            return;
        };
        projected.push(screen);
        view_positions.push(camera);
    }
    let edge_strength = if shadow_floor.edge_fade.unwrap_or(false) {
        shadow_floor.alpha.unwrap_or(0.5).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let base = scene_vertices.len() as u32;
    let tile_rect = atlas.material_rects.first().copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
    for index in 0..4 {
        let uv = quad_uv[index];
        let radial = ((uv[0] - 0.5) * (uv[0] - 0.5) + (uv[1] - 0.5) * (uv[1] - 0.5)).sqrt() * 2.0;
        let fade = if edge_strength > 0.0 {
            (1.0 - radial).clamp(0.0, 1.0)
        } else {
            1.0
        };
        scene_vertices.push(GpuVertex {
            position: screen_to_ndc(projected[index], rect),
            uv0: uv,
            color: [1.0, 1.0, 1.0, fade * alpha],
            view_pos: view_positions[index],
            normal: floor_normal,
            tangent: orbit_direction([1.0, 0.0, 0.0], yaw, pitch),
            bitangent: orbit_direction([0.0, 0.0, 1.0], yaw, pitch),
            material: [0.0, 1.0, 0.0, fade * alpha],
            uv1: uv,
            tile_rect,
            ao_range: [1.0, 1.0, 0.0, 0.0],
            emissive_range: [1.0, 1.0, 0.0, 0.0],
            flags: [0.0, 0.0, 3.0, 0.0],
        });
    }
    scene_indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn append_sky_background(
    scene_vertices: &mut Vec<GpuVertex>,
    scene_indices: &mut Vec<u32>,
    project: &ProjectDocument,
    rect: egui::Rect,
    atlas: &TextureAtlasData,
) {
    let Some(tile_rect) = atlas.background_rect else {
        return;
    };
    let background_mode = project
        .scene
        .sky
        .as_ref()
        .and_then(|sky| sky.background_mode)
        .unwrap_or(0);
    if background_mode < 1 {
        return;
    }
    let brightness = project
        .scene
        .sky
        .as_ref()
        .and_then(|sky| sky.background_brightness)
        .unwrap_or(1.0)
        .clamp(0.0, 4.0);
    let alpha = 1.0;
    let quad = [
        ([-1.0f32, 1.0], [0.0f32, 0.0]),
        ([-1.0f32, -1.0], [0.0f32, 1.0]),
        ([1.0f32, -1.0], [1.0f32, 1.0]),
        ([1.0f32, 1.0], [1.0f32, 0.0]),
    ];
    let base = scene_vertices.len() as u32;
    for (position, uv) in quad {
        scene_vertices.push(GpuVertex {
            position,
            uv0: uv,
            color: [brightness, brightness, brightness, alpha],
            view_pos: [0.0, 0.0, -1.0],
            normal: [0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0],
            bitangent: [0.0, 1.0, 0.0],
            material: [0.0, 1.0, 0.0, alpha],
            uv1: uv,
            tile_rect,
            ao_range: [1.0, 1.0, 0.0, 0.0],
            emissive_range: [1.0, 1.0, 0.0, 0.0],
            flags: [0.0, 0.0, 2.0, 0.0],
        });
    }
    scene_indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    let _ = rect;
}

fn scene_bounds(project: &ProjectDocument, visible_meshes: &[bool]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for (index, mesh) in project.runtime.meshes.iter().enumerate() {
        if !visible_meshes.get(index).copied().unwrap_or(false) {
            continue;
        }
        let mesh_transform = mesh_world_transform(project, index, None, 0.0);
        for position in &mesh.decoded.positions {
            let world = apply_transform(Some(mesh_transform), *position);
            for axis in 0..3 {
                min[axis] = min[axis].min(world[axis]);
                max[axis] = max[axis].max(world[axis]);
            }
        }
    }
    if !min[0].is_finite() {
        ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])
    } else {
        (min, max)
    }
}

fn apply_transform(transform: Option<[f32; 16]>, position: [f32; 3]) -> [f32; 3] {
    let matrix = transform.unwrap_or([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    [
        matrix[0] * position[0] + matrix[4] * position[1] + matrix[8] * position[2] + matrix[12],
        matrix[1] * position[0] + matrix[5] * position[1] + matrix[9] * position[2] + matrix[13],
        matrix[2] * position[0] + matrix[6] * position[1] + matrix[10] * position[2] + matrix[14],
    ]
}

fn apply_direction(transform: Option<[f32; 16]>, direction: [f32; 3]) -> [f32; 3] {
    let matrix = transform.unwrap_or([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    [
        matrix[0] * direction[0] + matrix[4] * direction[1] + matrix[8] * direction[2],
        matrix[1] * direction[0] + matrix[5] * direction[1] + matrix[9] * direction[2],
        matrix[2] * direction[0] + matrix[6] * direction[1] + matrix[10] * direction[2],
    ]
}

fn orbit_point(vertex: [f32; 3], center: [f32; 3], yaw: f32, pitch: f32, camera_distance: f32) -> [f32; 3] {
    let px = vertex[0] - center[0];
    let py = vertex[1] - center[1];
    let pz = vertex[2] - center[2];
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    let x1 = cos_yaw * px + sin_yaw * pz;
    let z1 = -sin_yaw * px + cos_yaw * pz;
    let y2 = cos_pitch * py - sin_pitch * z1;
    let z2 = sin_pitch * py + cos_pitch * z1 - camera_distance;
    [x1, y2, z2]
}

fn orbit_direction(direction: [f32; 3], yaw: f32, pitch: f32) -> [f32; 3] {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    let x1 = cos_yaw * direction[0] + sin_yaw * direction[2];
    let z1 = -sin_yaw * direction[0] + cos_yaw * direction[2];
    let y2 = cos_pitch * direction[1] - sin_pitch * z1;
    let z2 = sin_pitch * direction[1] + cos_pitch * z1;
    normalize3([x1, y2, z2])
}

fn project_camera_vertex(camera_vertex: [f32; 3], focal: f32, center: egui::Pos2) -> Option<egui::Pos2> {
    if camera_vertex[2] >= -0.01 {
        return None;
    }
    let scale = focal / -camera_vertex[2];
    Some(egui::pos2(
        center.x + camera_vertex[0] * scale,
        center.y - camera_vertex[1] * scale,
    ))
}

fn screen_to_ndc(point: egui::Pos2, rect: egui::Rect) -> [f32; 2] {
    let x = ((point.x - rect.left()) / rect.width()) * 2.0 - 1.0;
    let y = 1.0 - ((point.y - rect.top()) / rect.height()) * 2.0;
    [x, y]
}

fn preview_color(base_color: [f32; 3], alpha: f32) -> [f32; 4] {
    [
        base_color[0].clamp(0.0, 1.0),
        base_color[1].clamp(0.0, 1.0),
        base_color[2].clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    ]
}

#[derive(Clone, Copy)]
struct MaterialPreview {
    base_color: [f32; 3],
    metallic: f32,
    roughness: f32,
    emissive: f32,
    alpha: f32,
}

fn mesh_preview_material(project: &ProjectDocument, mesh: &crate::runtime::RuntimeMesh) -> MaterialPreview {
    mesh.material_indices
        .first()
        .and_then(|index| project.runtime.materials.get(*index))
        .map(material_preview_for)
        .unwrap_or(MaterialPreview {
            base_color: [0.82, 0.56, 0.34],
            metallic: 0.0,
            roughness: 0.85,
            emissive: 0.0,
            alpha: 1.0,
        })
}

fn material_preview_for(material: &crate::runtime::RuntimeMaterial) -> MaterialPreview {
    MaterialPreview {
        base_color: material.preview_color,
        metallic: material.preview_metallic,
        roughness: material.preview_roughness,
        emissive: material.preview_emissive,
        alpha: if material.desc.blend.as_deref() == Some("alpha") { 0.82 } else { 1.0 },
    }
}

fn background_color(project: &ProjectDocument) -> egui::Color32 {
    let background_mode = project
        .scene
        .sky
        .as_ref()
        .and_then(|sky| sky.background_mode)
        .unwrap_or(0);
    if background_mode >= 1 {
        return egui::Color32::from_rgb(14, 16, 20);
    }
    if let Some(sky) = &project.scene.sky {
        if let Some(color) = &sky.background_color {
            if color.len() >= 3 {
                return egui::Color32::from_rgb(
                    (color[0].clamp(0.0, 1.0) * 255.0) as u8,
                    (color[1].clamp(0.0, 1.0) * 255.0) as u8,
                    (color[2].clamp(0.0, 1.0) * 255.0) as u8,
                );
            }
        }
    }
    if let Some(fog) = &project.scene.fog {
        if let Some(color) = fog.color {
            return egui::Color32::from_rgb(
                (color[0].clamp(0.0, 1.0) * 255.0) as u8,
                (color[1].clamp(0.0, 1.0) * 255.0) as u8,
                (color[2].clamp(0.0, 1.0) * 255.0) as u8,
            );
        }
    }
    egui::Color32::from_rgb(24, 28, 34)
}

fn scene_lights(project: &ProjectDocument, yaw: f32, pitch: f32) -> Vec<LightPreview> {
    let mut result = Vec::new();
    let light_scale = project
        .scene
        .fog
        .as_ref()
        .and_then(|fog| fog.light_illum)
        .unwrap_or(1.0)
        .clamp(0.0, 4.0);
    if let Some(lights) = &project.scene.lights {
        if let (Some(directions), Some(colors)) = (&lights.directions, &lights.colors) {
            let count = directions.len().min(colors.len()) / 3;
            for index in 0..count.min(MAX_VIEWER_LIGHTS) {
                let dir = normalize3([
                    directions[index * 3],
                    directions[index * 3 + 1],
                    directions[index * 3 + 2],
                ]);
                let color = [
                    (colors[index * 3] * light_scale).clamp(0.0, 8.0),
                    (colors[index * 3 + 1] * light_scale).clamp(0.0, 8.0),
                    (colors[index * 3 + 2] * light_scale).clamp(0.0, 8.0),
                ];
                result.push(LightPreview {
                    direction: orbit_direction(dir, yaw, pitch),
                    color,
                });
            }
        }
    }
    if result.is_empty() {
        result.push(LightPreview {
            direction: normalize3([0.35, 0.65, -1.0]),
            color: [1.0, 1.0, 1.0],
        });
    }
    result
}

fn scene_environment(project: &ProjectDocument) -> SceneEnvironment {
    let post = project
        .scene
        .main_camera
        .as_ref()
        .and_then(|camera| camera.post.as_ref());
    let post_brightness = post
        .and_then(|post| post.brightness)
        .unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let post_contrast = post
        .and_then(|post| post.contrast)
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let post_saturation = post
        .and_then(|post| post.saturation)
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let post_vignette = post
        .and_then(|post| post.vignette)
        .unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let post_misc = [
        post.and_then(|post| post.bias).map(|bias| bias[0]).unwrap_or(0.0),
        post.and_then(|post| post.vignette_curve).unwrap_or(2.0),
        post.and_then(|post| post.tone_map).unwrap_or(0) as f32,
        0.0,
    ];
    let sky_illum = project
        .scene
        .fog
        .as_ref()
        .and_then(|fog| fog.sky_illum)
        .unwrap_or(1.0)
        .clamp(0.0, 4.0);
    let ambient_color = project
        .scene
        .sky
        .as_ref()
        .and_then(|sky| sky.diffuse_coefficients.as_ref())
        .and_then(|coeffs| {
            if coeffs.len() >= 9 {
                let mut rgb = [0.0f32; 3];
                let samples = (coeffs.len() / 3).min(3);
                for sample in 0..samples {
                    rgb[0] += coeffs[sample * 3];
                    rgb[1] += coeffs[sample * 3 + 1];
                    rgb[2] += coeffs[sample * 3 + 2];
                }
                let inv = 1.0 / samples as f32;
                Some([rgb[0] * inv, rgb[1] * inv, rgb[2] * inv])
            } else if coeffs.len() >= 3 {
                Some([coeffs[0], coeffs[1], coeffs[2]])
            } else {
                None
            }
        })
        .map(|coeffs| {
            [
                (coeffs[0] * sky_illum).clamp(0.0, 3.0),
                (coeffs[1] * sky_illum).clamp(0.0, 3.0),
                (coeffs[2] * sky_illum).clamp(0.0, 3.0),
            ]
        })
        .or_else(|| {
            project
                .scene
                .sky
                .as_ref()
                .and_then(|sky| sky.background_color.as_ref())
                .and_then(|color| {
                    (color.len() >= 3).then_some([
                        color[0].clamp(0.0, 1.0),
                        color[1].clamp(0.0, 1.0),
                        color[2].clamp(0.0, 1.0),
                    ])
                })
        })
        .map(|color| [color[0] * sky_illum, color[1] * sky_illum, color[2] * sky_illum])
        .unwrap_or([0.24, 0.24, 0.24]);
    let ambient_strength = 1.0;
    if let Some(fog) = &project.scene.fog {
        let color = fog.color.unwrap_or([0.0, 0.0, 0.0]);
        let opacity = fog.opacity.unwrap_or(0.0).clamp(0.0, 1.0);
        let distance = fog.distance.unwrap_or(1.0).max(0.001);
        let dispersion = fog.dispersion.unwrap_or(0.0).clamp(0.0, 1.0);
        let fog_type = fog.fog_type.unwrap_or(0).min(2) as f32;
        return SceneEnvironment {
            ambient_color,
            ambient_strength,
            fog_color: color,
            fog_opacity: opacity,
            fog_inv_distance: 1.0 / distance,
            fog_dispersion: dispersion,
            fog_type,
            post_brightness,
            post_contrast,
            post_saturation,
            post_vignette,
            post_misc,
        };
    }
    SceneEnvironment {
        ambient_color,
        ambient_strength,
        fog_color: [0.0, 0.0, 0.0],
        fog_opacity: 0.0,
        fog_inv_distance: 0.0,
        fog_dispersion: 0.0,
        fog_type: 0.0,
        post_brightness,
        post_contrast,
        post_saturation,
        post_vignette,
        post_misc,
    }
}

fn mesh_tile_rect(atlas: &TextureAtlasData, mesh: &crate::runtime::RuntimeMesh) -> [f32; 4] {
    mesh
        .material_indices
        .first()
        .map(|index| material_tile_rect(atlas, *index))
        .or_else(|| atlas.material_rects.first().copied())
        .unwrap_or([0.0, 0.0, 1.0, 1.0])
}

fn material_tile_rect(atlas: &TextureAtlasData, material_index: usize) -> [f32; 4] {
    atlas
        .material_rects
        .get(material_index + 1)
        .copied()
        .unwrap_or_else(|| atlas.material_rects.first().copied().unwrap_or([0.0, 0.0, 1.0, 1.0]))
}

fn mesh_uv_offset(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> [f32; 2] {
    mesh
        .material_indices
        .first()
        .and_then(|index| project.runtime.materials.get(*index))
        .map(|material| material_uv_offset(project, material, selected_clip, time_seconds))
        .unwrap_or([0.0, 0.0])
}

fn mesh_extras_range(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    key: &str,
) -> [f32; 4] {
    mesh
        .material_indices
        .first()
        .and_then(|index| project.runtime.materials.get(*index))
        .and_then(|material| material.desc.extras_tex_coord_ranges.as_ref())
        .and_then(|ranges| ranges.get(key))
        .map(|range| range.scale_bias)
        .unwrap_or([1.0, 1.0, 0.0, 0.0])
}

fn mesh_material_flags(project: &ProjectDocument, mesh: &crate::runtime::RuntimeMesh) -> [f32; 4] {
    let Some(material) = mesh
        .material_indices
        .first()
        .and_then(|index| project.runtime.materials.get(*index))
    else {
        return [0.0, 0.0, 0.0, 0.0];
    };
    material_flags(material)
}

fn material_flags(material: &crate::runtime::RuntimeMaterial) -> [f32; 4] {
    [
        if material.desc.ao_secondary_uv.unwrap_or(false) {
            1.0
        } else {
            0.0
        },
        if material.desc.emissive_secondary_uv.unwrap_or(false) {
            1.0
        } else {
            0.0
        },
        if material.desc.unlit_diffuse.unwrap_or(false) {
            1.0
        } else {
            0.0
        },
        if material.desc.vertex_color.unwrap_or(false)
            || material.desc.vertex_colors_rgb.unwrap_or(false)
            || material.desc.vertex_color_alpha.unwrap_or(false)
        {
            material.desc.alpha_test.unwrap_or(0.0) + 10.0
        } else {
            material.desc.alpha_test.unwrap_or(0.0)
        },
    ]
}

fn material_extras_range(
    material: &crate::runtime::RuntimeMaterial,
    key: &str,
) -> [f32; 4] {
    material
        .desc
        .extras_tex_coord_ranges
        .as_ref()
        .and_then(|ranges| ranges.get(key))
        .map(|range| range.scale_bias)
        .unwrap_or([1.0, 1.0, 0.0, 0.0])
}

fn mesh_world_transform(
    project: &ProjectDocument,
    mesh_index: usize,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> [f32; 16] {
    let fallback = project
        .runtime
        .meshes
        .get(mesh_index)
        .and_then(|mesh| mesh.desc.transform)
        .unwrap_or_else(identity_matrix);
    let Some(clip_index) = selected_clip else {
        return fallback;
    };
    let Some(mesh) = project.runtime.meshes.get(mesh_index) else {
        return fallback;
    };
    let Some(object_index) = mesh.animated_object_index else {
        return fallback;
    };
    let Some(animations) = &project.animations else {
        return fallback;
    };
    let Some(animation) = animations.animations.get(clip_index) else {
        return fallback;
    };
    if object_index >= animation.animated_objects.len() {
        return fallback;
    }
    animation.get_world_transform(object_index, time_seconds, animations.scene_scale)
}

fn mesh_deformed_positions(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> Vec<[f32; 3]> {
    let Some((animation, rig, object_index)) =
        viewer_skin_context(project, mesh, selected_clip)
    else {
        return mesh.decoded.positions.clone();
    };
    let cluster_matrices = sample_cluster_matrices(animation, rig, object_index, time_seconds);
    if cluster_matrices.is_empty() {
        return mesh.decoded.positions.clone();
    }

    let mut output = Vec::with_capacity(mesh.decoded.positions.len());
    let mut link_cursor = 0usize;
    for (vertex_index, position) in mesh.decoded.positions.iter().copied().enumerate() {
        let count = rig.link_map_count.get(vertex_index).copied().unwrap_or(0) as usize;
        if count == 0 {
            output.push(position);
            continue;
        }
        let mut accumulated = [0.0f32; 3];
        let mut total_weight = 0.0f32;
        for offset in 0..count {
            let cluster_index = rig
                .link_map_cluster_indices
                .get(link_cursor + offset)
                .copied()
                .unwrap_or(0) as usize;
            let weight = rig
                .link_map_weights
                .get(link_cursor + offset)
                .copied()
                .unwrap_or(0.0);
            if let Some(matrix) = cluster_matrices.get(cluster_index) {
                let transformed = apply_transform(Some(*matrix), position);
                accumulated[0] += transformed[0] * weight;
                accumulated[1] += transformed[1] * weight;
                accumulated[2] += transformed[2] * weight;
                total_weight += weight;
            }
        }
        link_cursor += count;
        if total_weight > f32::EPSILON {
            output.push([
                accumulated[0] / total_weight,
                accumulated[1] / total_weight,
                accumulated[2] / total_weight,
            ]);
        } else {
            output.push(position);
        }
    }
    output
}

fn mesh_deformed_normals(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> Vec<[f32; 3]> {
    let Some((animation, rig, object_index)) =
        viewer_skin_context(project, mesh, selected_clip)
    else {
        return mesh.decoded.normals.clone();
    };
    let cluster_matrices = sample_cluster_matrices(animation, rig, object_index, time_seconds);
    if cluster_matrices.is_empty() {
        return mesh.decoded.normals.clone();
    }

    let mut output = Vec::with_capacity(mesh.decoded.normals.len());
    let mut link_cursor = 0usize;
    for (vertex_index, normal) in mesh.decoded.normals.iter().copied().enumerate() {
        let count = rig.link_map_count.get(vertex_index).copied().unwrap_or(0) as usize;
        if count == 0 {
            output.push(normal);
            continue;
        }
        let mut accumulated = [0.0f32; 3];
        let mut total_weight = 0.0f32;
        for offset in 0..count {
            let cluster_index = rig
                .link_map_cluster_indices
                .get(link_cursor + offset)
                .copied()
                .unwrap_or(0) as usize;
            let weight = rig
                .link_map_weights
                .get(link_cursor + offset)
                .copied()
                .unwrap_or(0.0);
            if let Some(matrix) = cluster_matrices.get(cluster_index) {
                let transformed = apply_direction(Some(*matrix), normal);
                accumulated[0] += transformed[0] * weight;
                accumulated[1] += transformed[1] * weight;
                accumulated[2] += transformed[2] * weight;
                total_weight += weight;
            }
        }
        link_cursor += count;
        if total_weight > f32::EPSILON {
            output.push(normalize3([
                accumulated[0] / total_weight,
                accumulated[1] / total_weight,
                accumulated[2] / total_weight,
            ]));
        } else {
            output.push(normal);
        }
    }
    output
}

fn mesh_deformed_tangents(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> Vec<[f32; 3]> {
    mesh_deformed_directions(
        project,
        mesh,
        selected_clip,
        time_seconds,
        &mesh.decoded.tangents,
    )
}

fn mesh_deformed_bitangents(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> Vec<[f32; 3]> {
    mesh_deformed_directions(
        project,
        mesh,
        selected_clip,
        time_seconds,
        &mesh.decoded.bitangents,
    )
}

fn mesh_deformed_directions(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
    source: &[[f32; 3]],
) -> Vec<[f32; 3]> {
    let Some((animation, rig, object_index)) = viewer_skin_context(project, mesh, selected_clip) else {
        return source.to_vec();
    };
    let cluster_matrices = sample_cluster_matrices(animation, rig, object_index, time_seconds);
    if cluster_matrices.is_empty() {
        return source.to_vec();
    }

    let mut output = Vec::with_capacity(source.len());
    let mut link_cursor = 0usize;
    for (vertex_index, direction) in source.iter().copied().enumerate() {
        let count = rig.link_map_count.get(vertex_index).copied().unwrap_or(0) as usize;
        if count == 0 {
            output.push(direction);
            continue;
        }
        let mut accumulated = [0.0f32; 3];
        let mut total_weight = 0.0f32;
        for offset in 0..count {
            let cluster_index = rig
                .link_map_cluster_indices
                .get(link_cursor + offset)
                .copied()
                .unwrap_or(0) as usize;
            let weight = rig
                .link_map_weights
                .get(link_cursor + offset)
                .copied()
                .unwrap_or(0.0);
            if let Some(matrix) = cluster_matrices.get(cluster_index) {
                let transformed = apply_direction(Some(*matrix), direction);
                accumulated[0] += transformed[0] * weight;
                accumulated[1] += transformed[1] * weight;
                accumulated[2] += transformed[2] * weight;
                total_weight += weight;
            }
        }
        link_cursor += count;
        if total_weight > f32::EPSILON {
            output.push(normalize3([
                accumulated[0] / total_weight,
                accumulated[1] / total_weight,
                accumulated[2] / total_weight,
            ]));
        } else {
            output.push(direction);
        }
    }
    output
}

fn viewer_skin_context<'a>(
    project: &'a ProjectDocument,
    mesh: &'a crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
) -> Option<(
    &'a crate::animation::ParsedAnimation,
    &'a crate::animation::SkinningRig,
    usize,
)> {
    let clip_index = selected_clip?;
    let object_index = mesh.animated_object_index?;
    let rig_index = mesh.skinning_rig_index?;
    let animations = project.animations.as_ref()?;
    let animation = animations.animations.get(clip_index)?;
    let rig = animations.skinning_rigs.get(rig_index)?;
    Some((animation, rig, object_index))
}

fn sample_cluster_matrices(
    animation: &crate::animation::ParsedAnimation,
    rig: &crate::animation::SkinningRig,
    mesh_object_index: usize,
    time_seconds: f32,
) -> Vec<[f32; 16]> {
    let Some(mesh_object) = animation.find_object(mesh_object_index) else {
        return Vec::new();
    };
    let mesh_model_part_index = mesh_object.desc.model_part_index;
    let mesh_model_part = &animation.animated_objects[mesh_model_part_index];
    let frame_blend = (time_seconds * mesh_model_part.desc.model_part_fps).fract();
    let frame0 = animation
        .get_object_animation_frame_percent(mesh_model_part_index, time_seconds)
        .floor();
    let frame1 = frame0 + 1.0;
    rig.clusters
        .iter()
        .map(|cluster| {
            let matrix0 =
                solve_cluster_matrix_at_frame(animation, cluster, mesh_model_part_index, frame0);
            let matrix1 =
                solve_cluster_matrix_at_frame(animation, cluster, mesh_model_part_index, frame1);
            lerp_matrix4(&matrix0, &matrix1, frame_blend)
        })
        .collect()
}

fn solve_cluster_matrix_at_frame(
    animation: &crate::animation::ParsedAnimation,
    cluster: &crate::animation::SkinningCluster,
    mesh_model_part_index: usize,
    frame: f32,
) -> [f32; 16] {
    if cluster.link_mode == 1 {
        let link = animation.evaluate_model_part_transform_at_frame(
            cluster.link_object_index as usize,
            frame,
        );
        let link_base = mul_matrix4_view(&link, &cluster.default_cluster_base_transform);
        let Some(default_associate_world_transform) = cluster.default_associate_world_transform else {
            return identity_matrix();
        };
        let Some(associate_inverse) = invert_matrix4_view(&default_associate_world_transform) else {
            return identity_matrix();
        };
        let tmp = mul_matrix4_view(&associate_inverse, &link_base);
        let tmp = mul_matrix4_view(&associate_inverse, &tmp);
        let Some(cluster_world_inverse) = invert_matrix4_view(&cluster.default_cluster_world_transform) else {
            return identity_matrix();
        };
        mul_matrix4_view(&cluster_world_inverse, &tmp)
    } else {
        let link = animation.evaluate_model_part_transform_at_frame(
            cluster.link_object_index as usize,
            frame,
        );
        let mesh = animation.evaluate_model_part_transform_at_frame(mesh_model_part_index, frame);
        let Some(mesh_inverse) = invert_matrix4_view(&mesh) else {
            return identity_matrix();
        };
        let delta = mul_matrix4_view(&mesh_inverse, &link);
        mul_matrix4_view(&delta, &cluster.default_cluster_base_transform)
    }
}

fn mul_matrix4_view(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
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

fn lerp_matrix4(a: &[f32; 16], b: &[f32; 16], t: f32) -> [f32; 16] {
    let mut result = [0.0; 16];
    for i in 0..16 {
        result[i] = a[i] * (1.0 - t) + b[i] * t;
    }
    result
}

fn invert_matrix4_view(matrix: &[f32; 16]) -> Option<[f32; 16]> {
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

fn mesh_is_visible(
    project: &ProjectDocument,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> bool {
    let Some(clip_index) = selected_clip else {
        return true;
    };
    let Some(object_index) = mesh.animated_object_index else {
        return true;
    };
    let Some(animations) = &project.animations else {
        return true;
    };
    let Some(animation) = animations.animations.get(clip_index) else {
        return true;
    };
    if object_index >= animation.animated_objects.len() {
        return true;
    }
    let frame = animation.get_object_animation_frame_percent(object_index, time_seconds);
    animation.is_visible_at_frame_percent(object_index, frame)
}

fn material_uv_offset(
    project: &ProjectDocument,
    material: &crate::runtime::RuntimeMaterial,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> [f32; 2] {
    let Some(clip_index) = selected_clip else {
        return [0.0, 0.0];
    };
    let Some(object_index) = material.animated_object_index else {
        return [0.0, 0.0];
    };
    let Some(animations) = &project.animations else {
        return [0.0, 0.0];
    };
    let Some(animation) = animations.animations.get(clip_index) else {
        return [0.0, 0.0];
    };
    let Some(object) = animation.animated_objects.get(object_index) else {
        return [0.0, 0.0];
    };
    let frame = animation.get_object_animation_frame_percent(object_index, time_seconds);
    [
        object.sample_named_property("OffsetU", frame, 0.0).unwrap_or(0.0),
        object.sample_named_property("OffsetV", frame, 0.0).unwrap_or(0.0),
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= f32::EPSILON {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn distance3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn build_texture_atlas(project: &ProjectDocument) -> TextureAtlasData {
    let tile_size = 128u32;
    let background_tile = build_background_tile(project, tile_size);
    let extra_tiles = 1 + u32::from(background_tile.is_some());
    let tile_count = (project.runtime.materials.len() as u32).max(1) + extra_tiles;
    let columns = (tile_count as f32).sqrt().ceil() as u32;
    let rows = tile_count.div_ceil(columns);
    let width = columns * tile_size;
    let height = rows * tile_size;
    let mut albedo_atlas =
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
    let mut normal_atlas =
        image::RgbaImage::from_pixel(width, height, image::Rgba([128, 128, 255, 255]));
    let mut reflectivity_atlas =
        image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    let mut extras_atlas =
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
    let mut material_rects =
        vec![[0.0, 0.0, tile_size as f32 / width as f32, tile_size as f32 / height as f32]];
    let mut background_rect = None;

    if let Some(background) = background_tile {
        let tile_index = tile_count - 1;
        let column = tile_index % columns;
        let row = tile_index / columns;
        let x = column * tile_size;
        let y = row * tile_size;
        image::imageops::overlay(&mut albedo_atlas, &background, x.into(), y.into());
        background_rect = Some([
            x as f32 / width as f32,
            y as f32 / height as f32,
            tile_size as f32 / width as f32,
            tile_size as f32 / height as f32,
        ]);
    }

    for (material_index, material) in project.runtime.materials.iter().enumerate() {
        let tile_index = material_index as u32 + 1;
        let column = tile_index % columns;
        let row = tile_index / columns;
        let x = column * tile_size;
        let y = row * tile_size;
        let tiles = build_material_tiles(project, material, tile_size);
        image::imageops::overlay(&mut albedo_atlas, &tiles.albedo, x.into(), y.into());
        image::imageops::overlay(&mut normal_atlas, &tiles.normal, x.into(), y.into());
        image::imageops::overlay(
            &mut reflectivity_atlas,
            &tiles.reflectivity,
            x.into(),
            y.into(),
        );
        image::imageops::overlay(&mut extras_atlas, &tiles.extras, x.into(), y.into());
        material_rects.push([
            x as f32 / width as f32,
            y as f32 / height as f32,
            tile_size as f32 / width as f32,
            tile_size as f32 / height as f32,
        ]);
    }

    TextureAtlasData {
        albedo_rgba: albedo_atlas.into_raw(),
        normal_rgba: normal_atlas.into_raw(),
        reflectivity_rgba: reflectivity_atlas.into_raw(),
        extras_rgba: extras_atlas.into_raw(),
        width,
        height,
        material_rects,
        background_rect,
    }
}

fn build_background_tile(project: &ProjectDocument, tile_size: u32) -> Option<image::RgbaImage> {
    let sky = project.scene.sky.as_ref()?;
    if sky.background_mode.unwrap_or(0) < 1 {
        return None;
    }
    let image_name = sky
        .image_url
        .as_deref()
        .and_then(|name| resolve_archive_image_name(project, name))
        .or_else(|| resolve_archive_image_name(project, "sky.png"))?;
    load_resized_rgba(project, &image_name, tile_size).map(|mut image| {
        let brightness = sky.background_brightness.unwrap_or(1.0).clamp(0.0, 4.0);
        if (brightness - 1.0).abs() > f32::EPSILON {
            for pixel in image.pixels_mut() {
                pixel[0] = ((pixel[0] as f32 * brightness).clamp(0.0, 255.0)) as u8;
                pixel[1] = ((pixel[1] as f32 * brightness).clamp(0.0, 255.0)) as u8;
                pixel[2] = ((pixel[2] as f32 * brightness).clamp(0.0, 255.0)) as u8;
            }
        }
        image
    })
}

fn resolve_archive_image_name(project: &ProjectDocument, name: &str) -> Option<String> {
    if project.archive.get(name).is_some() {
        return Some(name.to_string());
    }
    let stripped = name.split('?').next().unwrap_or(name);
    let basename = stripped.rsplit(['/', '\\']).next().unwrap_or(stripped);
    if project.archive.get(basename).is_some() {
        return Some(basename.to_string());
    }
    None
}

struct MaterialTiles {
    albedo: image::RgbaImage,
    normal: image::RgbaImage,
    reflectivity: image::RgbaImage,
    extras: image::RgbaImage,
}

fn build_material_tiles(
    project: &ProjectDocument,
    material: &crate::runtime::RuntimeMaterial,
    tile_size: u32,
) -> MaterialTiles {
    let mut albedo = image::RgbaImage::from_pixel(
        tile_size,
        tile_size,
        image::Rgba([
            (material.preview_color[0] * 255.0) as u8,
            (material.preview_color[1] * 255.0) as u8,
            (material.preview_color[2] * 255.0) as u8,
            255,
        ]),
    );
    let mut normal =
        image::RgbaImage::from_pixel(tile_size, tile_size, image::Rgba([128, 128, 255, 255]));
    let mut reflectivity = image::RgbaImage::from_pixel(
        tile_size,
        tile_size,
        image::Rgba([
            (material.preview_metallic * 255.0) as u8,
            (material.preview_metallic * 255.0) as u8,
            (material.preview_metallic * 255.0) as u8,
            ((1.0 - material.preview_roughness).clamp(0.0, 1.0) * 255.0) as u8,
        ]),
    );
    let mut extras =
        image::RgbaImage::from_pixel(tile_size, tile_size, image::Rgba([255, 255, 255, 255]));

    if let Some(tile) = load_resized_rgba(project, &material.desc.albedo_tex, tile_size) {
        albedo = tile;
    }
    if let Some(alpha_name) = &material.desc.alpha_tex {
        if let Some(alpha) = load_resized_luma(project, alpha_name, tile_size) {
            for y in 0..tile_size {
                for x in 0..tile_size {
                    albedo.get_pixel_mut(x, y)[3] = alpha.get_pixel(x, y)[0];
                }
            }
        }
    }
    if let Some(normal_name) = &material.desc.normal_tex {
        if let Some(tile) = load_resized_rgba(project, normal_name, tile_size) {
            normal = tile;
        }
    }
    if let Some(name) = &material.desc.reflectivity_tex {
        if let Some(tile) = load_resized_rgba(project, name, tile_size) {
            reflectivity = tile;
        }
    }
    if let Some(name) = &material.desc.gloss_tex {
        if let Some(gloss) = load_resized_luma(project, name, tile_size) {
            for y in 0..tile_size {
                for x in 0..tile_size {
                    reflectivity.get_pixel_mut(x, y)[3] = gloss.get_pixel(x, y)[0];
                }
            }
        }
    }
    if let Some(name) = &material.desc.extras_tex {
        if let Some(tile) = load_resized_rgba(project, name, tile_size) {
            extras = tile;
        }
    }
    if let Some(name) = &material.desc.extras_tex_a {
        if let Some(alpha) = load_resized_luma(project, name, tile_size) {
            for y in 0..tile_size {
                for x in 0..tile_size {
                    extras.get_pixel_mut(x, y)[3] = alpha.get_pixel(x, y)[0];
                }
            }
        }
    }

    MaterialTiles {
        albedo,
        normal,
        reflectivity,
        extras,
    }
}

fn load_resized_rgba(
    project: &ProjectDocument,
    name: &str,
    tile_size: u32,
) -> Option<image::RgbaImage> {
    let entry = project.archive.get(name)?;
    let image = image::load_from_memory(&entry.data).ok()?;
    Some(
        image
            .resize_exact(tile_size, tile_size, FilterType::Triangle)
            .to_rgba8(),
    )
}

fn load_resized_luma(
    project: &ProjectDocument,
    name: &str,
    tile_size: u32,
) -> Option<image::GrayImage> {
    let entry = project.archive.get(name)?;
    let image = image::load_from_memory(&entry.data).ok()?;
    Some(
        image
            .resize_exact(tile_size, tile_size, FilterType::Triangle)
            .to_luma8(),
    )
}
