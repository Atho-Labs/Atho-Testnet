// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use crate::app::{
    mnemonic_ui, widgets, CreateWalletForm, DesktopApp, ImportWalletForm, LaunchPage,
};
use atho_node::mining_backend::MiningBackendKind;
use atho_rpc::response::{NetworkPeerDiagnostics, NetworkPeerDirection};
use eframe::egui;
use rfd::FileDialog;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Wallet");
        ui.add_space(12.0);
        ui.label(format!("Wallet name: {}", app.wallet_display_name()));
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
        ui.label(format!(
            "Configured recovery window: {}",
            app.wallet_configured_recovery_window()
        ));
        ui.label(format!(
            "Active scan window: {}",
            app.wallet_discovery_scan_limit
        ));
        ui.add_space(12.0);
        if app.wallet_path.is_some() {
            ui.label("Wallet name");
            if app.current_wallet_name.is_none() {
                app.current_wallet_name = Some(app.wallet_display_name());
            }
            if let Some(wallet_name) = app.current_wallet_name.as_mut() {
                widgets::text_input(ui, wallet_name, "Wallet 1");
            }
            ui.add_space(8.0);
            if ui
                .add_enabled(app.wallet.is_some(), egui::Button::new("Save Wallet Name"))
                .clicked()
            {
                match app.rename_current_wallet() {
                    Ok(()) => {
                        app.last_error = None;
                        app.send_status = String::from("Wallet name saved");
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
            ui.add_space(12.0);
        }
        ui.horizontal(|ui| {
            if ui.button("Create Wallet").clicked() {
                app.create_form = CreateWalletForm::new(app.connection.network());
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
        widgets::muted_label(
            ui,
            "Off by default. The current spend path signs one wallet address at a time, so rotating every mining reward can fragment spendable balance.",
        );
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Known Wallets");
        ui.add_space(12.0);
        let entries = app.wallet_registry_entries();
        if entries.is_empty() {
            widgets::muted_label(ui, "No saved wallets are registered yet.");
        } else {
            for entry in entries {
                let is_current = app.wallet_path_matches(&entry.wallet_path);
                egui::Frame::none()
                    .fill(widgets::SHELL_BG)
                    .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&entry.wallet_name)
                                        .size(14.0)
                                        .strong()
                                        .color(widgets::TEXT),
                                );
                                widgets::muted_label(
                                    ui,
                                    &format!(
                                        "{} words · updated {}",
                                        if entry.word_count > 0 {
                                            entry.word_count.to_string()
                                        } else {
                                            String::from("unknown")
                                        },
                                        format_recent(Some(entry.updated_at_unix))
                                    ),
                                );
                                widgets::muted_label(ui, &entry.wallet_path);
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Copy Path").clicked() {
                                        DesktopApp::copy_text(ui, entry.wallet_path.clone());
                                    }
                                    if ui
                                        .add_enabled(!is_current, egui::Button::new("Switch"))
                                        .clicked()
                                    {
                                        if let Err(err) =
                                            app.begin_wallet_switch(&entry.wallet_path)
                                        {
                                            app.last_error = Some(err);
                                        }
                                    }
                                },
                            );
                        });
                    });
                ui.add_space(8.0);
            }
        }
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Wallet Index Recovery");
        ui.add_space(12.0);
        widgets::muted_label(
            ui,
            "The address-pool page starts with the first discovery window for responsiveness. Raise the recovery window here when a restored wallet may have activity farther out on the derivation index.",
        );
        ui.add_space(12.0);
        let (receive_keypool_queued, change_keypool_queued) = app.wallet_keypool_depths();
        let (highest_generated_receive_index, highest_generated_change_index) =
            app.wallet_highest_generated_indices();
        let (highest_reserved_receive_index, highest_reserved_change_index) =
            app.wallet_highest_reserved_indices();
        ui.label(format!(
            "Queued keypool: {} receive / {} change",
            receive_keypool_queued, change_keypool_queued
        ));
        ui.label(format!(
            "Highest generated indexes: {} / {}",
            highest_generated_receive_index
                .map(|index| format!("R{index:04}"))
                .unwrap_or_else(|| String::from("none")),
            highest_generated_change_index
                .map(|index| format!("C{index:04}"))
                .unwrap_or_else(|| String::from("none")),
        ));
        ui.label(format!(
            "Highest reserved indexes: {} / {}",
            highest_reserved_receive_index
                .map(|index| format!("R{index:04}"))
                .unwrap_or_else(|| String::from("none")),
            highest_reserved_change_index
                .map(|index| format!("C{index:04}"))
                .unwrap_or_else(|| String::from("none")),
        ));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Recovery window");
            ui.add_sized(
                [120.0, 28.0],
                egui::TextEdit::singleline(
                    &mut app.wallet_management_form.restore_gap_limit_input,
                ),
            );
            if ui
                .add_enabled(app.wallet.is_some(), egui::Button::new("Apply & Rescan"))
                .clicked()
            {
                match app.apply_wallet_recovery_window_setting() {
                    Ok(message) => {
                        app.last_error = None;
                        app.send_status = message;
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
        });
        widgets::muted_label(
            ui,
            "This updates the active scan immediately in memory. Export a backup or rewrite the wallet file if you want the new recovery window preserved outside this session.",
        );
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Backup & Passphrase");
        ui.add_space(12.0);
        widgets::muted_label(
            ui,
            "Export the active wallet as an encrypted backup, or export recovery details as JSON, TXT, or QR.",
        );
        ui.add_space(12.0);
        ui.label("Encrypted backup path");
        render_browse_save_row(
            ui,
            &mut app.wallet_management_form.backup_path,
            "Save backup as",
            None,
        );
        ui.add_space(8.0);
        ui.label("Recovery JSON path");
        render_browse_save_row(
            ui,
            &mut app.wallet_management_form.backup_json_path,
            "Save JSON as",
            Some(("JSON", &["json"])),
        );
        ui.add_space(8.0);
        ui.label("Recovery TXT path");
        render_browse_save_row(
            ui,
            &mut app.wallet_management_form.backup_text_path,
            "Save TXT as",
            Some(("Text", &["txt"])),
        );
        ui.add_space(8.0);
        ui.label("Recovery QR PNG path");
        render_browse_save_row(
            ui,
            &mut app.wallet_management_form.backup_phrase_qr_path,
            "Save PNG as",
            Some(("PNG", &["png"])),
        );
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
        ui.horizontal_wrapped(|ui| {
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
                .add_enabled(app.wallet.is_some(), egui::Button::new("Export JSON"))
                .clicked()
            {
                match app.export_wallet_recovery_json(&app.wallet_management_form.backup_json_path) {
                    Ok(()) => {
                        app.last_error = None;
                        app.send_status = String::from("Wallet recovery JSON exported");
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
            if ui
                .add_enabled(app.wallet.is_some(), egui::Button::new("Export TXT"))
                .clicked()
            {
                match app.export_wallet_recovery_text(&app.wallet_management_form.backup_text_path) {
                    Ok(()) => {
                        app.last_error = None;
                        app.send_status = String::from("Wallet recovery TXT exported");
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
            if ui
                .add_enabled(app.wallet.is_some(), egui::Button::new("Export Phrase QR"))
                .clicked()
            {
                match app.export_wallet_recovery_phrase_qr(
                    &app.wallet_management_form.backup_phrase_qr_path,
                ) {
                    Ok(()) => {
                        app.last_error = None;
                        app.send_status = String::from("Wallet recovery QR exported");
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
            if ui
                .add_enabled(ready, egui::Button::new("Change Passphrase"))
                .clicked()
            {
                let password = app.wallet_management_form.backup_password.clone();
                match app.change_wallet_passphrase(&password) {
                    Ok(()) => {
                        app.last_error = None;
                        app.send_status = String::from("Wallet passphrase updated");
                    }
                    Err(err) => app.last_error = Some(err),
                }
            }
        });
        widgets::muted_label(
            ui,
            "Backup export also writes a .meta.json companion file with the wallet's generated and reserved derivation index tips. Recovery phrase QR exports use a red label band so sensitive HD wallet material is easy to identify later.",
        );
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Recovery Phrase");
        ui.add_space(12.0);
        if let Some(phrase) = app.wallet_mnemonic_sentence() {
            let (next_receive_index, next_change_index) = app.wallet_next_indices();
            let (highest_generated_receive_index, highest_generated_change_index) =
                app.wallet_highest_generated_indices();
            let (highest_reserved_receive_index, highest_reserved_change_index) =
                app.wallet_highest_reserved_indices();
            widgets::muted_label(
                ui,
                "This wallet is unlocked. The recovery phrase is available for review.",
            );
            ui.add_space(10.0);
            if let Some(mnemonic) = app.wallet_ref().and_then(|wallet| wallet.mnemonic_phrase()) {
                ui.label(format!("Mnemonic words: {}", mnemonic.word_count()));
            }
            ui.label(format!(
                "Current receive index: {}",
                app.wallet_current_receive_index()
                    .map(|index| format!("R{index:04}"))
                    .unwrap_or_else(|| String::from("Unavailable"))
            ));
            ui.label(format!("Next receive index: R{next_receive_index:04}"));
            ui.label(format!("Next change index: C{next_change_index:04}"));
            ui.label(format!(
                "Highest generated: {} / {}",
                highest_generated_receive_index
                    .map(|index| format!("R{index:04}"))
                    .unwrap_or_else(|| String::from("Unavailable")),
                highest_generated_change_index
                    .map(|index| format!("C{index:04}"))
                    .unwrap_or_else(|| String::from("Unavailable")),
            ));
            ui.label(format!(
                "Highest reserved: {} / {}",
                highest_reserved_receive_index
                    .map(|index| format!("R{index:04}"))
                    .unwrap_or_else(|| String::from("Unavailable")),
                highest_reserved_change_index
                    .map(|index| format!("C{index:04}"))
                    .unwrap_or_else(|| String::from("Unavailable")),
            ));
            ui.add_space(10.0);
            let mut phrase_words = mnemonic_ui::words_from_sentence(&phrase);
            mnemonic_ui::render_word_grid(
                ui,
                &mut phrase_words,
                false,
                "settings_recovery_phrase_grid",
                false,
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Copy recovery phrase").clicked() {
                    DesktopApp::copy_text(ui, phrase.clone());
                }
                if ui.button("Save recovery QR").clicked() {
                    let mut dialog = FileDialog::new().add_filter("PNG", &["png"]);
                    if let Some(file_name) =
                        std::path::Path::new(&app.wallet_management_form.backup_phrase_qr_path)
                            .file_name()
                            .and_then(|name| name.to_str())
                    {
                        dialog = dialog.set_file_name(file_name);
                    }
                    if let Some(parent) =
                        std::path::Path::new(&app.wallet_management_form.backup_phrase_qr_path)
                            .parent()
                    {
                        dialog = dialog.set_directory(parent);
                    }
                    if let Some(path) = dialog.save_file() {
                        let selected =
                            crate::app::normalize_png_export_path(&path.to_string_lossy())
                                .to_string_lossy()
                                .into_owned();
                        app.wallet_management_form.backup_phrase_qr_path = selected.clone();
                        match app.export_wallet_recovery_phrase_qr(&selected) {
                            Ok(()) => {
                                app.last_error = None;
                                app.send_status = String::from("Wallet recovery QR exported");
                            }
                            Err(err) => app.last_error = Some(err),
                        }
                    }
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
        widgets::section_header(ui, "Storage & Paths");
        ui.add_space(12.0);
        widgets::muted_label(
            ui,
            "These paths are network-specific. Raw block files live under the block-storage root, while LMDB metadata, indexes, and chainstate live under the network data root.",
        );
        ui.add_space(12.0);
        render_copyable_path_row(ui, "Operator root", &app.operator_root_label());
        render_copyable_path_row(ui, "Network data root", &app.network_data_root_label());
        render_copyable_path_row(ui, "Block storage root", &app.block_storage_root_label());
        render_copyable_path_row(ui, "Chain recovery root", &app.chain_recovery_root_label());
        render_copyable_path_row(ui, "Quarantine root", &app.quarantine_root_label());
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Mining");
        ui.add_space(12.0);
        widgets::muted_label(
            ui,
            "Atho prefers GPU when available and falls back to CPU when GPU probing, initialization, or execution fails. CPU-only mode skips accelerator checks entirely.",
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Backend");
            let mut selected_backend = app.ui_state.mining_backend;
            egui::ComboBox::from_id_source("settings_mining_backend")
                .selected_text(mining_backend_label(selected_backend))
                .show_ui(ui, |ui: &mut egui::Ui| {
                    for backend in MiningBackendKind::variants() {
                        ui.selectable_value(
                            &mut selected_backend,
                            backend,
                            mining_backend_label(backend),
                        );
                    }
                });
            if selected_backend != app.ui_state.mining_backend {
                app.ui_state.set_mining_backend(selected_backend);
                app.refresh_mining_accelerator_info();
                if app.mining_job.is_some() {
                    app.restart_mining_job();
                } else {
                    app.mining_status = String::from("Idle");
                }
            }
        })
        .response
        .on_hover_text(
            "Auto prefers GPU and safely falls back to CPU. CPU-only skips accelerator probing. GPU mode now also falls back to CPU if GPU probing or execution fails.",
        );
        ui.add_space(8.0);
        if matches!(app.ui_state.mining_backend, MiningBackendKind::Cpu) {
            widgets::muted_label(
                ui,
                "CPU-only mode is active. The miner will skip GPU probing and run on CPU threads only.",
            );
        } else {
            widgets::muted_label(
                ui,
                &format!(
                    "Detected accelerator: {}",
                    app.mining_accelerator_info.summary()
                ),
            );
            if let Some(device) = app.mining_accelerator_info.device_name.as_deref() {
                ui.label(format!("Device: {device}"));
            }
            if let Some(vendor) = app.mining_accelerator_info.vendor.as_deref() {
                ui.label(format!("Vendor: {vendor}"));
            }
            if let Some(driver) = app.mining_accelerator_info.driver.as_deref() {
                ui.label(format!("Driver: {driver}"));
            }
            ui.label(format!(
                "Device type: {}",
                app.mining_accelerator_info.device_type.label()
            ));
            if let Some(compute_units) = app.mining_accelerator_info.compute_units {
                ui.label(format!("Compute units: {compute_units}"));
            }
            if let Some(global_mem_mb) = app.mining_accelerator_info.global_mem_mb {
                ui.label(format!("Global memory: {global_mem_mb} MiB"));
            }
            if let Some(local_mem_kb) = app.mining_accelerator_info.local_mem_kb {
                ui.label(format!("Local memory: {local_mem_kb} KiB"));
            }
            if let Some(clock_mhz) = app.mining_accelerator_info.clock_mhz {
                ui.label(format!("Clock: {clock_mhz} MHz"));
            }
            if let Some(code) = app.mining_accelerator_info.reason_code.as_deref() {
                ui.label(format!("Probe code: {code}"));
            }
            if let Some(reason) = app.mining_accelerator_info.reason_if_not.as_deref() {
                widgets::muted_label(ui, &format!("Probe note: {reason}"));
            }
        }
        ui.add_space(10.0);
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
        let mining_block_reason = app.wallet_mining_block_reason();
        let mining_ready = mining_block_reason.is_none();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(mining_ready, egui::Button::new("Mine Once"))
                .on_hover_text(
                    mining_block_reason
                        .as_deref()
                        .unwrap_or("Mine one block with the selected backend and current thread count."),
                )
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
                .add_enabled(mining_ready, egui::Button::new("Mine Loop"))
                .on_hover_text(
                    mining_block_reason
                        .as_deref()
                        .unwrap_or("Keep mining blocks until you stop the miner or change settings."),
                )
                .clicked()
            {
                app.ui_state.generate_coins = true;
                if app.mining_job.is_some() {
                    app.restart_mining_job();
                } else {
                    app.start_mining_job();
                }
            }
            if ui
                .button("Stop Miner")
                .on_hover_text("Stop the active miner loop and return the miner to Idle.")
                .clicked()
            {
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
        ui.label(format!("Chain synced: {}", app.view_model.chain_synced()));
        ui.label(format!("Sync stage: {}", app.view_model.sync_stage));
        ui.label(format!("Block height: {}", app.view_model.block_count));
        ui.label(format!(
            "Sync target height: {}",
            app.view_model.sync_target_height()
        ));
        ui.label(format!("Mempool entries: {}", app.view_model.mempool_count));
        ui.label(format!(
            "Mempool fees: {} atoms",
            app.view_model.mempool_total_fee_atoms
        ));
        ui.label(format!(
            "Connected peers: {} (inbound {} / outbound {})",
            app.view_model.peer_count,
            app.view_model.inbound_peer_count,
            app.view_model.outbound_peer_count
        ));
        ui.label(format!(
            "Connecting peers: {}",
            app.view_model.connecting_peer_count
        ));
        ui.label(format!(
            "Network traffic: sent {} / received {}",
            format_bytes(app.view_model.bytes_sent),
            format_bytes(app.view_model.bytes_received)
        ));
    });

    ui.add_space(14.0);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Peer Diagnostics");
        ui.add_space(12.0);
        widgets::muted_label(
            ui,
            "Peer details are shown only in this local diagnostics view. Public RPC remains loopback-only by default.",
        );
        ui.add_space(12.0);
        if app.view_model.peers.is_empty() {
            widgets::muted_label(ui, "No connected peers.");
            if app.view_model.connecting_peers.is_empty() {
                return;
            }
        }

        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("peer_diagnostics_grid")
                    .num_columns(10)
                    .spacing([12.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Dir");
                        ui.strong("Endpoint");
                        ui.strong("State");
                        ui.strong("Height");
                        ui.strong("Proto");
                        ui.strong("Agent");
                        ui.strong("Sent");
                        ui.strong("Recv");
                        ui.strong("Last Rx");
                        ui.strong("Quality");
                        ui.end_row();

                        for peer in &app.view_model.peers {
                            ui.label(peer_direction_label(peer.direction));
                            widgets::elided_label(ui, &peer.remote_addr, 24);
                            ui.label(if peer.handshake_ready {
                                "Ready"
                            } else {
                                "Handshake"
                            });
                            ui.label(
                                peer.best_height
                                    .map(|height| height.to_string())
                                    .unwrap_or_else(|| String::from("-")),
                            );
                            ui.label(
                                peer.protocol_version
                                    .map(|version| version.to_string())
                                    .unwrap_or_else(|| String::from("-")),
                            );
                            widgets::elided_label(
                                ui,
                                peer.user_agent
                                    .as_deref()
                                    .unwrap_or("-"),
                                20,
                            );
                            ui.label(format_bytes(peer.bytes_sent));
                            ui.label(format_bytes(peer.bytes_received));
                            ui.label(format_recent(peer.last_receive_unix));
                            ui.label(format_quality(peer));
                            ui.end_row();
                        }
                    });

                if !app.view_model.connecting_peers.is_empty() {
                    ui.add_space(12.0);
                    widgets::muted_label(
                        ui,
                        "Pending outbound connection attempts",
                    );
                    ui.add_space(6.0);
                    for peer in &app.view_model.connecting_peers {
                        ui.horizontal(|ui| {
                            ui.label(peer_direction_label(peer.direction));
                            widgets::elided_label(ui, &peer.remote_addr, 28);
                            ui.label("Handshake pending");
                        });
                    }
                }
            });
    });
}

fn peer_direction_label(direction: NetworkPeerDirection) -> &'static str {
    match direction {
        NetworkPeerDirection::Inbound => "In",
        NetworkPeerDirection::Outbound => "Out",
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;

    if bytes >= MIB as u64 {
        format!("{:.1} MiB", bytes as f64 / MIB)
    } else if bytes >= KIB as u64 {
        format!("{:.1} KiB", bytes as f64 / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn format_recent(last_unix: Option<u64>) -> String {
    let Some(last_unix) = last_unix else {
        return String::from("-");
    };
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let age = now_unix.saturating_sub(last_unix);
    if age < 60 {
        format!("{age}s ago")
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else {
        format!("{}h ago", age / 3600)
    }
}

fn format_quality(peer: &NetworkPeerDiagnostics) -> String {
    match (peer.quality_score, peer.consecutive_failures) {
        (Some(score), Some(failures)) => format!("{score} / fail {failures}"),
        (Some(score), None) => score.to_string(),
        _ => String::from("-"),
    }
}

fn render_copyable_path_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(format!("{label}:"));
        widgets::elided_label(ui, value, 44);
        if ui
            .button("Copy")
            .on_hover_text("Copy this path to the clipboard.")
            .clicked()
        {
            DesktopApp::copy_text(ui, value.to_string());
        }
    });
}

fn render_browse_save_row(
    ui: &mut egui::Ui,
    value: &mut String,
    button_label: &str,
    filter: Option<(&str, &[&str])>,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width((ui.available_width() - 112.0).max(160.0)),
        );
        if ui
            .add_sized([96.0, 28.0], egui::Button::new(button_label))
            .clicked()
        {
            let mut dialog = FileDialog::new();
            if let Some((name, extensions)) = filter {
                dialog = dialog.add_filter(name, extensions);
            }
            if let Some(path) = dialog.save_file() {
                *value = path.to_string_lossy().into_owned();
            }
        }
    });
}

fn mining_backend_label(backend: MiningBackendKind) -> &'static str {
    match backend {
        MiningBackendKind::Auto => "Auto (prefer GPU)",
        MiningBackendKind::Gpu => "GPU (fallback to CPU)",
        MiningBackendKind::Cpu => "CPU only",
    }
}
