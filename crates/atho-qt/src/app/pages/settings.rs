use crate::app::{
    mnemonic_ui, widgets, CreateWalletForm, DesktopApp, ImportWalletForm, LaunchPage,
    OpenWalletForm,
};
use atho_node::mining_backend::MiningBackendKind;
use atho_rpc::response::{NetworkPeerDiagnostics, NetworkPeerDirection};
use eframe::egui;
use std::time::{SystemTime, UNIX_EPOCH};

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
        ui.label(format!(
            "Configured recovery window: {}",
            app.wallet_configured_recovery_window()
        ));
        ui.label(format!(
            "Active scan window: {}",
            app.wallet_discovery_scan_limit
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
        widgets::muted_label(
            ui,
            "Off by default. The current spend path signs one wallet address at a time, so rotating every mining reward can fragment spendable balance.",
        );
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
        widgets::muted_label(
            ui,
            "Backup export also writes a .meta.json companion file with the wallet's generated and reserved derivation index tips.",
        );
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
        ui.horizontal(|ui| {
            let ready = app.ui_state.connected && app.wallet.is_some();
            if ui
                .add_enabled(ready, egui::Button::new("Mine Once"))
                .on_hover_text("Mine one block with the selected backend and current thread count.")
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
                .on_hover_text("Keep mining blocks until you stop the miner or change settings.")
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

fn mining_backend_label(backend: MiningBackendKind) -> &'static str {
    match backend {
        MiningBackendKind::Auto => "Auto (prefer GPU)",
        MiningBackendKind::Gpu => "GPU (fallback to CPU)",
        MiningBackendKind::Cpu => "CPU only",
    }
}
