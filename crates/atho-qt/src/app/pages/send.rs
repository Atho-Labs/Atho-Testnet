use crate::app::{widgets, DesktopApp};
use crate::resources;
use atho_core::constants::{DUST_RELAY_VALUE_ATOMS, MIN_TX_FEE_PER_VBYTE_ATOMS};
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let available_balance = app.wallet_balance_atoms();
    let send_block_reason = app.wallet_send_block_reason();

    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(430.0);
        render_send_form(app, ui, available_balance);
        ui.add_space(6.0);
        if let Some(reason) = send_block_reason.as_deref() {
            widgets::panel_frame().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(resources::warning_icon(18.0));
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(reason)
                            .size(12.5)
                            .color(egui::Color32::from_rgb(136, 86, 32)),
                    );
                });
            });
            ui.add_space(6.0);
        }
        render_fee_panel(app, ui);
        ui.add_space(6.0);
        widgets::muted_label(ui, &app.send_status);
        ui.add_space(6.0);
        render_send_actions(app, ui, available_balance, send_block_reason.as_deref());
    });
}

fn render_send_form(app: &mut DesktopApp, ui: &mut egui::Ui, _available_balance: u64) {
    widgets::panel_frame()
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            egui::Grid::new("send_form")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .min_col_width(96.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Pay To:").size(13.0).strong())
                        .on_hover_text(
                            "Enter a base56 Atho address for the active network. Example: R... on regnet.",
                        );
                    ui.horizontal(|ui| {
                        let address_width = (ui.available_width() - 102.0).max(220.0);
                        ui.add_sized(
                            [address_width, 28.0],
                            egui::TextEdit::singleline(&mut app.send_to)
                                .hint_text("Enter a base56 Atho address"),
                        )
                        .on_hover_text(
                            "Paste or type the recipient address. Wrong-network addresses are rejected before send.",
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

                    ui.label(egui::RichText::new("Label:").size(13.0).strong())
                        .on_hover_text("Optional local note for this payment.");
                    ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::TextEdit::singleline(&mut app.send_label)
                            .hint_text("Optional payment label"),
                    );
                    ui.end_row();

                    ui.label(egui::RichText::new("Amount (ATHO):").size(13.0).strong())
                        .on_hover_text(
                            "Examples: 1, 1.25, 0.50000000. Up to 8 decimal places are supported. Spendable outputs must be at least 50 atoms.",
                        );
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [150.0, 28.0],
                            egui::TextEdit::singleline(&mut app.send_amount)
                                .hint_text("Examples: 1, 1.25, 0.00000050"),
                        );
                        ui.checkbox(
                            &mut app.send_include_fee_in_total,
                            "Include fee in total amount",
                        )
                        .on_hover_text(
                            "If enabled, the fee is deducted from the typed amount instead of added on top.",
                        );
                        let fill_response = ui.button("Use available balance");
                        if fill_response.clicked() {
                            if let Err(err) = app.use_max_sendable_amount() {
                                app.last_error = Some(err.clone());
                                app.send_status = err;
                            }
                        }
                        fill_response.on_hover_text(
                            "Fill the largest amount the current spend path can send in one transaction.",
                        );
                    });
                    ui.end_row();
                });

            ui.add_space(8.0);
            widgets::muted_label(
                ui,
                &format!(
                    "The current spend path signs one wallet address at a time. Spendable outputs must be at least {}. “Use available balance” fills the largest amount currently spendable in one transaction.",
                    widgets::format_atoms(DUST_RELAY_VALUE_ATOMS)
                ),
            );
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

fn render_send_actions(
    app: &mut DesktopApp,
    ui: &mut egui::Ui,
    available_balance: u64,
    send_block_reason: Option<&str>,
) {
    let send_enabled = send_block_reason.is_none();
    let compact = ui.available_width() < 760.0;
    if compact {
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                let send_response = ui
                    .add_enabled_ui(send_enabled, |ui| {
                        ui.add_sized(
                            [104.0, 28.0],
                            egui::Button::image_and_text(resources::send_icon(13.0), "Send"),
                        )
                    })
                    .inner;
                if send_response.clicked() {
                    if let Err(err) = app.submit_send_transaction() {
                        app.last_error = Some(err.clone());
                        app.send_status = err;
                    }
                }
                if let Some(reason) = send_block_reason {
                    send_response.on_hover_text(reason);
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
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "Wallet total available: {}",
                    widgets::format_atoms(available_balance)
                ))
                .size(13.0)
                .strong(),
            )
            .on_hover_text(
                "Total wallet balance. The current send path may still be lower if funds are split across multiple receive addresses.",
            );
        });
        return;
    }

    ui.horizontal(|ui| {
        let send_response = ui
            .add_enabled_ui(send_enabled, |ui| {
                ui.add_sized(
                    [104.0, 28.0],
                    egui::Button::image_and_text(resources::send_icon(13.0), "Send"),
                )
            })
            .inner;
        if send_response.clicked() {
            if let Err(err) = app.submit_send_transaction() {
                app.last_error = Some(err.clone());
                app.send_status = err;
            }
        }
        if let Some(reason) = send_block_reason {
            send_response.on_hover_text(reason);
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
                    "Wallet total available: {}",
                    widgets::format_atoms(available_balance)
                ))
                .size(13.0)
                .strong(),
            )
            .on_hover_text(
                "Total wallet balance. The current send path may still be lower if funds are split across multiple receive addresses.",
            );
        });
    });
}
