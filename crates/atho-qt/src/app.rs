use crate::connection::{ConnectionStatus, ReadOnlyNodeConnection, StatusMonitor};
use crate::error::QtError;
use crate::state::UiState;
use crate::view::ViewModel;
use atho_core::address::decode_base56_address;
use atho_core::block::{merkle_root, Block};
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::constants::{ATOMS_PER_ATHO, MIN_TX_FEE_PER_VBYTE_ATOMS};
use atho_core::crypto::hash::sha3_256;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
use atho_crypto::falcon::{
    sign, FalconKeypair, FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES,
};
use atho_node::miner::Miner;
use atho_node::validation::{
    derive_sig_ref_short, derive_witness_commit_ref, finalize_witness_commit_refs,
};
use atho_rpc::request::{RpcRequest, WalletHistoryAddress};
use atho_rpc::response::{
    MempoolSpentInput, RpcResponse, WalletActivityEntry as RpcWalletActivityEntry,
    WalletActivityKind as RpcWalletActivityKind,
};
use atho_rpc::transport::RpcClient;
use atho_storage::utxo::UtxoEntry;
use atho_wallet::hd::AddressKind;
use atho_wallet::mnemonic::{MnemonicLength, MnemonicPhrase};
use atho_wallet::wallet::datafile::WalletEncryptionMode;
use atho_wallet::wallet::{Wallet, WalletAddress, DEFAULT_RESTORE_GAP_LIMIT};
use eframe::egui;
use getrandom::getrandom;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    AddressPoolFilter, CreateWalletForm, ImportWalletForm, LaunchPage, MiningJob, MiningJobResult,
    MiningOutcome, NavTab, OpenWalletForm, ReceiveAddressRow, ReceivePageTab, ReceiveRequestRecord,
    SendJob, SendOutcome, WalletActivityKind, WalletActivityRow, WalletBalanceSummary,
    WalletManagementForm,
};

const RECEIVE_ADDRESS_LIST_LIMIT: usize = 100;
const WALLET_DISCOVERY_SCAN_STEPS: &[usize] = &[32, 128, 256, 512, 1_000];
const MIN_WALLET_DISCOVERY_SCAN_LIMIT: usize = 32;
const MAX_WALLET_DISCOVERY_SCAN_LIMIT: usize = 20_000;
const TEST_WALLET_DATAFILE_ITERATIONS: u32 = 10_000;

pub struct DesktopApp {
    pub connection: ReadOnlyNodeConnection,
    status_monitor: StatusMonitor,
    pub ui_state: UiState,
    pub view_model: ViewModel,
    wallet: Option<Wallet>,
    wallet_path: Option<String>,
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
    receive_label: String,
    receive_amount: String,
    receive_message: String,
    send_status: String,
    send_job: Option<SendJob>,
    wallet_preparation_job: Option<WalletPreparationJob>,
    wallet_preparation_stage: String,
    wallet_preparation_progress: f32,
    wallet_preparation_completed: usize,
    wallet_preparation_total: usize,
    wallet_scan_job: Option<WalletScanJob>,
    wallet_readiness_gate_active: bool,
    mining_status: String,
    mining_job: Option<MiningJob>,
    pending_mining_restart: Option<u32>,
    last_mined_block_hash: Option<[u8; 48]>,
    requested_payments: Vec<ReceiveRequestRecord>,
    selected_receive_request: Option<usize>,
    transaction_search: String,
    transaction_min_amount: String,
    transaction_date_filter: usize,
    transaction_type_filter: usize,
    show_about_dialog: bool,
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
}

#[derive(Debug, Clone)]
struct SelectedSpendPlan {
    address: WalletAddress,
    utxos: Vec<UtxoEntry>,
    total_input_atoms: u64,
    output_count: usize,
    estimated_fee_atoms: u64,
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
    receiver: mpsc::Receiver<WalletPreparationEvent>,
}

#[derive(Debug)]
enum WalletPreparationEvent {
    Progress {
        stage: String,
        completed: usize,
        total: usize,
    },
    Finished(Result<WalletPreparationOutcome, String>),
}

#[derive(Debug)]
struct WalletPreparationOutcome {
    wallet: Wallet,
    wallet_path: String,
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

impl DesktopApp {
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

        let mut app = Self {
            connection,
            status_monitor,
            ui_state: UiState {
                mining_cores: available_cores,
                rotate_coinbase_address: true,
                ..UiState::default()
            },
            view_model: ViewModel::default(),
            wallet: None,
            wallet_path: None,
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
            receive_label: String::new(),
            receive_amount: String::new(),
            receive_message: String::new(),
            send_status: String::from("Enter a destination and ATHO amount."),
            send_job: None,
            wallet_preparation_job: None,
            wallet_preparation_stage: String::new(),
            wallet_preparation_progress: 0.0,
            wallet_preparation_completed: 0,
            wallet_preparation_total: 0,
            wallet_scan_job: None,
            wallet_readiness_gate_active: false,
            mining_status: String::from("Idle"),
            mining_job: None,
            pending_mining_restart: None,
            last_mined_block_hash: None,
            requested_payments: Vec::new(),
            selected_receive_request: None,
            transaction_search: String::new(),
            transaction_min_amount: String::new(),
            transaction_date_filter: 0,
            transaction_type_filter: 0,
            show_about_dialog: false,
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
        };

        app.view_model.network_label = app.connection.network().id().to_string();
        app.view_model.sync_stage = if app.connection.has_local_node() {
            String::from("Starting node")
        } else {
            String::from("Disconnected")
        };
        app.try_open_existing_wallet_on_startup();
        app
    }

    pub fn refresh(&mut self) -> Result<(), QtError> {
        let status = self.connection.status();
        self.apply_connection_status(status);
        self.poll_wallet_preparation_job();
        if self.wallet.is_some() {
            self.wallet_cache_dirty = true;
            self.refresh_wallet_cache_if_needed();
        }
        Ok(())
    }

    fn apply_connection_status(&mut self, status: ConnectionStatus) {
        let previous_block_count = self.view_model.block_count;
        let previous_mempool_count = self.view_model.mempool_count;
        let startup_error = status.startup_error.clone();
        self.view_model.network_label = status.network.id().to_string();
        self.view_model.block_count = status.block_count;
        self.view_model.mempool_count = status.mempool_count;
        self.view_model.mempool_total_fee_atoms = status.mempool_total_fee_atoms;
        self.view_model.peer_count = status.peer_count;
        self.view_model.inbound_peer_count = status.inbound_peer_count;
        self.view_model.outbound_peer_count = status.outbound_peer_count;
        self.view_model.bytes_sent = status.bytes_sent;
        self.view_model.bytes_received = status.bytes_received;
        self.view_model.peers = status.peers.clone();
        self.view_model.running = status.running;
        self.view_model.headers_synced = status.headers_synced;
        self.view_model.sync_best_height = status.sync_best_height.max(status.block_count);
        self.view_model.sync_stage = if let Some(error) = startup_error.as_ref() {
            format!("Startup error: {error}")
        } else if status.connected {
            if status.running && status.headers_synced {
                String::from("Synced")
            } else if status.running {
                format!("Running at height {}", self.view_model.sync_best_height)
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

        if self.wallet.is_some()
            && (status.block_count != previous_block_count
                || status.mempool_count != previous_mempool_count)
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
                PathBuf::from(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| String::from("wallet.dat"))
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
        // Managed local-node mode should not drop the user into the main shell until the wallet
        // has completed its first post-open scan against a live backend.
        self.wallet_preparation_blocks_startup() || self.wallet_readiness_gate_active
    }

    pub(crate) fn clear_wallet_state(&mut self) {
        self.stop_mining_job();
        self.send_job = None;
        self.mining_job = None;
        self.pending_mining_restart = None;
        self.wallet = None;
        self.wallet_path = None;
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
        self.send_status = String::from("Enter a destination and ATHO amount.");
        self.mining_status = String::from("Idle");
        self.last_mined_block_hash = None;
        self.last_error = None;
        self.wallet_management_form.backup_password.clear();
        self.wallet_management_form.backup_password_confirm.clear();
        self.wallet_management_form.restore_gap_limit_input = DEFAULT_RESTORE_GAP_LIMIT.to_string();
        self.receive_page_tab = ReceivePageTab::RequestPayment;
        self.address_pool_filter = AddressPoolFilter::Unused;
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
            status.sync_best_height.max(status.block_count)
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
        if !status.connected || !status.running {
            return Err(String::from("wallet scan deferred: node RPC is not ready"));
        }
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
        let current_height = status.sync_best_height.max(status.block_count);
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
                if err.contains("node RPC is not ready") {
                    self.wallet_cache_dirty = true;
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
            return;
        }
        if self.last_wallet_refresh_at.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.start_wallet_scan_job();
    }

    fn wallet_scan_rpc_ready(&self) -> bool {
        self.ui_state.connected && self.view_model.running
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
            let _ = sender.send(WalletPreparationEvent::Finished(result));
        });

        self.wallet_preparation_job = Some(WalletPreparationJob {
            started_at: Instant::now(),
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
        let Some(job) = self.wallet_preparation_job.take() else {
            return;
        };

        let completed_result = loop {
            match job.receiver.try_recv() {
                Ok(WalletPreparationEvent::Progress {
                    stage,
                    completed,
                    total,
                }) => {
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
                    break result;
                }
                Err(mpsc::TryRecvError::Empty) => {
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
                self.load_or_create_wallet(outcome.wallet, outcome.wallet_path);
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
        let wallet = self
            .wallet_mut()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        wallet.set_restore_gap_limit(limit);
        self.sync_wallet_recovery_window_form();

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

    fn attach_wallet(&mut self, wallet: Wallet, wallet_path: String) {
        let has_receive = wallet
            .address_book
            .snapshot()
            .iter()
            .any(|record| record.path.kind == AddressKind::Receive);

        if !has_receive {
            // Show the first deterministic receive preview without mutating persisted wallet
            // state during startup. Real checkout still happens only when the user explicitly
            // requests a new receive/change address.
            self.current_receive_address = wallet.receive_addresses(1).into_iter().next();
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
        self.wallet_management_form.backup_password.clear();
        self.wallet_management_form.backup_password_confirm.clear();
        self.wallet_path = Some(wallet_path);
        self.wallet = Some(wallet);
        self.wallet_discovery_scan_limit = WALLET_DISCOVERY_SCAN_STEPS[0];
        self.sync_wallet_recovery_window_form();
        self.wallet_readiness_gate_active = self.connection.has_local_node();
        self.refresh_wallet_address_views();
        self.wallet_cache_dirty = true;
        self.last_wallet_refresh_at = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        self.sync_wallet_state();
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
                self.last_mined_block_hash = Some(outcome.block_hash);
                self.mining_status = format!("{} at height {}", outcome.message, outcome.height);
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!(
                        "mining outcome accepted={} height={} hash={}",
                        outcome.accepted,
                        outcome.height,
                        hex::encode(outcome.block_hash)
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
                        }
                    }
                    self.wallet_cache_dirty = true;
                }
                if self.ui_state.generate_coins {
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
                self.mining_status = String::from("Mining failed");
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
        let Some(job) = self.send_job.take() else {
            return;
        };

        match job.receiver.try_recv() {
            Ok(Ok(outcome)) => {
                self.send_fee = widgets::format_atoms(outcome.fee_atoms);
                self.send_status = outcome.message;
                self.last_error = None;
                self.wallet_cache_dirty = true;
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!("send outcome fee_atoms={}", outcome.fee_atoms),
                );
            }
            Ok(Err(err)) => {
                self.send_status = format!("Submission failed: {err}");
                self.last_error = Some(err);
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
                self.send_status = String::from("Submission worker disconnected");
                self.last_error = Some(String::from("submission worker disconnected"));
                let _ = atho_node::dev::append_log("atho-qt", "send worker disconnected");
            }
        }

        if self.send_job.is_none() {
            let elapsed = job.started_at.elapsed();
            self.send_status = format!("{} ({}s)", self.send_status, elapsed.as_secs());
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
        let cores = self.clamp_mining_cores(self.ui_state.mining_cores);
        self.ui_state.mining_cores = cores;
        let connection = self.connection.clone();
        let reward_script = self.mining_reward_script();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let stop_for_thread = Arc::clone(&stop_requested);
        self.mining_status = format!("Starting generation with {} thread(s)", cores);
        self.last_error = None;
        self.pending_mining_restart = None;
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "starting mining job rpc={} cores={} max_cores={}",
                rpc_address,
                cores,
                self.max_mining_cores()
            ),
        );

        std::thread::spawn(move || {
            let result = mine_via_connection(connection, cores, reward_script, stop_for_thread);
            let _ = sender.send(result);
        });

        self.mining_job = Some(MiningJob {
            started_at: Instant::now(),
            stop_requested,
            receiver,
        });
    }

    fn mining_reward_script(&mut self) -> Option<Vec<u8>> {
        if let Some(address) = self.current_receive_address.as_ref() {
            return Some(address.payment_digest.to_vec());
        }

        let fallback = {
            let wallet = self.wallet_ref()?;
            wallet
                .address_book
                .snapshot()
                .into_iter()
                .rev()
                .find(|record| record.path.kind == AddressKind::Receive)
                .map(|record| wallet.address_for_path(record.path))
                .or_else(|| wallet.receive_addresses(1).into_iter().next())
        };

        if let Some(address) = fallback {
            self.current_receive_address = Some(address.clone());
            return Some(address.payment_digest.to_vec());
        }

        None
    }

    fn stop_mining_job(&mut self) {
        self.pending_mining_restart = None;
        self.ui_state.generate_coins = false;
        self.last_error = None;
        if let Some(job) = &self.mining_job {
            job.stop_requested.store(true, Ordering::Release);
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
        if self.mining_job.is_some() {
            self.pending_mining_restart = Some(cores);
            if let Some(job) = &self.mining_job {
                job.stop_requested.store(true, Ordering::Release);
            }
            self.mining_status = format!("Restarting miner with {} thread(s)", cores);
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!("restart miner requested cores={cores}"),
            );
            return;
        }
        if self.ui_state.generate_coins {
            self.start_mining_job();
        }
    }

    fn generate_create_mnemonic(&mut self) -> Result<(), String> {
        let mut entropy = [0u8; 32];
        getrandom(&mut entropy).map_err(|_| String::from("failed to gather wallet entropy"))?;
        let mnemonic = MnemonicPhrase::from_entropy(&entropy, MnemonicLength::Words24)
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
        mnemonic_text: String,
        mnemonic_passphrase: String,
        wallet_path: String,
        wallet_password: String,
        stage: &'static str,
    ) {
        let network = self.connection.network();
        let wallet_path_for_job = wallet_path.clone();
        let wallet_password_for_job = wallet_password.clone();
        self.start_wallet_preparation_job(stage, move |sender| {
            let mnemonic = MnemonicPhrase::parse(&mnemonic_text).map_err(|err| err.to_string())?;
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
                wallet_path: wallet_path_for_job,
            })
        });
    }

    fn start_open_wallet_preparation(&mut self, wallet_path: String, wallet_password: String) {
        let network = self.connection.network();
        let wallet_path_for_job = wallet_path.clone();
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
            Ok(WalletPreparationOutcome {
                wallet,
                wallet_path: wallet_path_for_job,
            })
        });
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
        let metadata_path = PathBuf::from(format!("{backup_path}.meta.json"));
        let metadata_bytes = serde_json::to_vec_pretty(&metadata).map_err(|err| err.to_string())?;
        fs::write(metadata_path, metadata_bytes).map_err(|err| err.to_string())
    }

    fn change_wallet_passphrase(&self, password: &str) -> Result<(), String> {
        let wallet_path = self
            .wallet_path
            .as_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        let wallet = self
            .wallet_ref()
            .ok_or_else(|| String::from("Load or create a wallet first"))?;
        Self::save_wallet_to_path(wallet, wallet_path, password)
    }

    pub(crate) fn wallet_mnemonic_sentence(&self) -> Option<String> {
        self.wallet_ref().and_then(Wallet::mnemonic_sentence)
    }

    fn try_open_existing_wallet_on_startup(&mut self) {
        let path = default_wallet_path(self.connection.network());
        if !path.exists() {
            self.launch_page = LaunchPage::Welcome;
            return;
        }

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

    fn load_or_create_wallet(&mut self, wallet: Wallet, wallet_path: String) {
        self.clear_wallet_state();
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
        self.append_generated_address(address.clone());
        self.current_receive_address = Some(address);
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        self.send_status = String::from("Receive address generated");
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
        self.send_status = String::from("Change address generated");
    }

    fn submit_send_transaction(&mut self) -> Result<(), String> {
        if self.send_job.is_some() {
            return Err(String::from("A send submission is already in progress"));
        }

        let destination = self.send_to.trim();
        if destination.is_empty() {
            return Err(String::from("Enter a destination address"));
        }
        let amount = Self::parse_send_amount_atoms(&self.send_amount)?;
        if amount == 0 {
            return Err(String::from("Amount must be greater than zero"));
        }

        let (recipient_digest, network) =
            decode_base56_address(destination).map_err(|err| err.to_string())?;
        if network != self.connection.network() {
            return Err(format!("Address belongs to {}", network.id()));
        }

        if self.wallet_utxos_cache.is_empty() {
            return Err(String::from(
                "No cached wallet UTXOs available; refresh the wallet first",
            ));
        }
        let reserved_inputs = self.mempool_reserved_inputs();

        if self.wallet_address_index_cache.is_empty() {
            if self.wallet_scan_job.is_none() {
                self.wallet_cache_dirty = true;
                self.start_wallet_scan_job();
            }
            return Err(String::from(
                "Wallet discovery is still scanning; try again after the refresh completes",
            ));
        }

        let selected_plan = {
            let wallet_addresses = &self.wallet_addresses_cache;
            let wallet_address_index_cache = &self.wallet_address_index_cache;
            let mut grouped: HashMap<[u8; 32], (WalletAddress, Vec<UtxoEntry>)> = HashMap::new();
            for utxo in self.wallet_utxos_cache.clone() {
                if reserved_inputs.contains(&(utxo.txid, utxo.output_index)) {
                    continue;
                }
                let digest: [u8; 32] = match utxo.locking_script.as_slice().try_into() {
                    Ok(digest) => digest,
                    Err(_) => continue,
                };
                if let Some(&index) = wallet_address_index_cache.get(&digest) {
                    let address = &wallet_addresses[index];
                    grouped
                        .entry(address.payment_digest)
                        .or_insert((address.clone(), Vec::new()))
                        .1
                        .push(utxo);
                }
            }

            let Some(selected_plan) = Self::select_wallet_utxos(
                grouped.into_values().collect(),
                amount,
                self.send_include_fee_in_total,
            )?
            else {
                return Err(String::from(
                    "No spendable wallet UTXOs available; refresh to clear mempool-locked outputs",
                ));
            };
            selected_plan
        };

        let keypair = {
            let wallet = self
                .wallet_ref()
                .ok_or_else(|| String::from("Load or create a wallet first"))?;
            wallet.keypair_for_path(selected_plan.address.path)
        };
        let total_input_atoms = selected_plan.total_input_atoms;
        let output_count = selected_plan.output_count;
        let tx_lock_time = Self::transaction_lock_time_nonce();
        let change_address = if output_count == 2 {
            let address = self
                .wallet_mut()
                .ok_or_else(|| String::from("Load or create a wallet first"))?
                .checkout_change_address_with_label(None);
            self.append_generated_address(address.clone());
            Some(address)
        } else {
            None
        };
        let mut fee_atoms = selected_plan.estimated_fee_atoms;
        let mut final_transaction = None;
        for _ in 0..4 {
            let (recipient_atoms, change_atoms) = if self.send_include_fee_in_total {
                if amount <= fee_atoms {
                    return Err(String::from("Amount must exceed the network fee"));
                }
                let recipient_atoms = amount
                    .checked_sub(fee_atoms)
                    .ok_or_else(|| String::from("Amount must exceed the network fee"))?;
                let change_atoms = total_input_atoms
                    .checked_sub(amount)
                    .ok_or_else(|| String::from("selected inputs do not cover amount"))?;
                (recipient_atoms, change_atoms)
            } else if output_count == 1 {
                (amount, 0)
            } else {
                let change_atoms = total_input_atoms
                    .checked_sub(amount)
                    .and_then(|value| value.checked_sub(fee_atoms))
                    .ok_or_else(|| String::from("selected inputs do not cover amount plus fee"))?;
                (amount, change_atoms)
            };
            let transaction = Self::build_signed_spend_transaction(
                &keypair,
                &selected_plan.utxos,
                recipient_digest,
                recipient_atoms,
                change_atoms,
                change_address.clone(),
                tx_lock_time,
            )?;
            let actual_fee = transaction.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
            if actual_fee == fee_atoms {
                final_transaction = Some(transaction);
                break;
            }
            fee_atoms = actual_fee;
            final_transaction = Some(transaction);
        }
        let final_transaction = final_transaction
            .ok_or_else(|| String::from("transaction fee calculation mismatch"))?;
        fee_atoms = final_transaction.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let actual_fee = total_input_atoms
            .checked_sub(final_transaction.output_value_atoms())
            .ok_or_else(|| String::from("selected inputs do not cover amount and fee"))?;
        if actual_fee != fee_atoms {
            return Err(String::from("transaction fee calculation mismatch"));
        }
        let (sender, receiver) = mpsc::channel();
        let rpc_address = self.connection.rpc_address().to_string();
        let use_local_node = self.connection.has_local_node();
        let connection = self.connection.clone();
        let txid = final_transaction.txid();
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "submitting transaction rpc={} txid={} amount_atoms={} fee_atoms={} include_fee_total={} inputs={} outputs={}",
                rpc_address,
                hex::encode(txid),
                amount,
                fee_atoms,
                self.send_include_fee_in_total,
                selected_plan.utxos.len(),
                output_count
            ),
        );
        std::thread::spawn(move || {
            let result = if use_local_node {
                match connection.request(RpcRequest::SubmitTransaction {
                    transaction: final_transaction,
                    fee_atoms,
                }) {
                    RpcResponse::TransactionSubmitted(txid) => Ok(SendOutcome {
                        fee_atoms,
                        message: format!("Transaction submitted {}", hex::encode(txid)),
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
                        message: format!("Transaction submitted {}", hex::encode(txid)),
                    }),
                    Ok(RpcResponse::Error(err)) => Err(err.to_string()),
                    Ok(other) => Err(format!("unexpected rpc response: {other:?}")),
                    Err(err) => Err(err.to_string()),
                }
            };
            let _ = sender.send(result);
        });

        self.send_fee = widgets::format_atoms(fee_atoms);
        self.send_status = String::from("Submitting transaction...");
        self.last_error = None;
        self.send_job = Some(SendJob {
            started_at: Instant::now(),
            receiver,
        });
        Ok(())
    }

    fn select_wallet_utxos(
        grouped: Vec<(WalletAddress, Vec<UtxoEntry>)>,
        amount_atoms: u64,
        include_fee_in_total: bool,
    ) -> Result<Option<SelectedSpendPlan>, String> {
        let best = grouped
            .into_par_iter()
            .map(|(address, mut utxos)| {
                utxos.sort_by(|a, b| b.value_atoms.cmp(&a.value_atoms).then(a.txid.cmp(&b.txid)));
                let mut best: Option<SelectedSpendPlan> = None;
                let mut selected = Vec::new();
                let mut total = 0u64;
                for utxo in utxos {
                    total = total.saturating_add(utxo.value_atoms);
                    selected.push(utxo);
                    let estimate_exact_fee = Self::estimate_fee(selected.len(), 1);
                    let estimate_change_fee = Self::estimate_fee(selected.len(), 2);

                    let candidate_output_count = if include_fee_in_total {
                        if total == amount_atoms && amount_atoms > estimate_exact_fee {
                            Some(1)
                        } else if total > amount_atoms && amount_atoms > estimate_change_fee {
                            Some(2)
                        } else {
                            None
                        }
                    } else {
                        let exact_target = amount_atoms.checked_add(estimate_exact_fee);
                        let change_target = amount_atoms.checked_add(estimate_change_fee);
                        if exact_target == Some(total) {
                            Some(1)
                        } else if change_target.is_some_and(|target| total > target) {
                            Some(2)
                        } else {
                            None
                        }
                    };

                    if let Some(output_count) = candidate_output_count {
                        let estimated_fee_atoms = Self::estimate_fee(selected.len(), output_count);
                        let candidate = SelectedSpendPlan {
                            address: address.clone(),
                            utxos: selected.clone(),
                            total_input_atoms: total,
                            output_count,
                            estimated_fee_atoms,
                        };
                        best = Self::prefer_candidate(best, candidate);
                        if output_count == 1 {
                            break;
                        }
                    }
                }
                best
            })
            .reduce(
                || None,
                |current, candidate| match (current, candidate) {
                    (None, next) => next,
                    (next, None) => next,
                    (Some(existing), Some(candidate)) => {
                        Self::prefer_candidate(Some(existing), candidate)
                    }
                },
            );

        Ok(best)
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
                        && candidate.output_count < existing.output_count)
                    || (candidate.estimated_fee_atoms == existing.estimated_fee_atoms
                        && candidate_inputs == existing_inputs
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

    fn estimate_fee(input_count: usize, output_count: usize) -> u64 {
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
        let witness = TxWitness {
            signature: vec![0; FALCON_512_SIGNATURE_BYTES],
            pubkey: vec![0; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: (0..input_count)
                .map(|_| atho_core::transaction::WitnessInputRef {
                    sig_ref_short: [0; 2],
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        }
        .canonical_bytes();
        Transaction {
            version: 1,
            inputs,
            outputs,
            lock_time: 0,
            witness,
        }
        .vsize_bytes() as u64
            * MIN_TX_FEE_PER_VBYTE_ATOMS
    }

    fn build_signed_spend_transaction(
        keypair: &FalconKeypair,
        selected_utxos: &[UtxoEntry],
        recipient_digest: [u8; 32],
        amount_atoms: u64,
        change_atoms: u64,
        change_address: Option<WalletAddress>,
        lock_time: u32,
    ) -> Result<Transaction, String> {
        let mut outputs = vec![TxOutput {
            value_atoms: amount_atoms,
            locking_script: recipient_digest.to_vec(),
        }];
        if change_atoms > 0 {
            let change = change_address.ok_or_else(|| String::from("missing change address"))?;
            outputs.push(TxOutput {
                value_atoms: change_atoms,
                locking_script: change.payment_digest.to_vec(),
            });
        }

        let inputs: Vec<TxInput> = selected_utxos
            .iter()
            .map(|utxo| TxInput {
                previous_txid: utxo.txid,
                output_index: utxo.output_index,
                unlocking_script: utxo.locking_script.clone(),
            })
            .collect();

        let mut tx = Transaction {
            version: 1,
            inputs,
            outputs,
            lock_time,
            witness: vec![],
        };
        let digest = transaction_signing_digest(&tx);
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &digest,
        )
        .map_err(|err: atho_crypto::error::CryptoError| err.to_string())?;
        let txid = tx.txid();
        let sig_bytes = signature.0.clone();
        tx.witness = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: selected_utxos
                .iter()
                .enumerate()
                .map(|(index, _utxo)| atho_core::transaction::WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        }
        .canonical_bytes();
        let witness_root = tx.witness_commitment_hash();
        let input_refs = selected_utxos
            .iter()
            .enumerate()
            .map(|(index, _utxo)| atho_core::transaction::WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                witness_commit_ref: derive_witness_commit_ref(&txid, &witness_root, index as u32),
            })
            .collect();
        tx.witness = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs,
        }
        .canonical_bytes();
        Ok(tx)
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

    pub(crate) fn format_send_amount_input(atoms: u64) -> String {
        let whole = atoms / ATOMS_PER_ATHO;
        let fractional = atoms % ATOMS_PER_ATHO;
        if fractional == 0 {
            return whole.to_string();
        }

        let mut fractional_text = format!("{fractional:08}");
        while fractional_text.ends_with('0') {
            fractional_text.pop();
        }
        format!("{whole}.{fractional_text}")
    }

    fn parse_send_amount_atoms(input: &str) -> Result<u64, String> {
        let normalized: String = input
            .trim()
            .chars()
            .filter(|ch| !ch.is_whitespace() && *ch != ',')
            .collect();
        if normalized.is_empty() {
            return Err(String::from("Enter an amount"));
        }
        if normalized.starts_with('-') {
            return Err(String::from("Amount must be greater than zero"));
        }
        let normalized = normalized.strip_prefix('+').unwrap_or(&normalized);
        let mut parts = normalized.split('.');
        let whole_text = parts.next().unwrap_or_default();
        let fractional_text = parts.next();
        if parts.next().is_some() {
            return Err(String::from("Amount may contain only one decimal point"));
        }

        let whole_atoms = if whole_text.is_empty() {
            0
        } else if whole_text.chars().all(|ch| ch.is_ascii_digit()) {
            whole_text
                .parse::<u64>()
                .map_err(|_| String::from("Amount is too large"))?
        } else {
            return Err(String::from(
                "Amount must contain only digits, commas, and one decimal point",
            ));
        };

        let fractional_atoms = match fractional_text {
            None => 0,
            Some(text) if text.is_empty() => 0,
            Some(text) => {
                if text.len() > 8 {
                    return Err(String::from("Amount supports up to 8 decimal places"));
                }
                if !text.chars().all(|ch| ch.is_ascii_digit()) {
                    return Err(String::from(
                        "Amount must contain only digits, commas, and one decimal point",
                    ));
                }
                let mut padded = text.to_string();
                while padded.len() < 8 {
                    padded.push('0');
                }
                padded
                    .parse::<u64>()
                    .map_err(|_| String::from("Amount is too large"))?
            }
        };

        let atoms = whole_atoms
            .checked_mul(ATOMS_PER_ATHO)
            .and_then(|value| value.checked_add(fractional_atoms))
            .ok_or_else(|| String::from("Amount is too large"))?;
        if atoms == 0 {
            return Err(String::from("Amount must be greater than zero"));
        }
        Ok(atoms)
    }

    fn current_receive_address_text(&self) -> String {
        self.current_receive_address
            .as_ref()
            .map(|address| address.address.clone())
            .unwrap_or_default()
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

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.theme_initialized {
            theme::install_fonts(ctx);
            self.theme_initialized = true;
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
    atho_node::dev::wallet_dir()
        .join(network.id())
        .join(Wallet::datafile_name())
}

fn alternate_wallet_path(network: Network) -> PathBuf {
    let mut path = default_wallet_path(network);
    let file_name = format!("{}.2", Wallet::datafile_name());
    path.set_file_name(file_name);
    path
}

fn backup_wallet_path(wallet_path: &str) -> String {
    format!("{wallet_path}.backup")
}

fn available_mining_cores() -> u32 {
    std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(1)
        .max(1)
}

fn mine_via_connection(
    connection: crate::connection::ReadOnlyNodeConnection,
    cores: u32,
    reward_script: Option<Vec<u8>>,
    stop_requested: Arc<AtomicBool>,
) -> MiningJobResult {
    if stop_requested.load(Ordering::Acquire) {
        return MiningJobResult::Cancelled;
    }
    let _ = atho_node::dev::append_log("atho-qt", "requesting block template");
    let template = match connection.request(RpcRequest::GetBlockTemplate) {
        RpcResponse::BlockTemplate(template) => template,
        RpcResponse::Error(err) => return MiningJobResult::Failed(err.to_string()),
        other => return MiningJobResult::Failed(format!("unexpected rpc response: {other:?}")),
    };
    let block = if let Some(reward_script) = reward_script.as_deref() {
        rewrite_reward_script(&template.block, reward_script)
    } else {
        template.block
    };

    let miner = Miner::new(cores);
    let _ = atho_node::dev::append_log(
        "atho-qt",
        &format!(
            "solving block height={} cores={} txs={} reward_bound={}",
            template.height,
            cores,
            template.transaction_count,
            reward_script.is_some()
        ),
    );
    let block = match miner.solve_block_with_cancel(block, stop_requested) {
        Ok(block) => block,
        Err(_) => return MiningJobResult::Cancelled,
    };
    let block_hash = block.header.block_hash();
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
            })
        }
        RpcResponse::BlockSubmitted {
            accepted: false, ..
        } => MiningJobResult::Completed(MiningOutcome {
            height: template.height,
            block_hash,
            accepted: false,
            message: format!("Block {} rejected", hex::encode(block_hash)),
        }),
        RpcResponse::Error(err) => MiningJobResult::Failed(err.to_string()),
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
    rebuilt.fees_burned_atoms = block.fees_burned_atoms;
    rebuilt.fees_pool_atoms = block.fees_pool_atoms;
    rebuilt.cumulative_burned_atoms = block.cumulative_burned_atoms;
    rebuilt
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::pow;
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::constants::{ATOMS_PER_ATHO, STANDARD_TX_CONFIRMATIONS};
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
    use atho_node::validation::{derive_sig_ref_short, derive_witness_commit_ref};
    use atho_rpc::response::{NetworkPeerDiagnostics, NetworkPeerDirection};
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use std::ffi::OsString;
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn test_keypair() -> FalconKeypair {
        generate_from_seed(b"atho-qt-rewrite-reward").expect("deterministic keypair")
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn set_value(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
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
            app.poll_wallet_preparation_job();
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
        let external_digest = [0x6c; 32];
        let mut funding_utxo = None;
        let seeded = app.with_local_system_for_test(|system| {
            system.sandbox_with_node_mut(|node| {
                let utxo = UtxoEntry::new(
                    Network::Regnet,
                    [0x6b; 48],
                    0,
                    value_atoms,
                    external_digest.to_vec(),
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
        let fee_atoms = DesktopApp::estimate_fee(1, 1);
        let credited_atoms = value_atoms.saturating_sub(fee_atoms);
        let transaction = DesktopApp::build_signed_spend_transaction(
            &test_keypair(),
            std::slice::from_ref(&funding_utxo),
            recipient_digest,
            credited_atoms,
            0,
            None,
            1,
        )
        .expect("build funding transaction");
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
            let cores = available_mining_cores().min(4).max(1);
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
        let digest = transaction_signing_digest(tx);
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
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        };
        let staged_tx = Transaction {
            witness: staged.canonical_bytes(),
            ..tx.clone()
        };
        let witness_root = staged_tx.witness_commitment_hash();
        TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: derive_witness_commit_ref(
                        &txid,
                        &witness_root,
                        index as u32,
                    ),
                })
                .collect(),
        }
        .canonical_bytes()
    }

    fn test_block() -> Block {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: 50 * ATOMS_PER_ATHO,
                locking_script: vec![0x11, 0x22, 0x33],
            }],
            lock_time: 1,
            witness: vec![],
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
        };
        let spend = Transaction {
            witness: witness_bytes(&spend),
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
        std::env::set_var("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        app.refresh().unwrap();
        assert!(app.ui_state.connected);
        assert_eq!(app.view_model.network_label, "atho-mainnet");
        assert!(matches!(
            app.launch_page,
            LaunchPage::Welcome | LaunchPage::OpenWallet
        ));
        std::env::remove_var("ATHO_QT_LOCAL");
    }

    #[test]
    fn desktop_app_applies_peer_diagnostics_from_connection_status() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        app.apply_connection_status(ConnectionStatus {
            network: Network::Regnet,
            rpc_address: String::from("127.0.0.1:18445"),
            block_count: 12,
            mempool_count: 1,
            mempool_total_fee_atoms: 44,
            peer_count: 2,
            inbound_peer_count: 1,
            outbound_peer_count: 1,
            bytes_sent: 8_192,
            bytes_received: 16_384,
            peers: vec![NetworkPeerDiagnostics {
                remote_addr: String::from("74.208.219.116:56000"),
                direction: NetworkPeerDirection::Outbound,
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
            running: true,
            headers_synced: true,
            sync_best_height: 12,
            connected: true,
            startup_error: None,
        });
        assert_eq!(app.view_model.peer_count, 2);
        assert_eq!(app.view_model.inbound_peer_count, 1);
        assert_eq!(app.view_model.outbound_peer_count, 1);
        assert_eq!(app.view_model.bytes_sent, 8_192);
        assert_eq!(app.view_model.bytes_received, 16_384);
        assert_eq!(app.view_model.peers.len(), 1);
        assert_eq!(app.view_model.peers[0].remote_addr, "74.208.219.116:56000");
    }

    #[test]
    fn parses_decimal_atho_amounts_with_commas() {
        let atoms = DesktopApp::parse_send_amount_atoms("10,000.44544444").unwrap();
        assert_eq!(atoms, 10_000 * ATOMS_PER_ATHO + 44_544_444);
        assert_eq!(
            DesktopApp::parse_send_amount_atoms("0.938449").unwrap(),
            93_844_900
        );
    }

    #[test]
    fn formats_atho_amounts_for_input() {
        let atoms = 10_000 * ATOMS_PER_ATHO + 44_544_444;
        assert_eq!(
            DesktopApp::format_send_amount_input(atoms),
            "10000.44544444"
        );
        assert_eq!(DesktopApp::format_send_amount_input(ATOMS_PER_ATHO), "1");
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
            for (index, input_ref) in witness.input_refs.iter().enumerate() {
                assert_eq!(
                    input_ref.witness_commit_ref,
                    derive_witness_commit_ref(
                        &tx.txid(),
                        &rewritten.header.witness_root,
                        index as u32,
                    )
                );
            }
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
        std::env::set_var("ATHO_QT_LOCAL", "1");
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
        std::env::remove_var("ATHO_QT_LOCAL");
    }

    #[test]
    fn attach_wallet_uses_receive_preview_without_mutating_wallet_state() {
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Regnet);
        let wallet = Wallet::from_mnemonic(
            MnemonicPhrase::from_entropy(&[0x44u8; 32], MnemonicLength::Words24).unwrap(),
            "",
            Network::Regnet,
        );

        assert_eq!(wallet.snapshot.receive_count, 0);
        assert!(wallet.address_book.snapshot().is_empty());

        app.attach_wallet(wallet, String::from("wallet.dat"));

        let attached = app.wallet_ref().expect("wallet attached");
        assert_eq!(attached.snapshot.receive_count, 0);
        assert!(attached.address_book.snapshot().is_empty());
        assert!(app.current_receive_address.is_some());
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
        std::env::remove_var("ATHO_QT_FORCE_RPC");

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
    fn startup_auto_open_reports_compact_local_node_timings() {
        let root = temp_sandbox_root("startup-timings");
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&data).expect("data");
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _data = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &data);
        let _local = EnvVarGuard::set_value("ATHO_QT_LOCAL", "1");
        std::env::remove_var("ATHO_QT_FORCE_RPC");

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
    fn mining_reward_script_falls_back_to_last_receive_address() {
        std::env::set_var("ATHO_QT_LOCAL", "1");
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
        std::env::remove_var("ATHO_QT_LOCAL");
    }

    #[test]
    fn wallet_scan_waits_for_rpc_readiness() {
        std::env::set_var("ATHO_QT_FORCE_RPC", "1");
        std::env::remove_var("ATHO_QT_LOCAL");

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

        std::env::remove_var("ATHO_QT_FORCE_RPC");
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
        std::env::remove_var("ATHO_QT_FORCE_RPC");

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
            |app| app.ui_state.connected && app.wallet.is_some(),
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

        let funding_atoms = 20 * ATOMS_PER_ATHO;
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
                app.view_model.block_count >= pre_funding_ready_height + 1
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
        app.send_amount = DesktopApp::format_send_amount_input(5 * ATOMS_PER_ATHO);
        app.submit_send_transaction().expect("submit send");
        wait_until_without_wallet_scan(
            "send transaction submitted",
            &mut app,
            Duration::from_secs(20),
            |app| app.send_job.is_none() && app.send_status.contains("Transaction submitted"),
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
            |app| app.view_model.block_count >= maturity_target_height + 1,
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
        std::env::remove_var("ATHO_QT_FORCE_RPC");

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
            9 * ATOMS_PER_ATHO,
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
}
