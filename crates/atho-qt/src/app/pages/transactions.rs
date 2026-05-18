// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Transaction history page with filters for wallet activity.

use crate::app::{widgets, DesktopApp, WalletActivityKind, WalletActivityRow};
use eframe::egui;

const DATE_FILTERS: &[&str] = &["All", "Confirmed"];
const TYPE_FILTERS: &[&str] = &["All", "Mined", "Received", "Sent"];

/// Renders the wallet transaction history page.
pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(480.0);
        let chain_synced = app.view_model.chain_synced();

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_source("tx_date_filter")
                .selected_text(
                    DATE_FILTERS[app.transaction_date_filter.min(DATE_FILTERS.len() - 1)],
                )
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (index, label) in DATE_FILTERS.iter().enumerate() {
                        ui.selectable_value(&mut app.transaction_date_filter, index, *label);
                    }
                });

            egui::ComboBox::from_id_source("tx_type_filter")
                .selected_text(
                    TYPE_FILTERS[app.transaction_type_filter.min(TYPE_FILTERS.len() - 1)],
                )
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (index, label) in TYPE_FILTERS.iter().enumerate() {
                        ui.selectable_value(&mut app.transaction_type_filter, index, *label);
                    }
                });

            ui.add_sized(
                [ui.available_width() - 170.0, 28.0],
                egui::TextEdit::singleline(&mut app.transaction_search)
                    .hint_text("Enter address, transaction id, or label to search"),
            );
            ui.add_sized(
                [110.0, 28.0],
                egui::TextEdit::singleline(&mut app.transaction_min_amount).hint_text("Min amount"),
            );
        });

        ui.add_space(8.0);
        widgets::table_header(ui, &["Date", "Type", "Label", "Amount"]);
        ui.separator();
        ui.add_space(6.0);

        let rows = filtered_rows(app);
        if rows.is_empty() {
            if !app.ui_state.connected {
                widgets::muted_label(ui, "Wallet transactions are unavailable while the node is disconnected.");
            } else if !chain_synced {
                widgets::muted_label(ui, "Wallet transactions are still synchronizing with the network tip.");
                widgets::muted_label(
                    ui,
                    "Recent mined, received, and spent entries may appear once Atho finishes synchronizing.",
                );
            } else {
                widgets::muted_label(ui, "No wallet transactions to display.");
            }
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(420.0)
                .show(ui, |ui| {
                    for row in &rows {
                        egui::Grid::new(ui.id().with(&row.reference))
                            .num_columns(4)
                            .spacing([12.0, 8.0])
                            .min_col_width(80.0)
                            .show(ui, |ui| {
                                ui.label(&row.when);
                                ui.label(row.kind.label());
                                widgets::elided_label(ui, &row.label, 56);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        widgets::row_value_signed(
                                            ui,
                                            row.amount_atoms,
                                            app.display_unit(),
                                        )
                                    },
                                );
                                ui.end_row();
                            });
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                });
        }
    });
}

/// Applies the current filter/search state to wallet activity rows.
fn filtered_rows(app: &DesktopApp) -> Vec<WalletActivityRow> {
    let search = app.transaction_search.trim().to_ascii_lowercase();
    let min_amount = app.transaction_min_amount.trim().parse::<u64>().ok();

    app.wallet_activity_rows()
        .iter()
        .filter(|row| {
            (app.transaction_type_filter == 0
                || matches!(
                    (app.transaction_type_filter, row.kind),
                    (1, WalletActivityKind::Mined)
                        | (2, WalletActivityKind::Received)
                        | (3, WalletActivityKind::Sent)
                ))
                && min_amount
                    .map(|min| row.amount_atoms.unsigned_abs() >= min as u128)
                    .unwrap_or(true)
                && (search.is_empty()
                    || row.label.to_ascii_lowercase().contains(&search)
                    || row.reference.to_ascii_lowercase().contains(&search))
        })
        .cloned()
        .collect()
}
