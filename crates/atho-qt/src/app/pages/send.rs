// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Send page for building, validating, and submitting wallet transactions.

use crate::app::{widgets, DesktopApp};
use crate::resources;
use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
use eframe::egui;

/// Renders the send page and its validation/mempool feedback panels.
pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let available_balance = app.wallet_balance_atoms();
    let send_block_reason = app.wallet_send_block_reason();
    let send_in_progress = app.send_job.is_some();

    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(430.0);
        render_send_form(app, ui, available_balance, send_in_progress);
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
        ui.horizontal_wrapped(|ui| {
            if send_in_progress {
                ui.add(egui::Spinner::new().size(14.0));
                ui.add_space(4.0);
            }
            widgets::muted_label(ui, &app.send_status);
        });
        ui.add_space(6.0);
        render_send_actions(
            app,
            ui,
            available_balance,
            send_block_reason.as_deref(),
            send_in_progress,
        );
    });
}

/// Renders the main send form and its field-level affordances.
fn render_send_form(
    app: &mut DesktopApp,
    ui: &mut egui::Ui,
    _available_balance: u64,
    send_in_progress: bool,
) {
    widgets::panel_frame()
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            let mut trigger_send = false;
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
                        let address_width = widgets::reserved_width(
                            widgets::finite_available_width(ui, 420.0),
                            102.0,
                            220.0,
                            420.0,
                        );
                        let address_response = ui.add_sized(
                            [address_width, 28.0],
                            egui::TextEdit::singleline(&mut app.send_to)
                                .hint_text("Enter a base56 Atho address"),
                        );
                        if address_response.has_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            trigger_send = true;
                        }
                        address_response.on_hover_text(
                            "Paste or type the recipient address. Wrong-network addresses are rejected before send.",
                        );
                        if widgets::icon_button(
                            ui,
                            resources::address_book_icon(14.0),
                            "Open recipient address book",
                        )
                        .clicked()
                        {
                            app.open_recipient_address_book();
                        }
                        if widgets::icon_button(
                            ui,
                            resources::receive_icon(14.0),
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
                    let label_width = widgets::finite_widget_width(ui, 420.0, 220.0);
                    ui.add_sized(
                        [label_width, 28.0],
                        egui::TextEdit::singleline(&mut app.send_label)
                            .hint_text("Optional payment label"),
                    );
                    ui.end_row();

                    ui.label(egui::RichText::new("Amount:").size(13.0).strong())
                        .on_hover_text(
                            format!(
                                "Up to {} decimal places are supported for {} input. Spendable outputs must be at least 1 μATHO / 100 atoms.",
                                app.send_input_unit().max_decimals(),
                                app.send_input_unit().label(),
                            ),
                        );
                    ui.horizontal(|ui| {
                        let amount_response = ui.add_sized(
                            [150.0, 28.0],
                            egui::TextEdit::singleline(&mut app.send_amount)
                                .hint_text("Enter amount"),
                        );
                        if amount_response.has_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            trigger_send = true;
                        }
                        egui::ComboBox::from_id_source("send_input_unit")
                            .selected_text(app.send_input_unit().label())
                            .width(84.0)
                            .show_ui(ui, |ui| {
                                for unit in crate::app::amounts::InputUnit::variants() {
                                    if ui
                                        .selectable_label(
                                            app.send_input_unit() == unit,
                                            unit.label(),
                                        )
                                        .clicked()
                                    {
                                        app.set_send_input_unit(unit);
                                    }
                                }
                            });
                        ui.checkbox(
                            &mut app.send_include_fee_in_total,
                            "Include fee in total amount",
                        )
                        .on_hover_text(
                            "If enabled, the fee is deducted from the typed amount instead of added on top.",
                        );
                        let fill_response = ui.button("Use max spendable");
                        if fill_response.clicked() {
                            if let Err(err) = app.use_max_sendable_amount() {
                                app.last_error = Some(err.clone());
                                app.send_status = err;
                            }
                        }
                        fill_response.on_hover_text(
                            "Fill the largest amount the current one-address spend path can send in one transaction.",
                        );
                    });
                    ui.end_row();
                });

            ui.add_space(8.0);
            widgets::muted_label(
                ui,
                "The current spend path can combine spendable wallet-owned inputs into one grouped-signature transaction. Minimum output: 1 μATHO / 100 atoms. “Use max spendable” fills the largest amount currently spendable in one transaction.",
            );
            ui.add_space(8.0);
            ui.separator();
            if app.recipient_address_book_open || app.recipient_address_editor_open {
                ui.add_space(10.0);
                render_recipient_address_book_panel(app, ui);
                ui.add_space(8.0);
                ui.separator();
            }
            ui.add_space(24.0);

            if trigger_send && !send_in_progress {
                if let Err(err) = app.submit_send_transaction() {
                    app.last_error = Some(err.clone());
                    app.send_status = err;
                }
            }
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
    send_in_progress: bool,
) {
    let send_enabled = send_block_reason.is_none() && !send_in_progress;
    let compact = widgets::finite_available_width(ui, 760.0) < 760.0;
    let send_hover_reason = if send_in_progress {
        Some("A transaction is already being finalized")
    } else {
        send_block_reason
    };
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
                if let Some(reason) = send_hover_reason {
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
                    app.send_status = String::from("Enter a destination address and amount.");
                }
                if ui
                    .add_sized(
                        [138.0, 28.0],
                        egui::Button::image_and_text(
                            resources::add_icon(14.0),
                            "Add to Address Book",
                        ),
                    )
                    .clicked()
                {
                    app.start_add_current_recipient_to_address_book();
                }
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "Wallet total available: {}",
                    app.format_amount(available_balance)
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
        if let Some(reason) = send_hover_reason {
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
            app.send_status = String::from("Enter a destination address and amount.");
        }
        if ui
            .add_sized(
                [138.0, 28.0],
                egui::Button::image_and_text(resources::add_icon(14.0), "Add to Address Book"),
            )
            .clicked()
        {
            app.start_add_current_recipient_to_address_book();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "Wallet total available: {}",
                    app.format_amount(available_balance)
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

fn render_recipient_address_book_panel(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame()
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::section_header(ui, "Recipient Address Book");
                ui.add_space(8.0);
                ui.add_sized(
                    [220.0, 28.0],
                    egui::TextEdit::singleline(&mut app.recipient_address_book_filter)
                        .hint_text("Search label or address"),
                );
                if ui.button("Save Current Recipient").clicked() {
                    app.start_add_current_recipient_to_address_book();
                }
                if ui.button("Close").clicked() {
                    app.recipient_address_book_open = false;
                    app.recipient_address_editor_open = false;
                }
            });
            ui.add_space(8.0);

            if app.recipient_address_editor_open {
                ui.horizontal(|ui| {
                    ui.label("Label");
                    ui.add_sized(
                        [200.0, 28.0],
                        egui::TextEdit::singleline(&mut app.recipient_address_editor_label)
                            .hint_text("Recipient label"),
                    );
                    ui.label("Address");
                    ui.add_sized(
                        [360.0, 28.0],
                        egui::TextEdit::singleline(&mut app.recipient_address_editor_address)
                            .hint_text("Recipient address"),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Notes");
                    ui.add_sized(
                        [640.0, 28.0],
                        egui::TextEdit::singleline(&mut app.recipient_address_editor_notes)
                            .hint_text("Optional notes"),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        if let Err(err) = app.save_recipient_address_book_entry() {
                            app.last_error = Some(err.clone());
                            app.send_status = err;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        app.recipient_address_editor_open = false;
                        app.recipient_address_editor_id = None;
                        app.recipient_address_editor_label.clear();
                        app.recipient_address_editor_address.clear();
                        app.recipient_address_editor_notes.clear();
                    }
                });
                ui.add_space(10.0);
            }

            let filter = app
                .recipient_address_book_filter
                .trim()
                .to_ascii_lowercase();
            let mut filtered_entries = app
                .recipient_address_book
                .iter()
                .filter(|entry| {
                    filter.is_empty()
                        || entry.label.to_ascii_lowercase().contains(&filter)
                        || entry.address.to_ascii_lowercase().contains(&filter)
                })
                .cloned()
                .collect::<Vec<_>>();
            filtered_entries.sort_by(|left, right| {
                right
                    .last_used_at_unix
                    .cmp(&left.last_used_at_unix)
                    .then(
                        left.label
                            .to_ascii_lowercase()
                            .cmp(&right.label.to_ascii_lowercase()),
                    )
                    .then(left.address.cmp(&right.address))
            });

            if filtered_entries.is_empty() {
                widgets::muted_label(
                    ui,
                    "No saved recipients yet. Save a destination here for one-click reuse.",
                );
                return;
            }

            let mut select_id = None::<String>;
            let mut edit_id = None::<String>;
            let mut delete_id = None::<String>;

            egui::ScrollArea::vertical()
                .max_height(180.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for entry in &filtered_entries {
                        widgets::panel_frame()
                            .inner_margin(egui::Margin::same(8.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(&entry.label).size(13.0).strong(),
                                        );
                                        widgets::muted_label(ui, &entry.address);
                                        if !entry.notes.trim().is_empty() {
                                            widgets::muted_label(ui, &entry.notes);
                                        }
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.button("Delete").clicked() {
                                                delete_id = Some(entry.id.clone());
                                            }
                                            if ui.button("Edit").clicked() {
                                                edit_id = Some(entry.id.clone());
                                            }
                                            if ui.button("Use").clicked() {
                                                select_id = Some(entry.id.clone());
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(4.0);
                    }
                });

            if let Some(id) = select_id {
                if let Err(err) = app.select_recipient_address_book_entry(&id) {
                    app.last_error = Some(err.clone());
                    app.send_status = err;
                }
            }
            if let Some(id) = edit_id {
                app.edit_recipient_address_book_entry(&id);
            }
            if let Some(id) = delete_id {
                if let Err(err) = app.delete_recipient_address_book_entry(&id) {
                    app.last_error = Some(err.clone());
                    app.send_status = err;
                }
            }
        });
}
