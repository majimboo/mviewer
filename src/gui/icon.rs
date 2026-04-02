use eframe::egui::IconData;

pub fn load_app_icon() -> IconData {
    let image = image::load_from_memory(include_bytes!("../../marmoset_logos.webp"))
        .expect("embedded GUI icon must decode");
    let mut rgba = image.into_rgba8();
    for pixel in rgba.pixels_mut() {
        if pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245 {
            pixel[3] = 0;
        }
    }
    let (width, height) = rgba.dimensions();
    IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    }
}
