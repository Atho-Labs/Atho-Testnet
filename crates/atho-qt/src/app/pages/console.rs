use crate::app::{widgets, DesktopApp};
use crate::resources;
use atho_core::network::Network;
use atho_rpc::command::{command_definition, search_commands, CommandDefinition, CommandGroup};
use eframe::egui;
use std::collections::BTreeMap;

pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    render_warning_banner(ui);
    ui.add_space(10.0);

    ui.columns(2, |columns| {
        render_browser_panel(app, &mut columns[0]);
        render_console_panel(app, &mut columns[1]);
    });
}

fn render_warning_banner(ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            let _ = ui.add(resources::warning_icon(20.0));
            ui.vertical(|ui| {
                widgets::section_header(ui, "Atho Debug Console");
                widgets::muted_label(
                    ui,
                    "This console talks to your local Atho node through the same validated RPC path as atho-cli. It is for inspection, diagnostics, and controlled local actions. Do not paste secrets here unless a future explicit wallet-secret command requires it.",
                );
            });
        });
    });
}

fn render_browser_panel(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Command Browser");
        ui.add_space(8.0);
        widgets::muted_label(
            ui,
            "Browse commands by group, insert examples, or preload workflow commands without typing them by hand.",
        );
        ui.add_space(10.0);

        render_workflows(app, ui);
        ui.add_space(12.0);
        render_command_groups(app, ui);
        ui.add_space(12.0);
        render_recent_history(app, ui);
    });
}

fn render_workflows(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::section_header(ui, "Workflows");
    ui.add_space(6.0);
    widgets::muted_label(
        ui,
        "These are common operator flows modeled after the kind of quick diagnostics people use in Bitcoin Core's debug console.",
    );
    ui.add_space(8.0);

    let workflows: &[(&str, &[&str])] = &[
        (
            "Node Health",
            &["getstatus", "gethealth", "getblockchaininfo"],
        ),
        (
            "Peer Diagnostics",
            &["getnetworkinfo", "getconnectioncount", "getpeerinfo"],
        ),
        (
            "Mining View",
            &["getmininginfo", "gettemplateinfo", "getblocktemplate"],
        ),
        ("Mempool View", &["getmempoolinfo", "help mempool"]),
        ("Utility View", &["help util", "sha3_384 ABC"]),
    ];

    for (label, commands) in workflows {
        ui.horizontal_wrapped(|ui| {
            ui.label(*label);
            for command in *commands {
                if ui.small_button(*command).clicked() {
                    app.debug_console_use_example(command);
                }
                if ui.small_button(format!("Run {}", command)).clicked() {
                    app.run_debug_console_line((*command).to_string(), true);
                }
            }
        });
        ui.add_space(4.0);
    }
}

fn render_command_groups(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::section_header(ui, "All Commands");
    ui.add_space(6.0);
    let filter = app.debug_console_input.trim().to_ascii_lowercase();
    let commands = if filter.is_empty() {
        search_commands("")
    } else {
        search_commands(&filter)
    };

    let mut grouped = BTreeMap::<&'static str, Vec<&'static CommandDefinition>>::new();
    for definition in commands {
        grouped
            .entry(definition.group.label())
            .or_default()
            .push(definition);
    }

    egui::ScrollArea::vertical()
        .max_height(460.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (group, definitions) in grouped {
                egui::CollapsingHeader::new(format!("{} ({})", group, definitions.len()))
                    .default_open(group == CommandGroup::Control.label())
                    .show(ui, |ui| {
                        for definition in definitions {
                            render_command_row(app, ui, definition);
                        }
                    });
            }
        });
}

fn render_command_row(app: &mut DesktopApp, ui: &mut egui::Ui, definition: &CommandDefinition) {
    ui.horizontal_wrapped(|ui| {
        if ui.button(definition.name).clicked() {
            app.debug_console_use_example(definition.name);
        }
        if ui.small_button("help").clicked() {
            app.debug_console_use_example(&format!("help {}", definition.name));
        }
        if let Some(example) = definition.examples.first() {
            if ui.small_button("example").clicked() {
                app.debug_console_use_example(example);
            }
        }
        widgets::muted_label(ui, definition.description);
    });

    ui.horizontal_wrapped(|ui| {
        widgets::muted_label(ui, &format!("usage: {}", definition.usage));
        ui.separator();
        widgets::muted_label(ui, definition.permission.label());
        if definition.dangerous {
            ui.separator();
            ui.colored_label(egui::Color32::from_rgb(170, 77, 50), "dangerous");
        }
        if !definition.mainnet_allowed && app.connection.network() == Network::Mainnet {
            ui.separator();
            ui.colored_label(egui::Color32::from_rgb(170, 77, 50), "blocked on mainnet");
        }
    });
    ui.add_space(6.0);
}

fn render_recent_history(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::section_header(ui, "Recent Commands");
    ui.add_space(6.0);
    if app.debug_console_history.is_empty() {
        widgets::muted_label(ui, "No command history yet.");
        return;
    }
    let recent = app
        .debug_console_history
        .iter()
        .rev()
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    ui.horizontal_wrapped(|ui| {
        for line in recent {
            if ui.small_button(&line).clicked() {
                app.debug_console_use_example(&line);
            }
        }
    });
}

fn render_console_panel(app: &mut DesktopApp, ui: &mut egui::Ui) {
    widgets::panel_frame().show(ui, |ui| {
        widgets::section_header(ui, "Console");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Network: {}", app.view_model.network_label));
            ui.separator();
            ui.label(format!(
                "Sync: {}",
                if app.view_model.headers_synced {
                    "synced"
                } else {
                    "syncing"
                }
            ));
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
            ui.label(format!("Peers: {}", app.view_model.peer_count));
        });

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Output");
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

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut app.debug_console_input)
                    .desired_width(f32::INFINITY)
                    .hint_text("Enter an Atho command, for example: getblockchaininfo"),
            );

            if response.has_focus() {
                let up = ui.input(|input| input.key_pressed(egui::Key::ArrowUp));
                let down = ui.input(|input| input.key_pressed(egui::Key::ArrowDown));
                if up {
                    app.debug_console_previous_history();
                } else if down {
                    app.debug_console_next_history();
                }
            }

            let enter_pressed =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("Run").clicked() || enter_pressed {
                app.run_debug_console_command();
            }
            if ui.button("Help").clicked() {
                let help_line = if app.debug_console_input.trim().is_empty() {
                    String::from("help")
                } else {
                    let token = app
                        .debug_console_input
                        .split_whitespace()
                        .next()
                        .unwrap_or("help");
                    format!("help {token}")
                };
                app.run_debug_console_line(help_line, true);
            }
            if ui.button("Clear").clicked() {
                app.clear_debug_console();
            }
            if ui.button("Copy Last").clicked() {
                app.copy_latest_debug_console_output(ui);
            }
        });

        render_selected_command_details(app, ui);

        ui.add_space(10.0);
        let suggestions = app.debug_console_suggestions();
        if !suggestions.is_empty() {
            widgets::muted_label(ui, "Suggestions");
            ui.horizontal_wrapped(|ui| {
                for definition in suggestions {
                    if ui
                        .small_button(definition.name)
                        .on_hover_text(definition.description)
                        .clicked()
                    {
                        app.debug_console_use_example(definition.name);
                    }
                }
            });
            ui.add_space(10.0);
        }

        widgets::section_header(ui, "Output");
        ui.add_space(8.0);
        if app.debug_console_entries.is_empty() {
            widgets::muted_label(
                ui,
                "Run `help` to list all current commands, or choose a workflow on the left.",
            );
            return;
        }

        egui::ScrollArea::vertical()
            .max_height(620.0)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &app.debug_console_entries {
                    egui::Frame::none()
                        .fill(widgets::PANEL_BG)
                        .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE))
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    egui::RichText::new(&entry.command_line)
                                        .strong()
                                        .color(widgets::TEXT),
                                );
                                ui.separator();
                                ui.label(entry.group.label());
                                ui.separator();
                                ui.label(entry.permission.label());
                                ui.separator();
                                ui.label(&entry.network_label);
                                if entry.dangerous {
                                    ui.separator();
                                    ui.colored_label(
                                        egui::Color32::from_rgb(170, 77, 50),
                                        "dangerous",
                                    );
                                }
                                if let Some(code) = &entry.error_code {
                                    ui.separator();
                                    ui.colored_label(egui::Color32::from_rgb(170, 77, 50), code);
                                }
                            });
                            ui.add_space(6.0);
                            ui.code(entry.output.as_str());
                        });
                    ui.add_space(8.0);
                }
            });
    });
}

fn render_selected_command_details(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let token = app
        .debug_console_input
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let Some(definition) = command_definition(token) else {
        return;
    };

    ui.add_space(8.0);
    widgets::panel_frame().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Command: {}", definition.name));
            ui.separator();
            ui.label(format!("Group: {}", definition.group.label()));
            ui.separator();
            ui.label(format!("Permission: {}", definition.permission.label()));
            if definition.dangerous {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(170, 77, 50), "Dangerous");
            }
            if !definition.mainnet_allowed && app.connection.network() == Network::Mainnet {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(170, 77, 50), "Blocked on mainnet");
            }
        });
        ui.add_space(6.0);
        widgets::muted_label(ui, definition.description);
        ui.add_space(6.0);
        widgets::muted_label(ui, &format!("Usage: {}", definition.usage));
        widgets::muted_label(ui, &format!("Arguments: {}", definition.args_schema));
        widgets::muted_label(ui, &format!("Returns: {}", definition.result_schema));
        if !definition.examples.is_empty() {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Examples:");
                for example in definition.examples {
                    if ui.small_button(*example).clicked() {
                        app.debug_console_use_example(example);
                    }
                }
            });
        }
    });
}
