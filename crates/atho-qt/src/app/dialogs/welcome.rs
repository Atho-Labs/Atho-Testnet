use crate::app::{widgets, DesktopApp, ImportWalletForm, LaunchPage};
use crate::resources;
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::dialog_frame().show(ui, |ui| {
        ui.set_width(600.0);
        ui.vertical_centered(|ui| {
            let _ = ui.add(resources::logo_mark(128.0));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("No wallet has been loaded.")
                    .size(19.0)
                    .strong()
                    .color(widgets::TEXT),
            );
            ui.add_space(6.0);
            widgets::muted_label(
                ui,
                "Open an existing wallet or create a new Atho HD wallet to continue.",
            );
            ui.add_space(16.0);
            if ui
                .add_sized([210.0, 36.0], egui::Button::new("Create a new wallet"))
                .clicked()
            {
                app.create_form = super::super::CreateWalletForm::new(app.connection.network());
                let _ = app.generate_create_mnemonic();
                app.launch_page = LaunchPage::CreateWallet;
            }
            ui.add_space(8.0);
            if ui
                .add_sized([210.0, 34.0], egui::Button::new("Open wallet"))
                .clicked()
            {
                app.open_form = super::super::OpenWalletForm::new(app.connection.network());
                app.launch_page = LaunchPage::OpenWallet;
            }
            ui.add_space(8.0);
            if ui
                .add_sized([210.0, 34.0], egui::Button::new("Import wallet"))
                .clicked()
            {
                app.import_form = ImportWalletForm::new(app.connection.network());
                app.launch_page = LaunchPage::ImportWallet;
            }
        });
    });
}
