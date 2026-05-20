// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Wallet summary page showing balances, sync hints, and recent activity.

use crate::app::amounts::DisplayUnit;
use crate::app::{widgets, DesktopApp};
use crate::resources;
use eframe::egui;
use std::time::{SystemTime, UNIX_EPOCH};

/// Renders the overview page for the active wallet.
pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let summary = app.wallet_balance_summary().clone();
    let rows = app.wallet_activity_rows().to_vec();
    let stacked = widgets::finite_available_width(ui, 760.0) < 760.0;
    let chain_synced = app.view_model.chain_synced();

    if stacked {
        render_balances_panel(app, ui, &summary);
        ui.add_space(8.0);
        render_recent_transactions(
            ui,
            app.ui_state.connected,
            chain_synced,
            app.display_unit(),
            &rows,
        );
    } else {
        ui.columns(2, |columns| {
            render_balances_panel(app, &mut columns[0], &summary);
            render_recent_transactions(
                &mut columns[1],
                app.ui_state.connected,
                chain_synced,
                app.display_unit(),
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
                    widgets::row_value(ui, &app.format_amount(summary.available_atoms));
                });
                ui.end_row();

                widgets::row_label(ui, "Pending:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(ui, &app.format_amount(summary.pending_atoms));
                });
                ui.end_row();

                widgets::row_label(ui, "Total:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::row_value(ui, &app.format_amount(summary.total_atoms));
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

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(12.0);
        render_miner_feedback(app, ui);
    });
}

fn render_miner_feedback(app: &DesktopApp, ui: &mut egui::Ui) {
    widgets::section_header(ui, "Miner feedback");
    ui.add_space(8.0);

    let runtime = app
        .mining_job
        .as_ref()
        .map(|job| format_duration(job.started_at.elapsed().as_secs()))
        .unwrap_or_else(|| String::from("Idle"));
    let last_mined = match (app.last_mined_height, app.last_mined_at_unix) {
        (Some(height), Some(at_unix)) => format!("#{} • {}", height, format_age(at_unix)),
        (Some(height), None) => format!("#{height}"),
        _ => String::from("No blocks yet"),
    };
    let last_hash = app
        .last_mined_block_hash
        .map(|hash| widgets::short_hash(&hash))
        .unwrap_or_else(|| String::from("Unavailable"));
    let hashes_per_second = format_hashrate(app.view_model.estimated_hashrate_hps);

    egui::Grid::new("overview_miner_feedback")
        .num_columns(2)
        .spacing([18.0, 10.0])
        .min_col_width(150.0)
        .show(ui, |ui| {
            widgets::row_label(ui, "Runtime:");
            widgets::row_value(ui, &runtime);
            ui.end_row();

            widgets::row_label(ui, "Last mined:");
            ui.label(
                egui::RichText::new(last_mined)
                    .size(12.0)
                    .color(widgets::TEXT),
            );
            ui.end_row();

            widgets::row_label(ui, "Last hash:");
            ui.label(
                egui::RichText::new(last_hash)
                    .size(12.0)
                    .monospace()
                    .color(widgets::ACCENT),
            );
            ui.end_row();

            widgets::row_label(ui, "Hashes/s:");
            widgets::row_value(ui, &hashes_per_second);
            ui.end_row();
        });
}

fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3600 {
        return format!("{}m {}s", seconds / 60, seconds % 60);
    }
    format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
}

fn format_age(recorded_at_unix: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(recorded_at_unix);
    let age = now.saturating_sub(recorded_at_unix);
    if age < 60 {
        format!("{age}s ago")
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else {
        format!("{}h ago", age / 3600)
    }
}

fn format_hashrate(hps: u64) -> String {
    const UNITS: [&str; 5] = ["H/s", "KH/s", "MH/s", "GH/s", "TH/s"];
    if hps == 0 {
        return String::from("0 H/s");
    }

    let mut value = hps as f64;
    let mut unit = 0usize;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", value.round() as u64, UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn render_recent_transactions(
    ui: &mut egui::Ui,
    connected: bool,
    chain_synced: bool,
    display_unit: DisplayUnit,
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
                    widgets::row_value_signed(ui, row.amount_atoms, display_unit);
                });
            });
            let label_width = widgets::finite_available_width(ui, 220.0);
            let response = ui.add_sized(
                [label_width, 0.0],
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
