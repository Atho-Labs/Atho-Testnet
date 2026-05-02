use super::{
    pages, widgets, CreateWalletForm, DebugWindowTab, DesktopApp, ImportWalletForm, LaunchPage,
    NavTab, OpenWalletForm,
};
use crate::resources;
use eframe::egui;

pub(crate) fn render_main_shell(app: &mut DesktopApp, ctx: &egui::Context) {
    render_menu_bar(app, ctx);
    render_toolbar(app, ctx);
    render_status_bar(app, ctx);
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
                    if ui.button("Open Wallet").clicked() {
                        app.open_form = OpenWalletForm::new(app.connection.network());
                        app.launch_page = LaunchPage::OpenWallet;
                        app.clear_wallet_state();
                        ui.close_menu();
                    }
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
                    if response.clicked() {
                        app.active_tab = tab;
                    }
                }

                ui.add_space(4.0);
                if ui
                    .add_sized([90.0, 28.0], egui::Button::new("Console"))
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
            let status_text = if app.ui_state.connected {
                String::from("Up to date")
            } else if app.connection.has_local_node() {
                String::from("Synchronizing with network...")
            } else {
                String::from("Disconnected")
            };
            let progress = if app.ui_state.connected {
                1.0
            } else if app.connection.has_local_node() {
                0.0
            } else {
                0.0
            };

            ui.horizontal(|ui| {
                let available = ui.available_width();
                let right_width = (available * 0.34).clamp(220.0, 320.0);
                let left_width = (available - right_width - 6.0).max(140.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, ui.available_height()),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(egui::RichText::new(status_text).color(widgets::TEXT));
                        let progress_width = (left_width * 0.32).clamp(90.0, 210.0);
                        ui.add(egui::ProgressBar::new(progress).desired_width(progress_width));
                        ui.separator();
                        widgets::muted_label(ui, &format!("Height {}", app.view_model.block_count));
                        ui.separator();
                        widgets::muted_label(
                            ui,
                            &format!("Best {}", app.view_model.sync_best_height),
                        );
                        ui.separator();
                        widgets::muted_label(
                            ui,
                            &format!("Mempool {}", app.view_model.mempool_count),
                        );
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
                        let _ = ui.add(resources::sync_icon(16.0, app.ui_state.connected));
                        let _ = ui.add(resources::network_icon(16.0, app.ui_state.connected));
                        ui.separator();
                        let _ = ui.add(resources::hd_enabled_icon(16.0));
                        ui.label(
                            egui::RichText::new("ATHO")
                                .size(13.0)
                                .strong()
                                .color(widgets::TEXT),
                        );
                        ui.separator();
                        widgets::muted_label(ui, &app.view_model.network_label);
                    },
                );
            });
        });
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
