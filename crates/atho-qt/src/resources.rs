use eframe::egui::{self, IconData, Image};

fn sized_icon(source: egui::ImageSource<'static>, size: f32) -> Image<'static> {
    Image::new(source).fit_to_exact_size(egui::vec2(size, size))
}

fn sized_tinted_icon(
    source: egui::ImageSource<'static>,
    size: f32,
    tint: egui::Color32,
) -> Image<'static> {
    Image::new(source)
        .fit_to_exact_size(egui::vec2(size, size))
        .tint(tint)
}

fn sized_logo(source: egui::ImageSource<'static>, width: f32, height: f32) -> Image<'static> {
    Image::new(source).fit_to_exact_size(egui::vec2(width, height))
}

pub fn app_icon() -> IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/branding/atho-icon.png"))
        .expect("atho-icon.png must be a valid application icon")
}

pub fn logo_badge(size: f32) -> Image<'static> {
    sized_icon(
        egui::include_image!("../assets/branding/atho-icon.png"),
        size,
    )
}

pub fn logo_mark(width: f32) -> Image<'static> {
    sized_logo(
        egui::include_image!("../assets/branding/atho-mark.png"),
        width,
        width * 1.5,
    )
}

pub fn overview_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/overview.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn send_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/send.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn receive_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/receive.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn history_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/history.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn warning_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/warning.png"),
        size,
        egui::Color32::from_rgb(185, 84, 56),
    )
}

pub fn export_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/export.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn copy_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/editcopy.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn paste_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/editpaste.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn clear_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/remove.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn add_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/add.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn address_book_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/address-book.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn hd_enabled_icon(size: f32) -> Image<'static> {
    sized_tinted_icon(
        egui::include_image!("../assets/icons/hd_enabled.png"),
        size,
        egui::Color32::from_rgb(50, 54, 50),
    )
}

pub fn network_icon(size: f32, connected: bool) -> Image<'static> {
    if connected {
        sized_tinted_icon(
            egui::include_image!("../assets/icons/connect4.png"),
            size,
            egui::Color32::from_rgb(50, 54, 50),
        )
    } else {
        sized_tinted_icon(
            egui::include_image!("../assets/icons/network_disabled.png"),
            size,
            egui::Color32::from_rgb(117, 88, 62),
        )
    }
}

pub fn sync_icon(size: f32, synced: bool) -> Image<'static> {
    if synced {
        sized_tinted_icon(
            egui::include_image!("../assets/icons/synced.png"),
            size,
            egui::Color32::from_rgb(50, 54, 50),
        )
    } else {
        sized_tinted_icon(
            egui::include_image!("../assets/icons/connect0.png"),
            size,
            egui::Color32::from_rgb(117, 88, 62),
        )
    }
}
