use crate::app::{widgets, AddressPoolFilter, DesktopApp, ReceivePageTab};
use crate::resources;
use eframe::egui;
use qrcodegen::{QrCode, QrCodeEcc};

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
    let selected_request = app.selected_receive_request().cloned();
    let current_address = app.current_receive_address_text();
    let detail_address = selected_request
        .as_ref()
        .map(|request| request.address.clone())
        .or_else(|| (!current_address.is_empty()).then_some(current_address.clone()));

    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(300.0);
        if ui.available_width() > 820.0 {
            ui.columns(2, |columns| {
                render_request_form(app, &mut columns[0]);
                render_receive_detail_card(
                    app,
                    &mut columns[1],
                    selected_request.as_ref(),
                    detail_address.as_deref(),
                );
            });
        } else {
            render_request_form(app, ui);
            ui.add_space(10.0);
            render_receive_detail_card(
                app,
                ui,
                selected_request.as_ref(),
                detail_address.as_deref(),
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
            let response = ui.add(
                egui::TextEdit::singleline(&mut selected_address)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
            response.context_menu(|ui| {
                if ui.button("Copy address").clicked() {
                    DesktopApp::copy_text(ui, selected_address.clone());
                    ui.close_menu();
                }
            });
        }
    });
}

fn render_request_form(app: &mut DesktopApp, ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new("Use this form to request Atho payments. All fields are optional.")
                .size(13.0),
        );
        ui.add_space(10.0);

        egui::Grid::new("receive_form")
            .num_columns(2)
            .spacing([12.0, 9.0])
            .min_col_width(120.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Label:").size(13.0).strong());
                ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut app.receive_label)
                        .hint_text("Optional internal label")
                        .desired_width(f32::INFINITY),
                );
                ui.end_row();

                ui.label(egui::RichText::new("Amount:").size(13.0).strong());
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [180.0, 30.0],
                        egui::TextEdit::singleline(&mut app.receive_amount)
                            .hint_text("Atom amount, for example 50000000"),
                    );
                    widgets::muted_label(ui, "atoms");
                });
                ui.end_row();

                ui.label(egui::RichText::new("Message:").size(13.0).strong());
                ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut app.receive_message)
                        .hint_text("Optional payment note")
                        .desired_width(f32::INFINITY),
                );
                ui.end_row();
            });

        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
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
            ui.horizontal_wrapped(|ui| {
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
            let response = ui.add(
                egui::TextEdit::singleline(&mut address_text)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
            response.context_menu(|ui| {
                if ui.button("Copy address").clicked() {
                    DesktopApp::copy_text(ui, address_text.clone());
                    ui.close_menu();
                }
            });
        }
    });
}

fn render_receive_detail_card(
    app: &mut DesktopApp,
    ui: &mut egui::Ui,
    selected_request: Option<&crate::app::ReceiveRequestRecord>,
    detail_address: Option<&str>,
) {
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Address request");
        ui.add_space(10.0);

        let Some(address) = detail_address else {
            widgets::muted_label(
                ui,
                "Create a receiving address or select a requested payment to show the address and QR code here.",
            );
            return;
        };

        let title = if selected_request.is_some() {
            "Selected request"
        } else {
            "Current receiving address"
        };
        ui.label(egui::RichText::new(title).strong().color(widgets::ACCENT));
        ui.add_space(8.0);

        ui.horizontal_wrapped(|ui| {
            if let Some(request) = selected_request {
                ui.label(
                    egui::RichText::new(format!("#{}", request.sequence))
                        .monospace()
                        .color(widgets::TEXT),
                );
                if !request.label.is_empty() {
                    ui.separator();
                    widgets::muted_label(ui, &request.label);
                }
                if let Some(amount_atoms) = request.amount_atoms {
                    ui.separator();
                    widgets::muted_label(ui, &format!("{amount_atoms} atoms"));
                }
            } else if let Some(index) = app.wallet_current_receive_index() {
                ui.label(
                    egui::RichText::new(format!("Wallet index R{index:04}"))
                        .monospace()
                        .color(widgets::TEXT),
                );
            }
        });
        if let Some(request) = selected_request {
            if !request.message.is_empty() {
                ui.add_space(4.0);
                widgets::muted_label(ui, &request.message);
            }
        }

        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            paint_qr_code(ui, address, 176.0);
            ui.add_space(8.0);
            let response = ui.label(
                egui::RichText::new(widgets::elide_text(address, 42))
                    .monospace()
                    .color(widgets::TEXT),
            );
            response.on_hover_text(address);
        });
        ui.add_space(10.0);

        ui.horizontal_wrapped(|ui| {
            if widgets::icon_button(ui, resources::copy_icon(15.0), "Copy address").clicked() {
                DesktopApp::copy_text(ui, address.to_owned());
            }
            widgets::muted_label(ui, "The QR encodes the same base56 Atho address shown below it.");
        });
    });
}

fn paint_qr_code(ui: &mut egui::Ui, payload: &str, size: f32) {
    let Ok(code) = QrCode::encode_text(payload, QrCodeEcc::Medium) else {
        widgets::muted_label(ui, "QR unavailable");
        return;
    };
    let module_count = code.size();
    if module_count <= 0 {
        widgets::muted_label(ui, "QR unavailable");
        return;
    }

    let quiet_zone = 2;
    let total_modules = module_count + quiet_zone * 2;
    let (response, painter) = ui.allocate_painter(egui::vec2(size, size), egui::Sense::hover());
    painter.rect_filled(response.rect, 4.0, egui::Color32::WHITE);
    let module_size = response.rect.width() / total_modules as f32;

    for y in 0..module_count {
        for x in 0..module_count {
            if !code.get_module(x, y) {
                continue;
            }
            let left = response.rect.left() + (x + quiet_zone) as f32 * module_size;
            let top = response.rect.top() + (y + quiet_zone) as f32 * module_size;
            let rect = egui::Rect::from_min_size(
                egui::pos2(left, top),
                egui::vec2(module_size, module_size),
            );
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(28, 31, 28));
        }
    }
    painter.rect_stroke(
        response.rect,
        4.0,
        egui::Stroke::new(1.0, widgets::PANEL_STROKE),
    );
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
