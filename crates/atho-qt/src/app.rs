use crate::connection::ReadOnlyNodeConnection;
use crate::error::QtError;
use crate::state::UiState;
use crate::view::ViewModel;
use atho_core::constants::{BLOCK_TIME_SECONDS, MIN_TX_FEE_ATOMS};
use atho_core::network::Network;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavTab {
    Overview,
    Send,
    Receive,
    Transactions,
    Mining,
    Settings,
}

impl NavTab {
    fn all() -> [NavTab; 6] {
        [
            NavTab::Overview,
            NavTab::Send,
            NavTab::Receive,
            NavTab::Transactions,
            NavTab::Mining,
            NavTab::Settings,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            NavTab::Overview => "Overview",
            NavTab::Send => "Send",
            NavTab::Receive => "Receive",
            NavTab::Transactions => "Transactions",
            NavTab::Mining => "Mining",
            NavTab::Settings => "Settings",
        }
    }
}

pub struct DesktopApp {
    pub connection: ReadOnlyNodeConnection,
    pub ui_state: UiState,
    pub view_model: ViewModel,
    needs_initial_refresh: bool,
    last_error: Option<String>,
    active_tab: NavTab,
    send_to: String,
    send_amount: String,
    send_fee: String,
    receive_label: String,
    logo_texture: Option<egui::TextureHandle>,
}

impl DesktopApp {
    pub fn new(network: Network) -> Self {
        Self {
            connection: ReadOnlyNodeConnection::new(network),
            ui_state: UiState {
                mining_cores: 4,
                ..UiState::default()
            },
            view_model: ViewModel::default(),
            needs_initial_refresh: true,
            last_error: None,
            active_tab: NavTab::Overview,
            send_to: String::new(),
            send_amount: String::new(),
            send_fee: String::new(),
            receive_label: String::new(),
            logo_texture: None,
        }
    }

    pub fn refresh(&mut self) -> Result<(), QtError> {
        let network = self.connection.request(RpcRequest::GetNetwork);
        let count = self.connection.request(RpcRequest::GetBlockCount);
        let status = self.connection.status();

        self.handle_response(network)?;
        self.handle_response(count)?;

        self.view_model.mempool_count = status.mempool_count;
        self.view_model.sync_best_height = status.sync_best_height;
        self.view_model.ui_state.wallet_snapshot = status.wallet_snapshot.clone();
        self.view_model.sync_stage = if status.headers_synced {
            String::from("Synchronized")
        } else {
            String::from("Syncing")
        };
        self.ui_state.wallet_snapshot = status.wallet_snapshot;
        self.ui_state.set_connected(status.running);
        Ok(())
    }

    fn handle_response(&mut self, response: RpcResponse) -> Result<(), QtError> {
        match response {
            RpcResponse::Error(err) => Err(QtError::Rpc(err)),
            other => {
                self.view_model.update_from_network(other);
                Ok(())
            }
        }
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_logo_texture(ctx);
        self.apply_theme(ctx);

        if self.needs_initial_refresh {
            self.needs_initial_refresh = false;
            if let Err(err) = self.refresh() {
                self.last_error = Some(err.to_string());
            }
        }

        egui::TopBottomPanel::top("title_bar")
            .exact_height(36.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(texture) = &self.logo_texture {
                        ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(16.0, 16.0)));
                        ui.add_space(6.0);
                    }
                    ui.strong("Atho Core");
                });
            });

        egui::TopBottomPanel::top("menu_bar")
            .exact_height(28.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("File");
                    ui.label("Settings");
                    ui.label("Help");
                });
            });

        egui::TopBottomPanel::top("tab_bar")
            .exact_height(40.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for tab in NavTab::all() {
                        let selected = self.active_tab == tab;
                        let mut button = egui::Button::new(tab.label());
                        if selected {
                            button = button.fill(egui::Color32::from_rgb(30, 104, 60));
                        }
                        if ui.add(button).clicked() {
                            self.active_tab = tab;
                        }
                    }
                });
            });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Network: {}", self.view_model.network_label));
                ui.separator();
                ui.label(format!("Block: {}", self.view_model.block_count));
                ui.separator();
                ui.label(format!("Mempool: {}", self.view_model.mempool_count));
                ui.separator();
                ui.label(format!("Best height: {}", self.view_model.sync_best_height));
                ui.separator();
                ui.label(format!("Connected: {}", self.ui_state.connected));
                if let Some(error) = &self.last_error {
                    ui.separator();
                    ui.label(format!("Error: {}", error));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let sync_text = if self.ui_state.connected {
                        "Synchronized"
                    } else {
                        "Connecting"
                    };
                    let progress = if self.ui_state.connected { 1.0 } else { 0.25 };
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .desired_width(150.0)
                            .show_percentage()
                            .text(sync_text),
                    );
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(self.active_tab.label());
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(10.0);
            match self.active_tab {
                NavTab::Overview => self.show_overview(ui),
                NavTab::Send => self.show_send(ui),
                NavTab::Receive => self.show_receive(ui),
                NavTab::Transactions => self.show_transactions(ui),
                NavTab::Mining => self.show_mining(ui),
                NavTab::Settings => self.show_settings(ui),
            }
        });
    }
}

impl DesktopApp {
    fn apply_theme(&self, ctx: &egui::Context) {
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = egui::Color32::from_rgb(248, 248, 246);
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(248, 248, 246);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(241, 241, 239);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(235, 240, 235);
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(212, 223, 212);
        visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(52, 52, 52);
        visuals.selection.bg_fill = egui::Color32::from_rgb(34, 86, 52);
        visuals.selection.stroke.color = egui::Color32::from_rgb(238, 248, 238);
        visuals.window_fill = egui::Color32::from_rgb(246, 245, 241);
        ctx.set_visuals(visuals);
    }

    fn ensure_logo_texture(&mut self, ctx: &egui::Context) {
        if self.logo_texture.is_some() {
            return;
        }

        let Ok(image) = image::load_from_memory(include_bytes!("atho.png")) else {
            return;
        };
        let rgba = image.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        self.logo_texture = Some(ctx.load_texture("atho-logo", color_image, egui::TextureOptions::default()));
    }

    fn show_overview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(300.0);
                ui.label("Balances");
                ui.separator();
                ui.label("Available: 0.00000000 ATHO");
                ui.label("Pending: 0.00000000 ATHO");
                ui.label("Total: 0.00000000 ATHO");
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(300.0);
                ui.label("Network");
                ui.separator();
                ui.label(format!("Network: {}", self.view_model.network_label));
                ui.label(format!("Block count: {}", self.view_model.block_count));
                ui.label(format!("Mempool entries: {}", self.view_model.mempool_count));
                ui.label(format!("Sync: {}", self.view_model.sync_stage));
                ui.label(format!("Best height: {}", self.view_model.sync_best_height));
            });
        });
        ui.add_space(10.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Wallet session");
            ui.separator();
            ui.label(format!("Receive addresses: {}", self.ui_state.wallet_snapshot.receive_count));
            ui.label(format!("Change addresses: {}", self.ui_state.wallet_snapshot.change_count));
            ui.label("Transactions will appear here once the wallet is wired to live history.");
        });
    }

    fn show_send(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Policy");
            ui.separator();
            ui.label(format!("Fee floor: {} atoms", MIN_TX_FEE_ATOMS));
            ui.label(format!("Target block time: {} seconds", BLOCK_TIME_SECONDS));
            ui.label(format!("Network: {}", self.view_model.network_label));
        });
        ui.add_space(10.0);
        ui.label("Pay to");
        ui.text_edit_singleline(&mut self.send_to);
        ui.add_space(8.0);
        ui.label("Amount");
        ui.text_edit_singleline(&mut self.send_amount);
        ui.add_space(8.0);
        ui.label("Fee");
        ui.text_edit_singleline(&mut self.send_fee);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let _ = ui.button("Add recipient");
            let _ = ui.button("Clear");
            let _ = ui.button("Send");
        });
    }

    fn show_receive(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Wallet");
            ui.separator();
            ui.label(format!("Network: {}", self.view_model.network_label));
            ui.label(format!("Receive addresses: {}", self.ui_state.wallet_snapshot.receive_count));
            ui.label(format!("Change addresses: {}", self.ui_state.wallet_snapshot.change_count));
            ui.label("Restore gap limit: 20");
        });
        ui.add_space(12.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Receive label");
            ui.text_edit_singleline(&mut self.receive_label);
            ui.add_space(8.0);
            ui.label("Address generation stays in the wallet service.");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let _ = ui.button("Copy");
                let _ = ui.button("New address");
            });
        });
    }

    fn show_transactions(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Chain state");
            ui.separator();
            ui.label(format!("Network: {}", self.view_model.network_label));
            ui.label(format!("Block count: {}", self.view_model.block_count));
            ui.label(format!("Mempool entries: {}", self.view_model.mempool_count));
            ui.label(format!("Sync: {}", self.view_model.sync_stage));
            ui.label(format!("Best height: {}", self.view_model.sync_best_height));
        });
        ui.add_space(10.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Recent transactions");
            ui.separator();
            ui.label("The thin client does not yet fetch historical transactions.");
        });
    }

    fn show_mining(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Mining");
            ui.separator();
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.ui_state.generate_coins, "Generate coins");
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Cores");
                ui.add(egui::Slider::new(&mut self.ui_state.mining_cores, 1..=64).show_value(true));
            });
        });
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Client");
            ui.separator();
            ui.label("Thin client");
            ui.label(format!("Network: {}", self.view_model.network_label));
            ui.label(format!("Connected: {}", self.ui_state.connected));
            ui.label(format!("Sync: {}", self.view_model.sync_stage));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_app_refreshes_view_state() {
        let mut app = DesktopApp::new(Network::Mainnet);
        app.refresh().unwrap();
        assert!(app.ui_state.connected);
        assert_eq!(app.view_model.network_label, "atho-mainnet");
    }
}
