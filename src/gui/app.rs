use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::{CreationContext, egui};
use rfd::FileDialog;

use crate::{ExportOptions, ProjectDocument, default_output_dir, export_project, load_project};
use crate::gui::viewer::RuntimeViewer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum NavTab {
    #[default]
    Scene,
    Materials,
    Animations,
    Export,
}

pub struct MviewerGuiApp {
    project: Option<ProjectDocument>,
    runtime_viewer: RuntimeViewer,
    mesh_export_selection: Vec<bool>,
    mesh_preview_visibility: Vec<bool>,
    include_cameras: bool,
    include_lights: bool,
    include_animations: bool,
    output_dir: String,
    status: String,
    active_tab: NavTab,
    selected_mesh_index: Option<usize>,
    selected_material_index: Option<usize>,
    selected_animation_index: Option<usize>,
    animation_time: f32,
    animation_playing: bool,
}

impl Default for MviewerGuiApp {
    fn default() -> Self {
        Self {
            project: None,
            runtime_viewer: RuntimeViewer::new(None),
            mesh_export_selection: Vec::new(),
            mesh_preview_visibility: Vec::new(),
            include_cameras: true,
            include_lights: true,
            include_animations: true,
            output_dir: String::new(),
            status: String::new(),
            active_tab: NavTab::Scene,
            selected_mesh_index: None,
            selected_material_index: None,
            selected_animation_index: None,
            animation_time: 0.0,
            animation_playing: false,
        }
    }
}

impl MviewerGuiApp {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        Self {
            runtime_viewer: RuntimeViewer::new(cc.wgpu_render_state.clone()),
            ..Default::default()
        }
    }

    pub fn open_project(&mut self, path: PathBuf) {
        match load_project(&path) {
            Ok(project) => {
                self.output_dir = default_output_dir(&path).display().to_string();
                self.mesh_export_selection = vec![true; project.runtime.meshes.len()];
                self.mesh_preview_visibility = vec![true; project.runtime.meshes.len()];
                self.include_cameras = true;
                self.include_lights = true;
                self.include_animations = true;
                self.selected_mesh_index = (!project.runtime.meshes.is_empty()).then_some(0);
                self.selected_material_index = (!project.runtime.materials.is_empty()).then_some(0);
                self.selected_animation_index = project
                    .animations
                    .as_ref()
                    .and_then(|animations| (!animations.animations.is_empty()).then_some(0));
                self.animation_time = 0.0;
                self.animation_playing = project
                    .scene
                    .anim_data
                    .as_ref()
                    .and_then(|anim| anim.auto_play_anims)
                    .unwrap_or(false);
                self.status = format!("Loaded {}", path.display());
                self.project = Some(project);
                self.active_tab = NavTab::Scene;
            }
            Err(err) => {
                self.status = format!("Load failed: {err:#}");
            }
        }
    }

    pub fn export_selected(&mut self) {
        let Some(project) = &self.project else {
            self.status = "Load a .mview file first.".to_string();
            return;
        };

        let included_meshes: BTreeSet<_> = self
            .mesh_export_selection
            .iter()
            .enumerate()
            .filter_map(|(index, selected)| selected.then_some(index))
            .collect();
        if included_meshes.is_empty() {
            self.status = "Select at least one mesh to export.".to_string();
            return;
        }

        let output_dir = PathBuf::from(self.output_dir.trim());
        match export_project(
            project,
            &output_dir,
            &ExportOptions {
                included_meshes,
                include_cameras: self.include_cameras,
                include_lights: self.include_lights,
                include_animations: self.include_animations,
            },
        ) {
            Ok(report) => {
                self.status = format!(
                    "Exported {} of {} meshes to {}",
                    report.exported_meshes,
                    report.total_meshes,
                    report.output_dir.display()
                );
            }
            Err(err) => {
                self.status = format!("Export failed: {err:#}");
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped_files = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped_files {
            if let Some(path) = file.path {
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("mview"))
                    .unwrap_or(false)
                {
                    self.open_project(path);
                    break;
                }
            }
        }
    }

    fn selected_mesh_count(&self) -> usize {
        self.mesh_export_selection.iter().filter(|selected| **selected).count()
    }

    fn visible_mesh_count(&self) -> usize {
        self.mesh_preview_visibility
            .iter()
            .filter(|selected| **selected)
            .count()
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("mviewer");
            ui.separator();
            if let Some(project) = &self.project {
                ui.label(project.input_path.display().to_string());
            } else {
                ui.label("Open or drop a .mview file");
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let export_enabled = self.project.is_some();
                if ui
                    .add_enabled(export_enabled, egui::Button::new("Export Selected"))
                    .clicked()
                {
                    self.export_selected();
                }
                if ui.button("Choose Output Folder").clicked() {
                    if let Some(path) = FileDialog::new().pick_folder() {
                        self.output_dir = path.display().to_string();
                    }
                }
                if ui.button("Open .mview").clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Marmoset Viewer scene", &["mview"])
                        .pick_file()
                    {
                        self.open_project(path);
                    }
                }
            });
        });
    }

    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Project");
        if let Some(project) = &self.project {
            if let Some(title) = &project.summary.title {
                ui.label(title);
            }
            if let Some(author) = &project.summary.author {
                ui.small(format!("by {author}"));
            }
            ui.small(format!("Visible in viewer: {}", self.visible_mesh_count()));
            ui.small(format!("Selected for export: {}", self.selected_mesh_count()));
        } else {
            ui.label("No scene loaded");
        }

        ui.separator();
        ui.label("Navigate");
        for (tab, label) in [
            (NavTab::Scene, "Scene"),
            (NavTab::Materials, "Materials"),
            (NavTab::Animations, "Animations"),
            (NavTab::Export, "Export"),
        ] {
            ui.selectable_value(&mut self.active_tab, tab, label);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.separator();
            ui.small("Majid Siddiqui");
            ui.hyperlink_to("me@majidarif.com", "mailto:me@majidarif.com");
        });
    }

    fn draw_runtime_panel(&mut self, ui: &mut egui::Ui, project: &ProjectDocument) {
        self.runtime_viewer.draw(
            ui,
            project,
            &self.mesh_preview_visibility,
            self.selected_animation_index,
            self.animation_time,
        );
    }

    fn draw_scene_tab(&mut self, ui: &mut egui::Ui, project: &ProjectDocument) {
        ui.heading("Scene");
        ui.label("Preview visibility and export selection are separated.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Show All In Viewer").clicked() {
                self.mesh_preview_visibility.fill(true);
            }
            if ui.button("Hide All In Viewer").clicked() {
                self.mesh_preview_visibility.fill(false);
            }
            if ui.button("Select All For Export").clicked() {
                self.mesh_export_selection.fill(true);
            }
            if ui.button("Select None For Export").clicked() {
                self.mesh_export_selection.fill(false);
            }
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, mesh) in project.runtime.meshes.iter().enumerate() {
                ui.horizontal(|ui| {
                    if let Some(visible) = self.mesh_preview_visibility.get_mut(index) {
                        ui.checkbox(visible, "");
                    }
                    if let Some(selected) = self.mesh_export_selection.get_mut(index) {
                        ui.checkbox(selected, "");
                    }
                    if ui
                        .selectable_label(self.selected_mesh_index == Some(index), &mesh.desc.name)
                        .clicked()
                    {
                        self.selected_mesh_index = Some(index);
                    }
                    ui.small(format!("v:{} i:{}", mesh.desc.vertex_count, mesh.desc.index_count));
                });
            }
        });
    }

    fn draw_materials_tab(&mut self, ui: &mut egui::Ui, project: &ProjectDocument) {
        ui.heading("Materials");
        ui.columns(2, |columns| {
            columns[0].group(|ui| {
                for (index, material) in project.runtime.materials.iter().enumerate() {
                    if ui
                        .selectable_label(self.selected_material_index == Some(index), &material.desc.name)
                        .clicked()
                    {
                        self.selected_material_index = Some(index);
                    }
                }
            });
            columns[1].group(|ui| {
                if let Some(index) = self.selected_material_index {
                    if let Some(material) = project.runtime.materials.get(index) {
                        ui.heading(&material.desc.name);
                        ui.colored_label(
                            egui::Color32::from_rgb(
                                (material.preview_color[0] * 255.0) as u8,
                                (material.preview_color[1] * 255.0) as u8,
                                (material.preview_color[2] * 255.0) as u8,
                            ),
                            "Preview color",
                        );
                        for texture in &material.textures {
                            ui.label(format!("{}: {}", texture.slot, texture.name));
                        }
                    }
                } else {
                    ui.label("Select a material.");
                }
            });
        });
    }

    fn draw_animations_tab(&mut self, ui: &mut egui::Ui, project: &ProjectDocument) {
        ui.heading("Animations");
        let Some(animations) = &project.animations else {
            ui.label("No animation data found.");
            return;
        };

        ui.columns(2, |columns| {
            columns[0].group(|ui| {
                for (index, clip) in animations.animations.iter().enumerate() {
                    if ui
                        .selectable_label(self.selected_animation_index == Some(index), &clip.desc.name)
                        .clicked()
                    {
                        self.selected_animation_index = Some(index);
                        self.animation_time = 0.0;
                    }
                }
            });
            columns[1].group(|ui| {
                if let Some(index) = self.selected_animation_index {
                    if let Some(clip) = animations.animations.get(index) {
                        ui.heading(&clip.desc.name);
                        ui.label(format!("Length: {:.2}s", clip.desc.length));
                        ui.label(format!("Frames: {}", clip.desc.total_frames));
                        ui.label(format!("Animated objects: {}", clip.animated_objects.len()));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button(if self.animation_playing { "Pause" } else { "Play" })
                                .clicked()
                            {
                                self.animation_playing = !self.animation_playing;
                            }
                            if ui.button("Reset").clicked() {
                                self.animation_time = 0.0;
                            }
                        });
                        if clip.desc.length > f32::EPSILON {
                            ui.add(
                                egui::Slider::new(&mut self.animation_time, 0.0..=clip.desc.length)
                                    .text("Time"),
                            );
                        }
                    }
                } else {
                    ui.label("Select an animation clip.");
                }
            });
        });
    }

    fn draw_export_tab(&mut self, ui: &mut egui::Ui, project: &ProjectDocument) {
        ui.heading("Export");
        ui.group(|ui| {
            ui.label("Output directory");
            ui.text_edit_singleline(&mut self.output_dir);
        });
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.heading("Included content");
            ui.checkbox(&mut self.include_cameras, "Include cameras");
            ui.checkbox(&mut self.include_lights, "Include lights");
            ui.checkbox(&mut self.include_animations, "Include animations");
            ui.label(format!(
                "Selected meshes: {} / {}",
                self.selected_mesh_count(),
                project.runtime.meshes.len()
            ));
        });
        ui.add_space(12.0);
        if ui.button("Export Selected Scene").clicked() {
            self.export_selected();
        }
    }
}

impl eframe::App for MviewerGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_dropped_files(ctx);

        if self.animation_playing {
            if let Some(project) = &self.project {
                if let Some(animations) = &project.animations {
                    if let Some(index) = self.selected_animation_index {
                        if let Some(clip) = animations.animations.get(index) {
                            if clip.desc.length > f32::EPSILON {
                                self.animation_time =
                                    (self.animation_time + ctx.input(|input| input.stable_dt))
                                        % clip.desc.length.max(0.0001);
                                ctx.request_repaint();
                            }
                        }
                    }
                }
            }
        }

        egui::TopBottomPanel::top("top_bar")
            .exact_height(40.0)
            .show(ctx, |ui| self.draw_toolbar(ui));

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.label(&self.status);
        });

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(230.0)
            .show(ctx, |ui| self.draw_sidebar(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.project.is_none() {
                ui.heading("mviewer GUI");
                ui.label("Open or drag a `.mview` file to begin.");
                ui.separator();
                ui.label("The GUI has been reset and is being rebuilt around the runtime viewer architecture.");
                return;
            }

            let project = self.project.take().expect("project checked above");
            self.draw_runtime_panel(ui, &project);
            ui.add_space(10.0);

            match self.active_tab {
                NavTab::Scene => self.draw_scene_tab(ui, &project),
                NavTab::Materials => self.draw_materials_tab(ui, &project),
                NavTab::Animations => self.draw_animations_tab(ui, &project),
                NavTab::Export => self.draw_export_tab(ui, &project),
            }

            self.project = Some(project);
        });
    }
}
