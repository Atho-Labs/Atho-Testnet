use super::{dialogs, widgets, DesktopApp, LaunchPage};
use eframe::egui;

pub(crate) fn render_startup_screen(app: &mut DesktopApp, ctx: &egui::Context) {
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(widgets::SHELL_BG))
        .show(ctx, |ui| {
            widgets::shell_frame()
                .inner_margin(egui::Margin::symmetric(8.0, 8.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(14.0);

                        if !matches!(app.launch_page, LaunchPage::Welcome) {
                            dialogs::render(app, ui);
                        } else {
                            render_welcome_actions(app, ui);
                        }

                        if let Some(error) = &app.last_error {
                            ui.add_space(8.0);
                            ui.colored_label(egui::Color32::from_rgb(170, 77, 50), error);
                        }
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
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Welcome to Atho Core")
            .size(20.0)
            .strong(),
    );
    ui.add_space(6.0);
    ui.horizontal_centered(|ui| {
        if ui
            .add_sized([142.0, 28.0], egui::Button::new("Create Wallet"))
            .clicked()
        {
            app.create_form = super::CreateWalletForm::new(app.connection.network());
            let _ = app.generate_create_mnemonic();
            app.launch_page = LaunchPage::CreateWallet;
        }
        if ui
            .add_sized([138.0, 28.0], egui::Button::new("Open Wallet"))
            .clicked()
        {
            app.open_form = super::OpenWalletForm::new(app.connection.network());
            app.launch_page = LaunchPage::OpenWallet;
        }
        if ui
            .add_sized([138.0, 28.0], egui::Button::new("Import Wallet"))
            .clicked()
        {
            app.import_form = super::ImportWalletForm::new(app.connection.network());
            app.launch_page = LaunchPage::ImportWallet;
        }
    });
    ui.add_space(6.0);
    widgets::muted_label(
        ui,
        "Open an existing wallet or create a new wallet to continue.",
    );
}
