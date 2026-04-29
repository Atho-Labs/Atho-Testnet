use atho_core::constants::ATOMS_PER_ATHO;
use eframe::egui;

pub(crate) const SHELL_BG: egui::Color32 = egui::Color32::from_rgb(238, 239, 237);
pub(crate) const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(247, 248, 246);
pub(crate) const PANEL_STROKE: egui::Color32 = egui::Color32::from_rgb(199, 204, 199);
pub(crate) const MENU_BG: egui::Color32 = egui::Color32::from_rgb(246, 246, 244);
pub(crate) const TOOLBAR_BG: egui::Color32 = egui::Color32::from_rgb(243, 244, 242);
pub(crate) const TOOLBAR_ACTIVE: egui::Color32 = egui::Color32::from_rgb(217, 230, 224);
pub(crate) const STATUS_BG: egui::Color32 = egui::Color32::from_rgb(241, 242, 240);
pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(112, 116, 112);
pub(crate) const TEXT: egui::Color32 = egui::Color32::from_rgb(50, 54, 50);
pub(crate) const ACCENT: egui::Color32 = egui::Color32::from_rgb(26, 112, 70);
pub(crate) const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(226, 242, 232);

pub(crate) fn shell_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(SHELL_BG)
        .inner_margin(egui::Margin::same(6.0))
}

pub(crate) fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL_BG)
        .stroke(egui::Stroke::new(1.0, PANEL_STROKE))
        .inner_margin(egui::Margin::same(8.0))
}

pub(crate) fn dialog_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL_BG)
        .stroke(egui::Stroke::new(1.0, PANEL_STROKE))
        .inner_margin(egui::Margin::same(10.0))
}

pub(crate) fn toolbar_tab(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    icon: egui::Image<'static>,
) -> egui::Response {
    let fill = if selected { TOOLBAR_ACTIVE } else { TOOLBAR_BG };
    let stroke = if selected {
        egui::Stroke::new(1.0, ACCENT)
    } else {
        egui::Stroke::new(1.0, PANEL_STROKE)
    };

    ui.add_sized(
        [126.0, 52.0],
        egui::Button::image_and_text(icon, egui::RichText::new(label).size(12.0).color(TEXT))
            .fill(fill)
            .stroke(stroke)
            .frame(true),
    )
}

pub(crate) fn compact_tab(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    width: f32,
) -> egui::Response {
    let fill = if selected { TOOLBAR_ACTIVE } else { PANEL_BG };
    let stroke = if selected {
        egui::Stroke::new(1.0, ACCENT)
    } else {
        egui::Stroke::new(1.0, PANEL_STROKE)
    };

    ui.add_sized(
        [width, 28.0],
        egui::Button::new(egui::RichText::new(label).size(11.5).color(TEXT))
            .fill(fill)
            .stroke(stroke)
            .frame(true),
    )
}

pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    icon: egui::Image<'static>,
    tooltip: &str,
) -> egui::Response {
    ui.add(
        egui::Button::image(icon)
            .frame(true)
            .fill(PANEL_BG)
            .stroke(egui::Stroke::new(1.0, PANEL_STROKE)),
    )
    .on_hover_text(tooltip)
}

pub(crate) fn section_header(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).size(14.0).strong().color(TEXT));
}

pub(crate) fn row_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(13.0).color(TEXT));
}

pub(crate) fn row_value(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(13.0).strong().color(ACCENT));
}

pub(crate) fn row_value_signed(ui: &mut egui::Ui, atoms: i128) {
    let color = if atoms < 0 {
        egui::Color32::from_rgb(153, 64, 64)
    } else {
        ACCENT
    };
    ui.label(
        egui::RichText::new(format_signed_atoms(atoms))
            .size(13.0)
            .strong()
            .color(color),
    );
}

pub(crate) fn muted_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(MUTED));
}

pub(crate) fn elided_label(ui: &mut egui::Ui, text: &str, max_chars: usize) -> egui::Response {
    let display = elide_text(text, max_chars);
    let response = ui.label(egui::RichText::new(display.clone()).size(11.0).color(TEXT));
    if display != text {
        response.on_hover_text(text)
    } else {
        response
    }
}

pub(crate) fn text_input(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    );
}

pub(crate) fn table_header(ui: &mut egui::Ui, headers: &[&str]) {
    egui::Grid::new(ui.id().with("table_header"))
        .num_columns(headers.len())
        .min_col_width(90.0)
        .striped(false)
        .show(ui, |ui| {
            for header in headers {
                ui.label(egui::RichText::new(*header).strong().size(13.0).color(TEXT));
            }
            ui.end_row();
        });
}

pub(crate) fn format_atoms(atoms: u64) -> String {
    let whole = atoms / ATOMS_PER_ATHO;
    let fractional = atoms % ATOMS_PER_ATHO;
    format!("{whole}.{fractional:08} ATHO")
}

pub(crate) fn format_signed_atoms(atoms: i128) -> String {
    if atoms < 0 {
        format!("-{}", format_atoms(atoms.unsigned_abs() as u64))
    } else {
        format_atoms(atoms as u64)
    }
}

pub(crate) fn short_hash(bytes: &[u8]) -> String {
    let hex = hex::encode(bytes);
    if hex.len() <= 16 {
        hex
    } else {
        format!("{}…{}", &hex[..8], &hex[hex.len() - 8..])
    }
}

pub(crate) fn elide_text(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return text.to_owned();
    }
    if max_chars <= 1 {
        return String::from("…");
    }
    if max_chars == 2 {
        return String::from("…");
    }

    let front = (max_chars - 1) / 2;
    let back = max_chars.saturating_sub(front + 1);
    let mut out = String::with_capacity(max_chars);
    for ch in chars.iter().take(front) {
        out.push(*ch);
    }
    out.push('…');
    for ch in chars.iter().skip(chars.len().saturating_sub(back)) {
        out.push(*ch);
    }
    out
}
