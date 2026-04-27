use crate::app::{
    widgets, CreateWalletForm, DesktopApp, ImportWalletForm, LaunchPage, OpenWalletForm,
};
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
                app.clear_wallet_state();
            }
            if ui.button("Create Another Wallet").clicked() {
                app.create_form = CreateWalletForm::new(app.connection.network());
                app.create_form.wallet_path =
                    super::super::alternate_wallet_path(app.connection.network())
                        .to_string_lossy()
                        .into_owned();
                let _ = app.generate_create_mnemonic();
                app.launch_page = LaunchPage::CreateWallet;
                app.clear_wallet_state();
            }
            if ui.button("Import Wallet").clicked() {
                app.import_form = ImportWalletForm::new(app.connection.network());
                app.launch_page = LaunchPage::ImportWallet;
                app.clear_wallet_state();
            }
        });
        ui.add_space(10.0);
        ui.checkbox(
            &mut app.ui_state.rotate_coinbase_address,
            "Rotate coinbase to a fresh receive address",
        );
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Backup & Passphrase");
        ui.add_space(12.0);
        widgets::muted_label(
            ui,
            "Export a backup copy or rotate the current wallet passphrase without leaving the app.",
        );
        ui.add_space(12.0);
        ui.label("Backup path");
        widgets::text_input(ui, &mut app.wallet_management_form.backup_path, "");
        ui.add_space(8.0);
        ui.label("Passphrase");
        ui.add(
            egui::TextEdit::singleline(&mut app.wallet_management_form.backup_password)
                .desired_width(f32::INFINITY)
                .password(!app.wallet_management_form.show_passwords),
        );
        ui.add_space(8.0);
        ui.label("Confirm passphrase");
        ui.add(
            egui::TextEdit::singleline(&mut app.wallet_management_form.backup_password_confirm)
                .desired_width(f32::INFINITY)
                .password(!app.wallet_management_form.show_passwords),
        );
        ui.checkbox(
            &mut app.wallet_management_form.show_passwords,
            "Show passphrases",
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = app.wallet.is_some()
                && app.wallet_management_form.backup_password
                    == app.wallet_management_form.backup_password_confirm;
            if ui
                .add_enabled(ready, egui::Button::new("Export Backup"))
                .clicked()
            {
                match app.export_wallet_backup(
                    &app.wallet_management_form.backup_path,
                    &app.wallet_management_form.backup_password,
                ) {
                    Ok(()) => {
                        app.last_error = None;
                        app.send_status = String::from("Wallet backup exported");
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
            if ui
                .add_enabled(ready, egui::Button::new("Change Passphrase"))
                .clicked()
            {
                match app.change_wallet_passphrase(&app.wallet_management_form.backup_password) {
                    Ok(()) => {
                        app.last_error = None;
                        app.send_status = String::from("Wallet passphrase updated");
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
        });
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Recovery Phrase");
        ui.add_space(12.0);
        if let Some(phrase) = app.wallet_mnemonic_sentence() {
            widgets::muted_label(
                ui,
                "This wallet is unlocked. The recovery phrase is available for review.",
            );
            ui.add_space(10.0);
            let mut phrase_text = phrase;
            ui.add(
                egui::TextEdit::multiline(&mut phrase_text)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Copy recovery phrase").clicked() {
                    DesktopApp::copy_text(ui, phrase_text.clone());
                }
            });
        } else {
            widgets::muted_label(
                ui,
                "No recovery phrase is loaded. Open or import a wallet to unlock it first.",
            );
        }
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
