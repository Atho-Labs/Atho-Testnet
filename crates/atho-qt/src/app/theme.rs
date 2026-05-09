use super::widgets;
use eframe::egui;

pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "atho_terminal".to_string(),
        egui::FontData::from_static(include_bytes!("../../assets/fonts/RobotoMono-Bold.ttf")),
    );
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        family.insert(0, "atho_terminal".to_string());
    }
    ctx.set_fonts(fonts);
}

pub(crate) fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = widgets::SHELL_BG;
    visuals.window_fill = widgets::PANEL_BG;
    visuals.extreme_bg_color = widgets::SHELL_BG;
    visuals.faint_bg_color = widgets::STATUS_BG;
    visuals.widgets.noninteractive.bg_fill = widgets::PANEL_BG;
    visuals.widgets.noninteractive.fg_stroke.color = widgets::TEXT;
    visuals.widgets.inactive.bg_fill = widgets::PANEL_BG;
    visuals.widgets.inactive.fg_stroke.color = widgets::TEXT;
    visuals.widgets.hovered.bg_fill = widgets::TOOLBAR_ACTIVE;
    visuals.widgets.hovered.fg_stroke.color = widgets::TEXT;
    visuals.widgets.active.bg_fill = widgets::ACCENT_SOFT;
    visuals.widgets.active.fg_stroke.color = widgets::ACCENT;
    visuals.widgets.open.bg_fill = widgets::ACCENT_SOFT;
    visuals.widgets.open.fg_stroke.color = widgets::ACCENT;
    visuals.selection.bg_fill = widgets::ACCENT_SOFT;
    visuals.selection.stroke.color = widgets::ACCENT;
    visuals.hyperlink_color = widgets::ACCENT;
    visuals.override_text_color = Some(widgets::TEXT);
    visuals.window_rounding = egui::Rounding::same(3.0);
    visuals.menu_rounding = egui::Rounding::same(3.0);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.button_padding = egui::vec2(6.0, 4.0);
    style.spacing.item_spacing = egui::vec2(3.0, 3.0);
    style.spacing.menu_margin = egui::Margin::same(4.0);
    style.spacing.interact_size = egui::vec2(26.0, 22.0);
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(17.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(11.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(11.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(10.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(11.5, egui::FontFamily::Monospace),
    );
    ctx.set_style(style);
}
