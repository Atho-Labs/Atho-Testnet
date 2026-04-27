use crate::app::{widgets, DesktopApp};
use crate::resources;
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(280.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Use this form to request payments. All fields are optional.")
                    .size(13.0),
            );
        });
        ui.add_space(10.0);

        egui::Grid::new("receive_form")
            .num_columns(2)
            .spacing([12.0, 9.0])
            .min_col_width(120.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Label:").size(13.0).strong());
                ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut app.receive_label),
                );
                ui.end_row();

                ui.label(egui::RichText::new("Amount:").size(13.0).strong());
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [160.0, 30.0],
                        egui::TextEdit::singleline(&mut app.receive_amount),
                    );
                    let mut receive_unit = 0usize;
                    egui::ComboBox::from_id_source("receive_unit")
                        .width(120.0)
                        .selected_text("atoms")
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut receive_unit, 0, "atoms");
                        });
                    widgets::muted_label(ui, "Base56");
                });
                ui.end_row();

                ui.label(egui::RichText::new("Message:").size(13.0).strong());
                ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut app.receive_message),
                );
                ui.end_row();
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add_sized(
                    [190.0, 30.0],
                    egui::Button::image_and_text(
                        resources::receive_icon(14.0),
                        "Create new receiving address",
                    ),
                )
                .clicked()
            {
                app.create_receive_request();
            }
            if ui
                .add_sized(
                    [84.0, 30.0],
                    egui::Button::image_and_text(resources::clear_icon(14.0), "Clear"),
                )
                .clicked()
            {
                app.receive_label.clear();
                app.receive_amount.clear();
                app.receive_message.clear();
            }
            if !app.current_receive_address_text().is_empty() {
                if widgets::icon_button(ui, resources::copy_icon(15.0), "Copy current address")
                    .clicked()
                {
                    DesktopApp::copy_text(ui, app.current_receive_address_text());
                }
            }
        });
        if !app.current_receive_address_text().is_empty() {
            ui.add_space(10.0);
            widgets::muted_label(ui, "Current receiving address");
            let mut address_text = app.current_receive_address_text();
            ui.add(
                egui::TextEdit::singleline(&mut address_text)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        }
    });

    ui.add_space(10.0);
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(260.0);
        widgets::section_header(ui, "Requested payments history");
        ui.add_space(12.0);

        egui::Grid::new("requested_payments_header")
            .num_columns(4)
            .spacing([12.0, 8.0])
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.strong("Date");
                ui.strong("Label");
                ui.strong("Message");
                ui.strong("Requested");
                ui.end_row();
            });
        ui.separator();
        ui.add_space(8.0);

        if app.requested_payments.is_empty() {
            widgets::muted_label(ui, "No payment requests yet.");
        } else {
            let mut clicked_sequence = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(210.0)
                .show(ui, |ui| {
                    for request in app.requested_payments.iter().rev() {
                        egui::Grid::new(ui.id().with(request.sequence))
                            .num_columns(4)
                            .spacing([12.0, 6.0])
                            .min_col_width(80.0)
                            .show(ui, |ui| {
                                ui.label(format!("#{}", request.sequence));
                                let label = if request.label.is_empty() {
                                    String::from("(no label)")
                                } else {
                                    request.label.clone()
                                };
                                let selected =
                                    app.selected_receive_request == Some(request.sequence);
                                if ui.selectable_label(selected, label).clicked() {
                                    clicked_sequence = Some(request.sequence);
                                }
                                ui.label(if request.message.is_empty() {
                                    String::from("(no message)")
                                } else {
                                    request.message.clone()
                                });
                                ui.label(
                                    request
                                        .amount_atoms
                                        .map(|atoms| format!("{atoms} atoms"))
                                        .unwrap_or_else(|| String::from("(no amount requested)")),
                                );
                                ui.end_row();
                            });
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);
                    }
                });
            if let Some(sequence) = clicked_sequence {
                app.select_receive_request(sequence);
            }
        }

        let selected_request = app.selected_receive_request().cloned();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(selected_request.is_some(), egui::Button::new("Show"))
                .clicked()
            {
                if let Some(request) = &selected_request {
                    DesktopApp::copy_text(ui, request.address.clone());
                }
            }
            if ui
                .add_enabled(selected_request.is_some(), egui::Button::new("Remove"))
                .clicked()
            {
                app.remove_selected_receive_request();
            }
        });
        if let Some(request) = selected_request {
            ui.add_space(8.0);
            widgets::muted_label(ui, "Selected base56 receiving address");
            let mut selected_address = request.address;
            ui.add(
                egui::TextEdit::singleline(&mut selected_address)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        }
    });
}
