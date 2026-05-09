use super::{
    pages, widgets, CreateWalletForm, DebugWindowTab, DesktopApp, ImportWalletForm, LaunchPage,
    NavTab,
};
use crate::resources;
use eframe::egui;

pub(crate) fn render_main_shell(app: &mut DesktopApp, ctx: &egui::Context) {
    render_menu_bar(app, ctx);
    render_toolbar(app, ctx);
    render_status_bar(app, ctx);
    render_sync_status_window(app, ctx);
    render_storage_recovery_notice_window(app, ctx);
    render_about_window(app, ctx);
    pages::console::render_window(app, ctx);

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(widgets::SHELL_BG))
        .show(ctx, |ui| {
            widgets::shell_frame().show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| pages::render_active_page(app, ui));
            });
        });
}

fn render_menu_bar(app: &mut DesktopApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("menu_bar")
        .exact_height(24.0)
        .frame(
            egui::Frame::none()
                .fill(widgets::MENU_BG)
                .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE)),
        )
        .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Create Wallet").clicked() {
                        app.create_form = CreateWalletForm::new(app.connection.network());
                        let _ = app.generate_create_mnemonic();
                        app.launch_page = LaunchPage::CreateWallet;
                        app.clear_wallet_state();
                        ui.close_menu();
                    }
                    if ui.button("Import Wallet").clicked() {
                        app.import_form = ImportWalletForm::new(app.connection.network());
                        app.launch_page = LaunchPage::ImportWallet;
                        app.clear_wallet_state();
                        ui.close_menu();
                    }
                    let known_wallets = app.wallet_registry_entries();
                    if !known_wallets.is_empty() {
                        ui.separator();
                        ui.menu_button("Switch Wallet", |ui| {
                            for entry in known_wallets {
                                let is_current = app.wallet_path_matches(&entry.wallet_path);
                                let label = if is_current {
                                    format!("{}  [current]", entry.wallet_name)
                                } else {
                                    entry.wallet_name.clone()
                                };
                                let response = ui
                                    .add_enabled(!is_current, egui::Button::new(label))
                                    .on_hover_text(entry.wallet_path.clone());
                                if response.clicked() {
                                    if let Err(err) = app.begin_wallet_switch(&entry.wallet_path) {
                                        app.last_error = Some(err);
                                    }
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Settings", |ui| {
                    if ui.button("Options").clicked() {
                        app.active_tab = NavTab::Settings;
                        ui.close_menu();
                    }
                    if ui.button("Mine Once").clicked() {
                        app.active_tab = NavTab::Settings;
                        app.ui_state.generate_coins = false;
                        if app.ui_state.connected && app.wallet.is_some() {
                            if app.mining_job.is_some() {
                                app.restart_mining_job();
                            } else {
                                app.start_mining_job();
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Mine Loop").clicked() {
                        app.active_tab = NavTab::Settings;
                        app.ui_state.generate_coins = true;
                        if app.ui_state.connected && app.wallet.is_some() {
                            if app.mining_job.is_some() {
                                app.restart_mining_job();
                            } else {
                                app.start_mining_job();
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Stop Miner").clicked() {
                        app.stop_mining_job();
                        ui.close_menu();
                    }
                });

                ui.menu_button("Window", |ui| {
                    if ui.button("Main Window").clicked() {
                        app.close_debug_window();
                        ui.close_menu();
                    }
                    ui.separator();
                    for tab in DebugWindowTab::variants() {
                        if ui.button(tab.label()).clicked() {
                            app.open_debug_window(tab);
                            ui.close_menu();
                        }
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("Node Window").clicked() {
                        app.open_debug_window(DebugWindowTab::Console);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("About Atho").clicked() {
                        app.show_about_dialog = true;
                        ui.close_menu();
                    }
                });
            });
        });
}

fn render_toolbar(app: &mut DesktopApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("toolbar")
        .exact_height(64.0)
        .frame(
            egui::Frame::none()
                .fill(widgets::TOOLBAR_BG)
                .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for tab in NavTab::toolbar_tabs() {
                    let response = match tab {
                        NavTab::Overview => widgets::toolbar_tab(
                            ui,
                            app.active_tab == tab,
                            tab.label(),
                            resources::overview_icon(26.0),
                        ),
                        NavTab::Send => widgets::toolbar_tab(
                            ui,
                            app.active_tab == tab,
                            tab.label(),
                            resources::send_icon(26.0),
                        ),
                        NavTab::Receive => widgets::toolbar_tab(
                            ui,
                            app.active_tab == tab,
                            tab.label(),
                            resources::receive_icon(26.0),
                        ),
                        NavTab::Transactions => widgets::toolbar_tab(
                            ui,
                            app.active_tab == tab,
                            tab.label(),
                            resources::history_icon(26.0),
                        ),
                        NavTab::DebugConsole => continue,
                        NavTab::Settings => continue,
                    };
                    let response = response.on_hover_text(nav_tab_tooltip(tab));
                    if response.clicked() {
                        app.active_tab = tab;
                    }
                }

                ui.add_space(4.0);
                if ui
                    .add_sized([90.0, 28.0], egui::Button::new("Console"))
                    .on_hover_text(
                        "Open the Atho node window with console, peers, and network diagnostics.",
                    )
                    .clicked()
                {
                    app.open_debug_window(DebugWindowTab::Console);
                }
            });
        });
}

fn render_status_bar(app: &mut DesktopApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(24.0)
        .frame(
            egui::Frame::none()
                .fill(widgets::STATUS_BG)
                .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE)),
        )
        .show(ctx, |ui| {
            let status_text = if app.ui_state.connected && app.view_model.chain_synced() {
                String::from("Synced")
            } else if (app.ui_state.connected && app.view_model.running)
                || app.connection.has_local_node()
            {
                String::from("Synchronizing with network...")
            } else {
                String::from("Disconnected")
            };
            let show_sync_bar =
                app.ui_state.connected && app.view_model.running && !app.view_model.chain_synced();
            let progress = if show_sync_bar {
                app.view_model.sync_progress_display()
            } else {
                0.0
            };
            let blocks_left = app
                .view_model
                .sync_target_height()
                .saturating_sub(app.view_model.block_count);
            let sync_tooltip = sync_tooltip_text(app, blocks_left, progress);

            ui.horizontal(|ui| {
                let available = ui.available_width();
                let right_width = (available * 0.34).clamp(220.0, 320.0);
                let left_width = (available - right_width - 6.0).max(140.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, ui.available_height()),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let status_response = ui.add(
                            egui::Label::new(egui::RichText::new(status_text).color(widgets::TEXT))
                                .sense(egui::Sense::click()),
                        );
                        status_response.clone().on_hover_text(sync_tooltip.clone());
                        if status_response.clicked() {
                            app.show_sync_status_dialog = true;
                        }
                        if show_sync_bar {
                            let progress_width = (left_width * 0.32).clamp(110.0, 220.0);
                            let progress_text = if app.view_model.headers_synced {
                                format!("{:.2}%", progress * 100.0)
                            } else {
                                String::from("Syncing headers")
                            };
                            let progress_response = ui.add(
                                egui::ProgressBar::new(progress)
                                    .desired_width(progress_width)
                                    .fill(widgets::SYNC_PROGRESS_FILL)
                                    .text(progress_text),
                            );
                            progress_response
                                .clone()
                                .on_hover_text(sync_tooltip.clone());
                            if progress_response.clicked() {
                                app.show_sync_status_dialog = true;
                            }
                            ui.separator();
                        }
                        widgets::muted_label(ui, &format!("Height {}", app.view_model.block_count))
                            .on_hover_text("Local canonical chain height.");
                        ui.separator();
                        widgets::muted_label(
                            ui,
                            &format!("Target {}", app.view_model.sync_target_height()),
                        )
                        .on_hover_text(
                            "Best known sync target advertised by the current peer set.",
                        );
                        ui.separator();
                        widgets::muted_label(
                            ui,
                            &format!("Mempool {}", app.view_model.mempool_count),
                        )
                        .on_hover_text("Current local mempool transaction count.");
                        if let Some(error) = &app.last_error {
                            ui.separator();
                            ui.colored_label(egui::Color32::from_rgb(170, 77, 50), "!")
                                .on_hover_text(error);
                        }
                    },
                );

                ui.add_space(6.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), ui.available_height()),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add(resources::sync_icon(16.0, app.view_model.chain_synced()))
                            .on_hover_text(sync_tooltip);
                        ui.add(resources::network_icon(16.0, app.ui_state.connected))
                            .on_hover_text(if app.ui_state.connected {
                                "RPC connected to the expected Atho network."
                            } else {
                                "RPC is disconnected or the managed local node is still starting."
                            });
                        ui.separator();
                        ui.add(resources::hd_enabled_icon(16.0))
                            .on_hover_text("HD wallet mode is enabled.");
                        egui::ComboBox::from_id_source("shell_display_unit")
                            .selected_text(app.display_unit().label())
                            .width(92.0)
                            .show_ui(ui, |ui| {
                                for unit in crate::app::amounts::DisplayUnit::variants() {
                                    if ui
                                        .selectable_label(app.display_unit() == unit, unit.label())
                                        .clicked()
                                    {
                                        app.set_display_unit(unit);
                                    }
                                }
                            })
                            .response
                            .on_hover_text(
                                "Display unit for balances, history, and wallet amounts. This only changes UI formatting; consensus remains integer atoms.",
                            );
                        ui.separator();
                        widgets::muted_label(ui, &app.view_model.network_label);
                    },
                );
            });
        });
}

fn render_storage_recovery_notice_window(app: &mut DesktopApp, ctx: &egui::Context) {
    if !app.show_storage_recovery_notice_dialog {
        return;
    }
    let Some(notice) = app.storage_recovery_notice.clone() else {
        app.show_storage_recovery_notice_dialog = false;
        return;
    };

    let mut open = true;
    egui::Window::new("Storage Recovered")
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(&notice).color(widgets::TEXT).size(14.0));
            ui.add_space(12.0);
            if ui.button("OK").clicked() {
                app.dismiss_storage_recovery_notice();
            }
        });

    if !open {
        app.dismiss_storage_recovery_notice();
    }
}

fn render_sync_status_window(app: &mut DesktopApp, ctx: &egui::Context) {
    if !app.show_sync_status_dialog {
        return;
    }

    let target = app.view_model.sync_target_height();
    let blocks_left = target.saturating_sub(app.view_model.block_count);
    let progress = app.view_model.sync_progress_display();
    let estimated_time_left = estimate_sync_time_left(app);
    let progress_per_hour = estimated_progress_per_hour(app);
    let last_block_time = if app.view_model.tip_timestamp == 0 {
        String::from("Unknown")
    } else {
        format!("{} ago", age_label(app.view_model.tip_timestamp))
    };
    let blocks_left_label = if app.view_model.headers_synced {
        blocks_left.to_string()
    } else if target > app.view_model.block_count {
        format!("{blocks_left} (known target; syncing headers)")
    } else {
        String::from("Unknown. Discovering network tip")
    };

    let mut open = app.show_sync_status_dialog;
    let mut hide_requested = false;

    egui::Window::new("Synchronization status")
        .collapsible(false)
        .resizable(false)
        .default_size(egui::vec2(520.0, 280.0))
        .open(&mut open)
        .show(ctx, |ui| {
            widgets::dialog_frame().show(ui, |ui| {
                let warning = if app.view_model.chain_synced() {
                    "The local Atho chain is caught up with the advertised network target."
                } else {
                    "The local Atho chain is still synchronizing to the network tip. Sending transactions and mining stay blocked until sync completes because balances, history, and spendability may still change."
                };
                ui.horizontal_top(|ui| {
                    ui.add(resources::warning_icon(22.0));
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(warning)
                            .size(13.0)
                            .color(widgets::TEXT),
                    );
                });
                ui.add_space(14.0);

                egui::Grid::new("sync_status_grid")
                    .num_columns(2)
                    .spacing([18.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong("Number of blocks left");
                        ui.label(blocks_left_label);
                        ui.end_row();

                        ui.strong("Last local block time");
                        ui.label(last_block_time);
                        ui.end_row();

                        ui.strong("Progress");
                        ui.label(format!("{:.2}%", progress * 100.0));
                        ui.end_row();

                        ui.strong("Progress increase per hour");
                        ui.label(progress_per_hour.unwrap_or_else(|| String::from("Unknown")));
                        ui.end_row();

                        ui.strong("Estimated time left");
                        ui.label(
                            estimated_time_left
                                .map(format_duration_human)
                                .unwrap_or_else(|| String::from("Unknown")),
                        );
                        ui.end_row();

                        ui.strong("Connected peers");
                        ui.label(app.view_model.peer_count.to_string());
                        ui.end_row();

                        ui.strong("Connecting peers");
                        ui.label(app.view_model.connecting_peer_count.to_string());
                        ui.end_row();
                    });

                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Hide").clicked() {
                        hide_requested = true;
                    }
                });
            });
        });

    if !app.view_model.chain_synced() && (hide_requested || !open) {
        app.sync_status_hidden_until_synced = true;
    }
    if app.view_model.chain_synced() {
        app.sync_status_hidden_until_synced = false;
    }
    app.show_sync_status_dialog = open && !hide_requested;
}

fn nav_tab_tooltip(tab: NavTab) -> &'static str {
    match tab {
        NavTab::Overview => "Wallet balances, recent activity, and high-level node status.",
        NavTab::Send => "Create and submit outgoing Atho transactions.",
        NavTab::Receive => "Generate receiving addresses, payment requests, and QR codes.",
        NavTab::Transactions => "Review wallet transaction history and filters.",
        NavTab::DebugConsole => "Open the Atho node window.",
        NavTab::Settings => "Wallet, mining, and diagnostics settings.",
    }
}

fn sync_tooltip_text(app: &DesktopApp, blocks_left: u64, progress: f32) -> String {
    if app.view_model.chain_synced() {
        return format!(
            "Local height: {}\nSync target: {}\nChain is synced to the known Atho network tip.",
            app.view_model.block_count,
            app.view_model.sync_target_height(),
        );
    }

    let sync_mode = if app.view_model.headers_synced {
        "Synchronizing blocks"
    } else {
        "Synchronizing headers"
    };
    format!(
        "{sync_mode}\nLocal height: {}\nSync target: {}\nBlocks left: {}\nProgress: {:.2}%\nClick for detailed Atho sync status.",
        app.view_model.block_count,
        app.view_model.sync_target_height(),
        blocks_left,
        progress * 100.0
    )
}

fn estimated_progress_per_hour(app: &DesktopApp) -> Option<String> {
    let first = app.sync_progress_samples.first()?;
    let last = app.sync_progress_samples.last()?;
    let elapsed_secs = last
        .recorded_at
        .saturating_duration_since(first.recorded_at)
        .as_secs_f64();
    if elapsed_secs <= f64::EPSILON || last.progress < first.progress {
        return None;
    }
    let delta_percent = (last.progress - first.progress) as f64 * 100.0;
    let per_hour = delta_percent * (3600.0 / elapsed_secs);
    Some(format!("{per_hour:.2}%"))
}

fn estimate_sync_time_left(app: &DesktopApp) -> Option<u64> {
    let first = app.sync_progress_samples.first()?;
    let last = app.sync_progress_samples.last()?;
    let elapsed_secs = last
        .recorded_at
        .saturating_duration_since(first.recorded_at)
        .as_secs_f64();
    let height_delta = last.local_height.saturating_sub(first.local_height) as f64;
    if elapsed_secs <= f64::EPSILON || height_delta <= f64::EPSILON {
        return None;
    }
    let blocks_per_second = height_delta / elapsed_secs;
    if blocks_per_second <= f64::EPSILON {
        return None;
    }
    let remaining = app
        .view_model
        .sync_target_height()
        .saturating_sub(app.view_model.block_count) as f64;
    Some((remaining / blocks_per_second).max(0.0) as u64)
}

fn age_label(unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(unix);
    if now <= unix {
        return String::from("just now");
    }
    format_duration_human(now - unix)
}

fn format_duration_human(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 48 {
        return format!("{}h {}m", hours, minutes % 60);
    }
    let days = hours / 24;
    if days < 21 {
        return format!("{}d {}h", days, hours % 24);
    }
    let weeks = days / 7;
    format!("{}w {}d", weeks, days % 7)
}

fn render_about_window(app: &mut DesktopApp, ctx: &egui::Context) {
    if !app.show_about_dialog {
        return;
    }

    egui::Window::new("About Atho")
        .collapsible(false)
        .resizable(false)
        .default_size(egui::vec2(460.0, 220.0))
        .open(&mut app.show_about_dialog)
        .show(ctx, |ui| {
            widgets::panel_frame().show(ui, |ui| {
                ui.set_width(430.0);
                ui.horizontal(|ui| {
                    let _ = ui.add(resources::logo_badge(124.0));
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Atho Core")
                                .size(30.0)
                                .strong()
                                .color(widgets::TEXT),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(format!("Version v{}", env!("CARGO_PKG_VERSION")))
                                .size(20.0)
                                .color(widgets::MUTED),
                        );
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new("© 2024-2026 The Atho Core developers")
                                .size(14.0)
                                .color(widgets::MUTED),
                        );
                    });
                });
            });
        });
}
