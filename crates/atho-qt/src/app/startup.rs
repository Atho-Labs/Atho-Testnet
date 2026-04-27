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

fn render_welcome_actions(app: &mut DesktopApp, ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Welcome to Atho Core")
            .size(20.0)
            .strong(),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
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
