//! Embedded canonical Aevra assets; no network image loading.
use eframe::egui;

const LOGO: &[u8] = include_bytes!("../../../packaging/icons/aevra-logo.png");
const ICON: &[u8] = include_bytes!("../../../packaging/icons/aevra-app-icon.png");

pub fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    let image = image::load_from_memory(LOGO)
        .expect("embedded Aevra logo")
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    ctx.load_texture(
        "aevra-logo",
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    )
}

pub fn icon(size: u32) -> egui::IconData {
    let image = image::load_from_memory(ICON)
        .expect("embedded Aevra icon")
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    egui::IconData {
        rgba: image.into_raw(),
        width: size,
        height: size,
    }
}
