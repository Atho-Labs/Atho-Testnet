use crate::app::{widgets, DesktopApp};
use crate::resources;
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let balance = app.wallet_balance_atoms();
    let rows = app.wallet_activity_rows().to_vec();
    let stacked = ui.available_width() < 920.0;

    if stacked {
        render_balances_panel(app, ui, balance);
        ui.add_space(10.0);
        render_recent_transactions(ui, app.ui_state.connected, &rows);
    } else {
        ui.columns(2, |columns| {
            render_balances_panel(app, &mut columns[0], balance);
            render_recent_transactions(&mut columns[1], app.ui_state.connected, &rows);
        });
    }
}

fn render_balances_panel(app: &DesktopApp, ui: &mut egui::Ui, balance: u64) {
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(332.0);
        ui.horizontal(|ui| {
            widgets::section_header(ui, "Balances");
            if !app.ui_state.connected {
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
                    widgets::row_value(ui, &widgets::format_atoms(balance));
                });
                ui.end_row();

                widgets::row_label(ui, "Pending:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(ui, &widgets::format_atoms(0));
                });
                ui.end_row();
            });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        egui::Grid::new("overview_total")
            .num_columns(2)
            .spacing([18.0, 10.0])
            .min_col_width(160.0)
            .show(ui, |ui| {
                widgets::row_label(ui, "Total:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(ui, &widgets::format_atoms(balance));
                });
                ui.end_row();
            });
    });
}

fn render_recent_transactions(
    ui: &mut egui::Ui,
    connected: bool,
    rows: &[crate::app::WalletActivityRow],
) {
    widgets::panel_frame().show(ui, |ui| {
        ui.set_min_height(332.0);
        ui.horizontal(|ui| {
            widgets::section_header(ui, "Recent transactions");
            if !connected {
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
            widgets::muted_label(ui, "No transactions to show");
            widgets::muted_label(ui, "Mine or receive coins to populate wallet activity.");
            return;
        }

        for row in rows.iter().take(6) {
            ui.horizontal(|ui| {
                widgets::muted_label(ui, &row.when);
                ui.add_space(18.0);
                ui.label(row.kind);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(ui, &widgets::format_atoms(row.amount_atoms));
                });
            });
            widgets::muted_label(ui, &row.label);
            ui.add_space(6.0);
        }
    });
}
