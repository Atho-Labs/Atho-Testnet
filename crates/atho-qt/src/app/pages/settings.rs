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
            ui.label("Loop blocks");
            ui.checkbox(&mut app.ui_state.generate_coins, "");
            ui.add_space(18.0);
            ui.label("Threads");
            let max_cores = app.max_mining_cores();
            let response = ui.add(
                egui::Slider::new(&mut app.ui_state.mining_cores, 1..=max_cores).show_value(true),
            );
            if response.changed() {
                app.ui_state.mining_cores = app.clamp_mining_cores(app.ui_state.mining_cores);
                if app.mining_job.is_some() && app.ui_state.generate_coins {
                    app.restart_mining_job();
                }
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = app.ui_state.connected && app.wallet.is_some();
            if ui
                .add_enabled(ready, egui::Button::new("Mine Once"))
                .clicked()
            {
                app.ui_state.generate_coins = false;
                if app.mining_job.is_some() {
                    app.restart_mining_job();
                } else {
                    app.start_mining_job();
                }
            }
            if ui
                .add_enabled(ready, egui::Button::new("Mine Loop"))
                .clicked()
            {
                app.ui_state.generate_coins = true;
                if app.mining_job.is_some() {
                    app.restart_mining_job();
                } else {
                    app.start_mining_job();
                }
            }
            if ui.button("Stop Miner").clicked() {
                app.stop_mining_job();
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
        widgets::muted_label(ui, &format!("System max cores: {}", app.max_mining_cores()));
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Client");
        ui.add_space(12.0);
        ui.label(format!("RPC address: {}", app.connection.rpc_address()));
        ui.label(format!("Connected: {}", app.ui_state.connected));
        ui.label(format!("Node running: {}", app.view_model.running));
        ui.label(format!("Headers synced: {}", app.view_model.headers_synced));
        ui.label(format!("Sync stage: {}", app.view_model.sync_stage));
        ui.label(format!("Block height: {}", app.view_model.block_count));
        ui.label(format!(
            "Sync best height: {}",
            app.view_model.sync_best_height
        ));
        ui.label(format!("Mempool entries: {}", app.view_model.mempool_count));
        ui.label(format!(
            "Mempool fees: {} atoms",
            app.view_model.mempool_total_fee_atoms
        ));
    });
}
