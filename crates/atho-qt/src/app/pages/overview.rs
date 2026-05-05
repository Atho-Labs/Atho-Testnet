use crate::app::{widgets, DesktopApp};
use crate::resources;
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let summary = app.wallet_balance_summary().clone();
    let rows = app.wallet_activity_rows().to_vec();
    let stacked = ui.available_width() < 760.0;
    let chain_synced = app.view_model.chain_synced();

    if stacked {
        render_balances_panel(app, ui, &summary);
        ui.add_space(8.0);
        render_recent_transactions(
            ui,
            app.active_network(),
            app.ui_state.connected,
            chain_synced,
            &rows,
        );
    } else {
        ui.columns(2, |columns| {
            render_balances_panel(app, &mut columns[0], &summary);
            render_recent_transactions(
                &mut columns[1],
                app.active_network(),
                app.ui_state.connected,
                chain_synced,
                &rows,
            );
        });
    }
}

fn render_balances_panel(
    app: &DesktopApp,
    ui: &mut egui::Ui,
    summary: &crate::app::WalletBalanceSummary,
) {
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(252.0);
        let chain_synced = app.view_model.chain_synced();
        ui.horizontal(|ui| {
            widgets::section_header(ui, "Balances");
            if !app.ui_state.connected || !chain_synced {
                ui.add_space(8.0);
                let _ = ui.add(resources::warning_icon(20.0));
            }
        });
        ui.add_space(10.0);

        egui::Grid::new("overview_balances")
            .num_columns(2)
            .spacing([18.0, 16.0])
            .min_col_width(160.0)
            .show(ui, |ui| {
                widgets::row_label(ui, "Available:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(
                        ui,
                        &widgets::format_atoms(app.active_network(), summary.available_atoms),
                    );
                });
                ui.end_row();

                widgets::row_label(ui, "Pending:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(
                        ui,
                        &widgets::format_atoms(app.active_network(), summary.pending_atoms),
                    );
                });
                ui.end_row();

                widgets::row_label(ui, "Total:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(
                        ui,
                        &widgets::format_atoms(app.active_network(), summary.total_atoms),
                    );
                });
                ui.end_row();
            });

        if !app.ui_state.connected {
            ui.add_space(12.0);
            widgets::muted_label(
                ui,
                "Wallet balances are unavailable until the local node reconnects.",
            );
        } else if !chain_synced {
            ui.add_space(12.0);
            widgets::muted_label(
                ui,
                "Wallet balances may still change while Atho is synchronizing to the network tip.",
            );
        }
    });
}

fn render_recent_transactions(
    ui: &mut egui::Ui,
    network: atho_core::network::Network,
    connected: bool,
    chain_synced: bool,
    rows: &[crate::app::WalletActivityRow],
) {
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(252.0);
        ui.horizontal(|ui| {
            widgets::section_header(ui, "Recent transactions");
            if !connected || !chain_synced {
                ui.add_space(8.0);
                let _ = ui.add(resources::warning_icon(20.0));
            }
        });
        ui.add_space(10.0);

        egui::Grid::new("recent_transactions_header")
            .num_columns(3)
            .spacing([10.0, 8.0])
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.strong("Date");
                ui.strong("Type");
                ui.strong("Amount");
                ui.end_row();
            });
        ui.separator();
        ui.add_space(8.0);

        if rows.is_empty() {
            if !connected {
                widgets::muted_label(ui, "Wallet history is unavailable while the node is disconnected.");
                widgets::muted_label(ui, "Reconnect to Atho Core to refresh recent activity.");
            } else if !chain_synced {
                widgets::muted_label(ui, "Wallet history is still synchronizing.");
                widgets::muted_label(
                    ui,
                    "Recent transactions and balances may still change until Atho reaches the network tip.",
                );
            } else {
                widgets::muted_label(ui, "No transactions to show");
                widgets::muted_label(
                    ui,
                    "Mine, receive, or spend coins to populate wallet activity.",
                );
            }
            return;
        }

        for row in rows.iter().take(6) {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [68.0, 0.0],
                    egui::Label::new(
                        egui::RichText::new(&row.when)
                            .size(11.0)
                            .color(widgets::MUTED),
                    ),
                );
                ui.add_sized([84.0, 0.0], egui::Label::new(row.kind.label()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value_signed(ui, network, row.amount_atoms);
                });
            });
            let response = ui.add_sized(
                [ui.available_width(), 0.0],
                egui::Label::new(
                    egui::RichText::new(&row.label)
                        .size(11.0)
                        .color(widgets::TEXT),
                )
                .truncate(true),
            );
            if response.hovered() {
                response.on_hover_text(&row.label);
            }
            ui.add_space(6.0);
        }
    });
}
