//! Startup and first-run screens for the desktop client.
use super::{dialogs, widgets, DesktopApp, LaunchPage};
use crate::resources;
use eframe::egui;

pub(crate) fn render_startup_screen(app: &mut DesktopApp, ctx: &egui::Context) {
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(widgets::SHELL_BG))
        .show(ctx, |ui| {
            widgets::shell_frame()
                .inner_margin(egui::Margin::symmetric(12.0, 12.0))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);

                                if !matches!(app.launch_page, LaunchPage::Welcome) {
                                    dialogs::render(app, ui);
                                } else {
                                    render_welcome_actions(app, ui);
                                }

                                if let Some(error) = &app.last_error {
                                    ui.add_space(10.0);
                                    ui.colored_label(egui::Color32::from_rgb(170, 77, 50), error);
                                }
                                ui.add_space(10.0);
                            });
                        });
                });
        });
}

pub(crate) fn render_wallet_preparation_screen(app: &mut DesktopApp, ctx: &egui::Context) {
    let progress = app.wallet_preparation_progress.clamp(0.0, 1.0);
    let (title, stage, show_progress) = if app.wallet_preparation_job.is_some() {
        (
            String::from("Preparing wallet"),
            if app.wallet_preparation_stage.is_empty() {
                String::from("Preparing wallet")
            } else {
                app.wallet_preparation_stage.clone()
            },
            app.wallet_preparation_total > 0,
        )
    } else if !app.ui_state.connected || !app.view_model.running {
        (
            String::from("Starting local node"),
            app.view_model.sync_stage.clone(),
            false,
        )
    } else {
        (
            String::from("Refreshing wallet"),
            String::from("Scanning balances, history, and receive state"),
            false,
        )
    };
    let subtitle = if app.wallet.is_none() {
        "The client stays on this screen until the wallet is ready."
    } else {
        "The client stays on this screen until the wallet state is synchronized."
    };

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(widgets::SHELL_BG))
        .show(ctx, |ui| {
            widgets::shell_frame()
                .inner_margin(egui::Margin::symmetric(24.0, 22.0))
                .show(ui, |ui| {
                    let card_width = ui.available_width().min(560.0);
                    let top_padding = ((ui.available_height() - 260.0) * 0.38).max(18.0);
                    ui.add_space(top_padding);
                    ui.vertical_centered(|ui| {
                        egui::Frame::none()
                            .fill(widgets::PANEL_BG)
                            .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE))
                            .inner_margin(egui::Margin::symmetric(28.0, 24.0))
                            .show(ui, |ui| {
                                ui.set_width(card_width);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new("Atho")
                                            .size(18.0)
                                            .strong()
                                            .color(widgets::ACCENT),
                                    );
                                    ui.add_space(10.0);
                                    ui.add(egui::Spinner::new().size(26.0));
                                    ui.add_space(12.0);
                                    ui.label(
                                        egui::RichText::new(title)
                                            .size(24.0)
                                            .strong()
                                            .color(widgets::TEXT),
                                    );
                                    ui.add_space(8.0);
                                    widgets::muted_label(ui, subtitle);
                                    ui.add_space(16.0);
                                    ui.label(
                                        egui::RichText::new(stage)
                                            .size(13.0)
                                            .strong()
                                            .color(widgets::TEXT),
                                    );
                                    if show_progress && app.wallet_preparation_total > 0 {
                                        ui.add_space(6.0);
                                        widgets::muted_label(
                                            ui,
                                            &format!(
                                                "{} / {} steps",
                                                app.wallet_preparation_completed,
                                                app.wallet_preparation_total
                                            ),
                                        );
                                    }
                                    ui.add_space(14.0);
                                    if show_progress {
                                        ui.add(
                                            egui::ProgressBar::new(progress)
                                                .desired_width((card_width - 56.0).max(240.0))
                                                .animate(true),
                                        );
                                    } else {
                                        ui.add(egui::Spinner::new().size(20.0));
                                    }
                                });
                            });
                    });
                });
        });
}

fn render_welcome_actions(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let content_width = ui.available_width().min(1080.0);
    let compact = content_width < 860.0;
    let card_width = (content_width * if compact { 0.98 } else { 0.94 }).min(1000.0);
    let action_width = if compact {
        (card_width - 32.0).max(260.0)
    } else {
        ((card_width - 40.0) / 2.0).max(220.0)
    };

    ui.add_space(10.0);
    egui::Frame::none()
        .fill(widgets::ACCENT_SOFT)
        .stroke(egui::Stroke::new(1.0, widgets::ACCENT))
        .inner_margin(egui::Margin::symmetric(24.0, 24.0))
        .show(ui, |ui| {
            ui.set_width(card_width);
            if compact {
                ui.vertical_centered(|ui| {
                    let _ = ui.add(resources::logo_mark(74.0));
                    ui.add_space(10.0);
                    render_welcome_copy(app, ui, true);
                    ui.add_space(18.0);
                    render_action_buttons(app, ui, action_width, true);
                });
            } else {
                ui.columns(2, |columns| {
                    columns[0].vertical(|ui| {
                        let _ = ui.add(resources::logo_mark(92.0));
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Atho Core")
                                .size(15.0)
                                .strong()
                                .color(widgets::ACCENT),
                        );
                    });
                    columns[1].vertical(|ui| {
                        render_welcome_copy(app, ui, false);
                        ui.add_space(18.0);
                        render_action_buttons(app, ui, action_width, false);
                    });
                });
            }
        });

    ui.add_space(16.0);
    ui.set_width(card_width);
    if compact {
        render_info_card(
            ui,
            "First Run",
            &[
                "Create or import a wallet.",
                "Use regnet or testnet first to keep the workflow local.",
                "Open Settings to confirm the active mining backend.",
            ],
            "The Mining panel shows the effective backend and any fallback reason before you leave the app running.",
        );
        ui.add_space(10.0);
        render_info_card(
            ui,
            "Safety",
            &[
                "Write down the recovery phrase when it is shown.",
                "Do not paste secrets into the debug console.",
                "Wallet encryption is optional but recommended.",
            ],
            "The launch screens now scroll on smaller windows, including recovery-phrase workflows.",
        );
    } else {
        ui.columns(2, |columns| {
            render_info_card(
                &mut columns[0],
                "First Run",
                &[
                    "Create or import a wallet.",
                    "Use regnet or testnet first to keep the workflow local.",
                    "Open Settings to confirm the active mining backend.",
                ],
                "The Mining panel shows the effective backend and any fallback reason before you leave the app running.",
            );
            render_info_card(
                &mut columns[1],
                "Safety",
                &[
                    "Write down the recovery phrase when it is shown.",
                    "Do not paste secrets into the debug console.",
                    "Wallet encryption is optional but recommended.",
                ],
                "The launch screens now scroll on smaller windows, including recovery-phrase workflows.",
            );
        });
    }
}

fn render_welcome_copy(app: &DesktopApp, ui: &mut egui::Ui, centered: bool) {
    let network = app.connection.network().id();
    if centered {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Welcome to Atho Core")
                    .size(34.0)
                    .strong()
                    .color(widgets::ACCENT),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("A production-focused Atho node, wallet, and miner client.")
                    .size(17.0)
                    .color(widgets::TEXT),
            );
            ui.add_space(10.0);
            widgets::muted_label(
                ui,
                &format!(
                    "Current network target: {}. You can create a wallet or restore from a recovery phrase below and move straight into the full desktop client.",
                    network
                ),
            );
        });
    } else {
        ui.label(
            egui::RichText::new("Welcome to Atho Core")
                .size(34.0)
                .strong()
                .color(widgets::ACCENT),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("A production-focused Atho node, wallet, and miner client.")
                .size(17.0)
                .color(widgets::TEXT),
        );
        ui.add_space(10.0);
        widgets::muted_label(
            ui,
            &format!(
                "Current network target: {}. Create a new HD wallet or restore from a recovery phrase.",
                network
            ),
        );
    }
}

fn render_action_buttons(app: &mut DesktopApp, ui: &mut egui::Ui, width: f32, compact: bool) {
    let render_button = |ui: &mut egui::Ui, label: &str, hint: &str| {
        ui.vertical(|ui| {
            ui.add_sized(
                [width, 36.0],
                egui::Button::new(egui::RichText::new(label).size(16.0).strong()),
            )
            .on_hover_text(hint)
        })
        .inner
    };

    if compact {
        if render_button(
            ui,
            "Create Wallet",
            "Create a new Atho HD wallet and show the recovery phrase once.",
        )
        .clicked()
        {
            app.create_form = super::CreateWalletForm::new(app.connection.network());
            let _ = app.generate_create_mnemonic();
            app.launch_page = LaunchPage::CreateWallet;
        }
        ui.add_space(8.0);
        if render_button(
            ui,
            "Import Wallet",
            "Restore a wallet from an existing recovery phrase.",
        )
        .clicked()
        {
            app.import_form = super::ImportWalletForm::new(app.connection.network());
            app.launch_page = LaunchPage::ImportWallet;
        }
    } else {
        ui.horizontal(|ui| {
            if render_button(
                ui,
                "Create Wallet",
                "Create a new Atho HD wallet and show the recovery phrase once.",
            )
            .clicked()
            {
                app.create_form = super::CreateWalletForm::new(app.connection.network());
                let _ = app.generate_create_mnemonic();
                app.launch_page = LaunchPage::CreateWallet;
            }
            if render_button(
                ui,
                "Import Wallet",
                "Restore a wallet from an existing recovery phrase.",
            )
            .clicked()
            {
                app.import_form = super::ImportWalletForm::new(app.connection.network());
                app.launch_page = LaunchPage::ImportWallet;
            }
        });
    }
}

fn render_info_card(ui: &mut egui::Ui, title: &str, lines: &[&str], footer: &str) {
    egui::Frame::none()
        .fill(widgets::PANEL_BG)
        .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE))
        .inner_margin(egui::Margin::symmetric(18.0, 16.0))
        .show(ui, |ui| {
            widgets::section_header(ui, title);
            ui.add_space(8.0);
            for line in lines {
                ui.label(
                    egui::RichText::new(format!("• {line}"))
                        .size(14.0)
                        .color(widgets::TEXT),
                );
                ui.add_space(4.0);
            }
            ui.add_space(6.0);
            widgets::muted_label(ui, footer);
        });
}
