use std::sync::Mutex;

use bytemuck::{Pod, Zeroable};
use eframe::{egui, egui_wgpu, wgpu};
use image::imageops::FilterType;
use wgpu::util::DeviceExt as _;

use crate::animation::identity_matrix;
use crate::ProjectDocument;

pub struct RuntimeViewer {
    render_state: Option<egui_wgpu::RenderState>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    show_wireframe: bool,
    smooth_shading: bool,
    atlas_cache: Option<SceneTextureAtlas>,
}

impl RuntimeViewer {
    pub fn new(render_state: Option<egui_wgpu::RenderState>) -> Self {
        Self {
            render_state,
            yaw: 0.5,
            pitch: 0.3,
            zoom: 2.8,
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
                        self.yaw = 0.5;
                        self.pitch = 0.3;
                        self.zoom = 2.8;
                    }
                });

                ui.add_space(8.0);
                let desired = egui::vec2(ui.available_width(), 360.0);
                let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::drag());
                if response.dragged() {
                    let delta = response.drag_delta();
                    self.yaw += delta.x * 0.01;
                    self.pitch = (self.pitch - delta.y * 0.01).clamp(-1.35, 1.35);
                }
                if response.hovered() {
                    let scroll = ui.input(|input| input.raw_scroll_delta.y);
                    if scroll.abs() > f32::EPSILON {
                        self.zoom = (self.zoom * (1.0 - scroll * 0.0015)).clamp(0.4, 12.0);
                    }
                }

                ui.painter().rect_filled(rect, 12.0, background_color(project));
                draw_grid(ui.painter(), rect);

                let yaw = self.yaw;
                let pitch = self.pitch;
                let zoom = self.zoom;
                let smooth_shading = self.smooth_shading;
                let atlas = self.ensure_texture_atlas(project).clone();
                let scene = build_scene_mesh(
                    project,
                    &atlas,
                    visible_meshes,
                    rect,
                    yaw,
                    pitch,
                    zoom,
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
    }
}

#[derive(Clone)]
struct SceneGpuMesh {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    wire_segments: Vec<[egui::Pos2; 2]>,
    atlas: TextureAtlasData,
    light_dir: [f32; 3],
    light_color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    view_pos: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    material: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniforms {
    light_dir: [f32; 4],
    light_color: [f32; 4],
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
            contents: bytemuck::bytes_of(&SceneUniforms {
                light_dir: [
                    self.mesh.light_dir[0],
                    self.mesh.light_dir[1],
                    self.mesh.light_dir[2],
                    0.0,
                ],
                light_color: [
                    self.mesh.light_color[0],
                    self.mesh.light_color[1],
                    self.mesh.light_color[2],
                    0.0,
                ],
            }),
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
}

struct SceneTextureAtlas {
    project_key: String,
    data: TextureAtlasData,
}

impl RuntimeViewer {
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
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) view_pos: vec3<f32>,
    @location(3) normal: vec3<f32>,
    @location(4) tangent: vec3<f32>,
    @location(5) bitangent: vec3<f32>,
    @location(6) material: vec4<f32>,
};

struct SceneUniforms {
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
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
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) view_pos: vec3<f32>,
    @location(4) normal: vec3<f32>,
    @location(5) tangent: vec3<f32>,
    @location(6) bitangent: vec3<f32>,
    @location(7) material: vec4<f32>
) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    out.view_pos = view_pos;
    out.normal = normal;
    out.tangent = tangent;
    out.bitangent = bitangent;
    out.material = material;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo_texel = textureSample(albedo_texture, atlas_sampler, in.uv);
    let normal_texel = textureSample(normal_texture, atlas_sampler, in.uv).xyz;
    let reflectivity_texel = textureSample(reflectivity_texture, atlas_sampler, in.uv);
    let extras_texel = textureSample(extras_texture, atlas_sampler, in.uv);

    let albedo = albedo_texel.rgb * albedo_texel.rgb;
    let reflectivity = reflectivity_texel.rgb * reflectivity_texel.rgb;
    let gloss = reflectivity_texel.a;
    let occlusion = extras_texel.r * extras_texel.r;
    let emissive = extras_texel.rgb * extras_texel.rgb * in.material.z;

    let mapped = normalize(normal_texel * 2.0 - vec3<f32>(1.0, 1.0, 1.0));
    let t = normalize(in.tangent);
    let b = normalize(in.bitangent);
    let n = normalize(in.normal);
    let shading_normal = normalize(t * mapped.x + b * mapped.y + n * mapped.z);

    let light_dir = normalize(-scene_uniforms.light_dir.xyz);
    let view_dir = normalize(-in.view_pos);
    let half_vec = normalize(light_dir + view_dir);

    let ndotl = max(dot(shading_normal, light_dir), 0.0);
    let ndoth = max(dot(shading_normal, half_vec), 0.0);
    let ambient = 0.12;
    let roughness = clamp(in.material.y, 0.04, 1.0);
    let shininess = mix(96.0, 6.0, roughness);
    let specular_strength = pow(ndoth, shininess) * (0.04 + max(max(reflectivity.r, reflectivity.g), reflectivity.b));
    let fresnel_factor = pow(1.0 - max(dot(view_dir, shading_normal), 0.0), 5.0);
    let fresnel = reflectivity + (vec3<f32>(1.0, 1.0, 1.0) - reflectivity) * fresnel_factor;

    let lit = albedo * (ambient + ndotl * occlusion) +
        fresnel * specular_strength * scene_uniforms.light_color.rgb +
        emissive;
    let alpha = albedo_texel.a * in.material.w;
    return vec4<f32>(lit, alpha);
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
    yaw: f32,
    pitch: f32,
    zoom: f32,
    smooth_shading: bool,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> SceneGpuMesh {
    let mut scene_vertices = Vec::new();
    let mut scene_indices = Vec::new();
    let mut wire_segments = Vec::new();

    let bounds = scene_bounds(project, visible_meshes);
    let center = [
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
            radius = radius.max(distance3(world, center));
        }
    }

    let focal = rect.width().min(rect.height()) * 0.5;
    let camera_distance = radius * zoom + 0.0001;
    let (scene_light_dir, light_color) = scene_light(project);
    let light_dir = orbit_direction(scene_light_dir, yaw, pitch);

    for (mesh_index, mesh) in project.runtime.meshes.iter().enumerate() {
        if !visible_meshes.get(mesh_index).copied().unwrap_or(false) {
            continue;
        }
        if !mesh_is_visible(project, mesh, selected_clip, time_seconds) {
            continue;
        }
        let mesh_transform = mesh_world_transform(project, mesh_index, selected_clip, time_seconds);
        let material_preview = mesh_preview_material(project, mesh);
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

        let mut faces = Vec::new();
        for triangle in mesh.decoded.indices.chunks_exact(3) {
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
            let normal = if smooth_shading {
                normalize3([
                    normals[ia][0] + normals[ib][0] + normals[ic][0],
                    normals[ia][1] + normals[ib][1] + normals[ic][1],
                    normals[ia][2] + normals[ib][2] + normals[ic][2],
                ])
            } else {
                let aw = apply_transform(Some(mesh_transform), deformed_positions[ia]);
                let bw = apply_transform(Some(mesh_transform), deformed_positions[ib]);
                let cw = apply_transform(Some(mesh_transform), deformed_positions[ic]);
                normalize3(cross3(sub3(bw, aw), sub3(cw, aw)))
            };
            faces.push(([ia, ib, ic], preview_lighting(material_preview, normal, light_dir, light_color)));
            wire_segments.push([a, b]);
            wire_segments.push([b, c]);
            wire_segments.push([c, a]);
        }

        for (triangle, color) in faces {
            let base = scene_vertices.len() as u32;
            let (uv_rect, uv_offset) = mesh_uv_rect(project, atlas, mesh, selected_clip, time_seconds);
            for index in triangle {
                let screen = projected[index].expect("projected vertex");
                let uv = mesh
                    .decoded
                    .texcoords
                    .get(index)
                    .copied()
                    .unwrap_or([0.5, 0.5]);
                scene_vertices.push(GpuVertex {
                    position: screen_to_ndc(screen, rect),
                    uv: remap_uv(uv, uv_rect, uv_offset),
                    color,
                    view_pos: view_positions[index],
                    normal: normals[index],
                    tangent: tangents[index],
                    bitangent: bitangents[index],
                    material: [
                        material_preview.metallic,
                        material_preview.roughness,
                        material_preview.emissive,
                        material_preview.alpha,
                    ],
                });
            }
            scene_indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
    }

    SceneGpuMesh {
        vertices: scene_vertices,
        indices: scene_indices,
        wire_segments,
        atlas: atlas.clone(),
        light_dir,
        light_color,
    }
}

fn draw_scene_fallback(painter: &egui::Painter, scene: &SceneGpuMesh) {
    let mut mesh = egui::Mesh::default();
    for vertex in &scene.vertices {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(vertex.position[0], vertex.position[1]),
            uv: egui::pos2(vertex.uv[0], vertex.uv[1]),
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
        .map(|material| MaterialPreview {
            base_color: material.preview_color,
            metallic: material.preview_metallic,
            roughness: material.preview_roughness,
            emissive: material.preview_emissive,
            alpha: if material.desc.blend.as_deref() == Some("alpha") { 0.82 } else { 1.0 },
        })
        .unwrap_or(MaterialPreview {
            base_color: [0.82, 0.56, 0.34],
            metallic: 0.0,
            roughness: 0.85,
            emissive: 0.0,
            alpha: 1.0,
        })
}

fn background_color(project: &ProjectDocument) -> egui::Color32 {
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

fn scene_light(project: &ProjectDocument) -> ([f32; 3], [f32; 3]) {
    if let Some(lights) = &project.scene.lights {
        if let Some(directions) = &lights.directions {
            if directions.len() >= 3 {
                let dir = normalize3([directions[0], directions[1], directions[2]]);
                let color = lights
                    .colors
                    .as_ref()
                    .filter(|colors| colors.len() >= 3)
                    .map(|colors| {
                        [
                            colors[0].clamp(0.0, 4.0),
                            colors[1].clamp(0.0, 4.0),
                            colors[2].clamp(0.0, 4.0),
                        ]
                    })
                    .unwrap_or([1.0, 1.0, 1.0]);
                return (dir, color);
            }
        }
    }
    (normalize3([0.35, 0.65, -1.0]), [1.0, 1.0, 1.0])
}

fn preview_lighting(
    material: MaterialPreview,
    normal: [f32; 3],
    light_dir: [f32; 3],
    light_color: [f32; 3],
) -> [f32; 4] {
    let ndotl = dot3(normal, [-light_dir[0], -light_dir[1], -light_dir[2]]).max(0.0);
    let diffuse = 0.12 + ndotl * (1.0 - material.metallic * 0.35);
    let specular = ndotl.powf((1.0 - material.roughness).clamp(0.05, 1.0) * 24.0)
        * (0.08 + material.metallic * 0.45);
    let lit = [
        (material.base_color[0] * diffuse + specular * light_color[0] + material.emissive * 0.25)
            .clamp(0.0, 1.0),
        (material.base_color[1] * diffuse + specular * light_color[1] + material.emissive * 0.25)
            .clamp(0.0, 1.0),
        (material.base_color[2] * diffuse + specular * light_color[2] + material.emissive * 0.25)
            .clamp(0.0, 1.0),
    ];
    preview_color(lit, material.alpha)
}

fn mesh_uv_rect(
    project: &ProjectDocument,
    atlas: &TextureAtlasData,
    mesh: &crate::runtime::RuntimeMesh,
    selected_clip: Option<usize>,
    time_seconds: f32,
) -> ([f32; 4], [f32; 2]) {
    let rect = mesh
        .material_indices
        .first()
        .and_then(|index| atlas.material_rects.get(*index))
        .copied()
        .unwrap_or_else(|| atlas.material_rects.first().copied().unwrap_or([0.0, 0.0, 1.0, 1.0]));
    let uv_offset = mesh
        .material_indices
        .first()
        .and_then(|index| project.runtime.materials.get(*index))
        .map(|material| material_uv_offset(project, material, selected_clip, time_seconds))
        .unwrap_or([0.0, 0.0]);
    (rect, uv_offset)
}

fn remap_uv(uv: [f32; 2], rect: [f32; 4], offset: [f32; 2]) -> [f32; 2] {
    let wrapped_u = (uv[0] + offset[0]).rem_euclid(1.0);
    let wrapped_v = (uv[1] + offset[1]).rem_euclid(1.0);
    [
        rect[0] + wrapped_u * rect[2],
        rect[1] + wrapped_v * rect[3],
    ]
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
    let tile_count = (project.runtime.materials.len() as u32).max(1) + 1;
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
    }
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
