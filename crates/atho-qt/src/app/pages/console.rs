use crate::app::{widgets, DebugWindowTab, DesktopApp};
use atho_rpc::response::{NetworkPeerDiagnostics, NetworkPeerDirection};
use eframe::egui;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    app.open_debug_window(DebugWindowTab::Console);
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Node Window");
        ui.add_space(8.0);
        widgets::muted_label(
            ui,
            "The debug console moved into a separate node window. Use the Console button or Window menu to reopen it.",
        );
    });
}

pub(crate) fn render_window(app: &mut DesktopApp, ctx: &egui::Context) {
    if !app.show_debug_window {
        return;
    }

    let viewport_id = egui::ViewportId::from_hash_of("atho_node_window");
    let builder = egui::ViewportBuilder::default()
        .with_title("Node Window")
        .with_inner_size([960.0, 620.0])
        .with_min_inner_size([780.0, 500.0]);

    ctx.show_viewport_immediate(viewport_id, builder, |ctx, class| {
        if ctx.input(|input| input.viewport().close_requested())
            || ctx.input(|input| input.key_pressed(egui::Key::Escape))
        {
            app.close_debug_window();
            return;
        }

        if matches!(class, egui::ViewportClass::Embedded) {
            let mut open = app.show_debug_window;
            egui::Window::new("Node Window")
                .default_size(egui::vec2(900.0, 560.0))
                .min_size(egui::vec2(760.0, 460.0))
                .open(&mut open)
                .show(ctx, |ui| render_window_contents(app, ui));
            app.show_debug_window = open;
            return;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(widgets::SHELL_BG))
            .show(ctx, |ui| {
                widgets::shell_frame().show(ui, |ui| {
                    render_window_contents(app, ui);
                });
            });
    });
}

fn render_window_contents(app: &mut DesktopApp, ui: &mut egui::Ui) {
    render_tab_bar(app, ui);
    ui.add_space(10.0);
    match app.debug_window_tab {
        DebugWindowTab::Information => render_information_tab(app, ui),
        DebugWindowTab::Console => render_console_tab(app, ui),
        DebugWindowTab::NetworkTraffic => render_network_traffic_tab(app, ui),
        DebugWindowTab::Peers => render_peers_tab(app, ui),
    }
}

fn render_tab_bar(app: &mut DesktopApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for tab in DebugWindowTab::variants() {
            if widgets::compact_tab(ui, app.debug_window_tab == tab, tab.label(), 118.0).clicked() {
                app.debug_window_tab = tab;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close Window").clicked() {
                app.close_debug_window();
            }
        });
    });
}

fn render_information_tab(app: &DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Information");
        ui.add_space(10.0);
        info_row(ui, "Network", &app.view_model.network_label);
        info_row(
            ui,
            "Node",
            if app.view_model.running {
                "Running"
            } else {
                "Stopped"
            },
        );
        info_row(ui, "Local Height", &app.view_model.block_count.to_string());
        info_row(
            ui,
            "Sync Target",
            &app.view_model.sync_target_height().to_string(),
        );
        info_row(
            ui,
            "Headers",
            if app.view_model.headers_synced {
                "Synced"
            } else {
                "Syncing"
            },
        );
        info_row(
            ui,
            "Chain Sync",
            if app.view_model.chain_synced() {
                "Caught up"
            } else {
                "Behind target"
            },
        );
        info_row(ui, "Peers", &app.view_model.peer_count.to_string());
        info_row(
            ui,
            "Connecting Peers",
            &app.view_model.connecting_peer_count.to_string(),
        );
        info_row(
            ui,
            "Inbound Peers",
            &app.view_model.inbound_peer_count.to_string(),
        );
        info_row(
            ui,
            "Outbound Peers",
            &app.view_model.outbound_peer_count.to_string(),
        );
        info_row(
            ui,
            "Mempool",
            &format!(
                "{} tx / {}",
                app.view_model.mempool_count,
                widgets::format_atoms(app.view_model.mempool_total_fee_atoms)
            ),
        );
        info_row(
            ui,
            "Bytes Received",
            &format_bytes(app.view_model.bytes_received),
        );
        info_row(ui, "Bytes Sent", &format_bytes(app.view_model.bytes_sent));
        info_row(ui, "Sync Stage", &app.view_model.sync_stage);
        if let Some(error) = app.last_error.as_ref() {
            ui.add_space(12.0);
            ui.colored_label(
                egui::Color32::from_rgb(170, 77, 50),
                format!("Latest error: {error}"),
            );
        }
    });
}

fn render_console_tab(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            widgets::section_header(ui, "Console");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Copy Last").clicked() {
                    app.copy_latest_debug_console_output(ui);
                }
                if ui.button("Clear").clicked() {
                    app.clear_debug_console();
                }
                if ui.button("A+").clicked() {
                    app.debug_console_font_size = (app.debug_console_font_size + 1.0).min(18.0);
                }
                if ui.button("A-").clicked() {
                    app.debug_console_font_size = (app.debug_console_font_size - 1.0).max(11.0);
                }
            });
        });

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Network: {}", app.view_model.network_label));
            ui.separator();
            ui.label(format!("Peers: {}", app.view_model.peer_count));
            if app.view_model.connecting_peer_count > 0 {
                ui.separator();
                ui.label(format!(
                    "Connecting: {}",
                    app.view_model.connecting_peer_count
                ));
            }
            ui.separator();
            ui.label(format!(
                "Node: {}",
                if app.view_model.running {
                    "running"
                } else {
                    "stopped"
                }
            ));
            ui.separator();
            for mode in [
                super::super::DebugConsoleOutputMode::Pretty,
                super::super::DebugConsoleOutputMode::Json,
                super::super::DebugConsoleOutputMode::Table,
            ] {
                ui.selectable_value(&mut app.debug_console_output_mode, mode, mode.label());
            }
            ui.separator();
            ui.checkbox(
                &mut app.debug_console_confirmed,
                "Confirm dangerous commands",
            );
        });
        if !app.debug_console_status.is_empty() {
            ui.add_space(6.0);
            widgets::muted_label(ui, &app.debug_console_status);
        }

        ui.add_space(8.0);
        let output_height = (ui.available_height() - 92.0).max(220.0);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), output_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                widgets::panel_frame()
                    .fill(egui::Color32::from_rgb(252, 252, 251))
                    .show(ui, |ui| {
                        ui.set_min_height(output_height - 18.0);
                        egui::ScrollArea::both()
                            .id_source("node_window_console_output")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                render_console_welcome(app, ui);
                                for entry in &app.debug_console_entries {
                                    render_console_entry(app, ui, entry);
                                    ui.add_space(6.0);
                                }
                            });
                    });
            },
        );

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let prompt = ui.label(
                egui::RichText::new(">")
                    .monospace()
                    .size(app.debug_console_font_size + 2.0)
                    .strong(),
            );
            prompt.on_hover_text("Enter a local Atho RPC command.");
            let run_width = 68.0;
            let input_width = (ui.available_width() - run_width - 8.0).max(220.0);
            let response = ui.add_sized(
                [input_width, 28.0],
                egui::TextEdit::singleline(&mut app.debug_console_input)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Type help for grouped commands"),
            );
            response.context_menu(|ui| {
                if ui.button("Paste").clicked() {
                    if let Some(text) = DesktopApp::read_clipboard_text() {
                        app.debug_console_input.push_str(&text);
                    }
                    ui.close_menu();
                }
                if ui.button("Clear input").clicked() {
                    app.debug_console_input.clear();
                    ui.close_menu();
                }
            });

            if response.has_focus() {
                let up = ui.input(|input| input.key_pressed(egui::Key::ArrowUp));
                let down = ui.input(|input| input.key_pressed(egui::Key::ArrowDown));
                let clear =
                    ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::L));
                if up {
                    app.debug_console_previous_history();
                } else if down {
                    app.debug_console_next_history();
                } else if clear {
                    app.clear_debug_console();
                }
            }

            let enter_pressed =
                response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if enter_pressed
                || ui
                    .add_sized([run_width, 28.0], egui::Button::new("Run"))
                    .clicked()
            {
                app.run_debug_console_command();
            }
        });
        let suggestions = app.debug_console_suggestions(&app.debug_console_input);
        if !suggestions.is_empty() {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                widgets::muted_label(ui, "Suggestions:");
                for suggestion in suggestions {
                    if ui.small_button(&suggestion).clicked() {
                        app.debug_console_input = suggestion;
                    }
                }
            });
        }
    });
}

fn render_console_welcome(app: &DesktopApp, ui: &mut egui::Ui) {
    let font_size = app.debug_console_font_size;
    ui.label(
        egui::RichText::new("Welcome to the Atho RPC console.")
            .monospace()
            .size(font_size)
            .strong()
            .color(widgets::TEXT),
    );
    ui.label(
        egui::RichText::new(
            "Use up and down arrows to navigate history, and Ctrl-L to clear the console.",
        )
        .monospace()
        .size(font_size)
        .color(widgets::TEXT),
    );
    ui.label(
        egui::RichText::new("Type help for an overview of available commands.")
            .monospace()
            .size(font_size)
            .color(widgets::TEXT),
    );
    ui.label(
        egui::RichText::new("Type help <command> for command-specific usage.")
            .monospace()
            .size(font_size)
            .color(widgets::TEXT),
    );
    ui.label(
        egui::RichText::new(
            "WARNING: Do not paste secrets here unless a future explicit wallet-secret command requires it.",
        )
        .monospace()
        .size(font_size)
        .color(egui::Color32::from_rgb(185, 84, 56)),
    );
    ui.add_space(10.0);
}

fn render_console_entry(
    app: &DesktopApp,
    ui: &mut egui::Ui,
    entry: &super::super::DebugConsoleEntry,
) {
    let font_size = app.debug_console_font_size;
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [64.0, 0.0],
            egui::Label::new(
                egui::RichText::new(format_clock(entry.timestamp_unix))
                    .monospace()
                    .size(font_size - 1.0)
                    .color(widgets::MUTED),
            ),
        );
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(format!("> {}", entry.command_line))
                    .monospace()
                    .size(font_size)
                    .strong()
                    .color(widgets::TEXT),
            );
            let color = if entry.success {
                widgets::TEXT
            } else {
                egui::Color32::from_rgb(170, 77, 50)
            };
            let output_response = ui.add(
                egui::Label::new(
                    egui::RichText::new(&entry.output)
                        .monospace()
                        .size(font_size)
                        .color(color),
                )
                .wrap(true),
            );
            output_response.context_menu(|ui| {
                if ui.button("Copy output").clicked() {
                    DesktopApp::copy_text(ui, entry.output.clone());
                    ui.close_menu();
                }
                if ui.button("Copy command").clicked() {
                    DesktopApp::copy_text(ui, entry.command_line.clone());
                    ui.close_menu();
                }
            });
            ui.horizontal_wrapped(|ui| {
                widgets::muted_label(
                    ui,
                    &format!(
                        "{}  {}  {}",
                        entry.group.label(),
                        entry.permission.label(),
                        entry.network_label
                    ),
                );
                if let Some(code) = entry.error_code.as_ref() {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(170, 77, 50), code);
                }
            });
        });
    });
}

fn render_network_traffic_tab(app: &mut DesktopApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        widgets::panel_frame().show(ui, |ui| {
            widgets::section_header(ui, "Network Traffic");
            ui.add_space(10.0);
            let desired = egui::vec2((ui.available_width() - 8.0).max(320.0), 280.0);
            let (response, painter) = ui.allocate_painter(desired, egui::Sense::hover());
            paint_traffic_graph(&app.network_traffic_samples, response.rect, &painter);
            ui.add_space(8.0);
            widgets::muted_label(
                ui,
                &format!(
                    "Window: {}",
                    traffic_window_label(&app.network_traffic_samples)
                ),
            );
        });

        ui.add_space(8.0);
        widgets::panel_frame().show(ui, |ui| {
            widgets::section_header(ui, "Totals");
            ui.add_space(10.0);
            let latest = app.network_traffic_samples.last();
            info_row(ui, "Received", &format_bytes(app.view_model.bytes_received));
            info_row(ui, "Sent", &format_bytes(app.view_model.bytes_sent));
            info_row(
                ui,
                "Current Receive Rate",
                &latest
                    .map(|sample| format_rate(sample.bytes_received_per_second))
                    .unwrap_or_else(|| String::from("0 B/s")),
            );
            info_row(
                ui,
                "Current Send Rate",
                &latest
                    .map(|sample| format_rate(sample.bytes_sent_per_second))
                    .unwrap_or_else(|| String::from("0 B/s")),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(48, 156, 82), "Received");
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(178, 72, 72), "Sent");
            });
            ui.add_space(12.0);
            if ui.button("Clear").clicked() {
                app.clear_network_traffic_samples();
            }
        });
    });
}

fn render_peers_tab(app: &mut DesktopApp, ui: &mut egui::Ui) {
    ui.columns(2, |columns| {
        widgets::panel_frame().show(&mut columns[0], |ui| {
            widgets::section_header(ui, "Peers");
            ui.add_space(10.0);
            if app.view_model.connecting_peer_count > 0 {
                widgets::muted_label(
                    ui,
                    &format!(
                        "{} outbound connection attempt(s) in progress. Pending handshakes are shown below the connected peer list and no longer count as established peers.",
                        app.view_model.connecting_peer_count
                    ),
                );
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                ui.add_sized(
                    [48.0, 0.0],
                    egui::Label::new(egui::RichText::new("Node").strong()),
                );
                ui.add_sized(
                    [220.0, 0.0],
                    egui::Label::new(egui::RichText::new("Peer / Service").strong()),
                );
                ui.add_sized(
                    [90.0, 0.0],
                    egui::Label::new(egui::RichText::new("Type").strong()),
                );
                ui.add_sized(
                    [70.0, 0.0],
                    egui::Label::new(egui::RichText::new("Height").strong()),
                );
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_source("node_window_peers_list")
                .auto_shrink([false, false])
                .show(ui, |ui: &mut egui::Ui| {
                    if app.view_model.peers.is_empty() {
                        widgets::muted_label(ui, "No connected peers.");
                        ui.add_space(8.0);
                    }
                    for (index, peer) in app.view_model.peers.iter().enumerate() {
                        let selected = app
                            .debug_selected_peer
                            .as_deref()
                            .is_some_and(|selected| selected == peer.remote_addr);
                        let button = egui::Button::new("")
                            .fill(if selected {
                                widgets::TOOLBAR_ACTIVE
                            } else {
                                widgets::PANEL_BG
                            })
                            .stroke(egui::Stroke::new(
                                1.0,
                                if selected {
                                    widgets::ACCENT
                                } else {
                                    widgets::PANEL_STROKE
                                },
                            ));
                        let response = ui.add_sized([ui.available_width(), 30.0], button);
                        let row_rect = response.rect;
                        if response.clicked() {
                            app.debug_selected_peer = Some(peer.remote_addr.clone());
                        }
                        ui.allocate_ui_at_rect(row_rect.shrink2(egui::vec2(6.0, 4.0)), |ui| {
                            ui.horizontal(|ui| {
                                ui.add_sized([48.0, 0.0], egui::Label::new(index.to_string()));
                                ui.add_sized(
                                    [220.0, 0.0],
                                    egui::Label::new(widgets::elide_text(&peer.remote_addr, 28)),
                                );
                                ui.add_sized(
                                    [90.0, 0.0],
                                    egui::Label::new(connection_type_label(peer.direction)),
                                );
                                ui.add_sized(
                                    [70.0, 0.0],
                                    egui::Label::new(
                                        peer.best_height
                                            .map(|height| height.to_string())
                                            .unwrap_or_else(|| String::from("-")),
                                    ),
                                );
                            });
                        });
                    }
                    if !app.view_model.connecting_peers.is_empty() {
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        widgets::muted_label(ui, "Connecting");
                        ui.add_space(6.0);
                        for peer in &app.view_model.connecting_peers {
                            widgets::panel_frame().show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [220.0, 0.0],
                                        egui::Label::new(widgets::elide_text(
                                            &peer.remote_addr,
                                            28,
                                        )),
                                    );
                                    ui.add_sized(
                                        [90.0, 0.0],
                                        egui::Label::new(connection_type_label(peer.direction)),
                                    );
                                    widgets::muted_label(ui, "Handshake pending");
                                });
                            });
                            ui.add_space(4.0);
                        }
                    }
                });
        });

        widgets::panel_frame().show(&mut columns[1], |ui| {
            widgets::section_header(ui, "Peer Details");
            ui.add_space(10.0);
            let selected = app.debug_selected_peer.as_ref().and_then(|remote_addr| {
                app.view_model
                    .peers
                    .iter()
                    .find(|peer| &peer.remote_addr == remote_addr)
            });
            if let Some(peer) = selected {
                render_peer_details(ui, peer);
            } else {
                widgets::muted_label(ui, "No peer selected.");
            }
        });
    });
}

fn render_peer_details(ui: &mut egui::Ui, peer: &NetworkPeerDiagnostics) {
    info_row(ui, "Address", &peer.remote_addr);
    info_row(ui, "Connection Type", connection_type_label(peer.direction));
    info_row(
        ui,
        "Handshake",
        if peer.handshake_ready {
            "Ready"
        } else {
            "Pending"
        },
    );
    info_row(
        ui,
        "Version",
        &peer
            .protocol_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| String::from("N/A")),
    );
    info_row(
        ui,
        "User Agent",
        peer.user_agent.as_deref().unwrap_or("N/A"),
    );
    info_row(
        ui,
        "Services",
        &peer
            .services
            .map(|services| format!("0x{services:x}"))
            .unwrap_or_else(|| String::from("N/A")),
    );
    info_row(
        ui,
        "Ruleset Version",
        &peer
            .ruleset_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| String::from("N/A")),
    );
    info_row(
        ui,
        "Peer Best Height",
        &peer
            .best_height
            .map(|height| height.to_string())
            .unwrap_or_else(|| String::from("N/A")),
    );
    info_row(ui, "Received", &format_bytes(peer.bytes_received));
    info_row(ui, "Sent", &format_bytes(peer.bytes_sent));
    info_row(
        ui,
        "Last Receive",
        &peer
            .last_receive_unix
            .map(age_label)
            .unwrap_or_else(|| String::from("N/A")),
    );
    info_row(
        ui,
        "Last Send",
        &peer
            .last_send_unix
            .map(age_label)
            .unwrap_or_else(|| String::from("N/A")),
    );
    info_row(
        ui,
        "Quality Score",
        &peer
            .quality_score
            .map(|score| score.to_string())
            .unwrap_or_else(|| String::from("N/A")),
    );
    info_row(
        ui,
        "Consecutive Failures",
        &peer
            .consecutive_failures
            .map(|count| count.to_string())
            .unwrap_or_else(|| String::from("N/A")),
    );
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [160.0, 0.0],
            egui::Label::new(egui::RichText::new(label).color(widgets::MUTED)),
        );
        ui.label(egui::RichText::new(value).color(widgets::TEXT));
    });
    ui.add_space(4.0);
}

fn connection_type_label(direction: NetworkPeerDirection) -> &'static str {
    match direction {
        NetworkPeerDirection::Inbound => "Inbound",
        NetworkPeerDirection::Outbound => "Outbound",
    }
}

fn format_clock(unix: u64) -> String {
    let seconds = unix % 86_400;
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
}

fn age_label(unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(unix);
    if now <= unix {
        return String::from("just now");
    }
    let delta = now - unix;
    if delta < 60 {
        format!("{delta}s ago")
    } else if delta < 3_600 {
        format!("{}m ago", delta / 60)
    } else {
        format!("{}h ago", delta / 3_600)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.2} GB", value / GB)
    } else if value >= MB {
        format!("{:.2} MB", value / MB)
    } else if value >= KB {
        format!("{:.2} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn format_rate(bytes_per_second: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    if bytes_per_second >= MB {
        format!("{:.2} MB/s", bytes_per_second / MB)
    } else if bytes_per_second >= KB {
        format!("{:.2} KB/s", bytes_per_second / KB)
    } else {
        format!("{:.0} B/s", bytes_per_second)
    }
}

fn traffic_window_label(samples: &[super::super::NetworkTrafficSample]) -> String {
    let Some(first) = samples.first() else {
        return String::from("No samples yet");
    };
    let Some(last) = samples.last() else {
        return String::from("No samples yet");
    };
    let delta = last.timestamp_unix.saturating_sub(first.timestamp_unix);
    if delta < 60 {
        format!("{delta}s")
    } else {
        format!("{}m {}s", delta / 60, delta % 60)
    }
}

fn paint_traffic_graph(
    samples: &[super::super::NetworkTrafficSample],
    rect: egui::Rect,
    painter: &egui::Painter,
) {
    let graph_rect = rect.shrink(8.0);
    painter.rect_filled(graph_rect, 0.0, egui::Color32::from_rgb(250, 250, 248));
    painter.rect_stroke(
        graph_rect,
        0.0,
        egui::Stroke::new(1.0, widgets::PANEL_STROKE),
    );

    for step in 1..5 {
        let y = egui::lerp(graph_rect.bottom()..=graph_rect.top(), step as f32 / 5.0);
        painter.line_segment(
            [
                egui::pos2(graph_rect.left(), y),
                egui::pos2(graph_rect.right(), y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(231, 233, 229)),
        );
    }

    let max_rate = samples
        .iter()
        .fold(0.0_f64, |acc, sample| {
            acc.max(sample.bytes_received_per_second)
                .max(sample.bytes_sent_per_second)
        })
        .max(1024.0);

    if samples.len() < 2 {
        painter.text(
            graph_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Waiting for traffic samples",
            egui::TextStyle::Body.resolve(&painter.ctx().style()),
            widgets::MUTED,
        );
        return;
    }

    let point_for = |index: usize, value: f64| -> egui::Pos2 {
        let x = graph_rect.left()
            + (index as f32 / (samples.len().saturating_sub(1) as f32)) * graph_rect.width();
        let normalized = (value / max_rate).clamp(0.0, 1.0) as f32;
        let y = graph_rect.bottom() - normalized * graph_rect.height();
        egui::pos2(x, y)
    };

    let received_points = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| point_for(index, sample.bytes_received_per_second))
        .collect::<Vec<_>>();
    let sent_points = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| point_for(index, sample.bytes_sent_per_second))
        .collect::<Vec<_>>();

    painter.add(egui::Shape::line(
        received_points,
        egui::Stroke::new(1.8, egui::Color32::from_rgb(48, 156, 82)),
    ));
    painter.add(egui::Shape::line(
        sent_points,
        egui::Stroke::new(1.8, egui::Color32::from_rgb(178, 72, 72)),
    ));
    painter.text(
        egui::pos2(graph_rect.left() + 8.0, graph_rect.top() + 8.0),
        egui::Align2::LEFT_TOP,
        format_rate(max_rate),
        egui::TextStyle::Small.resolve(&painter.ctx().style()),
        widgets::MUTED,
    );
}
