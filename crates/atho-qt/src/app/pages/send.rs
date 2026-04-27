use crate::app::{widgets, DesktopApp};
use crate::resources;
use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let available_balance = app.wallet_balance_atoms();

    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(430.0);
        render_send_form(app, ui, available_balance);
        ui.add_space(6.0);
        render_fee_panel(app, ui);
        ui.add_space(6.0);
        widgets::muted_label(ui, &app.send_status);
        ui.add_space(6.0);
        render_send_actions(app, ui, available_balance);
    });
}

fn render_send_form(app: &mut DesktopApp, ui: &mut egui::Ui, available_balance: u64) {
    widgets::panel_frame()
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            egui::Grid::new("send_form")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .min_col_width(96.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Pay To:").size(13.0).strong());
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [ui.available_width() - 102.0, 28.0],
                            egui::TextEdit::singleline(&mut app.send_to)
                                .hint_text("Enter a base56 Atho address"),
                        );
                        if widgets::icon_button(
                            ui,
                            resources::address_book_icon(14.0),
                            "Use current receiving address",
                        )
                        .clicked()
                        {
                            let address = app.current_receive_address_text();
                            if address.is_empty() {
                                app.send_status = String::from("No receiving address available");
                            } else {
                                app.send_to = address;
                            }
                        }
                        if widgets::icon_button(ui, resources::paste_icon(15.0), "Paste address")
                            .clicked()
                        {
                            if let Some(text) = DesktopApp::read_clipboard_text() {
                                app.send_to = text;
                            } else {
                                app.send_status = String::from("Clipboard is empty");
                            }
                        }
                        if widgets::icon_button(ui, resources::clear_icon(15.0), "Clear address")
                            .clicked()
                        {
                            app.send_to.clear();
                        }
                    });
                    ui.end_row();

                    ui.label(egui::RichText::new("Label:").size(13.0).strong());
                    ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::TextEdit::singleline(&mut app.send_label)
                            .hint_text("Optional payment label"),
                    );
                    ui.end_row();

                    ui.label(egui::RichText::new("Amount (ATHO):").size(13.0).strong());
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [150.0, 28.0],
                            egui::TextEdit::singleline(&mut app.send_amount)
                                .hint_text("10,000.44544444"),
                        );
                        ui.checkbox(
                            &mut app.send_include_fee_in_total,
                            "Include fee in total amount",
                        );
                        if ui.button("Use available balance").clicked() {
                            app.send_amount =
                                DesktopApp::format_send_amount_input(available_balance);
                        }
                    });
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(24.0);
        });
}

fn render_fee_panel(app: &DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame()
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Transaction Fee:").size(13.0).strong());
                let fee_text = if app.send_fee.is_empty() {
                    format!("{MIN_TX_FEE_PER_VBYTE_ATOMS} atom/vbyte")
                } else {
                    app.send_fee.clone()
                };
                ui.label(egui::RichText::new(fee_text).size(13.0).strong());
                ui.colored_label(
                    egui::Color32::from_rgb(185, 110, 30),
                    "Fee is computed from transaction size.",
                );
            });
        });
}

fn render_send_actions(app: &mut DesktopApp, ui: &mut egui::Ui, available_balance: u64) {
    ui.horizontal(|ui| {
        if ui
            .add_sized(
                [104.0, 28.0],
                egui::Button::image_and_text(resources::send_icon(13.0), "Send"),
            )
            .clicked()
        {
            if let Err(err) = app.submit_send_transaction() {
                app.last_error = Some(err.clone());
                app.send_status = err;
            }
        }
        if ui
            .add_sized(
                [104.0, 28.0],
                egui::Button::image_and_text(resources::clear_icon(13.0), "Clear All"),
            )
            .clicked()
        {
            app.send_to.clear();
            app.send_label.clear();
            app.send_amount.clear();
            app.send_fee.clear();
            app.send_include_fee_in_total = false;
            app.send_status = String::from("Enter a destination and ATHO amount.");
        }
        let _ = ui.add_enabled(
            false,
            egui::Button::image_and_text(resources::add_icon(14.0), "Add Recipient"),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "Available: {}",
                    widgets::format_atoms(available_balance)
                ))
                .size(13.0)
                .strong(),
            );
        });
    });
}
