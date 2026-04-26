use crate::connection::{ConnectionStatus, ReadOnlyNodeConnection};
use crate::error::QtError;
use crate::state::UiState;
use crate::view::ViewModel;
use atho_core::address::decode_base56_address;
use atho_core::constants::{BLOCK_TIME_SECONDS, MIN_TX_FEE_ATOMS};
use atho_core::network::Network;
use atho_node::miner::Miner;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;
use atho_wallet::hd::AddressKind;
use atho_wallet::mnemonic::{MnemonicLength, MnemonicPhrase};
use atho_wallet::wallet::{Wallet, WalletAddress};
use eframe::egui;
use getrandom::getrandom;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavTab {
    Overview,
    Send,
    Receive,
    Transactions,
    Settings,
}

impl NavTab {
    fn all() -> [NavTab; 5] {
        [
            NavTab::Overview,
            NavTab::Send,
            NavTab::Receive,
            NavTab::Transactions,
            NavTab::Settings,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            NavTab::Overview => "Overview",
            NavTab::Send => "Send",
            NavTab::Receive => "Receive",
            NavTab::Transactions => "Transactions",
            NavTab::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchPage {
    Welcome,
    CreateWallet,
    ImportWallet,
    OpenWallet,
}

#[derive(Debug)]
struct CreateWalletForm {
    wallet_path: String,
    wallet_password: String,
    wallet_password_confirm: String,
    mnemonic_passphrase: String,
    mnemonic_text: String,
    acknowledged_backup: bool,
}

impl CreateWalletForm {
    fn new(network: Network) -> Self {
        Self {
            wallet_path: default_wallet_path(network).to_string_lossy().into_owned(),
            wallet_password: String::new(),
            wallet_password_confirm: String::new(),
            mnemonic_passphrase: String::new(),
            mnemonic_text: String::new(),
            acknowledged_backup: false,
        }
    }

    fn reset_phrase(&mut self) {
        self.mnemonic_text.clear();
        self.acknowledged_backup = false;
    }
}

#[derive(Debug)]
struct ImportWalletForm {
    wallet_path: String,
    wallet_password: String,
    wallet_password_confirm: String,
    mnemonic_phrase: String,
    mnemonic_passphrase: String,
}

impl ImportWalletForm {
    fn new(network: Network) -> Self {
        Self {
            wallet_path: default_wallet_path(network).to_string_lossy().into_owned(),
            wallet_password: String::new(),
            wallet_password_confirm: String::new(),
            mnemonic_phrase: String::new(),
            mnemonic_passphrase: String::new(),
        }
    }
}

#[derive(Debug)]
struct OpenWalletForm {
    wallet_path: String,
    wallet_password: String,
}

impl OpenWalletForm {
    fn new(network: Network) -> Self {
        Self {
            wallet_path: default_wallet_path(network).to_string_lossy().into_owned(),
            wallet_password: String::new(),
        }
    }
}

#[derive(Debug)]
struct MiningOutcome {
    height: u64,
    block_hash: [u8; 48],
    accepted: bool,
    message: String,
}

struct MiningJob {
    started_at: Instant,
    receiver: mpsc::Receiver<Result<MiningOutcome, String>>,
}

pub struct DesktopApp {
    pub connection: ReadOnlyNodeConnection,
    pub ui_state: UiState,
    pub view_model: ViewModel,
    wallet: Option<Wallet>,
    wallet_path: Option<String>,
    receive_addresses: Vec<WalletAddress>,
    current_receive_address: Option<WalletAddress>,
    launch_page: LaunchPage,
    create_form: CreateWalletForm,
    import_form: ImportWalletForm,
    open_form: OpenWalletForm,
    needs_initial_refresh: bool,
    last_error: Option<String>,
    active_tab: NavTab,
    send_to: String,
    send_label: String,
    send_amount: String,
    send_fee: String,
    receive_label: String,
    send_status: String,
    mining_status: String,
    mining_job: Option<MiningJob>,
    last_mined_block_hash: Option<[u8; 48]>,
    last_status_poll: Instant,
}

impl DesktopApp {
    pub fn new(network: Network) -> Self {
        Self::new_with_rpc(network, None)
    }

    pub fn new_with_rpc(network: Network, rpc_address: Option<String>) -> Self {
        let connection = match rpc_address {
            Some(address) => ReadOnlyNodeConnection::with_rpc_address(network, address),
            None => ReadOnlyNodeConnection::new(network),
        };
        let default_wallet_path = default_wallet_path(network);
        let launch_page = if default_wallet_path.exists() {
            LaunchPage::OpenWallet
        } else {
            LaunchPage::CreateWallet
        };

        Self {
            connection,
            ui_state: UiState {
                mining_cores: 4,
                ..UiState::default()
            },
            view_model: ViewModel::default(),
            wallet: None,
            wallet_path: None,
            receive_addresses: Vec::new(),
            current_receive_address: None,
            launch_page,
            create_form: CreateWalletForm::new(network),
            import_form: ImportWalletForm::new(network),
            open_form: OpenWalletForm::new(network),
            needs_initial_refresh: true,
            last_error: None,
            active_tab: NavTab::Overview,
            send_to: String::new(),
            send_label: String::new(),
            send_amount: String::new(),
            send_fee: String::new(),
            receive_label: String::new(),
            send_status: String::from("Enter a destination and integer atom amounts."),
            mining_status: String::from("Idle"),
            mining_job: None,
            last_mined_block_hash: None,
            last_status_poll: Instant::now(),
        }
    }

    pub fn refresh(&mut self) -> Result<(), QtError> {
        let status = self.connection.status();
        self.apply_connection_status(status);
        Ok(())
    }

    fn apply_connection_status(&mut self, status: ConnectionStatus) {
        self.view_model.network_label = status.network.id().to_string();
        self.view_model.block_count = status.block_count;
        self.view_model.mempool_count = status.mempool_count;
        self.view_model.sync_best_height = status.block_count;
        self.view_model.sync_stage = if status.connected {
            String::from("Connected")
        } else if self.connection.has_local_node() {
            String::from("Starting node")
        } else {
            String::from("Disconnected")
        };

        if let Some(wallet) = &self.wallet {
            self.view_model.ui_state.wallet_snapshot = wallet.snapshot.clone();
            self.ui_state.wallet_snapshot = wallet.snapshot.clone();
        }
        self.ui_state.set_connected(status.connected);
    }

    fn wallet_mut(&mut self) -> Option<&mut Wallet> {
        self.wallet.as_mut()
    }

    fn wallet_ref(&self) -> Option<&Wallet> {
        self.wallet.as_ref()
    }

    fn attach_wallet(&mut self, mut wallet: Wallet, wallet_path: String) {
        let has_receive = wallet
            .address_book
            .snapshot()
            .iter()
            .any(|record| record.path.kind == AddressKind::Receive);

        if !has_receive {
            let first = wallet.checkout_receive_address();
            self.receive_addresses = vec![first.clone()];
            self.current_receive_address = Some(first);
        } else {
            self.receive_addresses = wallet
                .address_book
                .snapshot()
                .into_iter()
                .filter(|record| record.path.kind == AddressKind::Receive)
                .map(|record| wallet.address_for_path(record.path))
                .collect();
            self.current_receive_address = self.receive_addresses.last().cloned();
            if self.receive_addresses.is_empty() {
                let first = wallet.checkout_receive_address();
                self.receive_addresses = vec![first.clone()];
                self.current_receive_address = Some(first);
            }
        }

        self.wallet_path = Some(wallet_path);
        self.wallet = Some(wallet);
        self.sync_wallet_state();
        self.active_tab = NavTab::Overview;
        self.launch_page = LaunchPage::Welcome;
    }

    fn sync_wallet_state(&mut self) {
        if let Some(wallet) = &self.wallet {
            self.ui_state.wallet_snapshot = wallet.snapshot.clone();
            self.view_model.ui_state.wallet_snapshot = wallet.snapshot.clone();
            self.current_receive_address = self.receive_addresses.last().cloned();
        }
    }

    fn poll_mining_job(&mut self) {
        let Some(job) = self.mining_job.take() else {
            return;
        };

        match job.receiver.try_recv() {
            Ok(Ok(outcome)) => {
                self.last_mined_block_hash = Some(outcome.block_hash);
                self.mining_status = format!("{} at height {}", outcome.message, outcome.height);
                if outcome.accepted {
                    self.last_error = None;
                    let _ = self.refresh();
                    if self.ui_state.generate_coins {
                        self.start_mining_job();
                    }
                }
            }
            Ok(Err(err)) => {
                self.mining_status = String::from("Mining failed");
                self.last_error = Some(err);
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.mining_job = Some(job);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.mining_status = String::from("Mining worker disconnected");
                self.last_error = Some(String::from("mining worker disconnected"));
            }
        }

        if self.mining_job.is_none() {
            let elapsed = job.started_at.elapsed();
            self.mining_status = format!("{} ({}s)", self.mining_status, elapsed.as_secs());
        }
    }

    fn start_mining_job(&mut self) {
        if self.mining_job.is_some() {
            self.mining_status = String::from("Mining already running");
            return;
        }
        if !self.ui_state.connected {
            self.mining_status = String::from("Node is not connected yet");
            return;
        }

        let rpc_address = self.connection.rpc_address().to_string();
        let cores = self.ui_state.mining_cores.max(1);
        let (sender, receiver) = mpsc::channel();
        self.mining_status = format!("Starting generation with {} thread(s)", cores);
        self.last_error = None;

        std::thread::spawn(move || {
            let result = mine_via_rpc(rpc_address, cores);
            let _ = sender.send(result);
        });

        self.mining_job = Some(MiningJob {
            started_at: Instant::now(),
            receiver,
        });
    }

    fn generate_create_mnemonic(&mut self) -> Result<(), String> {
        let mut entropy = [0u8; 32];
        getrandom(&mut entropy).map_err(|_| String::from("failed to gather wallet entropy"))?;
        let mnemonic = MnemonicPhrase::from_entropy(&entropy, MnemonicLength::Words24)
            .map_err(|err| err.to_string())?;
        self.create_form.mnemonic_text = mnemonic.as_sentence();
        self.create_form.acknowledged_backup = false;
        Ok(())
    }

    fn make_wallet_from_mnemonic(&self, mnemonic: MnemonicPhrase, passphrase: &str) -> Wallet {
        Wallet::from_mnemonic(mnemonic, passphrase, self.connection.network())
    }

    fn save_wallet_to_path(
        wallet: &Wallet,
        wallet_path: &str,
        password: &str,
    ) -> Result<(), String> {
        let path = PathBuf::from(wallet_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        wallet
            .save_to_datafile(&path, password)
            .map_err(|err| err.to_string())
    }

    fn open_wallet_from_path(&self, wallet_path: &str, password: &str) -> Result<Wallet, String> {
        let path = PathBuf::from(wallet_path);
        let wallet = Wallet::load_from_datafile(&path, password).map_err(|err| err.to_string())?;
        if wallet.network != self.connection.network() {
            return Err(format!(
                "wallet belongs to {} not {}",
                wallet.network.id(),
                self.connection.network().id()
            ));
        }
        Ok(wallet)
    }

    fn load_or_create_wallet(&mut self, wallet: Wallet, wallet_path: String) {
        self.attach_wallet(wallet, wallet_path);
        self.send_status = String::from("Wallet loaded");
        self.last_error = None;
    }

    fn generate_receive_address(&mut self) {
        let label = if self.receive_label.trim().is_empty() {
            None
        } else {
            Some(self.receive_label.trim().to_owned())
        };

        let (address, snapshot) = {
            let Some(wallet) = self.wallet_mut() else {
                self.send_status = String::from("Load or create a wallet first");
                return;
            };
            let address = wallet.checkout_receive_address_with_label(label);
            let snapshot = wallet.snapshot.clone();
            (address, snapshot)
        };
        self.receive_addresses.push(address.clone());
        self.current_receive_address = Some(address);
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        self.send_status = String::from("Receive address generated");
        self.receive_label.clear();
    }

    #[allow(dead_code)]
    fn generate_change_address(&mut self) {
        let (address, snapshot) = {
            let Some(wallet) = self.wallet_mut() else {
                self.send_status = String::from("Load or create a wallet first");
                return;
            };
            let address = wallet.checkout_change_address_with_label(None);
            let snapshot = wallet.snapshot.clone();
            (address, snapshot)
        };
        self.receive_addresses.push(address);
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        self.send_status = String::from("Change address generated");
    }

    fn current_receive_address_text(&self) -> String {
        self.current_receive_address
            .as_ref()
            .map(|address| address.address.clone())
            .unwrap_or_default()
    }

    fn validate_send_draft(&mut self) {
        let address = self.send_to.trim();
        if address.is_empty() {
            self.send_status = String::from("Enter a destination address");
            return;
        }

        let amount = match self.send_amount.trim().parse::<u64>() {
            Ok(value) if value > 0 => value,
            _ => {
                self.send_status = String::from("Amount must be an integer atom value");
                return;
            }
        };
        let fee = match self.send_fee.trim().parse::<u64>() {
            Ok(value) => value,
            _ => {
                self.send_status = String::from("Fee must be an integer atom value");
                return;
            }
        };

        match decode_base56_address(address) {
            Ok((_digest, network)) if network == self.connection.network() => {
                self.send_status =
                    format!("Draft prepared for {} atoms with {} atom fee", amount, fee);
                self.last_error = None;
            }
            Ok((_digest, network)) => {
                self.send_status = format!("Address belongs to {}", network.id());
            }
            Err(err) => {
                self.send_status = err.to_string();
            }
        }
    }

    fn copy_text(ui: &mut egui::Ui, text: String) {
        ui.output_mut(|output| {
            output.copied_text = text;
        });
    }

    fn show_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for tab in NavTab::all() {
                let selected = self.active_tab == tab;
                let mut button = egui::Button::new(tab.label());
                if selected {
                    button =
                        button
                            .fill(egui::Color32::from_rgb(72, 72, 72))
                            .stroke(egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgb(247, 147, 26),
                            ));
                }
                if ui.add_sized([96.0, 28.0], button).clicked() {
                    self.active_tab = tab;
                }
            }

            ui.separator();
            if ui.button("Refresh").clicked() {
                if let Err(err) = self.refresh() {
                    self.last_error = Some(err.to_string());
                }
            }
        });
    }

    fn card<R>(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
        egui::Frame::group(ui.style())
            .fill(egui::Color32::from_rgb(50, 50, 50))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(24, 24, 24)))
            .rounding(egui::Rounding::same(8.0))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(title)
                        .size(18.0)
                        .strong()
                        .color(egui::Color32::from_rgb(248, 248, 248)),
                );
                ui.add_space(6.0);
                add_contents(ui)
            })
            .inner
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(45, 45, 45);
        visuals.window_fill = egui::Color32::from_rgb(41, 41, 41);
        visuals.extreme_bg_color = egui::Color32::from_rgb(24, 24, 24);
        visuals.faint_bg_color = egui::Color32::from_rgb(56, 56, 56);
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(41, 41, 41);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(70, 70, 70);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(82, 82, 82);
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(95, 95, 95);
        visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(237, 237, 237);
        visuals.widgets.hovered.fg_stroke.color = egui::Color32::from_rgb(255, 255, 255);
        visuals.widgets.active.fg_stroke.color = egui::Color32::from_rgb(255, 255, 255);
        visuals.selection.bg_fill = egui::Color32::from_rgb(247, 147, 26);
        visuals.selection.stroke.color = egui::Color32::from_rgb(247, 147, 26);
        visuals.override_text_color = Some(egui::Color32::from_rgb(236, 236, 236));
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        let heading = egui::FontId::new(20.0, egui::FontFamily::Proportional);
        let body = egui::FontId::new(14.0, egui::FontFamily::Proportional);
        let button = egui::FontId::new(13.0, egui::FontFamily::Proportional);
        let small = egui::FontId::new(12.0, egui::FontFamily::Proportional);
        style.text_styles.insert(egui::TextStyle::Heading, heading);
        style
            .text_styles
            .insert(egui::TextStyle::Body, body.clone());
        style.text_styles.insert(egui::TextStyle::Button, button);
        style.text_styles.insert(egui::TextStyle::Monospace, body);
        style.text_styles.insert(egui::TextStyle::Small, small);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.interact_size = egui::vec2(36.0, 24.0);
        ctx.set_style(style);
    }

    fn show_welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading("Atho");
            ui.label("A lightweight full node and HD wallet client.");
            ui.add_space(16.0);
            Self::card(ui, "Start", |ui| {
                ui.label(format!("Network: {}", self.view_model.network_label));
                ui.label(format!("RPC: {}", self.connection.rpc_address()));
                ui.label(format!("Connected: {}", self.ui_state.connected));
                ui.add_space(10.0);
                if ui
                    .add_sized([180.0, 30.0], egui::Button::new("Create Wallet"))
                    .clicked()
                {
                    self.create_form = CreateWalletForm::new(self.connection.network());
                    if let Err(err) = self.generate_create_mnemonic() {
                        self.last_error = Some(err);
                    }
                    self.launch_page = LaunchPage::CreateWallet;
                }
                if ui
                    .add_sized([180.0, 30.0], egui::Button::new("Import Wallet"))
                    .clicked()
                {
                    self.import_form = ImportWalletForm::new(self.connection.network());
                    self.launch_page = LaunchPage::ImportWallet;
                }
                if ui
                    .add_sized([180.0, 30.0], egui::Button::new("Open Wallet"))
                    .clicked()
                {
                    self.open_form = OpenWalletForm::new(self.connection.network());
                    self.launch_page = LaunchPage::OpenWallet;
                }
            });
        });
    }

    fn show_create_wallet(&mut self, ui: &mut egui::Ui) {
        let mut create_clicked = false;
        let mut cancel_clicked = false;
        Self::card(ui, "Create Wallet", |ui| {
            ui.label("Create a new HD wallet and encrypt it on disk.");
            ui.add_space(6.0);
            ui.label("Wallet file");
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.create_form.wallet_path),
            );
            ui.label("Wallet password");
            ui.add(
                egui::TextEdit::singleline(&mut self.create_form.wallet_password).password(true),
            );
            ui.label("Confirm password");
            ui.add(
                egui::TextEdit::singleline(&mut self.create_form.wallet_password_confirm)
                    .password(true),
            );
            ui.label("Seed passphrase (optional)");
            ui.add(
                egui::TextEdit::singleline(&mut self.create_form.mnemonic_passphrase)
                    .password(true),
            );
            ui.add_space(10.0);
            if !self.create_form.mnemonic_text.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(247, 147, 26),
                    "Write this recovery phrase down now. It is shown once.",
                );
                let mut phrase = self.create_form.mnemonic_text.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut phrase)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false),
                );
                ui.horizontal(|ui| {
                    if ui.button("Copy phrase").clicked() {
                        Self::copy_text(ui, self.create_form.mnemonic_text.clone());
                    }
                    ui.checkbox(
                        &mut self.create_form.acknowledged_backup,
                        "I have backed up the recovery phrase",
                    );
                });
            } else {
                ui.label("Mnemonic generation failed. Go back and try again.");
            }

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                let ready = !self.create_form.mnemonic_text.is_empty()
                    && self.create_form.acknowledged_backup
                    && !self.create_form.wallet_password.is_empty()
                    && self.create_form.wallet_password == self.create_form.wallet_password_confirm;
                if ui
                    .add_enabled(ready, egui::Button::new("Create Wallet"))
                    .clicked()
                {
                    create_clicked = true;
                }
                if ui.button("Back").clicked() {
                    cancel_clicked = true;
                }
            });
        });

        if create_clicked {
            if self.create_form.wallet_password != self.create_form.wallet_password_confirm {
                self.last_error = Some(String::from("wallet passwords do not match"));
                return;
            }

            let mnemonic = match MnemonicPhrase::parse(&self.create_form.mnemonic_text) {
                Ok(mnemonic) => mnemonic,
                Err(err) => {
                    self.last_error = Some(err.to_string());
                    return;
                }
            };
            let wallet =
                self.make_wallet_from_mnemonic(mnemonic, &self.create_form.mnemonic_passphrase);
            match Self::save_wallet_to_path(
                &wallet,
                &self.create_form.wallet_path,
                &self.create_form.wallet_password,
            ) {
                Ok(()) => {
                    self.load_or_create_wallet(wallet, self.create_form.wallet_path.clone());
                    self.last_error = None;
                    self.create_form.wallet_password.clear();
                    self.create_form.wallet_password_confirm.clear();
                    self.create_form.mnemonic_passphrase.clear();
                    self.create_form.reset_phrase();
                }
                Err(err) => {
                    self.last_error = Some(err);
                }
            }
        }

        if cancel_clicked {
            self.create_form.reset_phrase();
            self.launch_page = LaunchPage::Welcome;
        }
    }

    fn show_import_wallet(&mut self, ui: &mut egui::Ui) {
        let mut import_clicked = false;
        let mut cancel_clicked = false;
        Self::card(ui, "Import Wallet", |ui| {
            ui.label("Restore a wallet from an existing recovery phrase.");
            ui.add_space(8.0);
            ui.label("Wallet file");
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.import_form.wallet_path),
            );
            ui.label("Mnemonic phrase");
            ui.add(
                egui::TextEdit::multiline(&mut self.import_form.mnemonic_phrase)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            ui.label("Seed passphrase (optional)");
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.import_form.mnemonic_passphrase),
            );
            ui.label("Wallet password");
            ui.add(
                egui::TextEdit::singleline(&mut self.import_form.wallet_password).password(true),
            );
            ui.label("Confirm password");
            ui.add(
                egui::TextEdit::singleline(&mut self.import_form.wallet_password_confirm)
                    .password(true),
            );

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                let ready = !self.import_form.wallet_password.is_empty()
                    && self.import_form.wallet_password == self.import_form.wallet_password_confirm
                    && !self.import_form.mnemonic_phrase.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new("Import Wallet"))
                    .clicked()
                {
                    import_clicked = true;
                }
                if ui.button("Back").clicked() {
                    cancel_clicked = true;
                }
            });
        });

        if import_clicked {
            if self.import_form.wallet_password != self.import_form.wallet_password_confirm {
                self.last_error = Some(String::from("wallet passwords do not match"));
                return;
            }

            let mnemonic = match MnemonicPhrase::parse(&self.import_form.mnemonic_phrase) {
                Ok(mnemonic) => mnemonic,
                Err(err) => {
                    self.last_error = Some(err.to_string());
                    return;
                }
            };

            let wallet =
                self.make_wallet_from_mnemonic(mnemonic, &self.import_form.mnemonic_passphrase);
            match Self::save_wallet_to_path(
                &wallet,
                &self.import_form.wallet_path,
                &self.import_form.wallet_password,
            ) {
                Ok(()) => {
                    self.load_or_create_wallet(wallet, self.import_form.wallet_path.clone());
                    self.last_error = None;
                    self.import_form.wallet_password.clear();
                    self.import_form.wallet_password_confirm.clear();
                    self.import_form.mnemonic_phrase.clear();
                    self.import_form.mnemonic_passphrase.clear();
                }
                Err(err) => {
                    self.last_error = Some(err);
                }
            }
        }

        if cancel_clicked {
            self.launch_page = LaunchPage::Welcome;
        }
    }

    fn show_open_wallet(&mut self, ui: &mut egui::Ui) {
        let mut open_clicked = false;
        let mut cancel_clicked = false;
        Self::card(ui, "Open Wallet", |ui| {
            ui.label("Enter the wallet password to unlock your wallet.dat file.");
            ui.add_space(8.0);
            ui.label("Wallet file");
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.open_form.wallet_path),
            );
            ui.label("Wallet password");
            ui.add(
                egui::TextEdit::singleline(&mut self.open_form.wallet_password)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.open_form.wallet_password.is_empty(),
                        egui::Button::new("Open Wallet"),
                    )
                    .clicked()
                {
                    open_clicked = true;
                }
                if ui.button("Back").clicked() {
                    cancel_clicked = true;
                }
            });
        });

        if open_clicked {
            match self
                .open_wallet_from_path(&self.open_form.wallet_path, &self.open_form.wallet_password)
            {
                Ok(wallet) => {
                    self.load_or_create_wallet(wallet, self.open_form.wallet_path.clone());
                    self.last_error = None;
                    self.open_form.wallet_password.clear();
                }
                Err(err) => {
                    self.last_error = Some(err);
                }
            }
        }

        if cancel_clicked {
            self.launch_page = LaunchPage::Welcome;
        }
    }

    fn show_overview(&mut self, ui: &mut egui::Ui) {
        let wide = ui.available_width() > 860.0;
        if wide {
            ui.columns(2, |columns| {
                Self::card(&mut columns[0], "Balances", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Available:");
                        ui.monospace("0 atoms");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Pending:");
                        ui.monospace("0 atoms");
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Total:");
                        ui.monospace("0 atoms");
                    });
                    ui.add_space(8.0);
                    ui.label("Wallet file");
                    ui.monospace(self.wallet_path.as_deref().unwrap_or("No wallet loaded"));
                });
                Self::card(&mut columns[1], "Recent Transactions", |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Date");
                        ui.add_space(24.0);
                        ui.strong("Type");
                        ui.add_space(24.0);
                        ui.strong("Label");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.strong("Amount");
                        });
                    });
                    ui.separator();
                    ui.label("No wallet activity indexed yet.");
                    ui.add_space(6.0);
                    ui.label("Wallet-specific transaction history will appear here.");
                });
            });
        } else {
            Self::card(ui, "Balances", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Available:");
                    ui.monospace("0 atoms");
                });
                ui.horizontal(|ui| {
                    ui.label("Pending:");
                    ui.monospace("0 atoms");
                });
                ui.horizontal(|ui| {
                    ui.label("Total:");
                    ui.monospace("0 atoms");
                });
                ui.add_space(8.0);
                ui.label("Wallet file");
                ui.monospace(self.wallet_path.as_deref().unwrap_or("No wallet loaded"));
            });
            ui.add_space(10.0);
            Self::card(ui, "Recent Transactions", |ui| {
                ui.label("No wallet activity indexed yet.");
                ui.add_space(6.0);
                ui.label("Wallet-specific transaction history will appear here.");
            });
        }

        ui.add_space(10.0);
        if wide {
            ui.columns(2, |columns| {
                Self::card(&mut columns[0], "Node", |ui| {
                    ui.label(format!("RPC: {}", self.connection.rpc_address()));
                    ui.label(format!("Connected: {}", self.ui_state.connected));
                    ui.label(format!("Sync: {}", self.view_model.sync_stage));
                    ui.label(format!("Height: {}", self.view_model.block_count));
                });
                Self::card(&mut columns[1], "Policy", |ui| {
                    ui.label(format!("Target block time: {}s", BLOCK_TIME_SECONDS));
                    ui.label(format!("Minimum fee: {} atoms", MIN_TX_FEE_ATOMS));
                    ui.label(format!(
                        "Receive addresses: {}",
                        self.ui_state.wallet_snapshot.receive_count
                    ));
                });
            });
        } else {
            Self::card(ui, "Node", |ui| {
                ui.label(format!("RPC: {}", self.connection.rpc_address()));
                ui.label(format!("Connected: {}", self.ui_state.connected));
                ui.label(format!("Sync: {}", self.view_model.sync_stage));
                ui.label(format!("Height: {}", self.view_model.block_count));
            });
            ui.add_space(10.0);
            Self::card(ui, "Policy", |ui| {
                ui.label(format!("Target block time: {}s", BLOCK_TIME_SECONDS));
                ui.label(format!("Minimum fee: {} atoms", MIN_TX_FEE_ATOMS));
                ui.label(format!(
                    "Receive addresses: {}",
                    self.ui_state.wallet_snapshot.receive_count
                ));
            });
        }
    }

    fn show_send(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Create Payment Draft", |ui| {
            ui.label("Pay to");
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.send_to)
                    .hint_text("Enter an Atho base56 address"),
            );
            ui.label("Label");
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.send_label)
                    .hint_text("Optional label for this payment"),
            );
            ui.horizontal(|ui| {
                ui.label("Amount (atoms)");
                ui.add_sized(
                    [160.0, 24.0],
                    egui::TextEdit::singleline(&mut self.send_amount),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Fee (atoms)");
                ui.add_sized(
                    [160.0, 24.0],
                    egui::TextEdit::singleline(&mut self.send_fee),
                );
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Draft payment").clicked() {
                    self.validate_send_draft();
                }
                if ui.button("Clear").clicked() {
                    self.send_to.clear();
                    self.send_label.clear();
                    self.send_amount.clear();
                    self.send_fee.clear();
                    self.send_status.clear();
                }
            });
            ui.add_space(8.0);
            ui.label(&self.send_status);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.strong("Transaction Fee:");
                ui.monospace(format!(
                    "{} atoms/kB",
                    self.send_fee.trim().parse::<u64>().unwrap_or(0)
                ));
                let _ = ui.button("Choose...");
                ui.colored_label(
                    egui::Color32::from_rgb(247, 147, 26),
                    "Warning: Fee estimation is currently not possible.",
                );
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Send").clicked() {
                    self.validate_send_draft();
                }
                if ui.button("Clear All").clicked() {
                    self.send_to.clear();
                    self.send_label.clear();
                    self.send_amount.clear();
                    self.send_fee.clear();
                    self.send_status.clear();
                }
                let _ = ui.button("Add Recipient");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Balance: {} atoms", 0u64));
                });
            });
        });
    }

    fn show_receive(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Receive Address", |ui| {
            ui.label("Label");
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.receive_label),
            );
            ui.add_space(8.0);
            ui.label("Current base56 address");
            ui.horizontal(|ui| {
                let mut current = self.current_receive_address_text();
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut current).desired_width(f32::INFINITY),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("New address").clicked() {
                    self.generate_receive_address();
                }
                if ui.button("Copy").clicked() {
                    Self::copy_text(ui, self.current_receive_address_text());
                    self.send_status = String::from("Address copied");
                }
            });
        });

        ui.add_space(12.0);
        Self::card(ui, "Your Addresses", |ui| {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    if let Some(wallet) = self.wallet_ref() {
                        for (index, record) in wallet
                            .address_book
                            .snapshot()
                            .iter()
                            .enumerate()
                            .rev()
                            .filter(|(_, record)| record.path.kind == AddressKind::Receive)
                        {
                            let address = wallet.address_for_path(record.path);
                            ui.horizontal(|ui| {
                                ui.label(format!("#{}", index + 1));
                                ui.monospace(&address.address);
                                if ui.small_button("Copy").clicked() {
                                    Self::copy_text(ui, address.address.clone());
                                }
                            });
                            ui.separator();
                        }
                    } else {
                        ui.label("Load a wallet to view addresses.");
                    }
                });
        });
    }

    fn show_transactions(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Wallet Transactions", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(false, |ui| {
                    ui.label("All");
                });
                ui.add_enabled_ui(false, |ui| {
                    ui.label("All");
                });
                ui.add_enabled_ui(false, |ui| {
                    let mut search = String::new();
                    ui.add_sized(
                        [ui.available_width().min(260.0), 24.0],
                        egui::TextEdit::singleline(&mut search)
                            .hint_text("Enter address, transaction id, or label to search"),
                    );
                });
                ui.add_enabled_ui(false, |ui| {
                    ui.label("Min amount");
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Date");
                ui.add_space(32.0);
                ui.strong("Type");
                ui.add_space(32.0);
                ui.strong("Label");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.strong("Amount");
                });
            });
            ui.separator();
            ui.label("No wallet activity indexed yet.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(format!("Network: {}", self.view_model.network_label));
                ui.separator();
                ui.label(format!("Height: {}", self.view_model.block_count));
                ui.separator();
                ui.label(format!("Mempool: {}", self.view_model.mempool_count));
            });
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let _ = ui.button("Export");
            });
        });
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        let wallet_path = self
            .wallet_path
            .clone()
            .unwrap_or_else(|| String::from("No wallet loaded"));
        Self::card(ui, "Settings", |ui| {
            ui.label(format!("Network: {}", self.view_model.network_label));
            ui.label(format!("RPC address: {}", self.connection.rpc_address()));
            ui.label(format!("Connected: {}", self.ui_state.connected));
            ui.label(format!("Sync stage: {}", self.view_model.sync_stage));
            ui.label(format!("Wallet file: {}", wallet_path));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Mining threads");
                ui.add(egui::Slider::new(&mut self.ui_state.mining_cores, 1..=64).show_value(true));
            });
            ui.horizontal_wrapped(|ui| {
                let ready = self.ui_state.connected && self.wallet.is_some();
                if ui
                    .add_enabled(ready, egui::Button::new("Start Miner"))
                    .clicked()
                {
                    self.start_mining_job();
                }
                if ui.button("Stop Miner").clicked() {
                    self.ui_state.generate_coins = false;
                }
                ui.checkbox(&mut self.ui_state.generate_coins, "Mine continuously");
            });
            ui.label(format!("Miner status: {}", self.mining_status));
            if let Some(job) = &self.mining_job {
                ui.label(format!("Elapsed: {}s", job.started_at.elapsed().as_secs()));
            }
            ui.separator();
            if ui.button("Open Wallet").clicked() {
                self.launch_page = LaunchPage::OpenWallet;
            }
            if ui.button("Create Another Wallet").clicked() {
                self.create_form = CreateWalletForm::new(self.connection.network());
                self.create_form.wallet_path = alternate_wallet_path(self.connection.network())
                    .to_string_lossy()
                    .into_owned();
                let _ = self.generate_create_mnemonic();
                self.launch_page = LaunchPage::CreateWallet;
            }
        });
    }

    fn show_main_shell(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar")
            .exact_height(24.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("File");
                    ui.label("Wallet");
                    ui.label("Help");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("RPC {}", self.connection.rpc_address()));
                    });
                });
            });

        egui::TopBottomPanel::top("toolbar")
            .exact_height(44.0)
            .show(ctx, |ui| {
                self.show_tabs(ui);
            });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Network: {}", self.view_model.network_label));
                ui.separator();
                ui.label(format!("Height: {}", self.view_model.block_count));
                ui.separator();
                ui.label(format!("Mempool: {}", self.view_model.mempool_count));
                ui.separator();
                ui.label(format!("Connected: {}", self.ui_state.connected));
                if let Some(error) = &self.last_error {
                    ui.separator();
                    ui.label(format!("Error: {}", error));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.active_tab {
                    NavTab::Overview => self.show_overview(ui),
                    NavTab::Send => self.show_send(ui),
                    NavTab::Receive => self.show_receive(ui),
                    NavTab::Transactions => self.show_transactions(ui),
                    NavTab::Settings => self.show_settings(ui),
                });
        });
    }

    fn show_startup_screen(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Network: {}", self.view_model.network_label));
                ui.separator();
                ui.label(format!("RPC: {}", self.connection.rpc_address()));
                ui.separator();
                ui.label(format!("Connected: {}", self.ui_state.connected));
                if let Some(error) = &self.last_error {
                    ui.separator();
                    ui.label(format!("Error: {}", error));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(22.0);
                        ui.heading("Atho");
                        ui.label("Lightweight full node, HD wallet, and miner client.");
                        ui.add_space(14.0);

                        match self.launch_page {
                            LaunchPage::Welcome => self.show_welcome(ui),
                            LaunchPage::CreateWallet => self.show_create_wallet(ui),
                            LaunchPage::ImportWallet => self.show_import_wallet(ui),
                            LaunchPage::OpenWallet => self.show_open_wallet(ui),
                        }
                    });
                });
        });
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_theme(ctx);
        self.poll_mining_job();
        ctx.request_repaint_after(Duration::from_millis(200));

        if self.needs_initial_refresh {
            self.needs_initial_refresh = false;
            if let Err(err) = self.refresh() {
                self.last_error = Some(err.to_string());
            }
        }

        if self.last_status_poll.elapsed() >= Duration::from_millis(750) {
            self.last_status_poll = Instant::now();
            if let Err(err) = self.refresh() {
                self.last_error = Some(err.to_string());
            }
        }

        if self.wallet.is_some() {
            self.show_main_shell(ctx);
        } else {
            self.show_startup_screen(ctx);
        }
    }
}

fn default_wallet_path(network: Network) -> PathBuf {
    let base = home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".atho")
        .join(network.id())
        .join(Wallet::datafile_name())
}

fn alternate_wallet_path(network: Network) -> PathBuf {
    let mut path = default_wallet_path(network);
    let file_name = format!("{}.2", Wallet::datafile_name());
    path.set_file_name(file_name);
    path
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn mine_via_rpc(rpc_address: String, cores: u32) -> Result<MiningOutcome, String> {
    let client = RpcClient::new(rpc_address);
    let template = match client.call(&RpcRequest::GetBlockTemplate) {
        Ok(RpcResponse::BlockTemplate(template)) => template,
        Ok(RpcResponse::Error(err)) => return Err(err.to_string()),
        Ok(other) => return Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => return Err(err.to_string()),
    };

    let miner = Miner::new(cores);
    let block = miner.solve_block(template.block);
    let block_hash = block.header.block_hash();
    match client.call(&RpcRequest::SubmitBlock(block)) {
        Ok(RpcResponse::BlockSubmitted { accepted: true, .. }) => Ok(MiningOutcome {
            height: template.height,
            block_hash,
            accepted: true,
            message: format!(
                "Block {} accepted at height {}",
                hex::encode(block_hash),
                template.height
            ),
        }),
        Ok(RpcResponse::BlockSubmitted {
            accepted: false, ..
        }) => Ok(MiningOutcome {
            height: template.height,
            block_hash,
            accepted: false,
            message: format!("Block {} rejected", hex::encode(block_hash)),
        }),
        Ok(RpcResponse::Error(err)) => Err(err.to_string()),
        Ok(other) => Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_app_refreshes_view_state() {
        std::env::set_var("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        app.refresh().unwrap();
        assert!(app.ui_state.connected);
        assert_eq!(app.view_model.network_label, "atho-mainnet");
        assert!(app.wallet.is_none());
        std::env::remove_var("ATHO_QT_LOCAL");
    }
}
