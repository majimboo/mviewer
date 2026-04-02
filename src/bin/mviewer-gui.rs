#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::sync::Arc;

use eframe::{Renderer, egui};
use mviewer::gui::app::MviewerGuiApp;
use mviewer::gui::icon::load_app_icon;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        renderer: Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_icon(Arc::new(load_app_icon())),
        ..Default::default()
    };

    eframe::run_native(
        "mviewer GUI",
        options,
        Box::new(|cc| Ok(Box::new(MviewerGuiApp::new(cc)))),
    )
}
