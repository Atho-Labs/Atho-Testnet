use crate::app::{widgets, CreateWalletForm, DesktopApp, LaunchPage, OpenWalletForm};
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Wallet");
        ui.add_space(12.0);
        ui.label(format!("Network: {}", app.view_model.network_label));
        ui.label(format!("Wallet file: {}", app.wallet_file_label()));
        ui.label(format!(
            "Receive addresses: {}",
            app.ui_state.wallet_snapshot.receive_count
        ));
        ui.label(format!(
            "Change addresses: {}",
            app.ui_state.wallet_snapshot.change_count
        ));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Open Wallet").clicked() {
                app.open_form = OpenWalletForm::new(app.connection.network());
                app.launch_page = LaunchPage::OpenWallet;
                app.wallet = None;
            }
            if ui.button("Create Another Wallet").clicked() {
                app.create_form = CreateWalletForm::new(app.connection.network());
                app.create_form.wallet_path =
                    super::super::alternate_wallet_path(app.connection.network())
                        .to_string_lossy()
                        .into_owned();
                let _ = app.generate_create_mnemonic();
                app.launch_page = LaunchPage::CreateWallet;
                app.wallet = None;
            }
        });
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Mining");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label("Generate coins");
            ui.checkbox(&mut app.ui_state.generate_coins, "");
            ui.add_space(18.0);
            ui.label("Threads");
            ui.add(egui::Slider::new(&mut app.ui_state.mining_cores, 1..=64).show_value(true));
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = app.ui_state.connected && app.wallet.is_some();
            if ui
                .add_enabled(ready, egui::Button::new("Start Miner"))
                .clicked()
            {
                app.start_mining_job();
            }
            if ui.button("Stop Miner").clicked() {
                app.ui_state.generate_coins = false;
                app.mining_status = String::from("Idle");
            }
        });
        ui.add_space(8.0);
        widgets::muted_label(ui, &format!("Miner status: {}", app.mining_status));
        if let Some(job) = &app.mining_job {
            widgets::muted_label(
                ui,
                &format!("Elapsed: {}s", job.started_at.elapsed().as_secs()),
            );
        }
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Client");
        ui.add_space(12.0);
        ui.label(format!("RPC address: {}", app.connection.rpc_address()));
        ui.label(format!("Connected: {}", app.ui_state.connected));
        ui.label(format!("Sync stage: {}", app.view_model.sync_stage));
        ui.label(format!("Block height: {}", app.view_model.block_count));
        ui.label(format!("Mempool entries: {}", app.view_model.mempool_count));
    });
}
