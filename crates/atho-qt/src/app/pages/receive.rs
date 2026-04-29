use crate::app::{widgets, AddressPoolFilter, DesktopApp, ReceivePageTab};
use crate::resources;
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            if widgets::compact_tab(
                ui,
                app.receive_page_tab == ReceivePageTab::RequestPayment,
                ReceivePageTab::RequestPayment.label(),
                136.0,
            )
            .clicked()
            {
                app.receive_page_tab = ReceivePageTab::RequestPayment;
            }
            if widgets::compact_tab(
                ui,
                app.receive_page_tab == ReceivePageTab::AddressPool,
                ReceivePageTab::AddressPool.label(),
                124.0,
            )
            .clicked()
            {
                app.receive_page_tab = ReceivePageTab::AddressPool;
            }
        });

        ui.add_space(10.0);
        match app.receive_page_tab {
            ReceivePageTab::RequestPayment => render_request_payment_tab(app, ui),
            ReceivePageTab::AddressPool => render_address_pool_tab(app, ui),
        }
    });
}

fn render_request_payment_tab(app: &mut DesktopApp, ui: &mut egui::Ui) {
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
            if !app.current_receive_address_text().is_empty()
                && widgets::icon_button(ui, resources::copy_icon(15.0), "Copy current address")
                    .clicked()
            {
                DesktopApp::copy_text(ui, app.current_receive_address_text());
            }
        });
        if !app.current_receive_address_text().is_empty() {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                widgets::muted_label(ui, "Current receiving address");
                if let Some(index) = app.wallet_current_receive_index() {
                    ui.label(
                        egui::RichText::new(format!("Wallet index R{index:04}"))
                            .monospace()
                            .color(widgets::ACCENT),
                    );
                }
                if let Some(row) = app.current_receive_address_row() {
                    let status = if row.used {
                        let suffix = if row.utxo_count == 1 { "UTXO" } else { "UTXOs" };
                        format!("Used - {} {}", row.utxo_count, suffix)
                    } else {
                        String::from("Unused")
                    };
                    ui.colored_label(
                        if row.used {
                            widgets::ACCENT
                        } else {
                            widgets::MUTED
                        },
                        status,
                    );
                }
            });
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
                .add_enabled_ui(selected_request.is_some(), |ui| {
                    widgets::icon_button(ui, resources::copy_icon(15.0), "Copy selected address")
                })
                .inner
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

fn render_address_pool_tab(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(420.0);
        widgets::section_header(ui, "Address pool");
        ui.add_space(8.0);
        widgets::muted_label(
            ui,
            "This page shows the receive indexes currently loaded into the wallet discovery window. It is not the full keypool size.",
        );
        ui.add_space(10.0);

        let (receive_keypool_queued, change_keypool_queued) = app.wallet_keypool_depths();
        let (highest_generated_receive_index, highest_generated_change_index) =
            app.wallet_highest_generated_indices();
        let (highest_reserved_receive_index, highest_reserved_change_index) =
            app.wallet_highest_reserved_indices();
        let (next_receive_index, next_change_index) = app.wallet_next_indices();

        widgets::muted_label(
            ui,
            &format!(
                "Active scan window: {} receive indexes | Configured recovery window: {}",
                app.wallet_discovery_scan_limit,
                app.wallet_configured_recovery_window()
            ),
        );
        widgets::muted_label(
            ui,
            &format!(
                "Keypool queued: {} receive / {} change | Next derivation tips: R{:04} / C{:04}",
                receive_keypool_queued,
                change_keypool_queued,
                next_receive_index,
                next_change_index
            ),
        );
        widgets::muted_label(
            ui,
            &format!(
                "Highest generated: {} / {} | Highest reserved: {} / {}",
                highest_generated_receive_index
                    .map(|index| format!("R{index:04}"))
                    .unwrap_or_else(|| String::from("none")),
                highest_generated_change_index
                    .map(|index| format!("C{index:04}"))
                    .unwrap_or_else(|| String::from("none")),
                highest_reserved_receive_index
                    .map(|index| format!("R{index:04}"))
                    .unwrap_or_else(|| String::from("none")),
                highest_reserved_change_index
                    .map(|index| format!("C{index:04}"))
                    .unwrap_or_else(|| String::from("none")),
            ),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    app.wallet_scan_job.is_none(),
                    egui::Button::new("Scan farther"),
                )
                .clicked()
            {
                if app.advance_wallet_discovery_scan_limit() {
                    app.send_status = format!(
                        "Address window expanded to {}. Background scan queued.",
                        app.wallet_discovery_scan_limit
                    );
                    app.last_error = None;
                } else {
                    app.send_status = String::from(
                        "Address window is already at the largest built-in quick step.",
                    );
                }
            }
            widgets::muted_label(
                ui,
                "Use Settings -> Wallet Index Recovery for a custom higher index range.",
            );
        });
        ui.add_space(10.0);

        ui.horizontal_wrapped(|ui| {
            for filter in AddressPoolFilter::variants() {
                if widgets::compact_tab(
                    ui,
                    app.address_pool_filter == filter,
                    filter.label(),
                    84.0,
                )
                .clicked()
                {
                    app.address_pool_filter = filter;
                }
            }
        });

        ui.add_space(8.0);
        let total = app.receive_address_rows.len();
        let used = app.receive_address_rows.iter().filter(|row| row.used).count();
        let unused = total.saturating_sub(used);
        widgets::muted_label(
            ui,
            &format!(
                "{total} receive index(es) loaded, {unused} unused, {used} used"
            ),
        );
        ui.add_space(10.0);

        let rows: Vec<&crate::app::ReceiveAddressRow> = app
            .receive_address_rows
            .iter()
            .filter(|row| app.address_pool_filter.matches(row.used))
            .collect();

        if rows.is_empty() {
            widgets::muted_label(ui, "No addresses match the current filter.");
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(300.0)
            .show(ui, |ui| {
                for row in rows {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("R{:04}", row.address.path.index))
                                .monospace(),
                        );
                        ui.add_space(6.0);
                        if row.is_current {
                            ui.colored_label(widgets::ACCENT, "Current");
                            ui.add_space(6.0);
                        }
                        ui.label(
                            egui::RichText::new(&row.address.address)
                                .monospace()
                                .color(widgets::TEXT),
                        );
                        ui.add_space(8.0);
                        if row.used {
                            let suffix = if row.utxo_count == 1 { "UTXO" } else { "UTXOs" };
                            ui.colored_label(
                                widgets::ACCENT,
                                format!("Used - {} {}", row.utxo_count, suffix),
                            )
                            .on_hover_text(format!(
                                "{} atoms across {} UTXO(s)",
                                widgets::format_atoms(row.total_atoms),
                                row.utxo_count
                            ));
                        } else {
                            ui.colored_label(widgets::MUTED, "Unused");
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(
                                ui,
                                resources::copy_icon(15.0),
                                "Copy address",
                            )
                            .clicked()
                            {
                                DesktopApp::copy_text(ui, row.address.address.clone());
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                }
            });
    });
}
