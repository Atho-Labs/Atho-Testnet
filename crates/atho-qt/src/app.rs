//! Top-level desktop application state and event handling.
use crate::connection::{ConnectionStatus, ReadOnlyNodeConnection, StatusMonitor};
use crate::error::QtError;
use crate::state::UiState;
use crate::view::ViewModel;
use ab_glyph::{FontArc, PxScale};
use amounts::{
    format_amount_atoms, format_amount_atoms_without_unit, format_fee_atoms, parse_amount_to_atoms,
    ClientDisplayPreferences, DisplayUnit, InputUnit,
};
use atho_core::address::decode_base56_address;
use atho_core::block::{merkle_root, Block};
use atho_core::consensus::tx_policy::minimum_required_fee_atoms;
use atho_core::constants::{
    DUST_RELAY_VALUE_ATOMS, MAX_TRANSACTION_RAW_BYTES, MAX_TRANSACTION_VBYTES,
};
use atho_core::crypto::hash::sha3_256;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
use atho_crypto::falcon::{FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES};
#[cfg(test)]
use atho_node::miner::Miner;
use atho_node::mining_backend::{
    MiningAcceleratorInfo, MiningBackendKind, MiningController, MiningDeviceType,
};
use atho_node::validation::finalize_witness_commit_refs;
use atho_rpc::command::CommandResponse;
use atho_rpc::command::{command_definition, help_payload, parse_command_line, search_commands};
use atho_rpc::error::RpcError;
use atho_rpc::request::{RpcRequest, WalletHistoryAddress};
use atho_rpc::response::{
    BlockTemplate, MempoolSpentInput, NodeStatus, RpcResponse,
    WalletActivityEntry as RpcWalletActivityEntry, WalletActivityKind as RpcWalletActivityKind,
};
use atho_rpc::transport::RpcClient;
use atho_storage::utxo::UtxoEntry;
use atho_wallet::hd::AddressKind;
use atho_wallet::mnemonic::{MnemonicLength, MnemonicPhrase};
use atho_wallet::wallet::datafile::WalletEncryptionMode;
use atho_wallet::wallet::{
    Wallet, WalletAddress, WalletSpendProgressStage, WalletSpendRequest, WalletSpendUtxo,
    DEFAULT_RESTORE_GAP_LIMIT,
};
use eframe::egui;
use getrandom::getrandom;
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use qrcodegen::{QrCode, QrCodeEcc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod amounts;
mod dialogs;
mod mnemonic_ui;
mod models;
mod pages;
mod shell;
mod startup;
mod theme;
mod wallet_ledger;
mod widgets;
pub(crate) use models::{
    AddressPoolFilter, CreateWalletForm, DebugConsoleEntry, DebugConsoleOutputMode, DebugWindowTab,
    ImportWalletForm, LaunchPage, MiningJob, MiningJobResult, MiningOutcome, MiningStaleTemplate,
    NavTab, NetworkTrafficSample, OpenWalletForm, ReceiveAddressRow, ReceivePageTab,
    ReceiveRequestRecord, SendJob, SendJobEvent, SendOutcome, SendProgressStage,
    SyncProgressSample, WalletActivityKind, WalletActivityRow, WalletBalanceSummary,
    WalletManagementForm,
};

const RECEIVE_ADDRESS_LIST_LIMIT: usize = 100;
const WALLET_DISCOVERY_SCAN_STEPS: &[usize] = &[32, 128, 256, 512, 1_000];
const MIN_WALLET_DISCOVERY_SCAN_LIMIT: usize = 32;
const MAX_WALLET_DISCOVERY_SCAN_LIMIT: usize = 20_000;
const TEST_WALLET_DATAFILE_ITERATIONS: u32 = 10_000;
const WALLET_PREPARATION_STALL_TIMEOUT: Duration = Duration::from_secs(120);
const WALLET_SCAN_STALL_TIMEOUT: Duration = Duration::from_secs(60);
const MINING_TEMPLATE_WATCH_INTERVAL: Duration = Duration::from_millis(500);
const MINING_TEMPLATE_WATCH_SLEEP_SLICE: Duration = Duration::from_millis(50);
const QR_EXPORT_MODULE_BLACK: [u8; 4] = [24, 24, 24, 255];
const QR_EXPORT_MODULE_RED: [u8; 4] = [191, 41, 48, 255];
const QR_EXPORT_HEADER_BG: [u8; 4] = [255, 239, 240, 255];
const QR_EXPORT_RULE_RED: [u8; 4] = [228, 92, 96, 255];
const QR_EXPORT_TEXT_DARK: [u8; 4] = [72, 24, 24, 255];

pub struct DesktopApp {
    pub connection: ReadOnlyNodeConnection,
    status_monitor: StatusMonitor,
    pub ui_state: UiState,
    pub view_model: ViewModel,
    wallet: Option<Wallet>,
    current_wallet_id: Option<String>,
    current_wallet_name: Option<String>,
    wallet_path: Option<String>,
    wallet_session_password: Option<String>,
    wallet_addresses_cache: Vec<WalletAddress>,
    wallet_address_index_cache: HashMap<[u8; 32], usize>,
    wallet_address_digests_cache: HashSet<[u8; 32]>,
    wallet_owned_utxos_cache: Vec<UtxoEntry>,
    receive_addresses: Vec<WalletAddress>,
    receive_address_rows: Vec<ReceiveAddressRow>,
    receive_page_tab: ReceivePageTab,
    address_pool_filter: AddressPoolFilter,
    current_receive_address: Option<WalletAddress>,
    launch_page: LaunchPage,
    create_form: CreateWalletForm,
    import_form: ImportWalletForm,
    open_form: OpenWalletForm,
    wallet_management_form: WalletManagementForm,
    last_error: Option<String>,
    active_tab: NavTab,
    send_to: String,
    send_label: String,
    send_amount: String,
    send_include_fee_in_total: bool,
    send_fee: String,
    display_preferences: ClientDisplayPreferences,
    recipient_address_book: Vec<RecipientAddressEntry>,
    recipient_address_book_filter: String,
    recipient_address_book_open: bool,
    recipient_address_editor_open: bool,
    recipient_address_editor_id: Option<String>,
    recipient_address_editor_label: String,
    recipient_address_editor_address: String,
    recipient_address_editor_notes: String,
    receive_label: String,
    receive_amount: String,
    receive_message: String,
    send_status: String,
    debug_console_status: String,
    send_job: Option<SendJob>,
    wallet_preparation_job: Option<WalletPreparationJob>,
    wallet_preparation_stage: String,
    wallet_preparation_progress: f32,
    wallet_preparation_completed: usize,
    wallet_preparation_total: usize,
    wallet_scan_job: Option<WalletScanJob>,
    wallet_readiness_gate_active: bool,
    mining_status: String,
    mining_accelerator_info: MiningAcceleratorInfo,
    mining_job: Option<MiningJob>,
    pending_mining_restart: Option<u32>,
    last_mined_height: Option<u64>,
    last_mined_block_hash: Option<[u8; 48]>,
    last_mined_at_unix: Option<u64>,
    requested_payments: Vec<ReceiveRequestRecord>,
    selected_receive_request: Option<usize>,
    transaction_search: String,
    transaction_min_amount: String,
    transaction_date_filter: usize,
    transaction_type_filter: usize,
    show_about_dialog: bool,
    show_debug_window: bool,
    debug_window_tab: DebugWindowTab,
    debug_console_input: String,
    debug_console_output_mode: DebugConsoleOutputMode,
    debug_console_entries: Vec<DebugConsoleEntry>,
    debug_console_history: Vec<String>,
    debug_console_history_index: Option<usize>,
    debug_console_confirmed: bool,
    debug_console_font_size: f32,
    debug_selected_peer: Option<String>,
    network_traffic_samples: Vec<NetworkTrafficSample>,
    sync_progress_samples: Vec<SyncProgressSample>,
    last_network_traffic_snapshot: Option<(Instant, u64, u64)>,
    wallet_utxos_cache: Vec<UtxoEntry>,
    wallet_activity_cache: Vec<WalletActivityRow>,
    wallet_balance_summary_cache: WalletBalanceSummary,
    wallet_cache_dirty: bool,
    wallet_scan_nonce: u64,
    wallet_discovery_scan_limit: usize,
    wallet_discovery_scan_limit_cached: usize,
    last_wallet_refresh_at: Instant,
    wallet_balance_cache: u64,
    theme_initialized: bool,
    compact_viewport: bool,
    show_sync_status_dialog: bool,
    sync_status_hidden_until_synced: bool,
    storage_recovery_notice: Option<String>,
    show_storage_recovery_notice_dialog: bool,
}

#[derive(Debug, Clone)]
struct SpendableWalletUtxo {
    address: WalletAddress,
    utxo: UtxoEntry,
}

#[derive(Debug, Clone)]
struct SelectedSpendPlan {
    utxos: Vec<UtxoEntry>,
    total_input_atoms: u64,
    output_count: usize,
    signer_group_count: usize,
    transaction_version: u16,
    estimated_fee_atoms: u64,
    estimated_raw_size_bytes: usize,
    estimated_vsize_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TransactionShapeEstimate {
    fee_atoms: u64,
    raw_size_bytes: usize,
    vsize_bytes: usize,
}

#[derive(Debug, Clone)]
struct WalletScanOutcome {
    wallet_scan_nonce: u64,
    scan_limit: usize,
    wallet_addresses_cache: Vec<WalletAddress>,
    receive_addresses: Vec<WalletAddress>,
    wallet_owned_utxos_cache: Vec<UtxoEntry>,
    wallet_utxos_cache: Vec<UtxoEntry>,
    wallet_balance_cache: u64,
    wallet_balance_summary_cache: WalletBalanceSummary,
    wallet_activity_cache: Vec<WalletActivityRow>,
}

#[derive(Debug)]
struct WalletPreparationJob {
    started_at: Instant,
    last_progress_at: Instant,
    receiver: mpsc::Receiver<WalletPreparationEvent>,
}

#[derive(Debug)]
enum WalletPreparationEvent {
    Progress {
        stage: String,
        completed: usize,
        total: usize,
    },
    Finished(Box<Result<WalletPreparationOutcome, String>>),
}

#[derive(Debug)]
struct WalletPreparationOutcome {
    wallet: Wallet,
    wallet_path: String,
    wallet_password: String,
    registry_entry: Option<WalletRegistryEntry>,
}

struct MnemonicWalletPreparationRequest {
    mnemonic_text: String,
    mnemonic_passphrase: String,
    wallet_path: String,
    wallet_password: String,
    wallet_name: String,
    wallet_word_count: usize,
    stage: &'static str,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct WalletRegistry {
    entries: Vec<WalletRegistryEntry>,
    last_opened_wallet_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WalletRegistryEntry {
    wallet_id: String,
    wallet_name: String,
    wallet_path: String,
    network: String,
    created_at_unix: u64,
    updated_at_unix: u64,
    last_opened_at_unix: Option<u64>,
    word_count: usize,
}

#[derive(Debug)]
struct WalletScanJob {
    started_at: Instant,
    receiver: mpsc::Receiver<Result<WalletScanOutcome, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WalletBackupMetadata {
    network: String,
    exported_at_unix: u64,
    configured_recovery_window: usize,
    active_scan_window: usize,
    receive_keypool_queued: usize,
    change_keypool_queued: usize,
    highest_generated_receive_index: Option<u32>,
    highest_generated_change_index: Option<u32>,
    highest_reserved_receive_index: Option<u32>,
    highest_reserved_change_index: Option<u32>,
    next_receive_index: u32,
    next_change_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WalletRecoveryExport {
    wallet_name: String,
    wallet_path: String,
    wallet_id: Option<String>,
    network: String,
    exported_at_unix: u64,
    mnemonic_word_count: usize,
    mnemonic_phrase: Option<String>,
    configured_recovery_window: usize,
    active_scan_window: usize,
    current_receive_index: Option<u32>,
    next_receive_index: u32,
    next_change_index: u32,
    highest_generated_receive_index: Option<u32>,
    highest_generated_change_index: Option<u32>,
    highest_reserved_receive_index: Option<u32>,
    highest_reserved_change_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WalletStartupMetadata {
    wallet_path: String,
    recorded_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecipientAddressEntry {
    id: String,
    label: String,
    address: String,
    notes: String,
    created_at_unix: u64,
    updated_at_unix: u64,
    last_used_at_unix: Option<u64>,
}

impl DesktopApp {
    fn active_network(&self) -> Network {
        self.connection.network()
    }

    pub fn new(network: Network) -> Self {
        Self::new_with_rpc(network, None)
    }

    pub fn new_with_rpc(network: Network, rpc_address: Option<String>) -> Self {
        let connection = match rpc_address {
            Some(address) => ReadOnlyNodeConnection::with_rpc_address(network, address),
            None => ReadOnlyNodeConnection::new(network),
        };
        let status_monitor = connection.spawn_status_monitor(if connection.has_local_node() {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(1)
        });
        let launch_page = LaunchPage::Welcome;
        let available_cores = available_mining_cores();
        let configured_backend = MiningBackendKind::from_env().unwrap_or_default();
        let display_preferences = load_client_display_preferences(network);
        let recipient_address_book = load_recipient_address_book(network);

        let mut app = Self {
            connection,
            status_monitor,
            ui_state: UiState {
                mining_cores: available_cores,
                mining_backend: configured_backend,
                rotate_coinbase_address: false,
                ..UiState::default()
            },
            view_model: ViewModel::default(),
            wallet: None,
            current_wallet_id: None,
            current_wallet_name: None,
            wallet_path: None,
            wallet_session_password: None,
            wallet_addresses_cache: Vec::new(),
            wallet_address_index_cache: HashMap::new(),
            wallet_address_digests_cache: HashSet::new(),
            wallet_owned_utxos_cache: Vec::new(),
            receive_addresses: Vec::new(),
            receive_address_rows: Vec::new(),
            receive_page_tab: ReceivePageTab::RequestPayment,
            address_pool_filter: AddressPoolFilter::Unused,
            current_receive_address: None,
            launch_page,
            create_form: CreateWalletForm::new(network),
            import_form: ImportWalletForm::new(network),
            open_form: OpenWalletForm::new(network),
            wallet_management_form: WalletManagementForm::new(network),
            last_error: None,
            active_tab: NavTab::Overview,
            send_to: String::new(),
            send_label: String::new(),
            send_amount: String::new(),
            send_include_fee_in_total: false,
            send_fee: String::new(),
            display_preferences,
            recipient_address_book,
            recipient_address_book_filter: String::new(),
            recipient_address_book_open: false,
            recipient_address_editor_open: false,
            recipient_address_editor_id: None,
            recipient_address_editor_label: String::new(),
            recipient_address_editor_address: String::new(),
            recipient_address_editor_notes: String::new(),
            receive_label: String::new(),
            receive_amount: String::new(),
            receive_message: String::new(),
            send_status: String::from("Enter a destination address and amount."),
            debug_console_status: String::from("Type help to see commands grouped by area."),
            send_job: None,
            wallet_preparation_job: None,
            wallet_preparation_stage: String::new(),
            wallet_preparation_progress: 0.0,
            wallet_preparation_completed: 0,
            wallet_preparation_total: 0,
            wallet_scan_job: None,
            wallet_readiness_gate_active: false,
            mining_status: String::from("Idle"),
            mining_accelerator_info: MiningAcceleratorInfo::unavailable("probe pending"),
            mining_job: None,
            pending_mining_restart: None,
            last_mined_height: None,
            last_mined_block_hash: None,
            last_mined_at_unix: None,
            requested_payments: Vec::new(),
            selected_receive_request: None,
            transaction_search: String::new(),
            transaction_min_amount: String::new(),
            transaction_date_filter: 0,
            transaction_type_filter: 0,
            show_about_dialog: false,
            show_debug_window: false,
            debug_window_tab: DebugWindowTab::Console,
            debug_console_input: String::new(),
            debug_console_output_mode: DebugConsoleOutputMode::Pretty,
            debug_console_entries: Vec::new(),
            debug_console_history: Vec::new(),
            debug_console_history_index: None,
            debug_console_confirmed: false,
            debug_console_font_size: 13.0,
            debug_selected_peer: None,
            network_traffic_samples: Vec::new(),
            sync_progress_samples: Vec::new(),
            last_network_traffic_snapshot: None,
            wallet_utxos_cache: Vec::new(),
            wallet_activity_cache: Vec::new(),
            wallet_balance_summary_cache: WalletBalanceSummary::default(),
            wallet_cache_dirty: true,
            wallet_scan_nonce: 0,
            wallet_discovery_scan_limit: WALLET_DISCOVERY_SCAN_STEPS[0],
            wallet_discovery_scan_limit_cached: 0,
            last_wallet_refresh_at: Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(Instant::now),
            wallet_balance_cache: 0,
            theme_initialized: false,
            compact_viewport: false,
            show_sync_status_dialog: false,
            sync_status_hidden_until_synced: false,
            storage_recovery_notice: None,
            show_storage_recovery_notice_dialog: false,
        };

        app.view_model.network_label = app.connection.network().id().to_string();
        app.view_model.sync_stage = if app.connection.has_local_node() {
            String::from("Starting node")
        } else {
            String::from("Disconnected")
        };
        app.refresh_mining_accelerator_info();
        app.poll_storage_recovery_notice();
        app.try_open_existing_wallet_on_startup();
        app
    }

    pub fn refresh(&mut self) -> Result<(), QtError> {
        let status = self.connection.status();
        self.apply_connection_status(status);
        self.poll_storage_recovery_notice();
        self.poll_wallet_preparation_job();
        if self.wallet.is_some() {
            self.wallet_cache_dirty = true;
            self.refresh_wallet_cache_if_needed();
        }
        Ok(())
    }

    fn apply_connection_status(&mut self, status: ConnectionStatus) {
        let previously_synced = self.view_model.chain_synced();
        let previous_block_count = self.view_model.block_count;
        let previous_mempool_count = self.view_model.mempool_count;
        let previous_tip_hash = self.view_model.tip_hash;
        let previous_mempool_fingerprint = self.view_model.mempool_fingerprint;
        let startup_error = status.startup_error.clone();
        self.view_model.network_label = status.network.id().to_string();
        self.view_model.block_count = status.block_count;
        self.view_model.tip_hash = status.tip_hash;
        self.view_model.tip_timestamp = status.tip_timestamp;
        self.view_model.estimated_hashrate_hps = status.estimated_hashrate_hps;
        self.view_model.mempool_count = status.mempool_count;
        self.view_model.mempool_total_fee_atoms = status.mempool_total_fee_atoms;
        self.view_model.mempool_fingerprint = status.mempool_fingerprint;
        self.view_model.peer_count = status.peer_count;
        self.view_model.inbound_peer_count = status.inbound_peer_count;
        self.view_model.outbound_peer_count = status.outbound_peer_count;
        self.view_model.connecting_peer_count = status.connecting_peer_count;
        self.view_model.bytes_sent = status.bytes_sent;
        self.view_model.bytes_received = status.bytes_received;
        self.view_model.peers = status.peers.clone();
        self.view_model.connecting_peers = status.connecting_peers.clone();
        self.view_model.running = status.running;
        self.view_model.headers_synced = status.headers_synced;
        self.view_model.sync_best_height = status.sync_best_height.max(status.block_count);
        self.record_network_traffic_sample(&status);
        self.record_sync_progress_sample();
        self.ensure_debug_peer_selection();
        self.view_model.sync_stage = if let Some(error) = startup_error.as_ref() {
            format!("Startup error: {error}")
        } else if status.connected {
            if self.view_model.chain_synced() {
                String::from("Synced")
            } else if status.running
                && self.view_model.sync_target_height() > self.view_model.block_count
            {
                format!(
                    "Syncing {}/{}",
                    self.view_model.block_count,
                    self.view_model.sync_target_height()
                )
            } else if status.running && !status.headers_synced {
                String::from("Syncing headers")
            } else if status.running {
                format!("Running at local height {}", self.view_model.block_count)
            } else {
                String::from("Connected")
            }
        } else if self.connection.has_local_node() {
            String::from("Starting node")
        } else {
            String::from("Disconnected")
        };
        if let Some(error) = startup_error {
            self.wallet_readiness_gate_active = false;
            self.last_error = Some(error);
        }

        let currently_syncing =
            status.connected && status.running && !self.view_model.chain_synced();
        if currently_syncing {
            if !self.sync_status_hidden_until_synced {
                self.show_sync_status_dialog = true;
            }
        } else if previously_synced != self.view_model.chain_synced()
            || self.view_model.chain_synced()
        {
            self.sync_status_hidden_until_synced = false;
        }

        if self.wallet.is_some()
            && (status.block_count != previous_block_count
                || status.mempool_count != previous_mempool_count
                || status.tip_hash != previous_tip_hash
                || status.mempool_fingerprint != previous_mempool_fingerprint)
        {
            self.wallet_cache_dirty = true;
        }

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

    fn wallet_file_label(&self) -> String {
        self.wallet_path
            .as_ref()
            .and_then(|path| {
                normalize_wallet_path_input(path)
                    .parent()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| String::from("wallet"))
    }

    fn wallet_display_name(&self) -> String {
        self.current_wallet_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| self.wallet_path.as_deref().map(infer_wallet_name_from_path))
            .unwrap_or_else(|| String::from("Wallet"))
    }

    fn wallet_registry_entries(&self) -> Vec<WalletRegistryEntry> {
        let mut entries = load_wallet_registry(self.connection.network()).entries;
        entries.sort_by(|left, right| {
            right
                .updated_at_unix
                .cmp(&left.updated_at_unix)
                .then(left.wallet_name.cmp(&right.wallet_name))
        });
        entries
    }

    fn wallet_path_matches(&self, wallet_path: &str) -> bool {
        self.wallet_path
            .as_deref()
            .map(|current| {
                normalize_wallet_path_input(current) == normalize_wallet_path_input(wallet_path)
            })
            .unwrap_or(false)
    }

    pub(crate) fn begin_wallet_switch(&mut self, wallet_path: &str) -> Result<(), String> {
        let normalized = normalize_wallet_path_input(wallet_path);
        let normalized_label = normalized.to_string_lossy().into_owned();
        if self.wallet_path_matches(&normalized_label) {
            return Ok(());
        }

        let metadata =
            Wallet::inspect_datafile(normalized.as_path()).map_err(|err| err.to_string())?;
        if metadata.network != self.connection.network() {
            return Err(format!(
                "Selected wallet belongs to {} not {}",
                metadata.network.id(),
                self.connection.network().id()
            ));
        }

        self.open_form.wallet_path = normalized_label.clone();
        self.open_form.wallet_password.clear();
        self.last_error = None;
        self.stop_mining_job();

        match metadata.encryption_mode {
            WalletEncryptionMode::Plaintext => {
                self.start_open_wallet_preparation(normalized_label, String::new());
            }
            WalletEncryptionMode::PasswordAes256Gcm => {
                self.launch_page = LaunchPage::OpenWallet;
                self.send_status = String::from("Enter the wallet passphrase to switch wallets.");
            }
        }
        Ok(())
    }

    fn upsert_wallet_registry_entry(
        &mut self,
        entry: WalletRegistryEntry,
        mark_last_opened: bool,
    ) -> Result<WalletRegistryEntry, String> {
        let network = self.connection.network();
        let mut registry = load_wallet_registry(network);
        let now = current_unix_seconds();
        let mut merged = entry.clone();

        if let Some(existing) = registry
            .entries
            .iter_mut()
            .find(|existing| existing.wallet_id == entry.wallet_id)
        {
            existing.wallet_name = entry.wallet_name.clone();
            existing.wallet_path = entry.wallet_path.clone();
            existing.network = entry.network.clone();
            existing.word_count = entry.word_count;
            existing.updated_at_unix = now;
            if mark_last_opened {
                existing.last_opened_at_unix = Some(now);
            }
            merged = existing.clone();
        } else {
            merged.updated_at_unix = now;
            if mark_last_opened {
                merged.last_opened_at_unix = Some(now);
            }
            registry.entries.push(merged.clone());
        }

        registry
            .entries
            .retain(|saved| PathBuf::from(&saved.wallet_path).exists());
        registry.entries.sort_by(|left, right| {
            right
                .updated_at_unix
                .cmp(&left.updated_at_unix)
                .then(left.wallet_name.cmp(&right.wallet_name))
        });
        if mark_last_opened {
            registry.last_opened_wallet_id = Some(merged.wallet_id.clone());
        }
        persist_wallet_registry(network, &registry)?;
        Ok(merged)
    }

    fn rename_current_wallet(&mut self) -> Result<(), String> {
        let wallet_id = self
            .current_wallet_id
            .clone()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let wallet_path = self
            .wallet_path
            .clone()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let wallet_name = self
            .current_wallet_name
            .clone()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if wallet_name.is_empty() {
            return Err(String::from("Enter a wallet name"));
        }
        let word_count = self
            .wallet_ref()
            .and_then(Wallet::mnemonic_phrase)
            .map(MnemonicPhrase::word_count)
            .unwrap_or_default();
        let created_at_unix = self
            .wallet_registry_entries()
            .into_iter()
            .find(|entry| entry.wallet_id == wallet_id)
            .map(|entry| entry.created_at_unix)
            .unwrap_or_else(current_unix_seconds);
        let entry = WalletRegistryEntry {
            wallet_id,
            wallet_name: wallet_name.clone(),
            wallet_path,
            network: self.connection.network().id().to_string(),
            created_at_unix,
            updated_at_unix: current_unix_seconds(),
            last_opened_at_unix: Some(current_unix_seconds()),
            word_count,
        };
        let saved = self.upsert_wallet_registry_entry(entry, true)?;
        self.current_wallet_name = Some(saved.wallet_name);
        Ok(())
    }

    fn wallet_recovery_export(&self) -> Result<WalletRecoveryExport, String> {
        let wallet = self
            .wallet_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let wallet_path = self
            .wallet_path
            .clone()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let (highest_generated_receive_index, highest_generated_change_index) =
            wallet.highest_generated_indices();
        let (highest_reserved_receive_index, highest_reserved_change_index) =
            wallet.highest_reserved_indices();
        let (next_receive_index, next_change_index) = wallet.next_indices();
        let mnemonic_word_count = wallet
            .mnemonic_phrase()
            .map(MnemonicPhrase::word_count)
            .unwrap_or_default();

        Ok(WalletRecoveryExport {
            wallet_name: self.wallet_display_name(),
            wallet_path: wallet_path.clone(),
            wallet_id: self.current_wallet_id.clone(),
            network: wallet.network.id().to_string(),
            exported_at_unix: current_unix_seconds(),
            mnemonic_word_count,
            mnemonic_phrase: wallet.mnemonic_sentence(),
            configured_recovery_window: wallet.restore_gap_limit(),
            active_scan_window: self.wallet_discovery_scan_limit,
            current_receive_index: self.wallet_current_receive_index(),
            next_receive_index,
            next_change_index,
            highest_generated_receive_index,
            highest_generated_change_index,
            highest_reserved_receive_index,
            highest_reserved_change_index,
        })
    }

    fn operator_root_label(&self) -> String {
        atho_storage::path::sandbox_root()
            .to_string_lossy()
            .into_owned()
    }

    fn network_data_root_label(&self) -> String {
        atho_storage::path::database_dir(self.connection.network())
            .to_string_lossy()
            .into_owned()
    }

    fn block_storage_root_label(&self) -> String {
        atho_storage::path::block_storage_dir(self.connection.network())
            .to_string_lossy()
            .into_owned()
    }

    fn chain_recovery_root_label(&self) -> String {
        atho_storage::path::chain_dir()
            .to_string_lossy()
            .into_owned()
    }

    fn quarantine_root_label(&self) -> String {
        atho_storage::path::quarantine_dir()
            .to_string_lossy()
            .into_owned()
    }

    fn wallet_keypool_depths(&self) -> (usize, usize) {
        self.wallet_ref()
            .map(Wallet::keypool_depths)
            .unwrap_or_default()
    }

    fn wallet_next_indices(&self) -> (u32, u32) {
        self.wallet_ref()
            .map(Wallet::next_indices)
            .unwrap_or((0, 0))
    }

    fn wallet_highest_reserved_indices(&self) -> (Option<u32>, Option<u32>) {
        self.wallet_ref()
            .map(Wallet::highest_reserved_indices)
            .unwrap_or((None, None))
    }

    fn wallet_highest_generated_indices(&self) -> (Option<u32>, Option<u32>) {
        self.wallet_ref()
            .map(Wallet::highest_generated_indices)
            .unwrap_or((None, None))
    }

    fn wallet_configured_recovery_window(&self) -> usize {
        self.wallet_ref()
            .map(Wallet::restore_gap_limit)
            .unwrap_or(DEFAULT_RESTORE_GAP_LIMIT)
    }

    fn wallet_current_receive_index(&self) -> Option<u32> {
        self.current_receive_address
            .as_ref()
            .map(|address| address.path.index)
    }

    fn sync_wallet_recovery_window_form(&mut self) {
        self.wallet_management_form.restore_gap_limit_input =
            self.wallet_configured_recovery_window().to_string();
    }

    fn queue_wallet_scan_to_limit(&mut self, limit: usize) -> bool {
        if limit <= self.wallet_discovery_scan_limit {
            return false;
        }
        self.wallet_discovery_scan_limit = limit;
        self.wallet_cache_dirty = true;
        self.last_wallet_refresh_at = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        true
    }

    fn wallet_preparation_blocks_startup(&self) -> bool {
        self.wallet.is_none() && self.wallet_preparation_job.is_some()
    }

    fn wallet_readiness_blocks_main_ui(&self) -> bool {
        // Keep the launch screen only for active wallet preparation or an in-flight first scan.
        // Deferred scans keep retrying from the main shell so backend churn cannot trap startup.
        self.wallet_preparation_blocks_startup() || self.wallet_readiness_gate_active
    }

    pub(crate) fn clear_wallet_state(&mut self) {
        self.stop_mining_job();
        self.send_job = None;
        self.mining_job = None;
        self.pending_mining_restart = None;
        self.wallet = None;
        self.current_wallet_id = None;
        self.current_wallet_name = None;
        self.wallet_path = None;
        self.wallet_session_password = None;
        self.wallet_addresses_cache.clear();
        self.wallet_address_index_cache.clear();
        self.wallet_address_digests_cache.clear();
        self.wallet_owned_utxos_cache.clear();
        self.receive_addresses.clear();
        self.receive_address_rows.clear();
        self.current_receive_address = None;
        self.requested_payments.clear();
        self.selected_receive_request = None;
        self.wallet_utxos_cache.clear();
        self.wallet_activity_cache.clear();
        self.wallet_balance_summary_cache = WalletBalanceSummary::default();
        self.wallet_balance_cache = 0;
        self.wallet_cache_dirty = true;
        self.wallet_scan_job = None;
        self.wallet_readiness_gate_active = false;
        self.wallet_scan_nonce = self.wallet_scan_nonce.wrapping_add(1);
        self.wallet_discovery_scan_limit = WALLET_DISCOVERY_SCAN_STEPS[0];
        self.wallet_discovery_scan_limit_cached = 0;
        self.receive_label.clear();
        self.receive_amount.clear();
        self.receive_message.clear();
        self.send_to.clear();
        self.send_label.clear();
        self.send_amount.clear();
        self.send_fee.clear();
        self.send_include_fee_in_total = false;
        self.send_status = String::from("Enter a destination address and amount.");
        self.mining_status = String::from("Idle");
        self.last_mined_height = None;
        self.last_mined_block_hash = None;
        self.last_mined_at_unix = None;
        self.last_error = None;
        self.wallet_management_form.backup_password.clear();
        self.wallet_management_form.backup_password_confirm.clear();
        let default_wallet_path = default_wallet_path(self.connection.network())
            .to_string_lossy()
            .into_owned();
        self.wallet_management_form.backup_path = backup_wallet_path(&default_wallet_path);
        self.wallet_management_form.backup_json_path =
            backup_wallet_json_path(&default_wallet_path);
        self.wallet_management_form.backup_text_path =
            backup_wallet_text_path(&default_wallet_path);
        self.wallet_management_form.backup_phrase_qr_path =
            backup_wallet_phrase_qr_path(&default_wallet_path);
        self.wallet_management_form.restore_gap_limit_input = DEFAULT_RESTORE_GAP_LIMIT.to_string();
        self.receive_page_tab = ReceivePageTab::RequestPayment;
        self.address_pool_filter = AddressPoolFilter::Unused;
        self.recipient_address_book_filter.clear();
        self.recipient_address_book_open = false;
        self.recipient_address_editor_open = false;
        self.recipient_address_editor_id = None;
        self.recipient_address_editor_label.clear();
        self.recipient_address_editor_address.clear();
        self.recipient_address_editor_notes.clear();
        self.ui_state.wallet_snapshot = Default::default();
        self.view_model.ui_state.wallet_snapshot = Default::default();
    }

    #[allow(dead_code)]
    fn refresh_wallet_cache(&mut self) {
        let Some(_wallet) = self.wallet_ref() else {
            self.wallet_addresses_cache.clear();
            self.wallet_address_index_cache.clear();
            self.wallet_address_digests_cache.clear();
            self.wallet_owned_utxos_cache.clear();
            self.wallet_utxos_cache.clear();
            self.wallet_activity_cache.clear();
            self.wallet_balance_summary_cache = WalletBalanceSummary::default();
            self.wallet_balance_cache = 0;
            self.receive_addresses.clear();
            self.receive_address_rows.clear();
            self.wallet_discovery_scan_limit = WALLET_DISCOVERY_SCAN_STEPS[0];
            self.wallet_discovery_scan_limit_cached = 0;
            self.current_receive_address = None;
            return;
        };
        if !self.wallet_scan_rpc_ready() {
            return;
        }
        let utxos = match self.connection.request(RpcRequest::ListUtxos) {
            RpcResponse::Utxos(utxos) => utxos,
            RpcResponse::Error(err) => {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("wallet utxo refresh deferred error={err}"),
                );
                return;
            }
            other => {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("wallet utxo refresh received unexpected response: {other:?}"),
                );
                return;
            }
        };

        if self.wallet_addresses_cache.is_empty()
            || self.wallet_discovery_scan_limit_cached != self.wallet_discovery_scan_limit
        {
            let wallet_addresses = {
                let Some(wallet) = self.wallet_ref() else {
                    return;
                };
                wallet.discovery_addresses_up_to(self.wallet_discovery_scan_limit)
            };
            self.cache_wallet_addresses(wallet_addresses);
        } else if self.wallet_address_index_cache.len() != self.wallet_addresses_cache.len()
            || self.wallet_address_digests_cache.len() != self.wallet_addresses_cache.len()
        {
            self.rebuild_wallet_address_caches();
        }

        let current_height = {
            let status = self.connection.status();
            Self::wallet_scan_height(&status)
        };
        let reserved_inputs = self.mempool_reserved_inputs();
        let address_digests = &self.wallet_address_digests_cache;
        let mut owned = utxos
            .into_par_iter()
            .filter_map(|utxo| {
                let digest: [u8; 32] = utxo.locking_script.as_slice().try_into().ok()?;
                if !address_digests.contains(&digest)
                    || reserved_inputs.contains(&(utxo.txid, utxo.output_index))
                {
                    return None;
                }
                Some(utxo)
            })
            .collect::<Vec<_>>();
        owned.sort_by(|left, right| {
            right
                .txid
                .cmp(&left.txid)
                .then(right.output_index.cmp(&left.output_index))
        });
        let available_owned = owned
            .iter()
            .filter(|utxo| utxo.is_spendable_at(current_height))
            .cloned()
            .collect::<Vec<_>>();
        let balance_summary = wallet_ledger::summarize_wallet_utxos(&owned, current_height);
        let available_balance = available_owned.iter().map(|utxo| utxo.value_atoms).sum();
        let activity = match Self::request_wallet_activity_rows(
            &self.connection,
            &self.wallet_addresses_cache,
        ) {
            Ok(activity) => activity,
            Err(err) => {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("wallet activity refresh failed error={err}"),
                );
                Vec::new()
            }
        };

        self.wallet_owned_utxos_cache = owned;
        self.refresh_receive_address_rows();
        self.wallet_balance_summary_cache = balance_summary;
        self.wallet_activity_cache = activity;

        self.wallet_utxos_cache = available_owned;
        self.wallet_balance_cache = available_balance;
        self.wallet_balance_summary_cache.available_atoms = available_balance;
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "wallet cache refreshed scan_limit={} addresses={} owned_utxos={} spendable_utxos={} balance_atoms={} height={}",
                self.wallet_discovery_scan_limit,
                self.wallet_addresses_cache.len(),
                self.wallet_owned_utxos_cache.len(),
                self.wallet_utxos_cache.len(),
                self.wallet_balance_cache,
                current_height
            ),
        );
        let continue_scanning = if Self::should_expand_wallet_discovery_scan_limit(
            self.wallet_discovery_scan_limit,
            &self.wallet_addresses_cache,
            &self.wallet_owned_utxos_cache,
        ) {
            self.advance_wallet_discovery_scan_limit()
        } else {
            false
        };
        if !continue_scanning {
            self.wallet_readiness_gate_active = false;
        }
        self.last_wallet_refresh_at = Instant::now();
        self.wallet_cache_dirty = continue_scanning;
    }

    fn wallet_balance_atoms(&self) -> u64 {
        self.wallet_balance_cache
    }

    #[cfg(test)]
    fn force_wallet_cache_refresh_for_test(&mut self) {
        self.wallet_scan_nonce = self.wallet_scan_nonce.wrapping_add(1);
        self.wallet_scan_job = None;
        self.wallet_cache_dirty = false;
        self.refresh_wallet_cache();
    }

    #[cfg(test)]
    fn refresh_status_only_for_test(&mut self) -> Result<(), QtError> {
        let status = self.connection.status();
        self.apply_connection_status(status);
        Ok(())
    }

    fn mempool_reserved_inputs(&self) -> HashSet<([u8; 48], u32)> {
        match self.connection.request(RpcRequest::GetMempoolSpentInputs) {
            RpcResponse::MempoolSpentInputs(inputs) => inputs
                .into_iter()
                .map(|MempoolSpentInput { txid, output_index }| (txid, output_index))
                .collect(),
            RpcResponse::Error(err) => {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("mempool reservation lookup failed error={err}"),
                );
                HashSet::new()
            }
            other => {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("unexpected mempool reservation response: {other:?}"),
                );
                HashSet::new()
            }
        }
    }

    fn spendable_wallet_inputs(
        &self,
        reserved_inputs: &HashSet<([u8; 48], u32)>,
    ) -> Result<Vec<SpendableWalletUtxo>, String> {
        if self.wallet_utxos_cache.is_empty() {
            return Err(String::from(
                "No cached wallet UTXOs available; wait for the wallet scan to complete",
            ));
        }
        if self.wallet_address_index_cache.is_empty() {
            return Err(String::from(
                "Wallet discovery is still scanning; try again after the refresh completes",
            ));
        }

        let wallet_addresses = &self.wallet_addresses_cache;
        let wallet_address_index_cache = &self.wallet_address_index_cache;
        let mut entries = Vec::new();
        for utxo in self.wallet_utxos_cache.clone() {
            if reserved_inputs.contains(&(utxo.txid, utxo.output_index)) {
                continue;
            }
            let digest: [u8; 32] = match utxo.locking_script.as_slice().try_into() {
                Ok(digest) => digest,
                Err(_) => continue,
            };
            let Some(&index) = wallet_address_index_cache.get(&digest) else {
                continue;
            };
            entries.push(SpendableWalletUtxo {
                address: wallet_addresses[index].clone(),
                utxo,
            });
        }
        Ok(entries)
    }

    fn wallet_activity_rows(&self) -> &[WalletActivityRow] {
        &self.wallet_activity_cache
    }

    fn wallet_balance_summary(&self) -> &WalletBalanceSummary {
        &self.wallet_balance_summary_cache
    }

    fn wallet_history_addresses(wallet_addresses: &[WalletAddress]) -> Vec<WalletHistoryAddress> {
        wallet_addresses
            .iter()
            .map(|address| WalletHistoryAddress {
                payment_digest: address.payment_digest,
                address: address.address.clone(),
            })
            .collect()
    }

    fn wallet_activity_row_from_rpc(entry: RpcWalletActivityEntry) -> WalletActivityRow {
        WalletActivityRow {
            when: format!("H{}", entry.height),
            kind: match entry.kind {
                RpcWalletActivityKind::Mined => WalletActivityKind::Mined,
                RpcWalletActivityKind::Received => WalletActivityKind::Received,
                RpcWalletActivityKind::Sent => WalletActivityKind::Sent,
            },
            label: entry.label,
            amount_atoms: entry.amount_atoms,
            reference: widgets::short_hash(&entry.txid),
        }
    }

    fn request_wallet_activity_rows(
        connection: &ReadOnlyNodeConnection,
        wallet_addresses: &[WalletAddress],
    ) -> Result<Vec<WalletActivityRow>, String> {
        match connection.request(RpcRequest::GetWalletActivity {
            addresses: Self::wallet_history_addresses(wallet_addresses),
        }) {
            RpcResponse::WalletActivity(entries) => Ok(entries
                .into_iter()
                .map(Self::wallet_activity_row_from_rpc)
                .collect()),
            RpcResponse::Error(err) => Err(format!("wallet activity lookup failed: {err}")),
            other => Err(format!("unexpected wallet activity response: {other:?}")),
        }
    }

    fn refresh_receive_address_rows(&mut self) {
        let current_digest = self
            .current_receive_address
            .as_ref()
            .map(|address| address.payment_digest);
        self.receive_address_rows = Self::build_receive_address_rows(
            &self.wallet_addresses_cache,
            &self.wallet_owned_utxos_cache,
            current_digest,
        );
    }

    fn build_wallet_scan_snapshot(
        connection: ReadOnlyNodeConnection,
        wallet_addresses_cache: Vec<WalletAddress>,
        receive_addresses: Vec<WalletAddress>,
        wallet_scan_nonce: u64,
        scan_limit: usize,
    ) -> Result<WalletScanOutcome, String> {
        let status = connection.status();
        let snapshot_token = Self::connection_snapshot_token(&status);
        let utxos = match connection.request(RpcRequest::ListUtxos) {
            RpcResponse::Utxos(utxos) => utxos,
            RpcResponse::Error(err) => return Err(format!("utxo lookup failed: {err}")),
            other => return Err(format!("unexpected UTXO response: {other:?}")),
        };
        let reserved_inputs = match connection.request(RpcRequest::GetMempoolSpentInputs) {
            RpcResponse::MempoolSpentInputs(inputs) => inputs
                .into_iter()
                .map(|MempoolSpentInput { txid, output_index }| (txid, output_index))
                .collect::<HashSet<_>>(),
            RpcResponse::Error(err) => {
                return Err(format!("mempool reservation lookup failed: {err}"));
            }
            other => {
                return Err(format!(
                    "unexpected mempool reservation response: {other:?}"
                ));
            }
        };

        let address_digests = wallet_addresses_cache
            .iter()
            .map(|address| address.payment_digest)
            .collect::<HashSet<_>>();
        let current_height = match connection.request(RpcRequest::GetBlockCount) {
            RpcResponse::BlockCount(count) => count,
            RpcResponse::Error(err) => return Err(format!("block count lookup failed: {err}")),
            other => return Err(format!("unexpected block count response: {other:?}")),
        };
        let mut owned = utxos
            .into_par_iter()
            .filter_map(|utxo| {
                let digest: [u8; 32] = utxo.locking_script.as_slice().try_into().ok()?;
                if !address_digests.contains(&digest)
                    || reserved_inputs.contains(&(utxo.txid, utxo.output_index))
                {
                    return None;
                }
                Some(utxo)
            })
            .collect::<Vec<_>>();
        owned.sort_by(|left, right| {
            right
                .txid
                .cmp(&left.txid)
                .then(right.output_index.cmp(&left.output_index))
        });
        let available_owned = owned
            .iter()
            .filter(|utxo| utxo.is_spendable_at(current_height))
            .cloned()
            .collect::<Vec<_>>();
        let balance_summary = wallet_ledger::summarize_wallet_utxos(&owned, current_height);
        let available_balance = available_owned.iter().map(|utxo| utxo.value_atoms).sum();
        let wallet_activity_cache =
            Self::request_wallet_activity_rows(&connection, &wallet_addresses_cache)?;
        let end_status = connection.status();
        if snapshot_token != Self::connection_snapshot_token(&end_status) {
            return Err(String::from(
                "wallet scan deferred: backend state changed during refresh",
            ));
        }

        Ok(WalletScanOutcome {
            wallet_scan_nonce,
            scan_limit,
            wallet_addresses_cache,
            receive_addresses,
            wallet_owned_utxos_cache: owned,
            wallet_utxos_cache: available_owned,
            wallet_balance_cache: available_balance,
            wallet_balance_summary_cache: balance_summary,
            wallet_activity_cache,
        })
    }

    fn cache_wallet_addresses(&mut self, wallet_addresses: Vec<WalletAddress>) {
        self.wallet_addresses_cache = wallet_addresses;
        self.wallet_discovery_scan_limit_cached = self.wallet_discovery_scan_limit;
        self.rebuild_wallet_address_caches();
    }

    fn rebuild_wallet_address_caches(&mut self) {
        self.wallet_address_index_cache.clear();
        self.wallet_address_digests_cache.clear();
        for (index, address) in self.wallet_addresses_cache.iter().enumerate() {
            self.wallet_address_index_cache
                .insert(address.payment_digest, index);
            self.wallet_address_digests_cache
                .insert(address.payment_digest);
        }
    }

    fn append_generated_address(&mut self, address: WalletAddress) {
        let index = self.wallet_addresses_cache.len();
        self.wallet_addresses_cache.push(address.clone());
        self.wallet_address_index_cache
            .insert(address.payment_digest, index);
        self.wallet_address_digests_cache
            .insert(address.payment_digest);

        if address.path.kind == AddressKind::Receive {
            for row in &mut self.receive_address_rows {
                row.is_current = false;
            }
            let (utxo_count, total_atoms) = self
                .wallet_owned_utxos_cache
                .iter()
                .filter_map(|utxo| {
                    let digest: [u8; 32] = utxo.locking_script.as_slice().try_into().ok()?;
                    (digest == address.payment_digest).then_some(utxo.value_atoms)
                })
                .fold((0usize, 0u64), |(count, total), value_atoms| {
                    (count.saturating_add(1), total.saturating_add(value_atoms))
                });
            self.receive_address_rows.push(ReceiveAddressRow {
                address: address.clone(),
                used: utxo_count > 0,
                utxo_count,
                total_atoms,
                is_current: true,
            });
        }
    }

    fn build_receive_address_rows(
        wallet_addresses: &[WalletAddress],
        owned_utxos: &[UtxoEntry],
        current_digest: Option<[u8; 32]>,
    ) -> Vec<ReceiveAddressRow> {
        let mut usage_by_digest: HashMap<[u8; 32], (usize, u64)> = HashMap::new();
        for utxo in owned_utxos {
            let Ok(digest) = utxo.locking_script.as_slice().try_into() else {
                continue;
            };
            let entry = usage_by_digest.entry(digest).or_insert((0, 0));
            entry.0 = entry.0.saturating_add(1);
            entry.1 = entry.1.saturating_add(utxo.value_atoms);
        }

        wallet_addresses
            .iter()
            .filter(|address| address.path.kind == AddressKind::Receive)
            .map(|address| {
                let (utxo_count, total_atoms) = usage_by_digest
                    .get(&address.payment_digest)
                    .copied()
                    .unwrap_or((0, 0));
                ReceiveAddressRow {
                    address: address.clone(),
                    used: utxo_count > 0,
                    utxo_count,
                    total_atoms,
                    is_current: current_digest == Some(address.payment_digest),
                }
            })
            .collect()
    }

    fn refresh_wallet_address_views(&mut self) {
        self.wallet_addresses_cache.clear();
        self.wallet_address_index_cache.clear();
        self.wallet_address_digests_cache.clear();
        self.wallet_owned_utxos_cache.clear();
        self.wallet_utxos_cache.clear();
        self.wallet_activity_cache.clear();
        self.wallet_balance_summary_cache = WalletBalanceSummary::default();
        self.wallet_balance_cache = 0;
        self.receive_addresses.clear();
        self.receive_address_rows.clear();
    }

    fn start_wallet_scan_job(&mut self) {
        if self.wallet.is_none() || self.wallet_scan_job.is_some() {
            return;
        }

        let scan_limit = self.wallet_discovery_scan_limit;
        let reuse_cached_addresses = self.wallet_discovery_scan_limit_cached == scan_limit
            && !self.wallet_addresses_cache.is_empty();
        let wallet_addresses_cache = if reuse_cached_addresses {
            self.wallet_addresses_cache.clone()
        } else {
            Vec::new()
        };
        let wallet_for_scan = if reuse_cached_addresses {
            None
        } else {
            self.wallet_ref().cloned()
        };
        let connection = self.connection.clone();
        let wallet_scan_nonce = self.wallet_scan_nonce;
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            let wallet_addresses_cache = if let Some(wallet) = wallet_for_scan {
                wallet.discovery_addresses_up_to(scan_limit)
            } else {
                wallet_addresses_cache
            };
            let receive_addresses = wallet_addresses_cache
                .iter()
                .filter(|address| address.path.kind == AddressKind::Receive)
                .take(RECEIVE_ADDRESS_LIST_LIMIT)
                .cloned()
                .collect::<Vec<_>>();
            let result = Self::build_wallet_scan_snapshot(
                connection,
                wallet_addresses_cache,
                receive_addresses,
                wallet_scan_nonce,
                scan_limit,
            );
            let _ = sender.send(result);
        });

        self.wallet_scan_job = Some(WalletScanJob {
            started_at: Instant::now(),
            receiver,
        });
        self.wallet_cache_dirty = false;
    }

    fn apply_wallet_scan_outcome(&mut self, outcome: WalletScanOutcome) {
        if outcome.wallet_scan_nonce != self.wallet_scan_nonce || self.wallet.is_none() {
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!(
                    "wallet scan outcome discarded stale_nonce={} current_nonce={}",
                    outcome.wallet_scan_nonce, self.wallet_scan_nonce
                ),
            );
            return;
        }

        self.cache_wallet_addresses(outcome.wallet_addresses_cache);
        self.receive_addresses = outcome.receive_addresses;
        self.wallet_owned_utxos_cache = outcome.wallet_owned_utxos_cache;
        self.wallet_utxos_cache = outcome.wallet_utxos_cache;
        self.wallet_balance_cache = outcome.wallet_balance_cache;
        self.wallet_balance_summary_cache = outcome.wallet_balance_summary_cache;
        self.wallet_activity_cache = outcome.wallet_activity_cache;
        self.refresh_receive_address_rows();
        let continue_scanning = if Self::should_expand_wallet_discovery_scan_limit(
            self.wallet_discovery_scan_limit,
            &self.wallet_addresses_cache,
            &self.wallet_owned_utxos_cache,
        ) {
            self.advance_wallet_discovery_scan_limit()
        } else {
            false
        };
        self.wallet_cache_dirty = self.wallet_cache_dirty || continue_scanning;
        if !continue_scanning {
            self.wallet_readiness_gate_active = false;
        }
        self.last_wallet_refresh_at = Instant::now();
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "wallet cache refreshed scan_limit={} addresses={} owned_utxos={} spendable_utxos={} balance_atoms={}",
                outcome.scan_limit,
                self.wallet_addresses_cache.len(),
                self.wallet_owned_utxos_cache.len(),
                self.wallet_utxos_cache.len(),
                self.wallet_balance_cache
            ),
        );
    }

    fn poll_wallet_scan_job(&mut self) {
        let Some(job) = self.wallet_scan_job.take() else {
            return;
        };

        match job.receiver.try_recv() {
            Ok(Ok(outcome)) => {
                self.apply_wallet_scan_outcome(outcome);
            }
            Ok(Err(err)) => {
                if err.contains("node RPC is not ready")
                    || err.contains("backend state changed during refresh")
                {
                    self.wallet_cache_dirty = true;
                    self.release_wallet_readiness_gate("wallet scan deferred");
                    self.last_wallet_refresh_at = Instant::now();
                    let _ = atho_node::dev::append_log(
                        "atho-qt",
                        &format!("wallet scan deferred error={err}"),
                    );
                    return;
                }
                self.wallet_cache_dirty = true;
                self.wallet_readiness_gate_active = false;
                self.last_error = Some(err.clone());
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("wallet scan worker failed error={err}"),
                );
            }
            Err(mpsc::TryRecvError::Empty) => {
                if job.started_at.elapsed() >= WALLET_SCAN_STALL_TIMEOUT {
                    self.wallet_cache_dirty = true;
                    self.wallet_readiness_gate_active = false;
                    self.last_error = Some(String::from("wallet scan timed out"));
                    let _ = atho_node::dev::append_log(
                        "atho-qt",
                        &format!(
                            "wallet scan timed out elapsed_ms={}",
                            job.started_at.elapsed().as_millis()
                        ),
                    );
                    return;
                }
                self.wallet_scan_job = Some(job);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.wallet_cache_dirty = true;
                self.wallet_readiness_gate_active = false;
                self.last_error = Some(String::from("wallet scan worker disconnected"));
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    "wallet scan worker disconnected error=channel closed",
                );
            }
        }

        if self.wallet_scan_job.is_none() {
            let elapsed = job.started_at.elapsed();
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!("wallet scan job finished in {}ms", elapsed.as_millis()),
            );
        }
    }

    fn refresh_wallet_cache_if_needed(&mut self) {
        self.poll_wallet_scan_job();
        if !self.wallet_cache_dirty || self.wallet.is_none() {
            return;
        }
        if !self.wallet_scan_rpc_ready() {
            if self.wallet_scan_job.take().is_some() {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    "wallet scan job dropped because RPC is not ready",
                );
            }
            self.release_wallet_readiness_gate("wallet scan RPC not ready");
            return;
        }
        if self.last_wallet_refresh_at.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.start_wallet_scan_job();
    }

    fn release_wallet_readiness_gate(&mut self, reason: &str) {
        if !self.wallet_readiness_gate_active {
            return;
        }
        self.wallet_readiness_gate_active = false;
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!("wallet readiness gate released reason={reason}"),
        );
    }

    fn wallet_scan_rpc_ready(&self) -> bool {
        self.ui_state.connected && self.view_model.running
    }

    fn wallet_scan_height(status: &ConnectionStatus) -> u64 {
        // Wallet maturity and spendability must follow the local canonical chain height.
        // Peer-advertised best height is useful for display, but it must not make immature
        // outputs appear spendable before the node has actually advanced its own tip.
        status.block_count
    }

    fn connection_snapshot_token(status: &ConnectionStatus) -> [u8; 32] {
        let mut preimage = Vec::with_capacity(
            status.network.id().len()
                + core::mem::size_of::<u64>()
                + status.tip_hash.len()
                + status.mempool_fingerprint.len(),
        );
        preimage.extend_from_slice(status.network.id().as_bytes());
        preimage.extend_from_slice(&status.block_count.to_be_bytes());
        preimage.extend_from_slice(&status.tip_hash);
        preimage.extend_from_slice(&status.mempool_fingerprint);
        sha3_256(&preimage)
    }

    fn start_wallet_preparation_job<F>(&mut self, stage: &str, worker: F)
    where
        F: FnOnce(mpsc::Sender<WalletPreparationEvent>) -> Result<WalletPreparationOutcome, String>
            + Send
            + 'static,
    {
        if self.wallet_preparation_job.is_some() {
            self.last_error = Some(String::from("wallet preparation already in progress"));
            return;
        }

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = worker(sender.clone());
            let _ = sender.send(WalletPreparationEvent::Finished(Box::new(result)));
        });

        let now = Instant::now();
        self.wallet_preparation_job = Some(WalletPreparationJob {
            started_at: now,
            last_progress_at: now,
            receiver,
        });
        self.wallet_preparation_stage = stage.to_owned();
        self.wallet_preparation_progress = 0.0;
        self.wallet_preparation_completed = 0;
        self.wallet_preparation_total = 0;
        self.last_error = None;
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!("wallet preparation started stage={stage}"),
        );
    }

    fn poll_wallet_preparation_job(&mut self) {
        let Some(mut job) = self.wallet_preparation_job.take() else {
            return;
        };

        let completed_result = loop {
            match job.receiver.try_recv() {
                Ok(WalletPreparationEvent::Progress {
                    stage,
                    completed,
                    total,
                }) => {
                    job.last_progress_at = Instant::now();
                    self.wallet_preparation_stage = stage;
                    self.wallet_preparation_completed = completed;
                    self.wallet_preparation_total = total;
                    self.wallet_preparation_progress = if total == 0 {
                        0.0
                    } else {
                        (completed as f32 / total as f32).clamp(0.0, 1.0)
                    };
                }
                Ok(WalletPreparationEvent::Finished(result)) => {
                    break *result;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if job.last_progress_at.elapsed() >= WALLET_PREPARATION_STALL_TIMEOUT {
                        self.last_error = Some(String::from("wallet preparation timed out"));
                        self.wallet_preparation_stage =
                            String::from("Wallet preparation timed out");
                        self.wallet_preparation_progress = 0.0;
                        self.wallet_preparation_completed = 0;
                        self.wallet_preparation_total = 0;
                        self.wallet_preparation_job = None;
                        let _ = atho_node::dev::append_log(
                            "atho-qt",
                            &format!(
                                "wallet preparation timed out elapsed_ms={}",
                                job.started_at.elapsed().as_millis()
                            ),
                        );
                        return;
                    }
                    self.wallet_preparation_job = Some(job);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.last_error = Some(String::from("wallet preparation worker disconnected"));
                    self.wallet_preparation_stage = String::from("Wallet preparation failed");
                    self.wallet_preparation_progress = 0.0;
                    self.wallet_preparation_completed = 0;
                    self.wallet_preparation_total = 0;
                    let _ = atho_node::dev::append_log(
                        "atho-qt",
                        "wallet preparation worker disconnected error=channel closed",
                    );
                    return;
                }
            }
        };

        let elapsed = job.started_at.elapsed();
        self.wallet_preparation_job = None;
        match completed_result {
            Ok(outcome) => {
                self.wallet_preparation_stage = String::from("Wallet ready");
                self.wallet_preparation_progress = 1.0;
                self.wallet_preparation_completed = self.wallet_preparation_total;
                self.load_or_create_wallet(
                    outcome.wallet,
                    outcome.wallet_path,
                    outcome.wallet_password,
                    outcome.registry_entry,
                );
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("wallet preparation finished in {}ms", elapsed.as_millis()),
                );
            }
            Err(err) => {
                self.wallet_preparation_stage = String::from("Wallet preparation failed");
                self.wallet_preparation_progress = 0.0;
                self.wallet_preparation_completed = 0;
                self.wallet_preparation_total = 0;
                self.last_error = Some(err.clone());
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!(
                        "wallet preparation failed error={err} elapsed_ms={}",
                        elapsed.as_millis()
                    ),
                );
            }
        }
    }

    fn should_expand_wallet_discovery_scan_limit(
        scan_limit: usize,
        wallet_addresses: &[WalletAddress],
        owned_utxos: &[UtxoEntry],
    ) -> bool {
        owned_utxos
            .iter()
            .filter_map(|utxo| {
                let digest: [u8; 32] = utxo.locking_script.as_slice().try_into().ok()?;
                wallet_addresses
                    .iter()
                    .find(|address| address.payment_digest == digest)
                    .map(|address| address.path.index.saturating_add(1) as usize)
            })
            .max()
            .is_some_and(|highest_used_index| highest_used_index >= scan_limit)
    }

    fn advance_wallet_discovery_scan_limit(&mut self) -> bool {
        for &step in WALLET_DISCOVERY_SCAN_STEPS {
            if step > self.wallet_discovery_scan_limit {
                return self.queue_wallet_scan_to_limit(step);
            }
        }
        false
    }

    fn apply_wallet_recovery_window_setting(&mut self) -> Result<String, String> {
        let limit = self
            .wallet_management_form
            .restore_gap_limit_input
            .trim()
            .parse::<usize>()
            .map_err(|_| String::from("Recovery window must be a whole number"))?;
        if limit < MIN_WALLET_DISCOVERY_SCAN_LIMIT {
            return Err(format!(
                "Recovery window must be at least {MIN_WALLET_DISCOVERY_SCAN_LIMIT}"
            ));
        }
        if limit > MAX_WALLET_DISCOVERY_SCAN_LIMIT {
            return Err(format!(
                "Recovery window must be at most {MAX_WALLET_DISCOVERY_SCAN_LIMIT}"
            ));
        }

        let current_scan_limit = self.wallet_discovery_scan_limit;
        {
            let wallet = self
                .wallet_mut()
                .ok_or_else(|| String::from("Load or create a wallet first"))?;
            wallet.set_restore_gap_limit(limit);
        }
        self.sync_wallet_recovery_window_form();
        self.persist_loaded_wallet_state()?;

        if self.queue_wallet_scan_to_limit(limit) {
            Ok(format!(
                "Recovery window set to {limit}. Background scan queued from {current_scan_limit} to {limit}."
            ))
        } else {
            self.wallet_cache_dirty = true;
            Ok(format!(
                "Recovery window set to {limit}. Current session already scanned to {current_scan_limit}."
            ))
        }
    }

    fn attach_wallet(&mut self, mut wallet: Wallet, wallet_path: String) {
        let has_receive = wallet
            .address_book
            .snapshot()
            .iter()
            .any(|record| record.path.kind == AddressKind::Receive);
        let mut generated_initial_receive = None;
        if !has_receive {
            let address = wallet.checkout_receive_address_with_label(None);
            self.current_receive_address = Some(address.clone());
            generated_initial_receive = Some(address);
        }

        let address_records = wallet.address_book.snapshot();
        self.receive_addresses = address_records
            .iter()
            .filter(|record| record.path.kind == AddressKind::Receive)
            .take(RECEIVE_ADDRESS_LIST_LIMIT)
            .map(|record| wallet.address_for_path(record.path))
            .collect();
        if self.current_receive_address.is_none() {
            self.current_receive_address = address_records
                .into_iter()
                .rev()
                .find(|record| record.path.kind == AddressKind::Receive)
                .map(|record| wallet.address_for_path(record.path))
                .or_else(|| self.receive_addresses.first().cloned());
        }

        self.wallet_management_form.backup_path = backup_wallet_path(&wallet_path);
        self.wallet_management_form.backup_json_path = backup_wallet_json_path(&wallet_path);
        self.wallet_management_form.backup_text_path = backup_wallet_text_path(&wallet_path);
        self.wallet_management_form.backup_phrase_qr_path =
            backup_wallet_phrase_qr_path(&wallet_path);
        self.wallet_management_form.backup_password.clear();
        self.wallet_management_form.backup_password_confirm.clear();
        self.wallet_path = Some(wallet_path);
        self.wallet = Some(wallet);
        if self.wallet_session_password.is_none() {
            self.wallet_session_password = Some(String::new());
        }
        if let Err(err) = self.persist_startup_wallet_metadata() {
            self.last_error = Some(format!(
                "Wallet loaded, but startup metadata save failed: {err}"
            ));
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!("persist startup wallet metadata failed error={err}"),
            );
        }
        self.wallet_discovery_scan_limit = WALLET_DISCOVERY_SCAN_STEPS[0];
        self.sync_wallet_recovery_window_form();
        self.wallet_readiness_gate_active =
            self.connection.has_local_node() && self.wallet_scan_rpc_ready();
        self.refresh_wallet_address_views();
        self.wallet_cache_dirty = true;
        self.last_wallet_refresh_at = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        self.sync_wallet_state();
        if let Some(address) = generated_initial_receive {
            self.append_generated_address(address);
            if let Err(err) = self.persist_loaded_wallet_state() {
                self.last_error = Some(format!(
                    "Wallet loaded, but initial receive address save failed: {err}"
                ));
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("persist initial receive address failed error={err}"),
                );
            }
        }
        if self.wallet_scan_rpc_ready() {
            self.start_wallet_scan_job();
        }
        self.active_tab = NavTab::Overview;
        self.launch_page = LaunchPage::Welcome;
    }

    fn sync_wallet_state(&mut self) {
        if let Some(wallet) = &self.wallet {
            self.ui_state.wallet_snapshot = wallet.snapshot.clone();
            self.view_model.ui_state.wallet_snapshot = wallet.snapshot.clone();
        }
    }

    fn poll_mining_job(&mut self) {
        let Some(job) = self.mining_job.take() else {
            return;
        };

        match job.receiver.try_recv() {
            Ok(MiningJobResult::Completed(outcome)) => {
                self.last_mined_height = Some(outcome.height);
                self.last_mined_block_hash = Some(outcome.block_hash);
                self.last_mined_at_unix = Some(current_unix_seconds());
                let mut backend_parts = vec![format!("backend={}", outcome.backend_used)];
                if let Some(accelerator) = outcome.accelerator_label.as_deref() {
                    backend_parts.push(format!("device={accelerator}"));
                }
                if let Some(reason) = outcome.fallback_reason.as_deref() {
                    backend_parts.push(format!("fallback={reason}"));
                }
                let backend_note = backend_parts.join(" ");
                self.mining_status = format!(
                    "{} at height {} [{}]",
                    outcome.message, outcome.height, backend_note
                );
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!(
                        "mining outcome accepted={} height={} hash={} backend={} accelerator={} fallback={}",
                        outcome.accepted,
                        outcome.height,
                        hex::encode(outcome.block_hash),
                        outcome.backend_used,
                        outcome.accelerator_label.as_deref().unwrap_or("none"),
                        outcome.fallback_reason.as_deref().unwrap_or("none")
                    ),
                );
                self.last_error = None;
                if outcome.accepted {
                    if self.ui_state.rotate_coinbase_address {
                        let next_coinbase_address = self.wallet_mut().map(|wallet| {
                            let address = wallet.checkout_receive_address_with_label(None);
                            let snapshot = wallet.snapshot.clone();
                            (address, snapshot)
                        });
                        if let Some((address, snapshot)) = next_coinbase_address {
                            self.append_generated_address(address.clone());
                            self.current_receive_address = Some(address);
                            self.ui_state.wallet_snapshot = snapshot.clone();
                            self.view_model.ui_state.wallet_snapshot = snapshot;
                            if let Err(err) = self.persist_loaded_wallet_state() {
                                self.last_error = Some(format!(
                                    "Coinbase rotation succeeded, but wallet save failed: {err}"
                                ));
                                let _ = atho_node::dev::append_log(
                                    "atho-qt",
                                    &format!("persist rotated coinbase address failed error={err}"),
                                );
                            }
                        }
                    }
                    self.wallet_cache_dirty = true;
                }
                if self.ui_state.generate_coins {
                    self.start_mining_job();
                }
            }
            Ok(MiningJobResult::StaleTemplate(stale)) => {
                let current_height = stale
                    .current_height
                    .map(|height| height.to_string())
                    .unwrap_or_else(|| String::from("unknown"));
                self.mining_status = format!(
                    "Mining template for height {} went stale; refreshing from height {}",
                    stale.height, current_height
                );
                self.last_error = None;
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!(
                        "mining template stale height={} prev={} current_height={} current_tip={} solved_hash={}",
                        stale.height,
                        hex::encode(stale.previous_block_hash),
                        current_height,
                        stale
                            .current_tip_hash
                            .map(hex::encode)
                            .unwrap_or_else(|| String::from("unknown")),
                        stale
                            .solved_block_hash
                            .map(hex::encode)
                            .unwrap_or_else(|| String::from("none")),
                    ),
                );
                self.pending_mining_restart = None;
                if !job.stop_requested.load(Ordering::Acquire) {
                    self.start_mining_job();
                }
            }
            Ok(MiningJobResult::Cancelled) => {
                let _ = atho_node::dev::append_log("atho-qt", "mining worker cancelled");
                self.last_error = None;
                if let Some(cores) = self.pending_mining_restart.take() {
                    self.ui_state.mining_cores = self.clamp_mining_cores(cores);
                    if self.ui_state.generate_coins
                        && self.ui_state.connected
                        && self.wallet.is_some()
                    {
                        self.mining_status = format!(
                            "Restarting miner with {} thread(s)",
                            self.ui_state.mining_cores
                        );
                        self.start_mining_job();
                    } else {
                        self.mining_status = String::from("Idle");
                    }
                } else {
                    self.mining_status = String::from("Idle");
                }
            }
            Ok(MiningJobResult::Failed(err)) => {
                self.mining_status = format!("Mining failed: {err}");
                self.last_error = Some(err.clone());
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("mining worker failed error={err}"),
                );
                self.pending_mining_restart = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.mining_job = Some(job);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.mining_status = String::from("Mining worker disconnected");
                self.last_error = Some(String::from("mining worker disconnected"));
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    "mining worker disconnected error=channel closed",
                );
                self.pending_mining_restart = None;
            }
        }

        if self.mining_job.is_none() {
            let elapsed = job.started_at.elapsed();
            self.mining_status = format!("{} ({}s)", self.mining_status, elapsed.as_secs());
        }
    }

    fn poll_send_job(&mut self) {
        let Some(mut job) = self.send_job.take() else {
            return;
        };

        let mut keep_job = true;
        let mut disconnected = false;
        while keep_job {
            match job.receiver.try_recv() {
                Ok(SendJobEvent::Progress { stage }) => {
                    job.stage = stage;
                    self.send_status = stage.label().to_string();
                }
                Ok(SendJobEvent::Finished(Ok(outcome))) => {
                    self.send_fee = self.format_fee_amount(outcome.fee_atoms);
                    self.send_status = format!(
                        "Accepted locally {} (relay pending with synced peers; confirm after mining, send proof {} bits @ nonce {})",
                        hex::encode(outcome.txid),
                        outcome.tx_pow_bits,
                        outcome.tx_pow_nonce
                    );
                    self.last_error = None;
                    self.wallet_cache_dirty = true;
                    let submitted_address = self.send_to.clone();
                    self.mark_recipient_address_book_entry_used(&submitted_address);
                    keep_job = false;
                    let _ = atho_node::dev::append_log(
                        "atho-qt",
                        &format!(
                            "send outcome fee_atoms={} tx_pow_bits={} tx_pow_nonce={}",
                            outcome.fee_atoms, outcome.tx_pow_bits, outcome.tx_pow_nonce
                        ),
                    );
                }
                Ok(SendJobEvent::Finished(Err(err))) => {
                    self.send_status = format!("Submission failed: {err}");
                    self.last_error = Some(err);
                    keep_job = false;
                    let _ = atho_node::dev::append_log(
                        "atho-qt",
                        &format!(
                            "send submission failed error={}",
                            self.last_error.as_deref().unwrap_or("unknown")
                        ),
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.send_job = Some(job);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    keep_job = false;
                }
            }
        }

        if disconnected {
            self.send_status = String::from("Submission worker disconnected");
            self.last_error = Some(String::from("submission worker disconnected"));
            let _ = atho_node::dev::append_log("atho-qt", "send worker disconnected");
        } else {
            let elapsed = job.started_at.elapsed();
            self.send_status = format!("{} ({}s)", self.send_status, elapsed.as_secs());
        }
    }

    fn poll_storage_recovery_notice(&mut self) {
        if self.storage_recovery_notice.is_some() {
            return;
        }
        let notice_path = atho_storage::path::storage_recovery_notice_path(self.active_network());
        let Ok(notice) = fs::read_to_string(&notice_path) else {
            return;
        };
        let trimmed = notice.trim();
        if trimmed.is_empty() {
            let _ = fs::remove_file(notice_path);
            return;
        }
        self.storage_recovery_notice = Some(trimmed.to_owned());
        self.show_storage_recovery_notice_dialog = true;
        let _ = fs::remove_file(notice_path);
    }

    pub(crate) fn dismiss_storage_recovery_notice(&mut self) {
        self.show_storage_recovery_notice_dialog = false;
        self.storage_recovery_notice = None;
    }

    fn start_mining_job(&mut self) {
        if self.mining_job.is_some() {
            self.mining_status = String::from("Mining already running");
            return;
        }
        if let Some(reason) = self.wallet_mining_block_reason() {
            self.ui_state.generate_coins = false;
            self.mining_status = reason.clone();
            self.last_error = Some(reason);
            return;
        }

        let rpc_address = self.connection.rpc_address().to_string();
        let cores = self.clamp_mining_cores(self.ui_state.mining_cores);
        self.ui_state.mining_cores = cores;
        self.refresh_mining_accelerator_info();
        let requested_backend = self.mining_backend_status_hint();
        let mining_backend = self.ui_state.mining_backend;
        let connection = self.connection.clone();
        let reward_script = self.mining_reward_script();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let mining_stop_requested = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let stop_for_thread = Arc::clone(&stop_requested);
        let mining_stop_for_thread = Arc::clone(&mining_stop_requested);
        self.mining_status = format!(
            "Starting generation with {} thread(s) [{}]",
            cores, requested_backend
        );
        self.last_error = None;
        self.pending_mining_restart = None;
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "starting mining job rpc={} cores={} max_cores={} backend_hint={}",
                rpc_address,
                cores,
                self.max_mining_cores(),
                requested_backend
            ),
        );

        std::thread::spawn(move || {
            let result = mine_via_connection(
                connection,
                cores,
                mining_backend,
                reward_script,
                stop_for_thread,
                mining_stop_for_thread,
            );
            let _ = sender.send(result);
        });

        self.mining_job = Some(MiningJob {
            started_at: Instant::now(),
            stop_requested,
            mining_stop_requested,
            receiver,
        });
    }

    fn mining_reward_script(&mut self) -> Option<Vec<u8>> {
        if let Some(address) = self.current_receive_address.as_ref() {
            return Some(address.payment_digest.to_vec());
        }

        let fallback = if let Some(wallet) = self.wallet_ref() {
            wallet
                .address_book
                .snapshot()
                .into_iter()
                .rev()
                .find(|record| record.path.kind == AddressKind::Receive)
                .map(|record| wallet.address_for_path(record.path))
        } else {
            None
        };

        if let Some(address) = fallback {
            self.current_receive_address = Some(address.clone());
            return Some(address.payment_digest.to_vec());
        }

        let (address, snapshot) = {
            let wallet = self.wallet_mut()?;
            let address = wallet.checkout_receive_address_with_label(None);
            let snapshot = wallet.snapshot.clone();
            (address, snapshot)
        };
        self.current_receive_address = Some(address.clone());
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        self.append_generated_address(address.clone());
        if let Err(err) = self.persist_loaded_wallet_state() {
            self.last_error = Some(format!(
                "Generated mining reward address, but wallet save failed: {err}"
            ));
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!("persist mining reward address failed error={err}"),
            );
        }
        Some(address.payment_digest.to_vec())
    }

    fn stop_mining_job(&mut self) {
        self.pending_mining_restart = None;
        self.ui_state.generate_coins = false;
        self.last_error = None;
        if let Some(job) = &self.mining_job {
            job.stop_requested.store(true, Ordering::Release);
            job.mining_stop_requested.store(true, Ordering::Release);
            self.mining_status = String::from("Stopping miner");
            let _ = atho_node::dev::append_log("atho-qt", "stop miner requested");
        } else {
            self.mining_status = String::from("Idle");
        }
    }

    fn restart_mining_job(&mut self) {
        let cores = self.clamp_mining_cores(self.ui_state.mining_cores);
        self.ui_state.mining_cores = cores;
        self.last_error = None;
        self.refresh_mining_accelerator_info();
        let requested_backend = self.mining_backend_status_hint();
        if self.mining_job.is_some() {
            self.pending_mining_restart = Some(cores);
            if let Some(job) = &self.mining_job {
                job.stop_requested.store(true, Ordering::Release);
                job.mining_stop_requested.store(true, Ordering::Release);
            }
            self.mining_status = format!(
                "Restarting miner with {} thread(s) [{}]",
                cores, requested_backend
            );
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!("restart miner requested cores={cores} backend_hint={requested_backend}"),
            );
            return;
        }
        if self.ui_state.generate_coins {
            self.start_mining_job();
        }
    }

    fn refresh_mining_accelerator_info(&mut self) {
        if matches!(self.ui_state.mining_backend, MiningBackendKind::Cpu) {
            self.mining_accelerator_info = MiningAcceleratorInfo {
                backend: String::from("cpu"),
                device_type: MiningDeviceType::Cpu,
                device_name: Some(String::from("CPU threads")),
                vendor: None,
                driver: None,
                compute_units: None,
                global_mem_mb: None,
                local_mem_kb: None,
                clock_mhz: None,
                kernel_path: None,
                supports_fixed: false,
                supports_template: false,
                max_batch: None,
                template_max_bytes: None,
                usable: false,
                reason_code: None,
                reason_if_not: None,
            };
            return;
        }
        let cores = self.ui_state.mining_cores.max(1);
        self.mining_accelerator_info =
            MiningController::new(self.ui_state.mining_backend, cores).gpu_probe_info();
    }

    fn mining_backend_status_hint(&self) -> String {
        let requested = self.ui_state.mining_backend.label();
        if matches!(self.ui_state.mining_backend, MiningBackendKind::Cpu) {
            return format!("requested backend={requested}");
        }
        if self.mining_accelerator_info.usable {
            return format!(
                "requested backend={} target gpu={}",
                requested,
                self.mining_accelerator_info
                    .runtime_label()
                    .unwrap_or_else(|| String::from("OpenCL GPU"))
            );
        }
        format!(
            "requested backend={} gpu unavailable={}",
            requested,
            self.mining_accelerator_info
                .reason_if_not
                .as_deref()
                .unwrap_or("unknown reason")
        )
    }

    fn generate_create_mnemonic(&mut self) -> Result<(), String> {
        let mnemonic_length = MnemonicLength::from_word_count(self.create_form.mnemonic_word_count)
            .ok_or_else(|| String::from("unsupported mnemonic word count"))?;
        let mut entropy = vec![0u8; mnemonic_length.entropy_bytes()];
        getrandom(&mut entropy).map_err(|_| String::from("failed to gather wallet entropy"))?;
        let mnemonic = MnemonicPhrase::from_entropy(&entropy, mnemonic_length)
            .map_err(|err| err.to_string())?;
        self.create_form.mnemonic_words = mnemonic_ui::words_from_sentence(&mnemonic.as_sentence());
        self.create_form.acknowledged_backup = false;
        Ok(())
    }

    #[allow(dead_code)]
    fn make_wallet_from_mnemonic(&self, mnemonic: MnemonicPhrase, passphrase: &str) -> Wallet {
        Wallet::from_mnemonic(mnemonic, passphrase, self.connection.network())
    }

    fn start_wallet_from_mnemonic_preparation(
        &mut self,
        request: MnemonicWalletPreparationRequest,
    ) {
        let MnemonicWalletPreparationRequest {
            mnemonic_text,
            mnemonic_passphrase,
            wallet_path,
            wallet_password,
            wallet_name,
            wallet_word_count,
            stage,
        } = request;
        let network = self.connection.network();
        let wallet_path_for_job = normalize_wallet_path_input(&wallet_path)
            .to_string_lossy()
            .into_owned();
        let wallet_password_for_job = wallet_password.clone();
        let requested_name = wallet_name.trim().to_owned();
        let requested_word_count = wallet_word_count;
        self.start_wallet_preparation_job(stage, move |sender| {
            let mnemonic = MnemonicPhrase::parse(&mnemonic_text).map_err(|err| err.to_string())?;
            let now = current_unix_seconds();
            let progress_sender = sender.clone();
            let wallet = Wallet::from_mnemonic_with_progress(
                mnemonic,
                &mnemonic_passphrase,
                network,
                move |completed, total| {
                    let _ = progress_sender.send(WalletPreparationEvent::Progress {
                        stage: String::from("Preparing keypool"),
                        completed,
                        total,
                    });
                },
            );
            let _ = sender.send(WalletPreparationEvent::Progress {
                stage: String::from("Saving wallet"),
                completed: 0,
                total: 1,
            });
            DesktopApp::save_wallet_to_path(
                &wallet,
                &wallet_path_for_job,
                &wallet_password_for_job,
            )?;
            let _ = sender.send(WalletPreparationEvent::Progress {
                stage: String::from("Saving wallet"),
                completed: 1,
                total: 1,
            });
            Ok(WalletPreparationOutcome {
                wallet,
                wallet_path: wallet_path_for_job.clone(),
                wallet_password: wallet_password_for_job,
                registry_entry: Some(WalletRegistryEntry {
                    wallet_id: wallet_registry_entry_id(&wallet_path_for_job, now),
                    wallet_name: requested_name.clone(),
                    wallet_path: wallet_path_for_job,
                    network: network.id().to_string(),
                    created_at_unix: now,
                    updated_at_unix: now,
                    last_opened_at_unix: Some(now),
                    word_count: requested_word_count,
                }),
            })
        });
    }

    fn start_open_wallet_preparation(&mut self, wallet_path: String, wallet_password: String) {
        let network = self.connection.network();
        let wallet_path_for_job = normalize_wallet_path_input(&wallet_path)
            .to_string_lossy()
            .into_owned();
        let wallet_password_for_job = wallet_password.clone();
        self.start_wallet_preparation_job("Loading wallet", move |sender| {
            let _ = sender.send(WalletPreparationEvent::Progress {
                stage: String::from("Reading wallet"),
                completed: 0,
                total: 0,
            });
            let progress_sender = sender.clone();
            let wallet_path_buf = PathBuf::from(&wallet_path_for_job);
            let wallet = Wallet::load_from_datafile_with_progress(
                wallet_path_buf.as_path(),
                &wallet_password_for_job,
                move |completed, total| {
                    let _ = progress_sender.send(WalletPreparationEvent::Progress {
                        stage: String::from("Preparing keypool"),
                        completed,
                        total,
                    });
                },
            )
            .map_err(|err| err.to_string())?;
            if wallet.network != network {
                return Err(format!(
                    "wallet belongs to {} not {}",
                    wallet.network.id(),
                    network.id()
                ));
            }
            let now = current_unix_seconds();
            let existing_entry = load_wallet_registry(network)
                .entries
                .into_iter()
                .find(|entry| entry.wallet_path == wallet_path_for_job);
            Ok(WalletPreparationOutcome {
                wallet,
                wallet_path: wallet_path_for_job.clone(),
                wallet_password: wallet_password_for_job,
                registry_entry: Some(existing_entry.unwrap_or_else(|| WalletRegistryEntry {
                    wallet_id: wallet_registry_entry_id(&wallet_path_for_job, now),
                    wallet_name: infer_wallet_name_from_path(&wallet_path_for_job),
                    wallet_path: wallet_path_for_job,
                    network: network.id().to_string(),
                    created_at_unix: now,
                    updated_at_unix: now,
                    last_opened_at_unix: Some(now),
                    word_count: 0,
                })),
            })
        });
    }

    fn save_wallet_to_path(
        wallet: &Wallet,
        wallet_path: &str,
        password: &str,
    ) -> Result<(), String> {
        let path = normalize_wallet_path_input(wallet_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        if cfg!(test) {
            wallet
                .save_to_datafile_with_iterations(&path, password, TEST_WALLET_DATAFILE_ITERATIONS)
                .map_err(|err| err.to_string())
        } else {
            wallet
                .save_to_datafile(&path, password)
                .map_err(|err| err.to_string())
        }
    }

    fn export_wallet_backup(&self, backup_path: &str, password: &str) -> Result<(), String> {
        let wallet = self
            .wallet_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        Self::save_wallet_to_path(wallet, backup_path, password)?;
        self.write_wallet_backup_metadata(backup_path)
    }

    fn export_wallet_recovery_json(&self, export_path: &str) -> Result<(), String> {
        let export = self.wallet_recovery_export()?;
        let path = PathBuf::from(export_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(&export).map_err(|err| err.to_string())?;
        fs::write(path, bytes).map_err(|err| err.to_string())
    }

    fn export_wallet_recovery_text(&self, export_path: &str) -> Result<(), String> {
        let export = self.wallet_recovery_export()?;
        let path = PathBuf::from(export_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let phrase = export
            .mnemonic_phrase
            .clone()
            .unwrap_or_else(|| String::from("Unavailable"));
        let current_receive_index = export
            .current_receive_index
            .map(|index| format!("R{index:04}"))
            .unwrap_or_else(|| String::from("Unavailable"));
        let next_receive_index = format!("R{:04}", export.next_receive_index);
        let next_change_index = format!("C{:04}", export.next_change_index);
        let highest_generated_receive_index = export
            .highest_generated_receive_index
            .map(|index| format!("R{index:04}"))
            .unwrap_or_else(|| String::from("Unavailable"));
        let highest_generated_change_index = export
            .highest_generated_change_index
            .map(|index| format!("C{index:04}"))
            .unwrap_or_else(|| String::from("Unavailable"));
        let highest_reserved_receive_index = export
            .highest_reserved_receive_index
            .map(|index| format!("R{index:04}"))
            .unwrap_or_else(|| String::from("Unavailable"));
        let highest_reserved_change_index = export
            .highest_reserved_change_index
            .map(|index| format!("C{index:04}"))
            .unwrap_or_else(|| String::from("Unavailable"));
        let text = format!(
            "Atho Wallet Recovery Export\n\
            ==========================\n\
            Wallet Name: {wallet_name}\n\
            Wallet ID: {wallet_id}\n\
            Network: {network}\n\
            Wallet Path: {wallet_path}\n\
            Exported At: {exported_at_unix}\n\
            Mnemonic Word Count: {mnemonic_word_count}\n\
            Recovery Phrase: {phrase}\n\
            Configured Recovery Window: {configured_recovery_window}\n\
            Active Scan Window: {active_scan_window}\n\
            Current Receive Index: {current_receive_index}\n\
            Next Receive Index: {next_receive_index}\n\
            Next Change Index: {next_change_index}\n\
            Highest Generated Receive Index: {highest_generated_receive_index}\n\
            Highest Generated Change Index: {highest_generated_change_index}\n\
            Highest Reserved Receive Index: {highest_reserved_receive_index}\n\
            Highest Reserved Change Index: {highest_reserved_change_index}\n",
            wallet_name = export.wallet_name,
            wallet_id = export
                .wallet_id
                .unwrap_or_else(|| String::from("Unavailable")),
            network = export.network,
            wallet_path = export.wallet_path,
            exported_at_unix = export.exported_at_unix,
            mnemonic_word_count = export.mnemonic_word_count,
            phrase = phrase,
            configured_recovery_window = export.configured_recovery_window,
            active_scan_window = export.active_scan_window,
            current_receive_index = current_receive_index,
            next_receive_index = next_receive_index,
            next_change_index = next_change_index,
            highest_generated_receive_index = highest_generated_receive_index,
            highest_generated_change_index = highest_generated_change_index,
            highest_reserved_receive_index = highest_reserved_receive_index,
            highest_reserved_change_index = highest_reserved_change_index,
        );
        fs::write(path, text).map_err(|err| err.to_string())
    }

    fn export_wallet_recovery_phrase_qr(&self, export_path: &str) -> Result<(), String> {
        let export = self.wallet_recovery_export()?;
        let phrase = export
            .mnemonic_phrase
            .clone()
            .ok_or_else(|| String::from("No recovery phrase is loaded for this wallet"))?;
        let subtitle = format!("Recovery phrase QR • {} words", export.mnemonic_word_count);
        let footer = format!("{} • keep offline", export.network);
        write_labeled_qr_png(
            export_path,
            &export.wallet_name,
            &subtitle,
            &footer,
            &phrase,
        )
    }

    fn export_receive_address_qr(&self, export_path: &str, address: &str) -> Result<(), String> {
        write_basic_qr_png(export_path, address, Rgba(QR_EXPORT_MODULE_BLACK))
    }

    fn suggested_receive_qr_export_path(
        &self,
        detail_address: &str,
        selected_request: Option<&ReceiveRequestRecord>,
    ) -> String {
        let wallet_path = self
            .wallet_path
            .as_deref()
            .unwrap_or(self.open_form.wallet_path.as_str());
        receive_address_qr_path(
            wallet_path,
            self.wallet_current_receive_index(),
            detail_address,
            selected_request,
        )
    }

    fn write_wallet_backup_metadata(&self, backup_path: &str) -> Result<(), String> {
        let wallet = self
            .wallet_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let (receive_keypool_queued, change_keypool_queued) = wallet.keypool_depths();
        let (highest_generated_receive_index, highest_generated_change_index) =
            wallet.highest_generated_indices();
        let (highest_reserved_receive_index, highest_reserved_change_index) =
            wallet.highest_reserved_indices();
        let (next_receive_index, next_change_index) = wallet.next_indices();
        let metadata = WalletBackupMetadata {
            network: wallet.network.id().to_string(),
            exported_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            configured_recovery_window: wallet.restore_gap_limit(),
            active_scan_window: self.wallet_discovery_scan_limit,
            receive_keypool_queued,
            change_keypool_queued,
            highest_generated_receive_index,
            highest_generated_change_index,
            highest_reserved_receive_index,
            highest_reserved_change_index,
            next_receive_index,
            next_change_index,
        };
        let normalized_backup_path = normalize_wallet_path_input(backup_path);
        let metadata_path = PathBuf::from(format!(
            "{}.meta.json",
            normalized_backup_path.to_string_lossy()
        ));
        let metadata_bytes = serde_json::to_vec_pretty(&metadata).map_err(|err| err.to_string())?;
        fs::write(metadata_path, metadata_bytes).map_err(|err| err.to_string())
    }

    fn persist_startup_wallet_metadata(&self) -> Result<(), String> {
        let wallet_path = self
            .wallet_path
            .as_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let metadata = WalletStartupMetadata {
            wallet_path: wallet_path.clone(),
            recorded_at_unix: current_unix_seconds(),
        };
        let metadata_path = startup_wallet_metadata_path(self.connection.network());
        if let Some(parent) = metadata_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(&metadata).map_err(|err| err.to_string())?;
        fs::write(metadata_path, bytes).map_err(|err| err.to_string())
    }

    fn persist_client_display_preferences(&self) -> Result<(), String> {
        let preferences_path = client_display_preferences_path(self.connection.network());
        if let Some(parent) = preferences_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let bytes =
            serde_json::to_vec_pretty(&self.display_preferences).map_err(|err| err.to_string())?;
        fs::write(preferences_path, bytes).map_err(|err| err.to_string())
    }

    fn persist_recipient_address_book(&self) -> Result<(), String> {
        let address_book_path = recipient_address_book_path(self.connection.network());
        if let Some(parent) = address_book_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(&self.recipient_address_book)
            .map_err(|err| err.to_string())?;
        fs::write(address_book_path, bytes).map_err(|err| err.to_string())
    }

    pub(crate) fn open_recipient_address_book(&mut self) {
        self.recipient_address_book_open = true;
        self.recipient_address_editor_open = false;
        self.recipient_address_book_filter.clear();
    }

    pub(crate) fn start_add_current_recipient_to_address_book(&mut self) {
        self.recipient_address_book_open = true;
        self.recipient_address_editor_open = true;
        self.recipient_address_editor_id = None;
        self.recipient_address_editor_label = self.send_label.trim().to_owned();
        self.recipient_address_editor_address = self.send_to.trim().to_owned();
        self.recipient_address_editor_notes.clear();
    }

    pub(crate) fn edit_recipient_address_book_entry(&mut self, id: &str) {
        let Some(entry) = self
            .recipient_address_book
            .iter()
            .find(|entry| entry.id == id)
        else {
            self.last_error = Some(String::from("Recipient address book entry not found"));
            return;
        };
        self.recipient_address_book_open = true;
        self.recipient_address_editor_open = true;
        self.recipient_address_editor_id = Some(entry.id.clone());
        self.recipient_address_editor_label = entry.label.clone();
        self.recipient_address_editor_address = entry.address.clone();
        self.recipient_address_editor_notes = entry.notes.clone();
    }

    pub(crate) fn save_recipient_address_book_entry(&mut self) -> Result<(), String> {
        let label = self.recipient_address_editor_label.trim();
        let address = self.recipient_address_editor_address.trim();
        let notes = self.recipient_address_editor_notes.trim();
        if label.is_empty() {
            return Err(String::from("Enter a recipient label"));
        }
        if address.is_empty() {
            return Err(String::from("Enter a recipient address"));
        }
        let (_, network) = decode_base56_address(address).map_err(|err| err.to_string())?;
        if network != self.connection.network() {
            return Err(format!("Address belongs to {}", network.id()));
        }
        let now = current_unix_seconds();
        if self.recipient_address_book.iter().any(|entry| {
            entry.address.eq_ignore_ascii_case(address)
                && match self.recipient_address_editor_id.as_ref() {
                    Some(current_id) => current_id != &entry.id,
                    None => true,
                }
        }) {
            return Err(String::from(
                "That address is already saved in the recipient address book",
            ));
        }
        if let Some(existing_id) = self.recipient_address_editor_id.as_ref() {
            if let Some(entry) = self
                .recipient_address_book
                .iter_mut()
                .find(|entry| &entry.id == existing_id)
            {
                entry.label = label.to_owned();
                entry.address = address.to_owned();
                entry.notes = notes.to_owned();
                entry.updated_at_unix = now;
            } else {
                return Err(String::from("Recipient address book entry not found"));
            }
        } else {
            self.recipient_address_book.push(RecipientAddressEntry {
                id: recipient_address_entry_id(address, now),
                label: label.to_owned(),
                address: address.to_owned(),
                notes: notes.to_owned(),
                created_at_unix: now,
                updated_at_unix: now,
                last_used_at_unix: None,
            });
        }
        self.recipient_address_book.sort_by(|left, right| {
            left.label
                .to_ascii_lowercase()
                .cmp(&right.label.to_ascii_lowercase())
                .then(left.address.cmp(&right.address))
        });
        self.persist_recipient_address_book()?;
        self.recipient_address_editor_open = false;
        self.recipient_address_editor_id = None;
        self.recipient_address_editor_label.clear();
        self.recipient_address_editor_address.clear();
        self.recipient_address_editor_notes.clear();
        self.last_error = None;
        self.send_status = String::from("Recipient saved to address book");
        Ok(())
    }

    pub(crate) fn delete_recipient_address_book_entry(&mut self, id: &str) -> Result<(), String> {
        let original_len = self.recipient_address_book.len();
        self.recipient_address_book.retain(|entry| entry.id != id);
        if self.recipient_address_book.len() == original_len {
            return Err(String::from("Recipient address book entry not found"));
        }
        self.persist_recipient_address_book()?;
        self.last_error = None;
        self.send_status = String::from("Recipient removed from address book");
        Ok(())
    }

    pub(crate) fn select_recipient_address_book_entry(&mut self, id: &str) -> Result<(), String> {
        let now = current_unix_seconds();
        let Some(index) = self
            .recipient_address_book
            .iter()
            .position(|entry| entry.id == id)
        else {
            return Err(String::from("Recipient address book entry not found"));
        };
        let (address, label) = {
            let entry = &mut self.recipient_address_book[index];
            entry.last_used_at_unix = Some(now);
            entry.updated_at_unix = now;
            (entry.address.clone(), entry.label.clone())
        };
        self.send_to = address;
        if !label.trim().is_empty() {
            self.send_label = label.clone();
        }
        self.persist_recipient_address_book()?;
        self.recipient_address_book_open = false;
        self.recipient_address_editor_open = false;
        self.last_error = None;
        self.send_status = format!("Loaded recipient {label}");
        Ok(())
    }

    fn mark_recipient_address_book_entry_used(&mut self, address: &str) {
        let normalized = address.trim();
        let Some(index) = self
            .recipient_address_book
            .iter()
            .position(|entry| entry.address == normalized)
        else {
            return;
        };
        let now = current_unix_seconds();
        {
            let entry = &mut self.recipient_address_book[index];
            entry.last_used_at_unix = Some(now);
            entry.updated_at_unix = now;
        }
        let _ = self.persist_recipient_address_book();
    }

    fn change_wallet_passphrase(&mut self, password: &str) -> Result<(), String> {
        let wallet_path = self
            .wallet_path
            .as_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let wallet = self
            .wallet_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        Self::save_wallet_to_path(wallet, wallet_path, password)?;
        self.wallet_session_password = Some(password.to_owned());
        Ok(())
    }

    pub(crate) fn wallet_mnemonic_sentence(&self) -> Option<String> {
        self.wallet_ref().and_then(Wallet::mnemonic_sentence)
    }

    fn try_open_existing_wallet_on_startup(&mut self) {
        let Some(path) = startup_wallet_path(self.connection.network()) else {
            self.launch_page = LaunchPage::Welcome;
            return;
        };

        let wallet_path = path.to_string_lossy().into_owned();
        self.open_form.wallet_path = wallet_path.clone();
        self.open_form.wallet_password.clear();

        // Never synchronously decrypt or hydrate a wallet in the constructor. Startup must remain
        // responsive and all real wallet preparation must flow through the same gated background
        // worker used by explicit open/create/import actions.
        match Wallet::inspect_datafile(path.as_path()) {
            Ok(metadata) if metadata.network == self.connection.network() => {
                match metadata.encryption_mode {
                    WalletEncryptionMode::Plaintext => {
                        self.start_open_wallet_preparation(wallet_path, String::new());
                    }
                    WalletEncryptionMode::PasswordAes256Gcm => {
                        self.launch_page = LaunchPage::OpenWallet;
                    }
                }
            }
            _ => {
                self.launch_page = LaunchPage::OpenWallet;
                self.last_error = None;
            }
        }
    }

    fn load_or_create_wallet(
        &mut self,
        wallet: Wallet,
        wallet_path: String,
        wallet_password: String,
        registry_entry: Option<WalletRegistryEntry>,
    ) {
        self.clear_wallet_state();
        self.wallet_session_password = Some(wallet_password);
        let fallback_entry = registry_entry.unwrap_or_else(|| {
            let now = current_unix_seconds();
            WalletRegistryEntry {
                wallet_id: wallet_registry_entry_id(&wallet_path, now),
                wallet_name: infer_wallet_name_from_path(&wallet_path),
                wallet_path: wallet_path.clone(),
                network: self.connection.network().id().to_string(),
                created_at_unix: now,
                updated_at_unix: now,
                last_opened_at_unix: Some(now),
                word_count: wallet
                    .mnemonic_phrase()
                    .map(MnemonicPhrase::word_count)
                    .unwrap_or_default(),
            }
        });
        self.current_wallet_id = Some(fallback_entry.wallet_id.clone());
        self.current_wallet_name = Some(fallback_entry.wallet_name.clone());
        self.attach_wallet(wallet, wallet_path);
        if let Err(err) = self.upsert_wallet_registry_entry(fallback_entry, true) {
            self.last_error = Some(format!(
                "Wallet loaded, but wallet registry save failed: {err}"
            ));
        }
        self.send_status = String::from("Wallet loaded");
    }

    fn persist_loaded_wallet_state(&mut self) -> Result<(), String> {
        let wallet_path = self
            .wallet_path
            .clone()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let password = self.wallet_session_password.clone().unwrap_or_default();
        let wallet = self
            .wallet_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        Self::save_wallet_to_path(wallet, &wallet_path, &password)?;
        self.persist_startup_wallet_metadata()?;
        Ok(())
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
        self.append_generated_address(address.clone());
        self.current_receive_address = Some(address);
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        match self.persist_loaded_wallet_state() {
            Ok(()) => {
                self.send_status = String::from("Receive address generated");
                self.last_error = None;
            }
            Err(err) => {
                self.send_status =
                    format!("Receive address generated, but wallet save failed: {err}");
                self.last_error = Some(err.clone());
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("persist receive address failed error={err}"),
                );
            }
        }
        self.receive_label.clear();
    }

    fn create_receive_request(&mut self) {
        if self.wallet.is_none() {
            self.last_error = Some(String::from("Load or create a wallet first"));
            return;
        }

        let requested_amount = if self.receive_amount.trim().is_empty() {
            None
        } else {
            match self.receive_amount.trim().parse::<u64>() {
                Ok(value) if value > 0 => Some(value),
                _ => {
                    self.last_error = Some(String::from(
                        "Requested amount must be an integer atom value",
                    ));
                    return;
                }
            }
        };

        let label = self.receive_label.trim().to_owned();
        let message = self.receive_message.trim().to_owned();
        self.generate_receive_address();
        let address = self.current_receive_address_text();
        if address.is_empty() {
            return;
        }
        let sequence = self
            .requested_payments
            .iter()
            .map(|request| request.sequence)
            .max()
            .unwrap_or(0)
            + 1;
        self.requested_payments.push(ReceiveRequestRecord {
            sequence,
            label,
            message,
            amount_atoms: requested_amount,
            address,
        });
        self.selected_receive_request = Some(sequence);
        self.receive_amount.clear();
        self.receive_message.clear();
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
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        self.append_generated_address(address);
        match self.persist_loaded_wallet_state() {
            Ok(()) => {
                self.send_status = String::from("Change address generated");
                self.last_error = None;
            }
            Err(err) => {
                self.send_status =
                    format!("Change address generated, but wallet save failed: {err}");
                self.last_error = Some(err.clone());
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("persist change address failed error={err}"),
                );
            }
        }
    }

    fn wallet_send_block_reason(&self) -> Option<String> {
        if self.wallet.is_none() {
            return Some(String::from("Load or create a wallet first"));
        }
        if !self.ui_state.connected || !self.view_model.running {
            return Some(String::from(
                "Cannot send while the local node is disconnected or still starting",
            ));
        }
        if matches!(self.active_network(), Network::Mainnet | Network::Testnet)
            && self.view_model.peer_count == 0
        {
            return Some(String::from(
                "Cannot send until Atho connects to at least one network peer",
            ));
        }
        if !self.view_model.chain_synced() {
            return Some(String::from(
                "Cannot send while Atho is still synchronizing to the network tip. Wallet balances and history may still change until sync completes.",
            ));
        }
        if self.wallet_readiness_gate_active {
            return Some(String::from(
                "Cannot send until the wallet finishes its initial chain scan",
            ));
        }
        None
    }

    fn wallet_mining_block_reason(&self) -> Option<String> {
        if self.wallet.is_none() {
            return Some(String::from("Load or create a wallet first"));
        }
        if !self.ui_state.connected || !self.view_model.running {
            return Some(String::from(
                "Cannot mine while the local node is disconnected or still starting",
            ));
        }
        if matches!(self.active_network(), Network::Mainnet | Network::Testnet)
            && self.view_model.peer_count == 0
        {
            return Some(String::from(
                "Cannot mine until Atho connects to at least one network peer",
            ));
        }
        if !self.view_model.chain_synced() {
            return Some(String::from(
                "Cannot mine while Atho is still synchronizing to the network tip. Mining stays disabled until sync completes.",
            ));
        }
        if self.wallet_readiness_gate_active {
            return Some(String::from(
                "Cannot mine until the wallet finishes its initial chain scan",
            ));
        }
        None
    }

    fn submit_send_transaction(&mut self) -> Result<(), String> {
        if self.send_job.is_some() {
            return Err(String::from("A send submission is already in progress"));
        }
        if let Some(reason) = self.wallet_send_block_reason() {
            return Err(reason);
        }

        let destination = self.send_to.trim();
        if destination.is_empty() {
            return Err(String::from("Enter a destination address"));
        }
        let amount = Self::parse_send_amount_atoms(self.send_input_unit(), &self.send_amount)?;
        if amount == 0 {
            return Err(String::from("Amount must be greater than zero"));
        }
        if !self.send_include_fee_in_total
            && amount < Self::wallet_min_output_amount_atoms(self.active_network())
        {
            return Err(Self::dust_amount_error_message(self.active_network()));
        }

        let (recipient_digest, network) =
            decode_base56_address(destination).map_err(|err| err.to_string())?;
        if network != self.connection.network() {
            return Err(format!("Address belongs to {}", network.id()));
        }

        let reserved_inputs = self.mempool_reserved_inputs();
        if self.wallet_address_index_cache.is_empty() && self.wallet_scan_job.is_none() {
            self.wallet_cache_dirty = true;
            self.start_wallet_scan_job();
        }
        let spendable_inputs = self.spendable_wallet_inputs(&reserved_inputs)?;

        if spendable_inputs.is_empty() {
            return Err(String::from(
                "No spendable wallet UTXOs available; refresh to clear mempool-locked outputs",
            ));
        }

        let selected_plan = Self::select_wallet_utxos(
            spendable_inputs.clone(),
            self.active_network(),
            amount,
            self.send_include_fee_in_total,
        )?
        .ok_or_else(|| self.unspendable_send_amount_message(amount, &spendable_inputs))?;

        let total_input_atoms = selected_plan.total_input_atoms;
        let output_count = selected_plan.output_count;
        let tx_lock_time = Self::transaction_lock_time_nonce();
        let change_address = if output_count == 2 {
            let (address, snapshot) = {
                let wallet = self
                    .wallet_mut()
                    .ok_or_else(|| String::from("Load or create a wallet first"))?;
                let address = wallet.checkout_change_address_with_label(None);
                let snapshot = wallet.snapshot.clone();
                (address, snapshot)
            };
            self.ui_state.wallet_snapshot = snapshot.clone();
            self.view_model.ui_state.wallet_snapshot = snapshot;
            self.append_generated_address(address.clone());
            self.persist_loaded_wallet_state()?;
            Some(address)
        } else {
            None
        };
        let spend_request = WalletSpendRequest {
            selected_utxos: selected_plan
                .utxos
                .iter()
                .map(|utxo| WalletSpendUtxo {
                    previous_txid: utxo.txid,
                    output_index: utxo.output_index,
                    value_atoms: utxo.value_atoms,
                    locking_script: utxo.locking_script.clone(),
                })
                .collect(),
            recipient_digest,
            amount_atoms: amount,
            include_fee_in_total: self.send_include_fee_in_total,
            transaction_version: selected_plan.transaction_version,
            lock_time: tx_lock_time,
            change_address: change_address.clone(),
        };
        let (sender, receiver) = mpsc::channel();
        let rpc_address = self.connection.rpc_address().to_string();
        let use_local_node = self.connection.has_local_node();
        let connection = self.connection.clone();
        let wallet = self
            .wallet_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?
            .clone();
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "submitting transaction rpc={} amount_atoms={} estimated_fee_atoms={} estimated_raw_bytes={} estimated_vbytes={} include_fee_total={} inputs={} outputs={}",
                rpc_address,
                amount,
                selected_plan.estimated_fee_atoms,
                selected_plan.estimated_raw_size_bytes,
                selected_plan.estimated_vsize_bytes,
                self.send_include_fee_in_total,
                selected_plan.utxos.len(),
                selected_plan.output_count
            ),
        );
        std::thread::spawn(move || {
            let _ = sender.send(SendJobEvent::Progress {
                stage: SendProgressStage::Preparing,
            });
            let built_spend =
                wallet.build_signed_payment_transaction_with_progress(spend_request, |stage| {
                    let mapped = match stage {
                        WalletSpendProgressStage::Preparing => SendProgressStage::Preparing,
                        WalletSpendProgressStage::Signing => SendProgressStage::Signing,
                        WalletSpendProgressStage::FinalizingProof => {
                            SendProgressStage::FinalizingProof
                        }
                    };
                    let _ = sender.send(SendJobEvent::Progress { stage: mapped });
                });
            let built_spend = match built_spend {
                Ok(spend) => spend,
                Err(err) => {
                    let _ = sender.send(SendJobEvent::Finished(Err(err.to_string())));
                    return;
                }
            };
            let final_transaction = built_spend.transaction;
            let fee_atoms = built_spend.fee_atoms;
            let output_total_atoms = match final_transaction.checked_output_value_atoms() {
                Some(total) => total,
                None => {
                    let _ = sender.send(SendJobEvent::Finished(Err(String::from(
                        "transaction output total overflow",
                    ))));
                    return;
                }
            };
            let actual_fee = match total_input_atoms.checked_sub(output_total_atoms) {
                Some(fee) => fee,
                None => {
                    let _ = sender.send(SendJobEvent::Finished(Err(String::from(
                        "selected inputs do not cover amount and fee",
                    ))));
                    return;
                }
            };
            if actual_fee != fee_atoms {
                let _ = sender.send(SendJobEvent::Finished(Err(String::from(
                    "transaction fee calculation mismatch",
                ))));
                return;
            }
            let tx_pow_nonce = final_transaction.tx_pow_nonce;
            let tx_pow_bits = final_transaction.tx_pow_bits;
            let _ = sender.send(SendJobEvent::Progress {
                stage: SendProgressStage::Broadcasting,
            });
            let result = if use_local_node {
                match connection.request(RpcRequest::SubmitTransaction {
                    transaction: final_transaction,
                    fee_atoms,
                }) {
                    RpcResponse::TransactionSubmitted(txid) => Ok(SendOutcome {
                        fee_atoms,
                        txid,
                        tx_pow_nonce,
                        tx_pow_bits,
                    }),
                    RpcResponse::Error(err) => Err(err.to_string()),
                    other => Err(format!("unexpected rpc response: {other:?}")),
                }
            } else {
                let client = RpcClient::new(rpc_address);
                match client.call(&RpcRequest::SubmitTransaction {
                    transaction: final_transaction,
                    fee_atoms,
                }) {
                    Ok(RpcResponse::TransactionSubmitted(txid)) => Ok(SendOutcome {
                        fee_atoms,
                        txid,
                        tx_pow_nonce,
                        tx_pow_bits,
                    }),
                    Ok(RpcResponse::Error(err)) => Err(err.to_string()),
                    Ok(other) => Err(format!("unexpected rpc response: {other:?}")),
                    Err(err) => Err(err.to_string()),
                }
            };
            let _ = sender.send(SendJobEvent::Finished(result));
        });

        self.send_fee = self.format_fee_amount(selected_plan.estimated_fee_atoms);
        self.send_status = SendProgressStage::Preparing.label().to_string();
        self.last_error = None;
        self.send_job = Some(SendJob {
            started_at: Instant::now(),
            stage: SendProgressStage::Preparing,
            receiver,
        });
        Ok(())
    }

    pub(crate) fn use_max_sendable_amount(&mut self) -> Result<(), String> {
        let reserved_inputs = self.mempool_reserved_inputs();
        if self.wallet_address_index_cache.is_empty() && self.wallet_scan_job.is_none() {
            self.wallet_cache_dirty = true;
            self.start_wallet_scan_job();
        }
        let spendable_inputs = self.spendable_wallet_inputs(&reserved_inputs)?;
        if spendable_inputs.is_empty() {
            return Err(String::from(
                "No spendable wallet UTXOs are available for a new transaction",
            ));
        }
        let sendable_atoms = Self::max_single_address_sendable_atoms(
            spendable_inputs,
            self.active_network(),
            self.send_include_fee_in_total,
        );
        if sendable_atoms == 0 {
            return Err(String::from(
                "No spendable wallet balance can cover a valid transaction fee",
            ));
        }
        self.send_amount = Self::format_send_amount_input(self.send_input_unit(), sendable_atoms);
        self.last_error = None;
        self.send_status = if self.send_include_fee_in_total {
            format!(
                "Filled the largest currently spendable wallet total amount: {}",
                self.format_amount(sendable_atoms)
            )
        } else {
            format!(
                "Filled the largest currently spendable wallet recipient amount: {}",
                self.format_amount(sendable_atoms)
            )
        };
        Ok(())
    }

    fn unspendable_send_amount_message(
        &self,
        amount: u64,
        spendable_inputs: &[SpendableWalletUtxo],
    ) -> String {
        let max_sendable = Self::max_single_address_sendable_atoms(
            spendable_inputs.to_vec(),
            self.active_network(),
            self.send_include_fee_in_total,
        );
        if max_sendable == 0 {
            return String::from("No spendable wallet balance can cover a valid transaction fee");
        }
        format!(
            "The wallet cannot cover {} after fees. Max spendable now: {}.",
            self.format_amount(amount),
            self.format_amount(max_sendable)
        )
    }

    fn select_wallet_utxos(
        candidates: Vec<SpendableWalletUtxo>,
        network: Network,
        amount_atoms: u64,
        include_fee_in_total: bool,
    ) -> Result<Option<SelectedSpendPlan>, String> {
        let min_output = Self::wallet_min_output_amount_atoms(network);
        let mut candidates = candidates;
        Self::sort_spend_candidates(&mut candidates);

        let mut best: Option<SelectedSpendPlan> = None;
        let mut selected = Vec::new();
        let mut total = 0u64;
        let mut signer_groups = std::collections::BTreeSet::<[u8; 32]>::new();

        for candidate in candidates {
            total = total.saturating_add(candidate.utxo.value_atoms);
            signer_groups.insert(candidate.address.payment_digest);
            selected.push(candidate);
            let signer_group_count = signer_groups.len();
            let estimate_exact =
                Self::estimate_transaction_shape(network, selected.len(), 1, signer_group_count);
            let estimate_change =
                Self::estimate_transaction_shape(network, selected.len(), 2, signer_group_count);
            if !Self::transaction_shape_is_standard(estimate_exact) {
                break;
            }

            let candidate_output_count = if include_fee_in_total {
                let recipient_one_output = amount_atoms.saturating_sub(estimate_exact.fee_atoms);
                let recipient_two_output = amount_atoms.saturating_sub(estimate_change.fee_atoms);
                let excess = total.saturating_sub(amount_atoms);
                if total >= amount_atoms
                    && excess < min_output
                    && recipient_one_output >= min_output
                {
                    Some(1)
                } else if total > amount_atoms
                    && excess >= min_output
                    && recipient_two_output >= min_output
                {
                    Some(2)
                } else {
                    None
                }
            } else if amount_atoms >= min_output {
                let exact_target = amount_atoms.checked_add(estimate_exact.fee_atoms);
                let change_target = amount_atoms.checked_add(estimate_change.fee_atoms);
                if exact_target.is_some_and(|target| {
                    total >= target && total.saturating_sub(target) < min_output
                }) {
                    Some(1)
                } else if change_target.is_some_and(|target| {
                    total > target && total.saturating_sub(target) >= min_output
                }) {
                    Some(2)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(output_count) = candidate_output_count {
                let estimate = if output_count == 1 {
                    estimate_exact
                } else {
                    estimate_change
                };
                if !Self::transaction_shape_is_standard(estimate) {
                    break;
                }
                let candidate = SelectedSpendPlan {
                    utxos: selected.iter().map(|entry| entry.utxo.clone()).collect(),
                    total_input_atoms: total,
                    output_count,
                    signer_group_count,
                    transaction_version: 1,
                    estimated_fee_atoms: estimate.fee_atoms,
                    estimated_raw_size_bytes: estimate.raw_size_bytes,
                    estimated_vsize_bytes: estimate.vsize_bytes,
                };
                best = Self::prefer_candidate(best, candidate);
                if output_count == 1 && total >= amount_atoms {
                    break;
                }
            }
        }

        Ok(best)
    }

    fn max_single_address_sendable_atoms(
        candidates: Vec<SpendableWalletUtxo>,
        network: Network,
        include_fee_in_total: bool,
    ) -> u64 {
        let min_output = Self::wallet_min_output_amount_atoms(network);
        let mut candidates = candidates;
        Self::sort_spend_candidates(&mut candidates);
        let mut total = 0u64;
        let mut included_inputs = 0usize;
        let mut signer_groups = std::collections::BTreeSet::<[u8; 32]>::new();
        for candidate in candidates {
            let next_signer_group_count = signer_groups.len()
                + usize::from(!signer_groups.contains(&candidate.address.payment_digest));
            let next_estimate = Self::estimate_transaction_shape(
                network,
                included_inputs + 1,
                1,
                next_signer_group_count,
            );
            if !Self::transaction_shape_is_standard(next_estimate) {
                break;
            }
            total = total.saturating_add(candidate.utxo.value_atoms);
            included_inputs += 1;
            signer_groups.insert(candidate.address.payment_digest);
        }
        if included_inputs == 0 {
            return 0;
        }
        let fee =
            Self::estimate_transaction_shape(network, included_inputs, 1, signer_groups.len())
                .fee_atoms;
        if include_fee_in_total {
            if total > fee && total.saturating_sub(fee) >= min_output {
                total
            } else {
                0
            }
        } else {
            let recipient_atoms = total.saturating_sub(fee);
            if recipient_atoms >= min_output {
                recipient_atoms
            } else {
                0
            }
        }
    }

    fn sort_spend_candidates(candidates: &mut [SpendableWalletUtxo]) {
        candidates.sort_by(|left, right| {
            right
                .utxo
                .value_atoms
                .cmp(&left.utxo.value_atoms)
                .then(
                    left.address
                        .payment_digest
                        .cmp(&right.address.payment_digest),
                )
                .then(left.utxo.txid.cmp(&right.utxo.txid))
                .then(left.utxo.output_index.cmp(&right.utxo.output_index))
        });
    }

    fn prefer_candidate(
        current: Option<SelectedSpendPlan>,
        candidate: SelectedSpendPlan,
    ) -> Option<SelectedSpendPlan> {
        match current {
            None => Some(candidate),
            Some(existing) => {
                let existing_inputs = existing.utxos.len();
                let candidate_inputs = candidate.utxos.len();
                if candidate.estimated_fee_atoms < existing.estimated_fee_atoms
                    || (candidate.estimated_fee_atoms == existing.estimated_fee_atoms
                        && candidate_inputs < existing_inputs)
                    || (candidate.estimated_fee_atoms == existing.estimated_fee_atoms
                        && candidate_inputs == existing_inputs
                        && candidate.signer_group_count < existing.signer_group_count)
                    || (candidate.estimated_fee_atoms == existing.estimated_fee_atoms
                        && candidate_inputs == existing_inputs
                        && candidate.signer_group_count == existing.signer_group_count
                        && candidate.output_count < existing.output_count)
                    || (candidate.estimated_fee_atoms == existing.estimated_fee_atoms
                        && candidate_inputs == existing_inputs
                        && candidate.signer_group_count == existing.signer_group_count
                        && candidate.output_count == existing.output_count
                        && candidate.total_input_atoms < existing.total_input_atoms)
                {
                    Some(candidate)
                } else {
                    Some(existing)
                }
            }
        }
    }

    #[cfg(test)]
    fn estimate_fee(
        network: Network,
        input_count: usize,
        output_count: usize,
        signer_group_count: usize,
    ) -> u64 {
        Self::estimate_transaction_shape(network, input_count, output_count, signer_group_count)
            .fee_atoms
    }

    fn estimate_transaction_shape(
        network: Network,
        input_count: usize,
        output_count: usize,
        signer_group_count: usize,
    ) -> TransactionShapeEstimate {
        let mut inputs = Vec::with_capacity(input_count);
        for index in 0..input_count {
            inputs.push(TxInput {
                previous_txid: [0; 48],
                output_index: index as u32,
                unlocking_script: vec![0; 32],
            });
        }
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            outputs.push(TxOutput {
                value_atoms: 1,
                locking_script: vec![0; 32],
            });
        }
        let signer_group_count = signer_group_count.max(1).min(input_count.max(1));
        let mut primary_input_refs = Vec::new();
        let mut additional_signers = Vec::new();
        for group_index in 0..signer_group_count {
            let input_index = group_index as u32;
            let input_ref = atho_core::transaction::WitnessInputRef {
                input_index,
                sig_ref_short: [0; 2],
                witness_commit_ref: [0; 16],
            };
            if group_index == 0 {
                primary_input_refs.push(input_ref);
            } else {
                additional_signers.push(atho_core::transaction::WitnessSignerGroup {
                    signature: vec![0; FALCON_512_SIGNATURE_BYTES],
                    pubkey: vec![0; FALCON_512_PUBLIC_KEY_BYTES],
                    input_refs: vec![input_ref],
                });
            }
        }
        for input_index in signer_group_count..input_count {
            primary_input_refs.push(atho_core::transaction::WitnessInputRef {
                input_index: input_index as u32,
                sig_ref_short: [0; 2],
                witness_commit_ref: [0; 16],
            });
        }
        let witness = TxWitness {
            signature: vec![0; FALCON_512_SIGNATURE_BYTES],
            pubkey: vec![0; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: primary_input_refs,
            additional_signers,
        }
        .canonical_bytes();
        let tx = Transaction {
            version: 1,
            inputs,
            outputs,
            lock_time: 0,
            witness,
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        TransactionShapeEstimate {
            fee_atoms: minimum_required_fee_atoms(network, &tx),
            raw_size_bytes: tx.full_size_bytes(),
            vsize_bytes: tx.vsize_bytes(),
        }
    }

    fn transaction_shape_is_standard(estimate: TransactionShapeEstimate) -> bool {
        estimate.raw_size_bytes <= MAX_TRANSACTION_RAW_BYTES
            && estimate.vsize_bytes <= MAX_TRANSACTION_VBYTES
    }

    fn wallet_min_output_amount_atoms(network: Network) -> u64 {
        let _ = network;
        DUST_RELAY_VALUE_ATOMS
    }

    fn dust_amount_error_message(network: Network) -> String {
        format!(
            "Spendable outputs must be at least {}",
            format_amount_atoms(
                Self::wallet_min_output_amount_atoms(network),
                DisplayUnit::NanoAtho
            )
        )
    }

    fn transaction_lock_time_nonce() -> u32 {
        let mut entropy = [0u8; 16];
        let _ = getrandom(&mut entropy);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let mut preimage = Vec::with_capacity(core::mem::size_of::<u128>() + entropy.len());
        preimage.extend_from_slice(&nanos.to_le_bytes());
        preimage.extend_from_slice(&entropy);
        let digest = sha3_256(&preimage);
        u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]])
    }

    pub(crate) fn format_send_amount_input(input_unit: InputUnit, atoms: u64) -> String {
        format_amount_atoms_without_unit(atoms, input_unit)
    }

    fn parse_send_amount_atoms(input_unit: InputUnit, input: &str) -> Result<u64, String> {
        parse_amount_to_atoms(input, input_unit)
    }

    pub(crate) fn display_unit(&self) -> DisplayUnit {
        self.display_preferences.display_unit
    }

    pub(crate) fn send_input_unit(&self) -> InputUnit {
        self.display_preferences.send_input_unit
    }

    pub(crate) fn set_display_unit(&mut self, unit: DisplayUnit) {
        if self.display_preferences.display_unit == unit {
            return;
        }
        self.display_preferences = self.display_preferences.with_display_unit(unit);
        let _ = self.persist_client_display_preferences();
    }

    pub(crate) fn set_send_input_unit(&mut self, unit: InputUnit) {
        if self.display_preferences.send_input_unit == unit {
            return;
        }
        if !self.send_amount.trim().is_empty() {
            if let Ok(amount_atoms) = Self::parse_send_amount_atoms(
                self.display_preferences.send_input_unit,
                &self.send_amount,
            ) {
                self.send_amount = Self::format_send_amount_input(unit, amount_atoms);
            }
        }
        self.display_preferences = self.display_preferences.with_send_input_unit(unit);
        let _ = self.persist_client_display_preferences();
    }

    pub(crate) fn format_amount(&self, atoms: u64) -> String {
        format_amount_atoms(atoms, self.display_unit())
    }

    pub(crate) fn format_fee_amount(&self, atoms: u64) -> String {
        format_fee_atoms(atoms, self.display_unit())
    }

    fn current_receive_address_text(&self) -> String {
        self.current_receive_address
            .as_ref()
            .map(|address| address.address.clone())
            .unwrap_or_default()
    }

    pub(crate) fn open_debug_window(&mut self, tab: DebugWindowTab) {
        self.show_debug_window = true;
        self.debug_window_tab = tab;
        self.ensure_debug_peer_selection();
    }

    pub(crate) fn close_debug_window(&mut self) {
        self.show_debug_window = false;
    }

    fn ensure_debug_peer_selection(&mut self) {
        if let Some(selected) = self.debug_selected_peer.as_ref() {
            if self
                .view_model
                .peers
                .iter()
                .any(|peer| &peer.remote_addr == selected)
            {
                return;
            }
        }
        self.debug_selected_peer = self
            .view_model
            .peers
            .first()
            .map(|peer| peer.remote_addr.clone());
    }

    fn record_network_traffic_sample(&mut self, status: &ConnectionStatus) {
        let now = Instant::now();
        let timestamp_unix = current_unix_seconds();
        let Some((previous_at, previous_sent, previous_received)) =
            self.last_network_traffic_snapshot
        else {
            self.network_traffic_samples.clear();
            self.network_traffic_samples.push(NetworkTrafficSample {
                timestamp_unix,
                bytes_sent_per_second: 0.0,
                bytes_received_per_second: 0.0,
                total_bytes_sent: status.bytes_sent,
                total_bytes_received: status.bytes_received,
            });
            self.last_network_traffic_snapshot =
                Some((now, status.bytes_sent, status.bytes_received));
            return;
        };

        let elapsed = now.saturating_duration_since(previous_at).as_secs_f64();
        if elapsed <= f64::EPSILON {
            return;
        }

        if status.bytes_sent < previous_sent || status.bytes_received < previous_received {
            self.network_traffic_samples.clear();
        }

        let sent_delta = status.bytes_sent.saturating_sub(previous_sent) as f64;
        let received_delta = status.bytes_received.saturating_sub(previous_received) as f64;
        self.network_traffic_samples.push(NetworkTrafficSample {
            timestamp_unix,
            bytes_sent_per_second: sent_delta / elapsed,
            bytes_received_per_second: received_delta / elapsed,
            total_bytes_sent: status.bytes_sent,
            total_bytes_received: status.bytes_received,
        });
        if self.network_traffic_samples.len() > 180 {
            let drop_count = self.network_traffic_samples.len() - 180;
            self.network_traffic_samples.drain(0..drop_count);
        }
        self.last_network_traffic_snapshot = Some((now, status.bytes_sent, status.bytes_received));
    }

    fn record_sync_progress_sample(&mut self) {
        if !self.view_model.running {
            return;
        }

        let now = Instant::now();
        let target_height = self.view_model.sync_target_height();
        let progress = self.view_model.sync_progress_display();
        let latest = self.sync_progress_samples.last();

        if latest.is_some_and(|sample| {
            sample.local_height == self.view_model.block_count
                && sample.target_height == target_height
                && sample.recorded_at.elapsed() < Duration::from_secs(20)
        }) {
            return;
        }

        self.sync_progress_samples.push(SyncProgressSample {
            recorded_at: now,
            local_height: self.view_model.block_count,
            target_height,
            progress,
        });

        if self.sync_progress_samples.len() > 360 {
            let drop_count = self.sync_progress_samples.len() - 360;
            self.sync_progress_samples.drain(0..drop_count);
        }

        let retention = Duration::from_secs(6 * 60 * 60);
        self.sync_progress_samples
            .retain(|sample| now.saturating_duration_since(sample.recorded_at) <= retention);
    }

    pub(crate) fn clear_network_traffic_samples(&mut self) {
        self.network_traffic_samples.clear();
        self.last_network_traffic_snapshot = Some((
            Instant::now(),
            self.view_model.bytes_sent,
            self.view_model.bytes_received,
        ));
    }

    fn current_receive_address_row(&self) -> Option<&ReceiveAddressRow> {
        let digest = self.current_receive_address.as_ref()?.payment_digest;
        self.receive_address_rows
            .iter()
            .find(|row| row.address.payment_digest == digest)
    }

    fn selected_receive_request(&self) -> Option<&ReceiveRequestRecord> {
        let selected = self.selected_receive_request?;
        self.requested_payments
            .iter()
            .find(|request| request.sequence == selected)
    }

    fn select_receive_request(&mut self, sequence: usize) {
        self.selected_receive_request = Some(sequence);
    }

    fn remove_selected_receive_request(&mut self) {
        let Some(selected) = self.selected_receive_request else {
            return;
        };
        self.requested_payments
            .retain(|request| request.sequence != selected);
        self.selected_receive_request = self
            .requested_payments
            .last()
            .map(|request| request.sequence);
    }

    pub(crate) fn run_debug_console_command(&mut self) {
        let line = self.debug_console_input.trim().to_string();
        if line.is_empty() {
            self.record_debug_console_problem(
                String::from("(empty)"),
                String::from("debug console command is empty"),
                Vec::new(),
                None,
            );
            return;
        }
        self.run_debug_console_line(line, true);
        self.debug_console_input.clear();
    }

    pub(crate) fn run_debug_console_line(&mut self, line: String, push_history: bool) {
        if line.is_empty() {
            self.last_error = Some(String::from("debug console command is empty"));
            return;
        }
        let timestamp_unix = current_unix_seconds();

        let parsed = match parse_command_line(&line) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.record_debug_console_problem(line, err, Vec::new(), None);
                return;
            }
        };

        let Some(definition) = command_definition(&parsed.name) else {
            let suggestions = self
                .debug_console_suggestions(&parsed.name)
                .into_iter()
                .filter(|suggestion| suggestion != &parsed.name)
                .collect::<Vec<_>>();
            self.record_debug_console_problem(
                line,
                format!("unknown command {}", parsed.name),
                suggestions,
                Some(parsed.name),
            );
            return;
        };

        if definition.name.eq_ignore_ascii_case("help") {
            let payload = match help_payload(parsed.args.first().map(String::as_str)) {
                Ok(payload) => payload,
                Err(err) => {
                    self.record_debug_console_problem(
                        line,
                        err,
                        self.debug_console_suggestions(
                            parsed.args.first().map(String::as_str).unwrap_or_default(),
                        ),
                        Some(definition.name.to_string()),
                    );
                    return;
                }
            };
            let entry = DebugConsoleEntry {
                timestamp_unix,
                command_line: line.clone(),
                command_name: definition.name.to_string(),
                group: definition.group,
                permission: definition.permission,
                dangerous: definition.dangerous,
                success: true,
                network_label: self.view_model.network_label.clone(),
                output: self.format_console_help_payload(&payload),
                error_code: None,
            };
            self.last_error = None;
            self.debug_console_status = String::from("Command help completed");
            self.push_debug_console_entry(line, push_history, entry);
            return;
        }

        let mut invocation = parsed;
        invocation.confirmed = self.debug_console_confirmed;
        let response = self
            .connection
            .request(RpcRequest::ExecuteCommand(invocation.clone()));
        let entry = match response {
            RpcResponse::Command(command) => {
                self.last_error = None;
                self.debug_console_status = format!("Command {} completed", command.command);
                self.debug_console_entry_success(line.clone(), timestamp_unix, command)
            }
            RpcResponse::Error(error) => {
                self.last_error = Some(error.to_string());
                DebugConsoleEntry {
                    timestamp_unix,
                    command_line: line.clone(),
                    command_name: definition.name.to_string(),
                    group: definition.group,
                    permission: definition.permission,
                    dangerous: definition.dangerous,
                    success: false,
                    network_label: self.view_model.network_label.clone(),
                    output: self.format_console_error(&error),
                    error_code: Some(error.code),
                }
            }
            other => {
                let message = format!("unexpected rpc response: {other:?}");
                self.last_error = Some(message.clone());
                DebugConsoleEntry {
                    timestamp_unix,
                    command_line: line.clone(),
                    command_name: definition.name.to_string(),
                    group: definition.group,
                    permission: definition.permission,
                    dangerous: definition.dangerous,
                    success: false,
                    network_label: self.view_model.network_label.clone(),
                    output: message,
                    error_code: None,
                }
            }
        };

        self.push_debug_console_entry(line, push_history, entry);
    }

    pub(crate) fn debug_console_previous_history(&mut self) {
        if self.debug_console_history.is_empty() {
            return;
        }
        let next_index = match self.debug_console_history_index {
            Some(index) if index > 0 => index - 1,
            Some(_) => 0,
            None => self.debug_console_history.len().saturating_sub(1),
        };
        self.debug_console_history_index = Some(next_index);
        self.debug_console_input = self.debug_console_history[next_index].clone();
    }

    pub(crate) fn debug_console_next_history(&mut self) {
        let Some(index) = self.debug_console_history_index else {
            return;
        };
        if index + 1 >= self.debug_console_history.len() {
            self.debug_console_history_index = None;
            self.debug_console_input.clear();
            return;
        }
        let next_index = index + 1;
        self.debug_console_history_index = Some(next_index);
        self.debug_console_input = self.debug_console_history[next_index].clone();
    }

    fn debug_console_entry_success(
        &self,
        command_line: String,
        timestamp_unix: u64,
        command: CommandResponse,
    ) -> DebugConsoleEntry {
        DebugConsoleEntry {
            timestamp_unix,
            command_line,
            command_name: command.command.clone(),
            group: command.group,
            permission: command.permission,
            dangerous: command.dangerous,
            success: true,
            network_label: command.network,
            output: self.format_console_command_output(&command.command, &command.data),
            error_code: None,
        }
    }

    fn format_console_command_output(
        &self,
        command_name: &str,
        value: &serde_json::Value,
    ) -> String {
        if command_name.eq_ignore_ascii_case("help") {
            return self.format_console_help_payload(value);
        }
        self.format_console_value(value)
    }

    fn format_console_value(&self, value: &serde_json::Value) -> String {
        match self.debug_console_output_mode {
            DebugConsoleOutputMode::Pretty | DebugConsoleOutputMode::Json => {
                serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
            }
            DebugConsoleOutputMode::Table => format_table_value(value).unwrap_or_else(|| {
                serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
            }),
        }
    }

    fn format_console_help_payload(&self, payload: &serde_json::Value) -> String {
        if matches!(self.debug_console_output_mode, DebugConsoleOutputMode::Json) {
            return serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
        }

        if let Some(groups) = payload.get("groups").and_then(|value| value.as_object()) {
            let mut out = String::from("Atho RPC Commands\n");
            out.push_str("Type help <command> for usage details.\n");
            for (group, commands) in groups {
                out.push_str(&format!("\n== {} ==\n", group));
                if let Some(items) = commands.as_array() {
                    for item in items {
                        let name = item
                            .get("name")
                            .and_then(|value| value.as_str())
                            .unwrap_or("?");
                        let description = item
                            .get("description")
                            .and_then(|value| value.as_str())
                            .unwrap_or("");
                        out.push_str(&format!("  {name:<24} {description}\n"));
                    }
                }
            }
            return out.trim_end().to_string();
        }

        if let Some(name) = payload.get("name").and_then(|value| value.as_str()) {
            let group = payload
                .get("group")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let description = payload
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let usage = payload
                .get("usage")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let permission = payload
                .get("permission")
                .and_then(|value| value.as_str())
                .unwrap_or("UNKNOWN");
            let examples = payload
                .get("examples")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();

            let mut out = format!(
                "{name}\n  Group: {group}\n  Permission: {permission}\n  Description: {description}\n  Usage: {usage}"
            );
            if !examples.is_empty() {
                out.push_str("\n  Examples:");
                for example in examples {
                    if let Some(example) = example.as_str() {
                        out.push_str(&format!("\n    {example}"));
                    }
                }
            }
            return out;
        }

        if let Some(commands) = payload.get("commands").and_then(|value| value.as_array()) {
            let query = payload
                .get("query")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let mut out = format!("Matches for {query}\n");
            for item in commands {
                let name = item
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?");
                let group = item
                    .get("group")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                let description = item
                    .get("description")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                out.push_str(&format!("  [{group}] {name:<24} {description}\n"));
            }
            return out.trim_end().to_string();
        }

        serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
    }

    fn format_console_error(&self, error: &atho_rpc::error::RpcError) -> String {
        let value = serde_json::json!({
            "success": false,
            "error": {
                "code": error.code,
                "title": error.title,
                "message": error.message,
                "severity": error.severity,
                "details": error.details,
            }
        });
        match self.debug_console_output_mode {
            DebugConsoleOutputMode::Json
            | DebugConsoleOutputMode::Pretty
            | DebugConsoleOutputMode::Table => {
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            }
        }
    }

    pub(crate) fn copy_latest_debug_console_output(&self, ui: &mut egui::Ui) {
        if let Some(entry) = self.debug_console_entries.last() {
            Self::copy_text(ui, entry.output.clone());
        }
    }

    pub(crate) fn clear_debug_console(&mut self) {
        self.debug_console_entries.clear();
        self.debug_console_history_index = None;
        self.debug_console_status = String::from("Console cleared");
    }

    fn push_debug_console_entry(
        &mut self,
        line: String,
        push_history: bool,
        entry: DebugConsoleEntry,
    ) {
        if push_history
            && self
                .debug_console_history
                .last()
                .is_none_or(|previous| previous != &line)
        {
            self.debug_console_history.push(line);
        }
        self.debug_console_history_index = None;
        self.debug_console_entries.push(entry);
    }

    fn record_debug_console_problem(
        &mut self,
        line: String,
        message: String,
        suggestions: Vec<String>,
        command_name: Option<String>,
    ) {
        let timestamp_unix = current_unix_seconds();
        self.last_error = Some(message.clone());
        self.debug_console_status = String::from("Command failed");
        let output = if suggestions.is_empty() {
            message
        } else {
            format!("{message}\nDid you mean:\n  {}", suggestions.join("\n  "))
        };
        let entry = DebugConsoleEntry {
            timestamp_unix,
            command_line: line.clone(),
            command_name: command_name.unwrap_or_else(|| String::from("unknown")),
            group: atho_rpc::command::CommandGroup::Debug,
            permission: atho_rpc::command::CommandPermission::PublicRead,
            dangerous: false,
            success: false,
            network_label: self.view_model.network_label.clone(),
            output,
            error_code: None,
        };
        let push_history = !line.trim().is_empty() && line != "(empty)";
        self.push_debug_console_entry(line, push_history, entry);
    }

    pub(crate) fn debug_console_suggestions(&self, query: &str) -> Vec<String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let token = trimmed.split_whitespace().next().unwrap_or(trimmed);
        let normalized = token.to_ascii_lowercase();
        let mut suggestions = search_commands(token)
            .into_iter()
            .map(|definition| definition.name.to_string())
            .filter(|name| name.starts_with(&normalized))
            .collect::<Vec<_>>();
        if suggestions.is_empty() {
            suggestions = search_commands(token)
                .into_iter()
                .map(|definition| definition.name.to_string())
                .collect::<Vec<_>>();
        }
        if suggestions.is_empty() {
            let mut ranked = search_commands("")
                .into_iter()
                .map(|definition| {
                    (
                        levenshtein_distance(&normalized, definition.name),
                        definition.name.to_string(),
                    )
                })
                .collect::<Vec<_>>();
            ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            suggestions = ranked
                .into_iter()
                .filter(|(distance, _)| *distance <= 4)
                .map(|(_, name)| name)
                .collect();
        }
        suggestions.sort();
        suggestions.dedup();
        suggestions.truncate(8);
        suggestions
    }

    fn copy_text(ui: &mut egui::Ui, text: String) {
        ui.output_mut(|output| {
            output.copied_text = text;
        });
    }

    fn max_mining_cores(&self) -> u32 {
        available_mining_cores()
    }

    fn render_wallet_preparation_overlay(&mut self, ctx: &egui::Context) {
        if self.wallet_preparation_job.is_none() || self.wallet.is_none() {
            return;
        }

        let progress = self.wallet_preparation_progress.clamp(0.0, 1.0);
        let show_progress = self.wallet_preparation_total > 0;
        let title = if self.wallet_preparation_stage.is_empty() {
            "Preparing wallet".to_owned()
        } else {
            self.wallet_preparation_stage.clone()
        };

        let screen_rect = ctx.input(|input| input.screen_rect());
        let layer_id = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("wallet_prep"));
        ctx.layer_painter(layer_id).rect_filled(
            screen_rect,
            0.0,
            egui::Color32::from_black_alpha(160),
        );

        egui::Window::new("Wallet preparation")
            .id(egui::Id::new("wallet_preparation_modal"))
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .fixed_size(egui::vec2(500.0, 190.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Preparing wallet").size(20.0).strong());
                    ui.add_space(4.0);
                    ui.label(title);
                    if self.wallet_preparation_total > 0 {
                        ui.label(format!(
                            "{} / {} steps",
                            self.wallet_preparation_completed, self.wallet_preparation_total
                        ));
                    }
                    ui.add_space(10.0);
                    if show_progress {
                        ui.add(
                            egui::ProgressBar::new(progress)
                                .desired_width(380.0)
                                .animate(true),
                        );
                        ui.add_space(4.0);
                    } else {
                        ui.add(egui::Spinner::new().size(18.0));
                        ui.add_space(6.0);
                    }
                    ui.label("Wallet actions stay locked until preparation finishes.");
                });
            });
    }

    fn clamp_mining_cores(&self, cores: u32) -> u32 {
        cores.clamp(1, self.max_mining_cores())
    }

    fn read_clipboard_text() -> Option<String> {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        let text = clipboard.get_text().ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }
}

fn format_table_value(value: &serde_json::Value) -> Option<String> {
    let rows = value.as_array()?;
    if rows.is_empty() {
        return Some(String::from("(empty)"));
    }
    let objects: Vec<_> = rows.iter().map(|row| row.as_object()).collect();
    if objects.iter().any(|row| row.is_none()) {
        return None;
    }

    let mut columns = Vec::<String>::new();
    for row in objects.iter().flatten() {
        for key in row.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }

    if columns.is_empty() {
        return None;
    }

    let mut widths = columns
        .iter()
        .map(|column| column.len())
        .collect::<Vec<_>>();
    let rendered_rows = objects
        .into_iter()
        .flatten()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    let cell = row
                        .get(column)
                        .map(render_table_cell)
                        .unwrap_or_else(|| String::from("-"));
                    widths[index] = widths[index].max(cell.len());
                    cell
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut output = String::new();
    output.push_str(&render_table_row(&columns, &widths));
    output.push('\n');
    output.push_str(&render_table_separator(&widths));
    for row in rendered_rows {
        output.push('\n');
        output.push_str(&render_table_row(&row, &widths));
    }
    Some(output)
}

fn render_table_row(row: &[String], widths: &[usize]) -> String {
    row.iter()
        .zip(widths.iter())
        .map(|(cell, width)| format!("{cell:<width$}", width = *width))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_table_separator(widths: &[usize]) -> String {
    widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>()
        .join("-+-")
}

fn render_table_cell(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::from("-"),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| String::from("?")),
    }
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    if left_chars.is_empty() {
        return right_chars.len();
    }
    if right_chars.is_empty() {
        return left_chars.len();
    }

    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0usize; right_chars.len() + 1];

    for (i, left_char) in left_chars.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in right_chars.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != right_char);
            let insertion = current[j] + 1;
            let deletion = previous[j + 1] + 1;
            current[j + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.theme_initialized {
            theme::install_fonts(ctx);
            self.theme_initialized = true;
        }
        if self.active_tab == NavTab::DebugConsole {
            self.open_debug_window(DebugWindowTab::Console);
            self.active_tab = NavTab::Overview;
        }
        if self.wallet.is_none() && !self.compact_viewport {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(700.0, 440.0)));
            self.compact_viewport = true;
        } else if self.wallet.is_some() && self.compact_viewport {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(860.0, 560.0)));
            self.compact_viewport = false;
        }
        theme::apply_theme(ctx);
        self.drain_status_updates();
        self.poll_send_job();
        self.poll_mining_job();
        self.poll_wallet_preparation_job();
        self.refresh_wallet_cache_if_needed();
        let repaint_after =
            if self.wallet_preparation_job.is_some() || self.wallet_readiness_gate_active {
                Duration::from_millis(50)
            } else if self.mining_job.is_some() || self.send_job.is_some() {
                Duration::from_millis(150)
            } else if self.wallet_cache_dirty || self.active_tab == NavTab::Overview {
                Duration::from_millis(400)
            } else {
                Duration::from_millis(750)
            };
        ctx.request_repaint_after(repaint_after);

        if self.wallet.is_some() && !self.wallet_readiness_gate_active {
            shell::render_main_shell(self, ctx);
        } else if self.wallet_readiness_blocks_main_ui() {
            startup::render_wallet_preparation_screen(self, ctx);
        } else {
            startup::render_startup_screen(self, ctx);
        }
        self.render_wallet_preparation_overlay(ctx);
    }
}

impl DesktopApp {
    fn drain_status_updates(&mut self) {
        while let Some(status) = self.status_monitor.try_recv_latest() {
            self.apply_connection_status(status);
        }
    }
}

fn default_wallet_path(network: Network) -> PathBuf {
    wallet_slot_path(network, 1)
}

pub(crate) fn suggested_wallet_path(network: Network) -> PathBuf {
    next_available_wallet_path(network)
}

pub(crate) fn default_wallet_name(network: Network) -> String {
    next_available_wallet_name(network)
}

fn wallet_storage_root(network: Network) -> PathBuf {
    atho_node::dev::wallet_dir().join(network.id())
}

fn wallet_registry_path(network: Network) -> PathBuf {
    wallet_storage_root(network).join("wallet-registry.json")
}

fn wallet_slot_name(index: usize) -> String {
    if index <= 1 {
        String::from("wallet")
    } else {
        format!("wallet{index}")
    }
}

fn wallet_slot_display_name(index: usize) -> String {
    format!("Wallet {index}")
}

fn wallet_slot_path(network: Network, index: usize) -> PathBuf {
    wallet_storage_root(network)
        .join(wallet_slot_name(index))
        .join(Wallet::datafile_name())
}

fn next_available_wallet_slot(network: Network) -> usize {
    for index in 1..10_000 {
        if !wallet_slot_path(network, index).exists() {
            return index;
        }
    }
    10_000
}

fn next_available_wallet_path(network: Network) -> PathBuf {
    wallet_slot_path(network, next_available_wallet_slot(network))
}

fn next_available_wallet_name(network: Network) -> String {
    wallet_slot_display_name(next_available_wallet_slot(network))
}

fn normalize_wallet_path_input(wallet_path: &str) -> PathBuf {
    let path = PathBuf::from(wallet_path.trim());
    if path
        .file_name()
        .is_some_and(|name| name == Wallet::datafile_name())
        || path.extension().is_some()
    {
        path
    } else {
        path.join(Wallet::datafile_name())
    }
}

fn infer_wallet_name_from_path(wallet_path: &str) -> String {
    let path = normalize_wallet_path_input(wallet_path);
    let candidate = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| String::from("Wallet"));

    let normalized = candidate.replace(['-', '_'], " ");
    let mut words = Vec::new();
    for part in normalized.split_whitespace() {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            let first_upper = first.to_uppercase().collect::<String>();
            words.push(format!("{first_upper}{}", chars.as_str()));
        }
    }
    if words.is_empty() {
        String::from("Wallet")
    } else {
        words.join(" ")
    }
}

fn wallet_registry_entry_id(wallet_path: &str, created_at_unix: u64) -> String {
    let mut preimage = Vec::with_capacity(wallet_path.len() + core::mem::size_of::<u64>());
    preimage.extend_from_slice(wallet_path.as_bytes());
    preimage.extend_from_slice(&created_at_unix.to_be_bytes());
    hex::encode(sha3_256(&preimage))
}

fn load_wallet_registry(network: Network) -> WalletRegistry {
    let path = wallet_registry_path(network);
    let Ok(bytes) = fs::read(path) else {
        return WalletRegistry::default();
    };
    serde_json::from_slice::<WalletRegistry>(&bytes).unwrap_or_default()
}

fn persist_wallet_registry(network: Network, registry: &WalletRegistry) -> Result<(), String> {
    let path = wallet_registry_path(network);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(registry).map_err(|err| err.to_string())?;
    fs::write(path, bytes).map_err(|err| err.to_string())
}

fn legacy_wallet_root() -> PathBuf {
    atho_storage::path::sandbox_root().join("wallet")
}

fn legacy_default_wallet_path(network: Network) -> PathBuf {
    legacy_wallet_root()
        .join(network.id())
        .join(Wallet::datafile_name())
}

fn startup_wallet_metadata_path(network: Network) -> PathBuf {
    wallet_storage_root(network).join("last-wallet.json")
}

fn client_display_preferences_path(network: Network) -> PathBuf {
    atho_storage::path::wallet_root()
        .join(network.id())
        .join("client-display.json")
}

fn recipient_address_book_path(network: Network) -> PathBuf {
    atho_storage::path::wallet_root()
        .join(network.id())
        .join("recipient-address-book.json")
}

fn load_client_display_preferences(network: Network) -> ClientDisplayPreferences {
    let path = client_display_preferences_path(network);
    let Ok(bytes) = fs::read(path) else {
        return ClientDisplayPreferences::default();
    };
    serde_json::from_slice::<ClientDisplayPreferences>(&bytes).unwrap_or_default()
}

fn load_recipient_address_book(network: Network) -> Vec<RecipientAddressEntry> {
    let path = recipient_address_book_path(network);
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<RecipientAddressEntry>>(&bytes).unwrap_or_default()
}

fn recipient_address_entry_id(address: &str, created_at_unix: u64) -> String {
    let mut preimage = Vec::with_capacity(address.len() + core::mem::size_of::<u64>());
    preimage.extend_from_slice(address.as_bytes());
    preimage.extend_from_slice(&created_at_unix.to_be_bytes());
    hex::encode(sha3_256(&preimage))
}

fn legacy_startup_wallet_metadata_path(network: Network) -> PathBuf {
    legacy_wallet_root()
        .join(network.id())
        .join("last-wallet.json")
}

fn startup_wallet_path_from_metadata(metadata_path: &Path) -> Option<PathBuf> {
    if let Ok(bytes) = fs::read(metadata_path) {
        if let Ok(metadata) = serde_json::from_slice::<WalletStartupMetadata>(&bytes) {
            let candidate = PathBuf::from(metadata.wallet_path);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn startup_wallet_path(network: Network) -> Option<PathBuf> {
    let metadata_path = startup_wallet_metadata_path(network);
    if let Some(candidate) = startup_wallet_path_from_metadata(&metadata_path) {
        return Some(candidate);
    }

    let legacy_metadata_path = legacy_startup_wallet_metadata_path(network);
    if legacy_metadata_path != metadata_path {
        if let Some(candidate) = startup_wallet_path_from_metadata(&legacy_metadata_path) {
            return Some(candidate);
        }
    }

    let registry = load_wallet_registry(network);
    if let Some(last_id) = registry.last_opened_wallet_id.as_deref() {
        if let Some(entry) = registry
            .entries
            .iter()
            .find(|entry| entry.wallet_id == last_id)
            .filter(|entry| PathBuf::from(&entry.wallet_path).exists())
        {
            return Some(PathBuf::from(&entry.wallet_path));
        }
    }

    let default = default_wallet_path(network);
    if default.exists() {
        return Some(default);
    }

    let legacy_default = legacy_default_wallet_path(network);
    if legacy_default != default && legacy_default.exists() {
        return Some(legacy_default);
    }

    None
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn backup_wallet_path(wallet_path: &str) -> String {
    format!(
        "{}.backup",
        normalize_wallet_path_input(wallet_path).to_string_lossy()
    )
}

fn backup_wallet_json_path(wallet_path: &str) -> String {
    format!(
        "{}.recovery.json",
        normalize_wallet_path_input(wallet_path).to_string_lossy()
    )
}

fn backup_wallet_text_path(wallet_path: &str) -> String {
    format!(
        "{}.recovery.txt",
        normalize_wallet_path_input(wallet_path).to_string_lossy()
    )
}

fn backup_wallet_phrase_qr_path(wallet_path: &str) -> String {
    format!(
        "{}.recovery-phrase.qr.png",
        normalize_wallet_path_input(wallet_path).to_string_lossy()
    )
}

pub(crate) fn normalize_png_export_path(export_path: &str) -> PathBuf {
    let mut path = PathBuf::from(export_path.trim());
    let is_png = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("png"))
        .unwrap_or(false);
    if !is_png {
        path.set_extension("png");
    }
    path
}

fn receive_address_qr_path(
    wallet_path: &str,
    current_receive_index: Option<u32>,
    address: &str,
    selected_request: Option<&ReceiveRequestRecord>,
) -> String {
    let suffix = selected_request
        .and_then(|request| {
            (!request.label.trim().is_empty())
                .then(|| sanitize_export_file_component(&request.label))
        })
        .filter(|label| !label.is_empty())
        .map(|label| format!("request-{label}"))
        .or_else(|| current_receive_index.map(|index| format!("receive-r{index:04}")))
        .unwrap_or_else(|| {
            let short = sanitize_export_file_component(&widgets::elide_text(address, 16));
            format!("receive-{short}")
        });
    format!(
        "{}.{suffix}.qr.png",
        normalize_wallet_path_input(wallet_path).to_string_lossy()
    )
}

fn sanitize_export_file_component(input: &str) -> String {
    let sanitized: String = input
        .trim()
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            '-' | '_' => ch,
            _ => '-',
        })
        .collect();
    let collapsed = sanitized
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        String::from("qr")
    } else {
        collapsed.to_ascii_lowercase()
    }
}

fn write_basic_qr_png(export_path: &str, payload: &str, dark: Rgba<u8>) -> Result<(), String> {
    let path = normalize_png_export_path(export_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let code = QrCode::encode_text(payload, QrCodeEcc::Medium)
        .map_err(|_| String::from("Unable to encode QR payload"))?;
    let module_count = code.size();
    if module_count <= 0 {
        return Err(String::from("QR payload produced an empty matrix"));
    }

    let quiet_zone = 4;
    let total_modules = (module_count + quiet_zone * 2).max(1) as u32;
    let target_qr_pixels = 520u32;
    let module_size = (target_qr_pixels / total_modules).max(8);
    let qr_pixels = total_modules * module_size;
    let margin = 36u32;
    let canvas = qr_pixels + margin * 2;
    let mut image = RgbaImage::from_pixel(canvas, canvas, Rgba([255, 255, 255, 255]));

    let qr_left = margin as i32;
    let qr_top = margin as i32;
    for y in 0..module_count {
        for x in 0..module_count {
            if !code.get_module(x, y) {
                continue;
            }
            let left = qr_left + (x + quiet_zone) * module_size as i32;
            let top = qr_top + (y + quiet_zone) * module_size as i32;
            fill_rect(
                &mut image,
                left.max(0) as u32,
                top.max(0) as u32,
                module_size,
                module_size,
                dark,
            );
        }
    }

    image.save(&path).map_err(|err| err.to_string())
}

fn write_labeled_qr_png(
    export_path: &str,
    wallet_name: &str,
    subtitle: &str,
    footer: &str,
    payload: &str,
) -> Result<(), String> {
    let path = normalize_png_export_path(export_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let code = QrCode::encode_text(payload, QrCodeEcc::Medium)
        .map_err(|_| String::from("Unable to encode QR payload"))?;
    let module_count = code.size();
    if module_count <= 0 {
        return Err(String::from("QR payload produced an empty matrix"));
    }

    let quiet_zone = 4;
    let total_modules = (module_count + quiet_zone * 2).max(1) as u32;
    let target_qr_pixels = 520u32;
    let module_size = (target_qr_pixels / total_modules).max(8);
    let qr_pixels = total_modules * module_size;
    let horizontal_margin = 36u32;
    let vertical_margin = 24u32;
    let header_height = 116u32;
    let footer_height = 72u32;
    let width = (qr_pixels + horizontal_margin * 2).max(720);
    let height = header_height + qr_pixels + footer_height + vertical_margin * 2;
    let mut image = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));

    fill_rect(
        &mut image,
        0,
        0,
        width,
        header_height,
        Rgba(QR_EXPORT_HEADER_BG),
    );
    fill_rect(
        &mut image,
        0,
        header_height.saturating_sub(6),
        width,
        6,
        Rgba(QR_EXPORT_RULE_RED),
    );

    let title = wallet_name.trim();
    let title = if title.is_empty() {
        "Atho Wallet"
    } else {
        title
    };
    let title = widgets::elide_text(title, 34);
    let subtitle = widgets::elide_text(subtitle.trim(), 52);
    let footer = widgets::elide_text(footer.trim(), 52);

    let font = FontArc::try_from_slice(include_bytes!("../assets/fonts/RobotoMono-Bold.ttf"))
        .map_err(|_| String::from("Failed to load QR export font"))?;
    draw_text_mut(
        &mut image,
        Rgba(QR_EXPORT_TEXT_DARK),
        28,
        20,
        PxScale::from(34.0),
        &font,
        &title,
    );
    draw_text_mut(
        &mut image,
        Rgba(QR_EXPORT_MODULE_RED),
        30,
        64,
        PxScale::from(20.0),
        &font,
        &subtitle,
    );
    draw_text_mut(
        &mut image,
        Rgba(QR_EXPORT_TEXT_DARK),
        30,
        height.saturating_sub(48) as i32,
        PxScale::from(18.0),
        &font,
        &footer,
    );

    let qr_left = ((width - qr_pixels) / 2) as i32;
    let qr_top = (header_height + vertical_margin) as i32;
    fill_rect(
        &mut image,
        qr_left.max(0) as u32,
        qr_top.max(0) as u32,
        qr_pixels,
        qr_pixels,
        Rgba([255, 255, 255, 255]),
    );

    for y in 0..module_count {
        for x in 0..module_count {
            if !code.get_module(x, y) {
                continue;
            }
            let left = qr_left + (x + quiet_zone) * module_size as i32;
            let top = qr_top + (y + quiet_zone) * module_size as i32;
            fill_rect(
                &mut image,
                left.max(0) as u32,
                top.max(0) as u32,
                module_size,
                module_size,
                Rgba(QR_EXPORT_MODULE_RED),
            );
        }
    }

    image
        .save(&path)
        .map_err(|err| format!("Failed to save QR image: {err}"))
}

fn fill_rect(image: &mut RgbaImage, left: u32, top: u32, width: u32, height: u32, color: Rgba<u8>) {
    let right = left.saturating_add(width).min(image.width());
    let bottom = top.saturating_add(height).min(image.height());
    for y in top..bottom {
        for x in left..right {
            image.put_pixel(x, y, color);
        }
    }
}

fn available_mining_cores() -> u32 {
    std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(1)
        .max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MiningTemplateTip {
    height: u64,
    previous_block_hash: [u8; 48],
}

impl From<&BlockTemplate> for MiningTemplateTip {
    fn from(template: &BlockTemplate) -> Self {
        Self {
            height: template.height,
            previous_block_hash: template.previous_block_hash,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MiningTemplateStaleStatus {
    current_height: u64,
    current_tip_hash: [u8; 48],
}

fn mining_template_stale_status_for_node(
    tip: MiningTemplateTip,
    status: &NodeStatus,
) -> Option<MiningTemplateStaleStatus> {
    if status.block_count.saturating_add(1) != tip.height
        || status.tip_hash != tip.previous_block_hash
    {
        Some(MiningTemplateStaleStatus {
            current_height: status.block_count,
            current_tip_hash: status.tip_hash,
        })
    } else {
        None
    }
}

fn mining_template_stale_status(
    connection: &ReadOnlyNodeConnection,
    tip: MiningTemplateTip,
) -> Option<MiningTemplateStaleStatus> {
    match connection.request(RpcRequest::GetNodeStatus) {
        RpcResponse::NodeStatus(status) => mining_template_stale_status_for_node(tip, &status),
        _ => None,
    }
}

fn stale_mining_template_result(
    tip: MiningTemplateTip,
    status: Option<MiningTemplateStaleStatus>,
    solved_block_hash: Option<[u8; 48]>,
) -> MiningJobResult {
    MiningJobResult::StaleTemplate(MiningStaleTemplate {
        height: tip.height,
        previous_block_hash: tip.previous_block_hash,
        current_height: status.map(|status| status.current_height),
        current_tip_hash: status.map(|status| status.current_tip_hash),
        solved_block_hash,
    })
}

fn is_invalid_block_height_error(error: &RpcError) -> bool {
    error.code == atho_errors::BLK_INVALID_HEIGHT.code.as_str()
}

fn mined_block_submit_error_result(
    connection: &ReadOnlyNodeConnection,
    tip: MiningTemplateTip,
    block_hash: [u8; 48],
    error: RpcError,
) -> MiningJobResult {
    if is_invalid_block_height_error(&error) {
        let stale_status = mining_template_stale_status(connection, tip);
        if stale_status.is_some() {
            return stale_mining_template_result(tip, stale_status, Some(block_hash));
        }
    }
    MiningJobResult::Failed(error.to_string())
}

fn spawn_mining_template_watcher(
    connection: ReadOnlyNodeConnection,
    tip: MiningTemplateTip,
    user_stop_requested: Arc<AtomicBool>,
    mining_stop_requested: Arc<AtomicBool>,
    stale_detected: Arc<AtomicBool>,
    watcher_finished: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !watcher_finished.load(Ordering::Acquire) {
            if user_stop_requested.load(Ordering::Acquire) {
                mining_stop_requested.store(true, Ordering::Release);
                return;
            }

            if let Some(status) = mining_template_stale_status(&connection, tip) {
                stale_detected.store(true, Ordering::Release);
                mining_stop_requested.store(true, Ordering::Release);
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!(
                        "mining template stale height={} prev={} current_height={} current_tip={}",
                        tip.height,
                        hex::encode(tip.previous_block_hash),
                        status.current_height,
                        hex::encode(status.current_tip_hash),
                    ),
                );
                return;
            }

            let sleep_started = Instant::now();
            while sleep_started.elapsed() < MINING_TEMPLATE_WATCH_INTERVAL {
                if watcher_finished.load(Ordering::Acquire) {
                    return;
                }
                if user_stop_requested.load(Ordering::Acquire) {
                    mining_stop_requested.store(true, Ordering::Release);
                    return;
                }
                thread::sleep(MINING_TEMPLATE_WATCH_SLEEP_SLICE);
            }
        }
    })
}

fn mine_via_connection(
    connection: crate::connection::ReadOnlyNodeConnection,
    cores: u32,
    backend: MiningBackendKind,
    reward_script: Option<Vec<u8>>,
    stop_requested: Arc<AtomicBool>,
    mining_stop_requested: Arc<AtomicBool>,
) -> MiningJobResult {
    if stop_requested.load(Ordering::Acquire) || mining_stop_requested.load(Ordering::Acquire) {
        return MiningJobResult::Cancelled;
    }
    let _ = atho_node::dev::append_log("atho-qt", "requesting block template");
    let template = match connection.request(RpcRequest::GetBlockTemplate) {
        RpcResponse::BlockTemplate(template) => template,
        RpcResponse::Error(err) => return MiningJobResult::Failed(err.to_string()),
        other => return MiningJobResult::Failed(format!("unexpected rpc response: {other:?}")),
    };
    let template_tip = MiningTemplateTip::from(&template);
    let block = if let Some(reward_script) = reward_script.as_deref() {
        rewrite_reward_script(&template.block, reward_script)
    } else {
        template.block.clone()
    };

    let controller = MiningController::new(backend, cores);
    let _ = atho_node::dev::append_log(
        "atho-qt",
        &format!(
            "solving block height={} cores={} requested_backend={} txs={} reward_bound={}",
            template.height,
            cores,
            controller.backend().label(),
            template.transaction_count,
            reward_script.is_some()
        ),
    );
    let mined_template = BlockTemplate {
        block,
        ..template.clone()
    };
    let stale_detected = Arc::new(AtomicBool::new(false));
    let watcher_finished = Arc::new(AtomicBool::new(false));
    let watcher = spawn_mining_template_watcher(
        connection.clone(),
        template_tip,
        Arc::clone(&stop_requested),
        Arc::clone(&mining_stop_requested),
        Arc::clone(&stale_detected),
        Arc::clone(&watcher_finished),
    );

    let mining_result =
        controller.mine_block_reported(mined_template, Arc::clone(&mining_stop_requested));
    watcher_finished.store(true, Ordering::Release);
    if watcher.join().is_err() {
        let _ = atho_node::dev::append_log("atho-qt", "mining template watcher panicked");
    }

    let report = match mining_result {
        Ok(report) => report,
        Err(atho_node::mining_backend::MiningBackendError::Cancelled) => {
            if stop_requested.load(Ordering::Acquire) {
                return MiningJobResult::Cancelled;
            }
            if stale_detected.load(Ordering::Acquire) {
                return stale_mining_template_result(
                    template_tip,
                    mining_template_stale_status(&connection, template_tip),
                    None,
                );
            }
            return MiningJobResult::Cancelled;
        }
        Err(err) => return MiningJobResult::Failed(err.to_string()),
    };
    if stop_requested.load(Ordering::Acquire) {
        return MiningJobResult::Cancelled;
    }
    let accelerator_label = report
        .accelerator
        .as_ref()
        .and_then(|info| info.runtime_label());
    let block = report.block;
    let block_hash = block.header.block_hash();
    let backend_used = report.backend_used.label().to_string();
    let fallback_reason = report.fallback_reason.clone();
    let stale_status = mining_template_stale_status(&connection, template_tip);
    if stale_status.is_some() || stale_detected.load(Ordering::Acquire) {
        return stale_mining_template_result(template_tip, stale_status, Some(block_hash));
    }
    match connection.request(RpcRequest::SubmitBlock(block)) {
        RpcResponse::BlockSubmitted { accepted: true, .. } => {
            MiningJobResult::Completed(MiningOutcome {
                height: template.height,
                block_hash,
                accepted: true,
                message: format!(
                    "Block {} accepted at height {}",
                    hex::encode(block_hash),
                    template.height
                ),
                backend_used: backend_used.clone(),
                accelerator_label: accelerator_label.clone(),
                fallback_reason: fallback_reason.clone(),
            })
        }
        RpcResponse::BlockSubmitted {
            accepted: false, ..
        } => MiningJobResult::Completed(MiningOutcome {
            height: template.height,
            block_hash,
            accepted: false,
            message: format!("Block {} rejected", hex::encode(block_hash)),
            backend_used,
            accelerator_label,
            fallback_reason,
        }),
        RpcResponse::Error(err) => {
            mined_block_submit_error_result(&connection, template_tip, block_hash, err)
        }
        other => MiningJobResult::Failed(format!("unexpected rpc response: {other:?}")),
    }
}

fn rewrite_reward_script(block: &Block, reward_script: &[u8]) -> Block {
    let mut transactions = block.transactions.clone();
    if let Some(coinbase) = transactions.first_mut() {
        if let Some(output) = coinbase.outputs.first_mut() {
            output.locking_script = reward_script.to_vec();
        }
    }
    let witness_root = atho_core::block::witness_root(&transactions);
    transactions = transactions
        .into_iter()
        .map(|tx| finalize_witness_commit_refs(&tx, witness_root))
        .collect();

    let mut header = block.header.clone();
    header.merkle_root = merkle_root(&transactions);
    header.witness_root = witness_root;
    let mut rebuilt = Block::new(header, transactions);
    rebuilt.fees_total_atoms = block.fees_total_atoms;
    rebuilt.fees_miner_atoms = block.fees_miner_atoms;
    rebuilt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::acquire_global_test_lock;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::pow;
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::constants::{atoms_per_atho_for_network, STANDARD_TX_CONFIRMATIONS};
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
    use atho_node::validation::{derive_sig_ref_short, derive_witness_commit_ref};
    use atho_rpc::response::{NetworkPeerDiagnostics, NetworkPeerDirection};
    use atho_storage::path::{ATHO_DATA_DIR_ENV, ATHO_WALLET_DIR_ENV};
    use std::ffi::OsString;
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn test_keypair() -> FalconKeypair {
        generate_from_seed(b"atho-qt-rewrite-reward").expect("deterministic keypair")
    }

    fn test_wallet(seed_byte: u8) -> Wallet {
        Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[seed_byte; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        )
    }

    fn build_wallet_test_spend(
        wallet: &Wallet,
        funding_utxos: &[UtxoEntry],
        recipient_digest: [u8; 32],
        amount_atoms: u64,
        include_fee_in_total: bool,
        change_address: Option<WalletAddress>,
    ) -> Transaction {
        let request = WalletSpendRequest {
            selected_utxos: funding_utxos
                .iter()
                .map(|utxo| WalletSpendUtxo {
                    previous_txid: utxo.txid,
                    output_index: utxo.output_index,
                    value_atoms: utxo.value_atoms,
                    locking_script: utxo.locking_script.clone(),
                })
                .collect(),
            recipient_digest,
            amount_atoms,
            include_fee_in_total,
            transaction_version: 1,
            lock_time: 0,
            change_address,
        };
        wallet
            .build_signed_payment_transaction(request)
            .expect("build wallet test spend")
            .transaction
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
        _lock: crate::test_support::TestLockGuard,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let lock = acquire_global_test_lock();
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock: lock,
            }
        }

        fn set_value(key: &'static str, value: &str) -> Self {
            let lock = acquire_global_test_lock();
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn temp_sandbox_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atho-qt-app-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn wait_until(
        label: &str,
        app: &mut DesktopApp,
        timeout: Duration,
        predicate: impl Fn(&DesktopApp) -> bool,
    ) {
        let started = Instant::now();
        while started.elapsed() < timeout {
            app.refresh().expect("app refresh");
            app.poll_send_job();
            app.poll_mining_job();
            app.poll_wallet_scan_job();
            app.poll_wallet_preparation_job();
            if predicate(app) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "timed out waiting for app lifecycle condition: {label}; connected={} wallet_loaded={} block_count={} best_height={} mempool={} send_status={:?} mining_status={:?} last_error={:?} mining_active={} send_active={} scan_active={}",
            app.ui_state.connected,
            app.wallet.is_some(),
            app.view_model.block_count,
            app.view_model.sync_best_height,
            app.view_model.mempool_count,
            app.send_status,
            app.mining_status,
            app.last_error,
            app.mining_job.is_some(),
            app.send_job.is_some(),
            app.wallet_scan_job.is_some(),
        );
    }

    fn wait_until_without_wallet_scan(
        label: &str,
        app: &mut DesktopApp,
        timeout: Duration,
        predicate: impl Fn(&DesktopApp) -> bool,
    ) {
        let started = Instant::now();
        while started.elapsed() < timeout {
            app.refresh_status_only_for_test().expect("app refresh");
            app.poll_send_job();
            app.poll_mining_job();
            app.poll_wallet_scan_job();
            app.poll_wallet_preparation_job();
            if app.wallet.is_some() {
                app.refresh_wallet_cache_if_needed();
            }
            if predicate(app) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "timed out waiting without wallet scan: {label}; connected={} wallet_loaded={} block_count={} best_height={} mempool={} send_status={:?} mining_status={:?} last_error={:?} mining_active={} send_active={} scan_active={}",
            app.ui_state.connected,
            app.wallet.is_some(),
            app.view_model.block_count,
            app.view_model.sync_best_height,
            app.view_model.mempool_count,
            app.send_status,
            app.mining_status,
            app.last_error,
            app.mining_job.is_some(),
            app.send_job.is_some(),
            app.wallet_scan_job.is_some(),
        );
    }

    fn submit_external_funding_tx(
        app: &ReadOnlyNodeConnection,
        recipient_digest: [u8; 32],
        value_atoms: u64,
    ) -> ([u8; 48], u64) {
        let mut funding_wallet = test_wallet(0x73);
        let funding_address = funding_wallet.checkout_receive_address();
        let mut funding_utxo = None;
        let seeded = app.with_local_system_for_test(|system| {
            system.sandbox_with_node_mut(|node| {
                let utxo = UtxoEntry::new(
                    Network::Regnet,
                    [0x6b; 48],
                    0,
                    value_atoms,
                    funding_address.payment_digest.to_vec(),
                    node.height()
                        .saturating_sub(STANDARD_TX_CONFIRMATIONS.saturating_sub(1)),
                    false,
                );
                node.dev_seed_chainstate(node.height(), node.tip_hash(), [utxo.clone()])
                    .expect("seed external utxo");
                funding_utxo = Some(utxo);
            });
        });
        assert!(seeded.is_some(), "expected local test backend");

        let funding_utxo = funding_utxo.expect("funding utxo");
        let fee_atoms = DesktopApp::estimate_fee(Network::Regnet, 1, 1, 1);
        let credited_atoms = value_atoms.saturating_sub(fee_atoms);
        let transaction = build_wallet_test_spend(
            &funding_wallet,
            std::slice::from_ref(&funding_utxo),
            recipient_digest,
            credited_atoms,
            false,
            None,
        );
        match app.request(RpcRequest::SubmitTransaction {
            transaction,
            fee_atoms,
        }) {
            RpcResponse::TransactionSubmitted(txid) => (txid, credited_atoms),
            other => panic!("unexpected funding submit response: {other:?}"),
        }
    }

    fn mine_local_block(app: &ReadOnlyNodeConnection) -> [u8; 48] {
        let mined = app.with_local_system_for_test(|system| {
            let cores = available_mining_cores().clamp(1, 4);
            system.sandbox_with_node_mut(|node| {
                node.mine_and_connect_candidate_block(&Miner::new(cores))
                    .expect("mine local block")
                    .header
                    .block_hash()
            })
        });
        mined.expect("expected local test backend")
    }

    fn mine_local_blocks(app: &ReadOnlyNodeConnection, count: u64) {
        for _ in 0..count {
            let _ = mine_local_block(app);
        }
    }

    fn witness_bytes(tx: &Transaction) -> Vec<u8> {
        let keypair = test_keypair();
        let txid = tx.txid();
        let digest = transaction_signing_digest(Network::Regnet, tx);
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &digest,
        )
        .expect("deterministic signature");
        let sig_bytes = signature.0.clone();
        let staged = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    input_index: index as u32,
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
            additional_signers: vec![],
        };
        let staged_tx = Transaction {
            witness: staged.canonical_bytes(),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx.clone()
        };
        let witness_root = staged_tx.witness_commitment_hash();
        TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    input_index: index as u32,
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: derive_witness_commit_ref(
                        &txid,
                        &witness_root,
                        index as u32,
                    ),
                })
                .collect(),
            additional_signers: vec![],
        }
        .canonical_bytes()
    }

    fn test_block() -> Block {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: atho_core::consensus::subsidy::block_subsidy_atoms(0),
                locking_script: vec![0x11, 0x22, 0x33],
            }],
            lock_time: 1,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let spend = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![4, 5, 6],
            }],
            lock_time: 2,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let spend = Transaction {
            witness: witness_bytes(&spend),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..spend
        };
        let transactions = vec![coinbase, spend];
        let header = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 2,
            previous_block_hash: [9; 48],
            merkle_root: merkle_root(&transactions),
            witness_root: witness_root(&transactions),
            timestamp: 150,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        Block::new(header, transactions)
    }

    #[test]
    fn desktop_app_refreshes_view_state() {
        let root = temp_sandbox_root("refresh-view");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        wait_until(
            "desktop app reaches connected state",
            &mut app,
            Duration::from_secs(5),
            |app| app.ui_state.connected,
        );
        assert!(app.ui_state.connected);
        assert_eq!(app.view_model.network_label, "atho-mainnet");
        assert!(matches!(
            app.launch_page,
            LaunchPage::Welcome | LaunchPage::OpenWallet
        ));
    }

    #[test]
    fn desktop_app_applies_peer_diagnostics_from_connection_status() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        app.apply_connection_status(ConnectionStatus {
            network: Network::Regnet,
            rpc_address: String::from("127.0.0.1:18445"),
            block_count: 12,
            tip_hash: [0x11; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 1,
            mempool_total_fee_atoms: 44,
            mempool_fingerprint: [0x22; 32],
            peer_count: 2,
            inbound_peer_count: 1,
            outbound_peer_count: 1,
            connecting_peer_count: 1,
            bytes_sent: 8_192,
            bytes_received: 16_384,
            peers: vec![NetworkPeerDiagnostics {
                remote_addr: String::from("74.208.219.116:56000"),
                direction: NetworkPeerDirection::Outbound,
                roles: vec![
                    String::from("OUTBOUND_PEER"),
                    String::from("FULL_RELAY_PEER"),
                    String::from("BLOCK_RELAY_PEER"),
                    String::from("SYNC_PEER"),
                    String::from("TX_RELAY_PEER"),
                    String::from("ADDR_RELAY_PEER"),
                ],
                handshake_ready: true,
                best_height: Some(12),
                protocol_version: Some(1),
                services: Some(9),
                user_agent: Some(String::from("/Atho:0.1.0/")),
                ruleset_version: Some(1),
                bytes_sent: 4_096,
                bytes_received: 12_288,
                last_send_unix: Some(1_777_416_445),
                last_receive_unix: Some(1_777_416_445),
                quality_score: Some(100),
                consecutive_failures: Some(0),
            }],
            connecting_peers: vec![NetworkPeerDiagnostics {
                remote_addr: String::from("9.9.9.9:9200"),
                direction: NetworkPeerDirection::Outbound,
                roles: vec![String::from("OUTBOUND_PEER")],
                handshake_ready: false,
                best_height: None,
                protocol_version: None,
                services: None,
                user_agent: None,
                ruleset_version: None,
                bytes_sent: 96,
                bytes_received: 0,
                last_send_unix: Some(1_777_416_445),
                last_receive_unix: None,
                quality_score: Some(85),
                consecutive_failures: Some(1),
            }],
            running: true,
            headers_synced: true,
            sync_best_height: 12,
            connected: true,
            startup_error: None,
        });
        assert_eq!(app.view_model.peer_count, 2);
        assert_eq!(app.view_model.inbound_peer_count, 1);
        assert_eq!(app.view_model.outbound_peer_count, 1);
        assert_eq!(app.view_model.connecting_peer_count, 1);
        assert_eq!(app.view_model.bytes_sent, 8_192);
        assert_eq!(app.view_model.bytes_received, 16_384);
        assert_eq!(app.view_model.peers.len(), 1);
        assert_eq!(app.view_model.peers[0].remote_addr, "74.208.219.116:56000");
        assert_eq!(app.view_model.connecting_peers.len(), 1);
        assert_eq!(
            app.view_model.connecting_peers[0].remote_addr,
            "9.9.9.9:9200"
        );
    }

    #[test]
    fn desktop_app_does_not_report_synced_when_local_height_is_behind_target() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        app.apply_connection_status(ConnectionStatus {
            network: Network::Mainnet,
            rpc_address: String::from("127.0.0.1:9210"),
            block_count: 0,
            tip_hash: [0x11; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x22; 32],
            peer_count: 1,
            inbound_peer_count: 0,
            outbound_peer_count: 1,
            connecting_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            connecting_peers: Vec::new(),
            running: true,
            headers_synced: true,
            sync_best_height: 128,
            connected: true,
            startup_error: None,
        });
        assert_eq!(app.view_model.sync_target_height(), 128);
        assert!(!app.view_model.chain_synced());
        assert_eq!(app.view_model.sync_stage, "Syncing 0/128");
    }

    #[test]
    fn parses_decimal_atho_amounts_with_commas() {
        let scale = atoms_per_atho_for_network(Network::Mainnet);
        let atoms =
            DesktopApp::parse_send_amount_atoms(InputUnit::Atho, "10,000.445444444444").unwrap();
        assert_eq!(atoms, 10_000 * scale + 445_444_444_444);
        assert_eq!(
            DesktopApp::parse_send_amount_atoms(InputUnit::Atho, "0.938449").unwrap(),
            938_449_000_000
        );
        assert_eq!(
            DesktopApp::parse_send_amount_atoms(InputUnit::Atho, "0.938449").unwrap(),
            938_449_000_000
        );
        assert_eq!(
            DesktopApp::parse_send_amount_atoms(InputUnit::Atho, "1").unwrap(),
            scale
        );
        assert_eq!(
            DesktopApp::parse_send_amount_atoms(InputUnit::Atho, "0.000000000001").unwrap(),
            1
        );
        assert!(DesktopApp::parse_send_amount_atoms(InputUnit::Atho, "0.0000000000001").is_err());
    }

    #[test]
    fn formats_atho_amounts_for_input() {
        let scale = atoms_per_atho_for_network(Network::Mainnet);
        let atoms = 10_000 * scale + 445_444_444_444;
        assert_eq!(
            DesktopApp::format_send_amount_input(InputUnit::Atho, atoms),
            "10,000.445444444444"
        );
        assert_eq!(
            DesktopApp::format_send_amount_input(InputUnit::Atho, scale),
            "1"
        );
        assert_eq!(
            DesktopApp::format_send_amount_input(InputUnit::Atho, scale),
            "1"
        );
    }

    #[test]
    fn display_preferences_persist_across_app_restart() {
        let root = temp_sandbox_root("display-preferences");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");

        let mut first = DesktopApp::new(Network::Regnet);
        first.set_display_unit(DisplayUnit::NanoAtho);
        first.set_send_input_unit(InputUnit::Atom);

        let second = DesktopApp::new(Network::Regnet);
        assert_eq!(second.display_unit(), DisplayUnit::NanoAtho);
        assert_eq!(second.send_input_unit(), InputUnit::Atom);
    }

    #[test]
    fn switching_send_input_units_reformats_existing_amount() {
        let root = temp_sandbox_root("switch-send-input-units");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let atoms = 1_250_000_000;
        app.set_send_input_unit(InputUnit::Atho);
        app.send_amount = DesktopApp::format_send_amount_input(InputUnit::Atho, atoms);

        app.set_send_input_unit(InputUnit::MilliAtho);
        assert_eq!(app.send_amount, "1.25");

        app.set_send_input_unit(InputUnit::Atom);
        assert_eq!(app.send_amount, "1,250,000,000");
    }

    #[test]
    fn recipient_address_book_persists_and_reloads() {
        let root = temp_sandbox_root("recipient-address-book");
        fs::create_dir_all(&root).expect("root");
        let wallet_root = root.join("wallet");
        fs::create_dir_all(&wallet_root).expect("wallet root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _wallet = EnvVarGuard::set_path(ATHO_WALLET_DIR_ENV, &wallet_root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");

        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x53u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let recipient = wallet.checkout_receive_address();

        let mut app = DesktopApp::new(Network::Regnet);
        app.send_to = recipient.address.clone();
        app.send_label = String::from("Test recipient");
        app.start_add_current_recipient_to_address_book();
        app.recipient_address_editor_notes = String::from("Sandbox");
        app.save_recipient_address_book_entry()
            .expect("save recipient address book entry");

        let reloaded = DesktopApp::new(Network::Regnet);
        assert_eq!(reloaded.recipient_address_book.len(), 1);
        assert_eq!(reloaded.recipient_address_book[0].label, "Test recipient");
        assert_eq!(
            reloaded.recipient_address_book[0].address,
            recipient.address
        );
    }

    #[test]
    fn startup_does_not_create_legacy_faucet_wallet_for_any_network() {
        let root = temp_sandbox_root("no-faucet-wallet");
        fs::create_dir_all(&root).expect("root");
        let wallet_root = root.join("wallet");
        fs::create_dir_all(&wallet_root).expect("wallet root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _wallet = EnvVarGuard::set_path(ATHO_WALLET_DIR_ENV, &wallet_root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");

        let _mainnet = DesktopApp::new(Network::Mainnet);
        let _testnet = DesktopApp::new(Network::Testnet);
        let _regnet = DesktopApp::new(Network::Regnet);

        assert!(!wallet_root
            .join(Network::Testnet.id())
            .join("local-testnet-faucet.datafile")
            .exists());
        assert!(!root.join("faucet-client-id.txt").exists());
    }

    #[test]
    fn storage_recovery_notice_is_network_aware_loaded_and_cleared() {
        let root = temp_sandbox_root("storage-recovery-notice");
        fs::create_dir_all(&root).expect("root");
        let wallet_root = root.join("wallet");
        fs::create_dir_all(&wallet_root).expect("wallet root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _wallet = EnvVarGuard::set_path(ATHO_WALLET_DIR_ENV, &wallet_root);

        fs::write(
            atho_storage::path::storage_recovery_notice_path(Network::Mainnet),
            "atho-mainnet storage was quarantined and rebuilt because local chain data needed recovery.\n",
        )
        .expect("write notice");
        fs::write(
            atho_storage::path::storage_recovery_notice_path(Network::Testnet),
            "atho-testnet storage was quarantined and rebuilt because local chain data needed recovery.\n",
        )
        .expect("write other-network notice");

        let mut app = DesktopApp::new(Network::Mainnet);
        app.poll_storage_recovery_notice();
        assert!(app.show_storage_recovery_notice_dialog);
        assert_eq!(
            app.storage_recovery_notice.as_deref(),
            Some("atho-mainnet storage was quarantined and rebuilt because local chain data needed recovery.")
        );
        assert!(!atho_storage::path::storage_recovery_notice_path(Network::Mainnet).exists());
        assert!(atho_storage::path::storage_recovery_notice_path(Network::Testnet).exists());

        app.dismiss_storage_recovery_notice();
        assert!(!app.show_storage_recovery_notice_dialog);
        assert!(app.storage_recovery_notice.is_none());
    }

    #[test]
    fn sync_status_dialog_auto_opens_until_hidden() {
        let mut app = DesktopApp::new(Network::Regnet);
        app.apply_connection_status(ConnectionStatus {
            network: Network::Regnet,
            rpc_address: String::from("127.0.0.1:18445"),
            block_count: 10,
            tip_hash: [0x11; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x22; 32],
            peer_count: 1,
            inbound_peer_count: 0,
            outbound_peer_count: 1,
            connecting_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            connecting_peers: Vec::new(),
            running: true,
            headers_synced: false,
            sync_best_height: 20,
            connected: true,
            startup_error: None,
        });
        assert!(app.show_sync_status_dialog);
        app.show_sync_status_dialog = false;
        app.sync_status_hidden_until_synced = true;

        app.apply_connection_status(ConnectionStatus {
            network: Network::Regnet,
            rpc_address: String::from("127.0.0.1:18445"),
            block_count: 20,
            tip_hash: [0x12; 48],
            tip_timestamp: 1_777_416_500,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x23; 32],
            peer_count: 1,
            inbound_peer_count: 0,
            outbound_peer_count: 1,
            connecting_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            connecting_peers: Vec::new(),
            running: true,
            headers_synced: true,
            sync_best_height: 20,
            connected: true,
            startup_error: None,
        });
        assert!(!app.sync_status_hidden_until_synced);
    }

    #[test]
    fn poll_send_job_updates_progress_and_submission_details() {
        let mut app = DesktopApp::new(Network::Mainnet);
        let (sender, receiver) = mpsc::channel();
        app.send_job = Some(SendJob {
            started_at: Instant::now(),
            stage: SendProgressStage::Preparing,
            receiver,
        });
        sender
            .send(SendJobEvent::Progress {
                stage: SendProgressStage::FinalizingProof,
            })
            .expect("progress");
        app.poll_send_job();
        assert!(app.send_job.is_some());
        assert_eq!(app.send_status, "Finalizing anti-spam proof…");

        sender
            .send(SendJobEvent::Finished(Ok(SendOutcome {
                fee_atoms: 650,
                txid: [0x22; 48],
                tx_pow_nonce: 123,
                tx_pow_bits: 19,
            })))
            .expect("finished");
        app.poll_send_job();
        assert!(app.send_job.is_none());
        assert!(app.send_status.contains("Accepted locally"));
        assert!(app.send_status.contains("relay pending"));
        assert!(app.send_status.contains("send proof 19 bits @ nonce 123"));
        assert!(app.send_fee.contains("650 atoms"));
    }

    #[test]
    fn build_signed_spend_transaction_omits_dust_change_outputs() {
        let mut wallet = test_wallet(0x6d);
        let spend_address = wallet.checkout_receive_address();
        let change_address = wallet.checkout_change_address();
        let funding = UtxoEntry::new(
            Network::Regnet,
            [0x31; 48],
            0,
            10_000,
            spend_address.payment_digest.to_vec(),
            24,
            false,
        );

        let tx = build_wallet_test_spend(
            &wallet,
            std::slice::from_ref(&funding),
            [0x22; 32],
            9_000,
            false,
            Some(change_address),
        );

        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].value_atoms, 9_000);
    }

    #[test]
    fn select_wallet_utxos_prefers_single_output_when_remaining_change_would_be_dust() {
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x6eu8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let address = wallet.checkout_receive_address();
        let exact_fee = DesktopApp::estimate_fee(Network::Regnet, 1, 1, 1);
        let total_input_atoms = 20_000u64;
        let amount_atoms = total_input_atoms
            .saturating_sub(exact_fee)
            .saturating_sub(DUST_RELAY_VALUE_ATOMS - 1);
        let candidates = vec![SpendableWalletUtxo {
            address: address.clone(),
            utxo: UtxoEntry::new(
                Network::Regnet,
                [0x41; 48],
                0,
                total_input_atoms,
                address.payment_digest.to_vec(),
                24,
                false,
            ),
        }];

        let plan =
            DesktopApp::select_wallet_utxos(candidates, Network::Regnet, amount_atoms, false)
                .expect("selection")
                .expect("plan");

        assert_eq!(plan.output_count, 1);
    }

    #[test]
    fn select_wallet_utxos_can_span_multiple_addresses() {
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x6au8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let first = wallet.checkout_receive_address();
        let second = wallet.checkout_receive_address();

        let candidates = vec![
            SpendableWalletUtxo {
                address: first.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x11; 48],
                    0,
                    60 * atoms_per_atho_for_network(Network::Regnet),
                    first.payment_digest.to_vec(),
                    24,
                    false,
                ),
            },
            SpendableWalletUtxo {
                address: second.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x22; 48],
                    0,
                    60 * atoms_per_atho_for_network(Network::Regnet),
                    second.payment_digest.to_vec(),
                    24,
                    false,
                ),
            },
        ];

        let plan = DesktopApp::select_wallet_utxos(
            candidates,
            Network::Regnet,
            100 * atoms_per_atho_for_network(Network::Regnet),
            false,
        )
        .expect("selection");

        let plan = plan.expect("multi-address plan");
        assert_eq!(plan.utxos.len(), 2);
        assert_eq!(plan.signer_group_count, 2);
    }

    #[test]
    fn select_wallet_utxos_can_use_multiple_inputs_from_one_address() {
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x6bu8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let address = wallet.checkout_receive_address();

        let candidates = vec![
            SpendableWalletUtxo {
                address: address.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x11; 48],
                    0,
                    60 * atoms_per_atho_for_network(Network::Regnet),
                    address.payment_digest.to_vec(),
                    24,
                    false,
                ),
            },
            SpendableWalletUtxo {
                address: address.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x22; 48],
                    0,
                    60 * atoms_per_atho_for_network(Network::Regnet),
                    address.payment_digest.to_vec(),
                    24,
                    false,
                ),
            },
        ];

        let plan = DesktopApp::select_wallet_utxos(
            candidates,
            Network::Regnet,
            100 * atoms_per_atho_for_network(Network::Regnet),
            false,
        )
        .expect("selection")
        .expect("single-address plan");

        assert_eq!(plan.utxos.len(), 2);
        assert_eq!(
            plan.total_input_atoms,
            120 * atoms_per_atho_for_network(Network::Regnet)
        );
        assert_eq!(plan.signer_group_count, 1);
    }

    #[test]
    fn select_wallet_utxos_rejects_oversized_transaction_shapes() {
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x5du8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let address = wallet.checkout_receive_address();
        let input_count = 3_000usize;
        let value_atoms = 1_000_000u64;
        let candidates = (0..input_count)
            .map(|index| SpendableWalletUtxo {
                address: address.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x5d; 48],
                    index as u32,
                    value_atoms,
                    address.payment_digest.to_vec(),
                    24,
                    false,
                ),
            })
            .collect::<Vec<_>>();
        let amount_atoms = (input_count as u64 - 1) * value_atoms;

        let plan =
            DesktopApp::select_wallet_utxos(candidates, Network::Regnet, amount_atoms, false)
                .expect("selection");

        assert!(plan.is_none());
        assert!(
            DesktopApp::estimate_transaction_shape(Network::Regnet, input_count, 1, 1)
                .raw_size_bytes
                > MAX_TRANSACTION_RAW_BYTES
        );
    }

    #[test]
    fn max_sendable_amount_uses_full_wallet_balance() {
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x6cu8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let first = wallet.checkout_receive_address();
        let second = wallet.checkout_receive_address();

        let candidates = vec![
            SpendableWalletUtxo {
                address: first.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x11; 48],
                    0,
                    25 * atoms_per_atho_for_network(Network::Regnet),
                    first.payment_digest.to_vec(),
                    24,
                    false,
                ),
            },
            SpendableWalletUtxo {
                address: first.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x12; 48],
                    0,
                    20 * atoms_per_atho_for_network(Network::Regnet),
                    first.payment_digest.to_vec(),
                    24,
                    false,
                ),
            },
            SpendableWalletUtxo {
                address: second.clone(),
                utxo: UtxoEntry::new(
                    Network::Regnet,
                    [0x21; 48],
                    0,
                    60 * atoms_per_atho_for_network(Network::Regnet),
                    second.payment_digest.to_vec(),
                    24,
                    false,
                ),
            },
        ];

        let sendable =
            DesktopApp::max_single_address_sendable_atoms(candidates, Network::Regnet, false);
        assert!(sendable > 104 * atoms_per_atho_for_network(Network::Regnet));
        assert!(sendable < 105 * atoms_per_atho_for_network(Network::Regnet));
    }

    #[test]
    fn rewrite_reward_script_keeps_witness_refs_valid() {
        let block = test_block();
        let reward_script = vec![0xaa, 0xbb, 0xcc, 0xdd];
        let rewritten = rewrite_reward_script(&block, &reward_script);

        assert_eq!(
            rewritten.transactions[0].outputs[0].locking_script,
            reward_script
        );
        assert_eq!(
            rewritten.header.witness_root,
            witness_root(&rewritten.transactions)
        );
        for tx in &rewritten.transactions[1..] {
            let witness = tx.witness_payload().expect("witness payload");
            witness.for_each_signer_group(|_, _, input_refs| {
                for input_ref in input_refs {
                    assert_eq!(
                        input_ref.witness_commit_ref,
                        derive_witness_commit_ref(
                            &tx.txid(),
                            &rewritten.header.witness_root,
                            input_ref.input_index,
                        )
                    );
                }
            });
        }
    }

    #[test]
    fn receive_address_rows_mark_used_and_current_addresses() {
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Testnet,
        );
        let first = wallet.checkout_receive_address();
        let second = wallet.checkout_receive_address();
        let _change = wallet.checkout_change_address();
        let utxo = UtxoEntry::new(
            Network::Testnet,
            [9; 48],
            0,
            12_345,
            first.payment_digest.to_vec(),
            12,
            false,
        );

        let rows = DesktopApp::build_receive_address_rows(
            &wallet.all_addresses(),
            &[utxo],
            Some(first.payment_digest),
        );

        assert_eq!(rows.len(), 2);
        assert!(rows[0].used);
        assert!(rows[0].is_current);
        assert_eq!(rows[0].utxo_count, 1);
        assert_eq!(rows[0].total_atoms, 12_345);
        assert!(!rows[1].used);
        assert!(!rows[1].is_current);
        assert_eq!(rows[1].utxo_count, 0);
        assert_eq!(rows[1].total_atoms, 0);
        assert_eq!(rows[0].address.address, first.address);
        assert_eq!(rows[1].address.address, second.address);
    }

    #[test]
    fn generated_addresses_do_not_invalidate_wallet_scan_progress() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[1u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Mainnet,
        );
        let address = wallet.checkout_receive_address();

        let before = app.wallet_scan_nonce;
        app.append_generated_address(address.clone());

        assert_eq!(app.wallet_scan_nonce, before);
        assert!(app
            .wallet_address_digests_cache
            .contains(&address.payment_digest));
        assert!(app
            .wallet_addresses_cache
            .iter()
            .any(|row| row.payment_digest == address.payment_digest));
    }

    #[test]
    fn attach_wallet_generates_and_persists_initial_receive_address() {
        let root = temp_sandbox_root("attach-wallet-persist");
        fs::create_dir_all(&root).expect("root");
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x44u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let wallet_path = root.join("wallet.dat");

        assert_eq!(wallet.snapshot.receive_count, 0);
        assert!(wallet.address_book.snapshot().is_empty());

        app.attach_wallet(wallet, wallet_path.to_string_lossy().into_owned());

        let attached = app.wallet_ref().expect("wallet attached");
        assert!(!attached.address_book.snapshot().is_empty());
        assert!(attached.snapshot.receive_count >= 1);
        let current = app
            .current_receive_address
            .as_ref()
            .expect("current receive address");
        assert!(attached.address_book.snapshot().iter().any(|entry| {
            entry.network == Network::Regnet
                && entry.path.kind == current.path.kind
                && entry.path.index == current.path.index
        }));

        let persisted =
            Wallet::load_from_datafile(wallet_path.as_path(), "").expect("reload persisted wallet");
        assert!(!persisted.address_book.snapshot().is_empty());
        assert!(persisted.snapshot.receive_count >= 1);
    }

    #[test]
    fn startup_auto_open_runs_through_background_wallet_preparation() {
        let root = temp_sandbox_root("startup-auto-open");
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let wallet_path = default_wallet_path(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x27u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        DesktopApp::save_wallet_to_path(&wallet, wallet_path.to_string_lossy().as_ref(), "")
            .expect("save wallet");

        let mut app = DesktopApp::new(Network::Regnet);
        assert!(app.wallet.is_none());
        assert!(app.wallet_preparation_job.is_some());
        assert!(app.wallet_readiness_blocks_main_ui());

        wait_until(
            "background wallet auto-open completes",
            &mut app,
            Duration::from_secs(10),
            |app| app.wallet.is_some() && app.wallet_preparation_job.is_none(),
        );
        assert!(app.current_receive_address.is_some());
    }

    #[test]
    fn startup_auto_open_prefers_the_last_opened_non_default_wallet_path() {
        let root = temp_sandbox_root("startup-remembered-wallet");
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x37u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let wallet_path = wallet_slot_path(Network::Regnet, 2);
        DesktopApp::save_wallet_to_path(&wallet, wallet_path.to_string_lossy().as_ref(), "")
            .expect("save remembered wallet");

        let mut remembered = DesktopApp::new(Network::Regnet);
        remembered.attach_wallet(wallet, wallet_path.to_string_lossy().into_owned());
        drop(remembered);

        let mut app = DesktopApp::new(Network::Regnet);
        assert_eq!(
            app.open_form.wallet_path,
            wallet_path.to_string_lossy().into_owned()
        );
        wait_until(
            "remembered wallet auto-open completes",
            &mut app,
            Duration::from_secs(10),
            |app| app.wallet.is_some() && app.wallet_preparation_job.is_none(),
        );
        assert_eq!(
            app.wallet_path.as_deref(),
            Some(wallet_path.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn startup_auto_open_falls_back_to_legacy_runtime_wallet_location() {
        let root = temp_sandbox_root("startup-legacy-wallet");
        let home = root.join("home");
        let data = root.join("data");
        let wallet_root = root.join("wallet-home");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        fs::create_dir_all(&wallet_root).expect("wallet root");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _wallet = EnvVarGuard::set_path(ATHO_WALLET_DIR_ENV, &wallet_root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x41u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let wallet_path = legacy_default_wallet_path(Network::Regnet);
        DesktopApp::save_wallet_to_path(&wallet, wallet_path.to_string_lossy().as_ref(), "")
            .expect("save legacy wallet");

        let mut app = DesktopApp::new(Network::Regnet);
        assert_eq!(
            app.open_form.wallet_path,
            wallet_path.to_string_lossy().into_owned()
        );
        wait_until(
            "legacy wallet auto-open completes",
            &mut app,
            Duration::from_secs(10),
            |app| app.wallet.is_some() && app.wallet_preparation_job.is_none(),
        );
        assert_eq!(
            app.wallet_path.as_deref(),
            Some(wallet_path.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn startup_auto_open_reports_compact_local_node_timings() {
        let root = temp_sandbox_root("startup-timings");
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let wallet_path = default_wallet_path(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x71u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        DesktopApp::save_wallet_to_path(&wallet, wallet_path.to_string_lossy().as_ref(), "")
            .expect("save wallet");

        let started = Instant::now();
        let mut app = DesktopApp::new(Network::Regnet);
        let constructor_elapsed = started.elapsed();

        wait_until(
            "wallet auto-open completes",
            &mut app,
            Duration::from_secs(10),
            |app| app.wallet.is_some() && app.wallet_preparation_job.is_none(),
        );
        let wallet_prepared_elapsed = started.elapsed();

        wait_until(
            "wallet readiness gate clears",
            &mut app,
            Duration::from_secs(10),
            |app| app.wallet.is_some() && !app.wallet_readiness_blocks_main_ui(),
        );
        let shell_ready_elapsed = started.elapsed();

        eprintln!(
            "qt_startup_timings constructor_ms={} wallet_prepared_ms={} shell_ready_ms={}",
            constructor_elapsed.as_millis(),
            wallet_prepared_elapsed.as_millis(),
            shell_ready_elapsed.as_millis()
        );

        assert!(app
            .connection
            .with_local_system_for_test(|_| true)
            .is_some());
        assert!(
            app.last_error.is_none(),
            "unexpected startup error: {:?}",
            app.last_error
        );
    }

    #[test]
    fn apply_wallet_recovery_window_updates_scan_target_and_wallet_setting() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x51u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        app.attach_wallet(wallet, String::from("wallet.dat"));

        app.wallet_management_form.restore_gap_limit_input = String::from("2048");
        let message = app
            .apply_wallet_recovery_window_setting()
            .expect("apply recovery window");

        assert!(message.contains("2048"));
        assert_eq!(app.wallet_discovery_scan_limit, 2048);
        assert_eq!(app.wallet_ref().expect("wallet").restore_gap_limit(), 2048);
        assert!(app.wallet_cache_dirty);
    }

    #[test]
    fn export_wallet_backup_writes_index_metadata() {
        let root = temp_sandbox_root("backup-metadata");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x61u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let _ = wallet.checkout_receive_address();
        let _ = wallet.checkout_change_address();
        app.attach_wallet(
            wallet,
            root.join("wallet.dat").to_string_lossy().into_owned(),
        );
        app.wallet_management_form.restore_gap_limit_input = String::from("2048");
        app.apply_wallet_recovery_window_setting()
            .expect("apply recovery window");

        let backup_path = root.join("wallet.dat.backup");
        app.export_wallet_backup(backup_path.to_string_lossy().as_ref(), "")
            .expect("export backup");

        let metadata_path = PathBuf::from(format!("{}.meta.json", backup_path.to_string_lossy()));
        let metadata_bytes = fs::read(metadata_path).expect("read metadata");
        let metadata: WalletBackupMetadata =
            serde_json::from_slice(&metadata_bytes).expect("parse metadata");
        assert_eq!(metadata.network, Network::Regnet.id());
        assert_eq!(metadata.configured_recovery_window, 2048);
        assert_eq!(metadata.highest_generated_receive_index, Some(0));
        assert!(metadata.receive_keypool_queued > 0);
    }

    #[test]
    fn export_wallet_recovery_phrase_qr_writes_png_file() {
        let root = temp_sandbox_root("recovery-qr");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x63u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        app.attach_wallet(
            wallet,
            root.join("wallet.dat").to_string_lossy().into_owned(),
        );

        let export_path = root.join("wallet.recovery-phrase.qr.png");
        app.export_wallet_recovery_phrase_qr(export_path.to_string_lossy().as_ref())
            .expect("export phrase qr");

        let bytes = fs::read(export_path).expect("read qr");
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn export_receive_address_qr_writes_png_file() {
        let root = temp_sandbox_root("receive-qr");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x64u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let address = wallet.checkout_receive_address();
        app.attach_wallet(
            wallet,
            root.join("wallet.dat").to_string_lossy().into_owned(),
        );

        let export_path = root.join("wallet.receive.qr.png");
        app.export_receive_address_qr(export_path.to_string_lossy().as_ref(), &address.address)
            .expect("export receive qr");

        let bytes = fs::read(export_path).expect("read qr");
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn export_wallet_recovery_phrase_qr_appends_png_extension_when_missing() {
        let root = temp_sandbox_root("recovery-qr-no-ext");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x67u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        app.attach_wallet(
            wallet,
            root.join("wallet.dat").to_string_lossy().into_owned(),
        );

        let export_path = root.join("wallet.recovery-phrase.qr");
        app.export_wallet_recovery_phrase_qr(export_path.to_string_lossy().as_ref())
            .expect("export phrase qr");

        let bytes = fs::read(export_path.with_extension("png")).expect("read qr");
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn export_receive_address_qr_appends_png_extension_when_missing() {
        let root = temp_sandbox_root("receive-qr-no-ext");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x68u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let address = wallet.checkout_receive_address();
        app.attach_wallet(
            wallet,
            root.join("wallet.dat").to_string_lossy().into_owned(),
        );

        let export_path = root.join("wallet.receive.qr");
        app.export_receive_address_qr(export_path.to_string_lossy().as_ref(), &address.address)
            .expect("export receive qr");

        let bytes = fs::read(export_path.with_extension("png")).expect("read qr");
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn begin_wallet_switch_requests_miner_stop_before_loading_target_wallet() {
        let root = temp_sandbox_root("switch-wallet-stops-miner");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");

        let mut app = DesktopApp::new(Network::Regnet);
        let current_wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x65u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        app.attach_wallet(
            current_wallet,
            root.join("wallet-current").to_string_lossy().into_owned(),
        );

        let target_wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x66u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let target_path = root.join("wallet-target");
        DesktopApp::save_wallet_to_path(&target_wallet, target_path.to_string_lossy().as_ref(), "")
            .expect("save target wallet");

        let (_sender, receiver) = mpsc::channel();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let mining_stop_requested = Arc::new(AtomicBool::new(false));
        app.mining_job = Some(MiningJob {
            started_at: Instant::now(),
            stop_requested: Arc::clone(&stop_requested),
            mining_stop_requested: Arc::clone(&mining_stop_requested),
            receiver,
        });
        app.ui_state.generate_coins = true;
        app.mining_status = String::from("Running");

        app.begin_wallet_switch(target_path.to_string_lossy().as_ref())
            .expect("switch wallet");

        assert!(stop_requested.load(Ordering::Acquire));
        assert!(mining_stop_requested.load(Ordering::Acquire));
        assert!(!app.ui_state.generate_coins);
        assert_eq!(app.mining_status, "Stopping miner");
    }

    #[test]
    fn receive_address_qr_path_prefers_request_label_when_present() {
        let request = ReceiveRequestRecord {
            sequence: 7,
            label: String::from("Primary Miner Wallet"),
            message: String::new(),
            amount_atoms: None,
            address: String::from("T8WWyujuhXSA7KWKSeVyu9SD94bx2q2FJtAsAXC6N26uT7zTenm"),
        };

        let path = receive_address_qr_path(
            "/tmp/atho-wallet/data",
            Some(4),
            &request.address,
            Some(&request),
        );

        assert!(path.ends_with(".request-primary-miner-wallet.qr.png"));
    }

    #[test]
    fn create_receive_request_preserves_label_even_after_address_generation_clears_form() {
        let root = temp_sandbox_root("receive-request-label");
        fs::create_dir_all(&root).expect("root");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x69u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        app.attach_wallet(
            wallet,
            root.join("wallet.dat").to_string_lossy().into_owned(),
        );

        app.receive_label = String::from("Primary payout");
        app.receive_amount = String::from("50000000");
        app.receive_message = String::from("Test invoice");

        app.create_receive_request();

        let request = app.selected_receive_request().expect("selected request");
        assert_eq!(request.label, "Primary payout");
        assert_eq!(request.amount_atoms, Some(50_000_000));
        assert_eq!(request.message, "Test invoice");
        assert_eq!(app.receive_label, "");
    }

    #[test]
    fn mining_reward_script_falls_back_to_last_receive_address() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        let mut wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[2u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Mainnet,
        );
        let _first = wallet.checkout_receive_address();
        let last = wallet.checkout_receive_address();
        app.wallet = Some(wallet);
        app.current_receive_address = None;

        let reward_script = app.mining_reward_script().expect("reward script");

        assert_eq!(reward_script, last.payment_digest.to_vec());
        assert_eq!(
            app.current_receive_address
                .as_ref()
                .expect("current receive address")
                .payment_digest,
            last.payment_digest
        );
    }

    #[test]
    fn mining_is_blocked_until_chain_sync_completes() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x47u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        app.attach_wallet(wallet, String::from("wallet.dat"));
        app.ui_state.connected = true;
        app.view_model.running = true;
        app.view_model.headers_synced = false;
        app.view_model.block_count = 3;
        app.view_model.sync_best_height = 10;
        app.ui_state.generate_coins = true;

        let reason = app
            .wallet_mining_block_reason()
            .expect("mining block reason");
        assert!(reason.contains("synchronizing"));

        app.start_mining_job();

        assert!(app.mining_job.is_none());
        assert!(!app.ui_state.generate_coins);
        assert!(
            app.mining_status.contains("synchronizing"),
            "unexpected mining status: {}",
            app.mining_status
        );
    }

    #[test]
    fn stale_mined_block_submit_refreshes_template_instead_of_failing() {
        let root = temp_sandbox_root("stale-mined-block");
        let data = root.join("data");
        fs::create_dir_all(&data).expect("data");
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let connection = ReadOnlyNodeConnection::new(Network::Regnet);
        assert!(connection.status().running);
        assert_eq!(connection.status().block_count, 0);

        let template = match connection.request(RpcRequest::GetBlockTemplate) {
            RpcResponse::BlockTemplate(template) => template,
            other => panic!("expected block template, got {other:?}"),
        };
        let template_tip = MiningTemplateTip::from(&template);
        let node_status = match connection.request(RpcRequest::GetNodeStatus) {
            RpcResponse::NodeStatus(status) => status,
            other => panic!("expected node status, got {other:?}"),
        };
        assert!(mining_template_stale_status_for_node(template_tip, &node_status).is_none());

        let stale_block = Miner::new(1).solve_block(template.block);
        let stale_block_hash = stale_block.header.block_hash();
        let _competing_hash = mine_local_block(&connection);

        let node_status = match connection.request(RpcRequest::GetNodeStatus) {
            RpcResponse::NodeStatus(status) => status,
            other => panic!("expected node status, got {other:?}"),
        };
        let stale_status = mining_template_stale_status_for_node(template_tip, &node_status)
            .expect("template should be stale after a competing block advances the tip");
        assert_eq!(stale_status.current_height, 1);

        let response = connection.request(RpcRequest::SubmitBlock(stale_block));
        let error = match response {
            RpcResponse::Error(error) => error,
            other => panic!("expected stale block-height error, got {other:?}"),
        };
        assert!(is_invalid_block_height_error(&error));

        match mined_block_submit_error_result(&connection, template_tip, stale_block_hash, error) {
            MiningJobResult::StaleTemplate(stale) => {
                assert_eq!(stale.height, template_tip.height);
                assert_eq!(stale.previous_block_hash, template_tip.previous_block_hash);
                assert_eq!(stale.current_height, Some(1));
                assert_eq!(stale.solved_block_hash, Some(stale_block_hash));
            }
            other => panic!("expected stale-template retry, got {other:?}"),
        }
    }

    #[test]
    fn wallet_scan_waits_for_rpc_readiness() {
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "1");
        let _clear_local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "0");

        let mut app = DesktopApp::new(Network::Mainnet);
        app.connection =
            ReadOnlyNodeConnection::with_rpc_address(Network::Mainnet, String::from("127.0.0.1:1"));
        app.status_monitor = app
            .connection
            .spawn_status_monitor(Duration::from_millis(5));
        app.apply_connection_status(app.connection.status());
        app.wallet = Some(Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[3u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Mainnet,
        ));
        app.wallet_cache_dirty = true;
        app.last_wallet_refresh_at = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);

        app.refresh_wallet_cache_if_needed();

        assert!(!app.wallet_scan_rpc_ready());
        assert!(app.wallet_scan_job.is_none());
        assert!(app.wallet_cache_dirty);
    }

    #[test]
    fn wallet_readiness_gate_releases_when_rpc_is_not_ready() {
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "1");
        let _clear_local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "0");

        let mut app = DesktopApp::new(Network::Mainnet);
        app.connection =
            ReadOnlyNodeConnection::with_rpc_address(Network::Mainnet, String::from("127.0.0.1:1"));
        app.status_monitor = app
            .connection
            .spawn_status_monitor(Duration::from_millis(5));
        app.apply_connection_status(app.connection.status());
        app.wallet = Some(Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[4u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Mainnet,
        ));
        app.wallet_cache_dirty = true;
        app.wallet_readiness_gate_active = true;
        app.last_wallet_refresh_at = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);

        app.refresh_wallet_cache_if_needed();

        assert!(!app.wallet_scan_rpc_ready());
        assert!(app.wallet_scan_job.is_none());
        assert!(app.wallet_cache_dirty);
        assert!(!app.wallet_readiness_gate_active);
    }

    #[test]
    fn wallet_scan_timeout_releases_readiness_gate() {
        let mut app = DesktopApp::new(Network::Regnet);
        let (_sender, receiver) = mpsc::channel();
        app.wallet_scan_job = Some(WalletScanJob {
            started_at: Instant::now()
                .checked_sub(WALLET_SCAN_STALL_TIMEOUT + Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
            receiver,
        });
        app.wallet_readiness_gate_active = true;

        app.poll_wallet_scan_job();

        assert!(app.wallet_scan_job.is_none());
        assert!(!app.wallet_readiness_gate_active);
        assert_eq!(app.last_error.as_deref(), Some("wallet scan timed out"));
    }

    #[test]
    fn wallet_preparation_timeout_fails_instead_of_looping() {
        let mut app = DesktopApp::new(Network::Regnet);
        let (_sender, receiver) = mpsc::channel();
        let started_at = Instant::now()
            .checked_sub(WALLET_PREPARATION_STALL_TIMEOUT + Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        app.wallet_preparation_job = Some(WalletPreparationJob {
            started_at,
            last_progress_at: started_at,
            receiver,
        });
        app.wallet_preparation_stage = String::from("Loading wallet");

        app.poll_wallet_preparation_job();

        assert!(app.wallet_preparation_job.is_none());
        assert_eq!(app.wallet_preparation_stage, "Wallet preparation timed out");
        assert_eq!(
            app.last_error.as_deref(),
            Some("wallet preparation timed out")
        );
    }

    #[test]
    fn deferred_wallet_scan_releases_readiness_gate() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        app.wallet = Some(Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[5u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        ));
        app.wallet_cache_dirty = false;
        app.wallet_readiness_gate_active = true;
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Err(String::from(
                "wallet scan deferred: backend state changed during refresh",
            )))
            .expect("send deferred scan result");
        app.wallet_scan_job = Some(WalletScanJob {
            started_at: Instant::now(),
            receiver,
        });

        app.poll_wallet_scan_job();

        assert!(app.wallet_scan_job.is_none());
        assert!(app.wallet_cache_dirty);
        assert!(!app.wallet_readiness_gate_active);
    }

    #[test]
    fn wallet_scan_height_tracks_local_block_count() {
        let status = ConnectionStatus {
            network: Network::Mainnet,
            rpc_address: String::from("127.0.0.1:18444"),
            block_count: 12,
            tip_hash: [0; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0; 32],
            peer_count: 0,
            inbound_peer_count: 0,
            outbound_peer_count: 0,
            connecting_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            connecting_peers: Vec::new(),
            running: true,
            headers_synced: false,
            sync_best_height: 1_000,
            connected: true,
            startup_error: None,
        };

        assert_eq!(DesktopApp::wallet_scan_height(&status), 12);
    }

    #[test]
    fn wallet_cache_invalidates_when_mempool_fingerprint_changes() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        app.wallet = Some(Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[4u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Mainnet,
        ));

        let status = ConnectionStatus {
            network: Network::Mainnet,
            rpc_address: String::from("127.0.0.1:18444"),
            block_count: 12,
            tip_hash: [0x11; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 3,
            mempool_total_fee_atoms: 44,
            mempool_fingerprint: [0x22; 32],
            peer_count: 0,
            inbound_peer_count: 0,
            outbound_peer_count: 0,
            connecting_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            connecting_peers: Vec::new(),
            running: true,
            headers_synced: true,
            sync_best_height: 12,
            connected: true,
            startup_error: None,
        };
        app.apply_connection_status(status.clone());
        app.wallet_cache_dirty = false;

        let updated = ConnectionStatus {
            mempool_fingerprint: [0x33; 32],
            ..status
        };
        app.apply_connection_status(updated);

        assert!(app.wallet_cache_dirty);
    }

    #[test]
    #[ignore = "slow sandbox lifecycle soak with real PoW"]
    fn full_local_wallet_lifecycle_mines_sends_restarts_and_keeps_tip_synced() {
        let root = temp_sandbox_root("full-lifecycle");
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let wallet_path = default_wallet_path(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[9u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );

        let mut app = DesktopApp::new(Network::Regnet);
        app.attach_wallet(wallet, wallet_path.to_string_lossy().into_owned());
        app.ui_state.mining_cores = app.max_mining_cores().min(4);
        wait_until_without_wallet_scan(
            "initial wallet open",
            &mut app,
            Duration::from_secs(5),
            |app| {
                app.ui_state.connected
                    && app.wallet.is_some()
                    && !app.wallet_readiness_gate_active
                    && app.wallet_mining_block_reason().is_none()
            },
        );

        app.receive_label = String::from("Primary");
        app.receive_message = String::from("Sandbox invoice");
        app.create_receive_request();
        assert_eq!(app.requested_payments.len(), 1);
        let current_address = app
            .current_receive_address
            .as_ref()
            .expect("current receive address")
            .clone();

        app.start_mining_job();
        wait_until_without_wallet_scan(
            "first mining job finished",
            &mut app,
            Duration::from_secs(15),
            |app| app.mining_job.is_none() && app.mining_status.contains("accepted"),
        );
        wait_until_without_wallet_scan(
            "first mined block accepted",
            &mut app,
            Duration::from_secs(5),
            |app| app.view_model.block_count >= 1,
        );
        app.force_wallet_cache_refresh_for_test();
        assert!(app.wallet_balance_summary().pending_atoms > 0);

        assert_eq!(app.view_model.block_count, 1);
        assert_eq!(app.view_model.sync_best_height, 1);
        assert!(app
            .wallet_activity_rows()
            .iter()
            .any(|row| row.kind == WalletActivityKind::Mined));

        let pre_funding_ready_height = STANDARD_TX_CONFIRMATIONS.saturating_sub(1).max(1);
        if app.view_model.block_count < pre_funding_ready_height {
            mine_local_blocks(
                &app.connection,
                pre_funding_ready_height.saturating_sub(app.view_model.block_count),
            );
            wait_until_without_wallet_scan(
                "external funding source reaches standard spendability height",
                &mut app,
                Duration::from_secs(20),
                |app| app.view_model.block_count >= pre_funding_ready_height,
            );
        }

        let funding_atoms = 20 * atoms_per_atho_for_network(Network::Regnet);
        let (_, credited_funding_atoms) = submit_external_funding_tx(
            &app.connection,
            current_address.payment_digest,
            funding_atoms,
        );
        wait_until_without_wallet_scan(
            "external funding reaches mempool",
            &mut app,
            Duration::from_secs(10),
            |app| app.view_model.mempool_count >= 1,
        );
        mine_local_block(&app.connection);
        wait_until_without_wallet_scan(
            "funding block accepted",
            &mut app,
            Duration::from_secs(5),
            |app| {
                app.view_model.block_count > pre_funding_ready_height
                    && app.view_model.mempool_count == 0
            },
        );
        app.force_wallet_cache_refresh_for_test();
        // Standard transactions are intentionally not spendable immediately. Keep this lifecycle
        // test aligned with consensus by asserting the inbound payment is pending until the
        // configured confirmation threshold is crossed.
        assert!(app.wallet_balance_summary().pending_atoms >= credited_funding_atoms);
        assert!(app.wallet_balance_summary().available_atoms < credited_funding_atoms);
        assert!(app
            .wallet_activity_rows()
            .iter()
            .any(|row| row.kind == WalletActivityKind::Received));

        let maturity_target_height = pre_funding_ready_height + STANDARD_TX_CONFIRMATIONS;
        mine_local_blocks(&app.connection, STANDARD_TX_CONFIRMATIONS.saturating_sub(1));
        wait_until_without_wallet_scan(
            "funding matures under standard confirmation rules",
            &mut app,
            Duration::from_secs(20),
            |app| app.view_model.block_count >= maturity_target_height,
        );
        app.force_wallet_cache_refresh_for_test();
        assert!(app.wallet_balance_summary().available_atoms >= credited_funding_atoms);
        let available_before_send = app.wallet_balance_summary().available_atoms;

        let mut recipient_wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[7u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        let recipient = recipient_wallet.checkout_receive_address();

        app.send_to = recipient.address.clone();
        app.send_amount = DesktopApp::format_send_amount_input(
            InputUnit::Atho,
            5 * atoms_per_atho_for_network(Network::Regnet),
        );
        app.submit_send_transaction().expect("submit send");
        wait_until_without_wallet_scan(
            "send transaction accepted locally",
            &mut app,
            Duration::from_secs(20),
            |app| app.send_job.is_none() && app.send_status.contains("Accepted locally"),
        );
        wait_until_without_wallet_scan(
            "send transaction reaches mempool",
            &mut app,
            Duration::from_secs(20),
            |app| app.view_model.mempool_count >= 1,
        );

        mine_local_block(&app.connection);
        wait_until_without_wallet_scan(
            "second mined block accepted",
            &mut app,
            Duration::from_secs(5),
            |app| app.view_model.block_count > maturity_target_height,
        );
        app.force_wallet_cache_refresh_for_test();
        wait_until_without_wallet_scan(
            "sent transaction confirmed and cleared",
            &mut app,
            Duration::from_secs(10),
            |app| app.view_model.mempool_count == 0,
        );

        let expected_available = app.wallet_balance_summary().available_atoms;
        let expected_pending = app.wallet_balance_summary().pending_atoms;
        let expected_total = app.wallet_balance_summary().total_atoms;
        assert!(expected_available < available_before_send);
        assert_eq!(app.view_model.block_count, maturity_target_height + 1);
        assert_eq!(app.view_model.sync_best_height, maturity_target_height + 1);
        assert!(app
            .wallet_activity_rows()
            .iter()
            .any(|row| row.kind == WalletActivityKind::Sent));

        let persisted_wallet = app.wallet_ref().expect("wallet").clone();
        DesktopApp::save_wallet_to_path(
            &persisted_wallet,
            wallet_path.to_string_lossy().as_ref(),
            "",
        )
        .expect("save wallet for restart");

        drop(app);

        let mut reopened = DesktopApp::new(Network::Regnet);
        wait_until_without_wallet_scan(
            "reopened wallet loads",
            &mut reopened,
            Duration::from_secs(10),
            |app| app.wallet.is_some() && app.view_model.block_count == maturity_target_height + 1,
        );
        reopened.force_wallet_cache_refresh_for_test();
        assert_eq!(
            reopened.wallet_balance_summary().total_atoms,
            expected_total
        );

        assert_eq!(
            reopened.view_model.sync_best_height,
            maturity_target_height + 1
        );
        assert_eq!(
            reopened.wallet_balance_summary().available_atoms,
            expected_available
        );
        assert_eq!(
            reopened.wallet_balance_summary().pending_atoms,
            expected_pending
        );
    }

    #[test]
    fn wallet_history_uses_canonical_backend_activity_for_received_rows() {
        let root = temp_sandbox_root("wallet-history");
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let wallet_path = default_wallet_path(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x15u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );
        DesktopApp::save_wallet_to_path(&wallet, wallet_path.to_string_lossy().as_ref(), "")
            .expect("save wallet");

        let mut app = DesktopApp::new(Network::Regnet);
        wait_until("wallet open", &mut app, Duration::from_secs(5), |app| {
            app.ui_state.connected && app.wallet.is_some()
        });

        let receive_address = app
            .current_receive_address
            .as_ref()
            .expect("current receive address")
            .clone();
        for expected_height in 1..=6 {
            let _ = mine_local_block(&app.connection);
            wait_until(
                "warmup block accepted",
                &mut app,
                Duration::from_secs(5),
                |app| app.view_model.block_count >= expected_height,
            );
        }
        let (funding_txid, _) = submit_external_funding_tx(
            &app.connection,
            receive_address.payment_digest,
            9 * atoms_per_atho_for_network(Network::Regnet),
        );
        wait_until(
            "funding tx reaches mempool",
            &mut app,
            Duration::from_secs(10),
            |app| app.view_model.mempool_count >= 1,
        );

        let _ = mine_local_block(&app.connection);
        wait_until(
            "funding block accepted",
            &mut app,
            Duration::from_secs(5),
            |app| app.view_model.block_count >= 7,
        );
        app.force_wallet_cache_refresh_for_test();
        assert!(app.wallet_activity_rows().iter().any(|row| {
            row.kind == WalletActivityKind::Received
                && row.reference == widgets::short_hash(&funding_txid)
        }));
    }

    #[test]
    fn debug_console_help_runs_without_rpc_dependency() {
        let mut app = DesktopApp::new(Network::Mainnet);
        let original_send_status = app.send_status.clone();
        app.debug_console_input = String::from("help");
        app.run_debug_console_command();

        assert!(app.last_error.is_none());
        assert_eq!(app.debug_console_entries.len(), 1);
        let entry = app.debug_console_entries.last().expect("console entry");
        assert!(entry.success);
        assert_eq!(entry.command_name, "help");
        assert!(entry.output.contains("Atho RPC Commands"));
        assert!(entry.output.contains("getblockchaininfo"));
        assert_eq!(app.send_status, original_send_status);
        assert_eq!(app.debug_console_history, vec![String::from("help")]);
    }

    #[test]
    fn debug_console_history_navigation_replays_commands() {
        let mut app = DesktopApp::new(Network::Mainnet);
        app.debug_console_history = vec![
            String::from("help"),
            String::from("getstatus"),
            String::from("getblockchaininfo"),
        ];

        app.debug_console_previous_history();
        assert_eq!(app.debug_console_input, "getblockchaininfo");
        app.debug_console_previous_history();
        assert_eq!(app.debug_console_input, "getstatus");
        app.debug_console_next_history();
        assert_eq!(app.debug_console_input, "getblockchaininfo");
        app.debug_console_next_history();
        assert_eq!(app.debug_console_input, "");
    }

    #[test]
    fn debug_console_executes_status_command_against_local_node() {
        let root = temp_sandbox_root("debug-console-status");
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let _force_rpc = EnvVarGuard::set_value("ATHO_QT_FORCE_RPC", "0");

        let mut app = DesktopApp::new(Network::Regnet);
        app.debug_console_input = String::from("getstatus");
        app.run_debug_console_command();

        assert!(app.last_error.is_none());
        let entry = app.debug_console_entries.last().expect("console entry");
        assert!(entry.success);
        assert_eq!(entry.command_name, "getstatus");
        assert!(entry.output.contains("\"network\""));
        assert_eq!(entry.network_label, "atho-regnet");
    }

    #[test]
    fn debug_console_unknown_command_records_suggestions() {
        let mut app = DesktopApp::new(Network::Mainnet);
        app.debug_console_input = String::from("getblok");
        app.run_debug_console_command();

        assert!(app.last_error.is_some());
        let entry = app.debug_console_entries.last().expect("console entry");
        assert!(!entry.success);
        assert!(entry.output.contains("Did you mean"));
        assert!(entry.output.contains("getblock"));
    }
}
