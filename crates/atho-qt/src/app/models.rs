// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! View-model types and small UI enums shared across the desktop client.

use super::{default_wallet_name, default_wallet_path, suggested_wallet_path};
use atho_core::network::Network;
use atho_node::config::NodeConfig;
use atho_rpc::command::{CommandGroup, CommandPermission};
use atho_wallet::mnemonic::DEFAULT_MNEMONIC_WORD_COUNT;
use atho_wallet::wallet::WalletAddress;
use atho_wallet::wallet::DEFAULT_RESTORE_GAP_LIMIT;
use std::sync::mpsc;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;

/// Primary navigation tabs in the main application shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavTab {
    Overview,
    Send,
    Receive,
    Transactions,
    DebugConsole,
    Settings,
}

/// Sub-tabs within the receive page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceivePageTab {
    RequestPayment,
    AddressPool,
}

impl ReceivePageTab {
    /// Returns the button label used for the sub-tab.
    pub(crate) fn label(self) -> &'static str {
        match self {
            ReceivePageTab::RequestPayment => "Request payment",
            ReceivePageTab::AddressPool => "Address pool",
        }
    }
}

/// Filters for the receive-address pool table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressPoolFilter {
    Unused,
    Used,
    All,
}

impl AddressPoolFilter {
    /// Returns the UI label for the filter option.
    pub(crate) fn label(self) -> &'static str {
        match self {
            AddressPoolFilter::Unused => "Unused",
            AddressPoolFilter::Used => "Used",
            AddressPoolFilter::All => "All",
        }
    }

    /// Returns whether the filter should include an address with the given usage flag.
    pub(crate) fn matches(self, used: bool) -> bool {
        match self {
            AddressPoolFilter::Unused => !used,
            AddressPoolFilter::Used => used,
            AddressPoolFilter::All => true,
        }
    }

    /// Returns every selectable address-pool filter.
    pub(crate) fn variants() -> [AddressPoolFilter; 3] {
        [
            AddressPoolFilter::Unused,
            AddressPoolFilter::Used,
            AddressPoolFilter::All,
        ]
    }
}

impl NavTab {
    /// Returns the tabs shown in the primary toolbar.
    pub(crate) fn toolbar_tabs() -> [NavTab; 4] {
        [
            NavTab::Overview,
            NavTab::Send,
            NavTab::Receive,
            NavTab::Transactions,
        ]
    }

    /// Returns the visible tab label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            NavTab::Overview => "Overview",
            NavTab::Send => "Send",
            NavTab::Receive => "Receive",
            NavTab::Transactions => "Transactions",
            NavTab::DebugConsole => "Debug Console",
            NavTab::Settings => "Settings",
        }
    }
}

/// Output formatting modes for the embedded debug console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebugConsoleOutputMode {
    Pretty,
    Json,
    Table,
}

impl DebugConsoleOutputMode {
    /// Returns the short label used in the console UI.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pretty => "Pretty",
            Self::Json => "JSON",
            Self::Table => "Table",
        }
    }
}

/// Tabs within the separate node/debug window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebugWindowTab {
    Information,
    Console,
    NetworkTraffic,
    Peers,
}

impl DebugWindowTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Information => "Information",
            Self::Console => "Console",
            Self::NetworkTraffic => "Network Traffic",
            Self::Peers => "Peers",
        }
    }

    pub(crate) fn variants() -> [DebugWindowTab; 4] {
        [
            Self::Information,
            Self::Console,
            Self::NetworkTraffic,
            Self::Peers,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DebugConsoleEntry {
    pub(crate) timestamp_unix: u64,
    pub(crate) command_line: String,
    pub(crate) command_name: String,
    pub(crate) group: CommandGroup,
    pub(crate) permission: CommandPermission,
    pub(crate) dangerous: bool,
    pub(crate) success: bool,
    pub(crate) network_label: String,
    pub(crate) output: String,
    pub(crate) error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NetworkTrafficSample {
    pub(crate) timestamp_unix: u64,
    pub(crate) bytes_sent_per_second: f64,
    pub(crate) bytes_received_per_second: f64,
    pub(crate) total_bytes_sent: u64,
    pub(crate) total_bytes_received: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct SyncProgressSample {
    pub(crate) recorded_at: Instant,
    pub(crate) local_height: u64,
    pub(crate) target_height: u64,
    pub(crate) progress: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchPage {
    Welcome,
    CreateWallet,
    ImportWallet,
    OpenWallet,
}

#[derive(Debug)]
pub(crate) struct CreateWalletForm {
    pub(crate) wallet_name: String,
    pub(crate) wallet_path: String,
    pub(crate) mnemonic_word_count: usize,
    pub(crate) encrypt_wallet: bool,
    pub(crate) wallet_password: String,
    pub(crate) wallet_password_confirm: String,
    pub(crate) mnemonic_passphrase: String,
    pub(crate) mnemonic_words: Vec<String>,
    pub(crate) acknowledged_backup: bool,
    pub(crate) show_passwords: bool,
}

impl CreateWalletForm {
    pub(crate) fn new(network: Network) -> Self {
        Self {
            wallet_name: default_wallet_name(network),
            wallet_path: suggested_wallet_path(network)
                .to_string_lossy()
                .into_owned(),
            mnemonic_word_count: DEFAULT_MNEMONIC_WORD_COUNT,
            encrypt_wallet: true,
            wallet_password: String::new(),
            wallet_password_confirm: String::new(),
            mnemonic_passphrase: String::new(),
            mnemonic_words: vec![String::new(); DEFAULT_MNEMONIC_WORD_COUNT],
            acknowledged_backup: false,
            show_passwords: false,
        }
    }

    pub(crate) fn reset_phrase(&mut self) {
        self.mnemonic_words = vec![String::new(); self.mnemonic_word_count];
        self.acknowledged_backup = false;
    }

    pub(crate) fn set_mnemonic_word_count(&mut self, count: usize) {
        self.mnemonic_word_count = count;
        self.mnemonic_words = vec![String::new(); count];
        self.acknowledged_backup = false;
    }
}

#[derive(Debug)]
pub(crate) struct WalletManagementForm {
    pub(crate) backup_path: String,
    pub(crate) backup_json_path: String,
    pub(crate) backup_text_path: String,
    pub(crate) backup_phrase_qr_path: String,
    pub(crate) backup_password: String,
    pub(crate) backup_password_confirm: String,
    pub(crate) restore_gap_limit_input: String,
    pub(crate) show_passwords: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeSettingsForm {
    pub(crate) rpc_auth_enabled: bool,
    pub(crate) rpc_cookie_auth: bool,
    pub(crate) rpc_user: String,
    pub(crate) rpc_password: String,
    pub(crate) mining_reward_address: String,
    pub(crate) wallet_enabled: bool,
    pub(crate) wallet_require_encryption: bool,
    pub(crate) max_mempool_mib: String,
    pub(crate) max_mempool_transactions: String,
    pub(crate) prune_mib: String,
    pub(crate) db_cache_mib: String,
    pub(crate) max_peer_connections: String,
    pub(crate) fast_sync_enabled: bool,
    pub(crate) background_validation_enabled: bool,
    pub(crate) checkpoint_sync_enabled: bool,
    pub(crate) bootstrap_snapshot_path: String,
    pub(crate) bootstrap_snapshot_hash: String,
}

impl NodeSettingsForm {
    pub(crate) fn load(network: Network) -> Self {
        Self::from_config(&NodeConfig::from_env(network))
    }

    pub(crate) fn from_config(config: &NodeConfig) -> Self {
        Self {
            rpc_auth_enabled: config.rpc_auth.enabled,
            rpc_cookie_auth: config.rpc_auth.cookie_auth,
            rpc_user: config.rpc_auth.username.clone(),
            rpc_password: String::new(),
            mining_reward_address: config.mining_reward_address.clone(),
            wallet_enabled: config.wallet.enabled,
            wallet_require_encryption: config.wallet.require_encryption,
            max_mempool_mib: bytes_to_mib_ceil(config.mempool.max_vbytes as u64).to_string(),
            max_mempool_transactions: config.mempool.max_transactions.to_string(),
            prune_mib: bytes_to_mib_ceil(config.storage.prune_target_bytes).to_string(),
            db_cache_mib: bytes_to_mib_ceil(config.storage.db_cache_bytes).to_string(),
            max_peer_connections: config.peers.max_connections.to_string(),
            fast_sync_enabled: config.sync.fast_body_download,
            background_validation_enabled: config.sync.background_validation,
            checkpoint_sync_enabled: config.sync.checkpoint_anchored_sync,
            bootstrap_snapshot_path: config.sync.bootstrap_snapshot_path.clone(),
            bootstrap_snapshot_hash: config.sync.bootstrap_snapshot_hash.clone(),
        }
    }
}

fn bytes_to_mib_ceil(bytes: u64) -> u64 {
    bytes.saturating_add(1024 * 1024 - 1) / (1024 * 1024)
}

impl WalletManagementForm {
    pub(crate) fn new(network: Network) -> Self {
        Self {
            backup_path: default_backup_wallet_path(network),
            backup_json_path: default_backup_wallet_json_path(network),
            backup_text_path: default_backup_wallet_text_path(network),
            backup_phrase_qr_path: default_backup_wallet_phrase_qr_path(network),
            backup_password: String::new(),
            backup_password_confirm: String::new(),
            restore_gap_limit_input: DEFAULT_RESTORE_GAP_LIMIT.to_string(),
            show_passwords: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ImportWalletForm {
    pub(crate) wallet_name: String,
    pub(crate) wallet_path: String,
    pub(crate) encrypt_wallet: bool,
    pub(crate) wallet_password: String,
    pub(crate) wallet_password_confirm: String,
    pub(crate) mnemonic_words: Vec<String>,
    pub(crate) mnemonic_word_count: usize,
    pub(crate) mnemonic_passphrase: String,
    pub(crate) show_passwords: bool,
}

impl ImportWalletForm {
    pub(crate) fn new(network: Network) -> Self {
        Self {
            wallet_name: default_wallet_name(network),
            wallet_path: suggested_wallet_path(network)
                .to_string_lossy()
                .into_owned(),
            encrypt_wallet: true,
            wallet_password: String::new(),
            wallet_password_confirm: String::new(),
            mnemonic_words: vec![String::new(); DEFAULT_MNEMONIC_WORD_COUNT],
            mnemonic_word_count: DEFAULT_MNEMONIC_WORD_COUNT,
            mnemonic_passphrase: String::new(),
            show_passwords: false,
        }
    }

    pub(crate) fn set_mnemonic_word_count(&mut self, count: usize) {
        self.mnemonic_word_count = count;
        self.mnemonic_words.resize_with(count, String::new);
        self.mnemonic_words.truncate(count);
    }
}

fn default_backup_wallet_path(network: Network) -> String {
    format!("{}.backup", default_wallet_path(network).to_string_lossy())
}

fn default_backup_wallet_json_path(network: Network) -> String {
    format!(
        "{}.recovery.json",
        default_wallet_path(network).to_string_lossy()
    )
}

fn default_backup_wallet_text_path(network: Network) -> String {
    format!(
        "{}.recovery.txt",
        default_wallet_path(network).to_string_lossy()
    )
}

fn default_backup_wallet_phrase_qr_path(network: Network) -> String {
    format!(
        "{}.recovery-phrase.qr.png",
        default_wallet_path(network).to_string_lossy()
    )
}

#[derive(Debug)]
pub(crate) struct OpenWalletForm {
    pub(crate) wallet_path: String,
    pub(crate) wallet_password: String,
    pub(crate) show_password: bool,
}

impl OpenWalletForm {
    pub(crate) fn new(network: Network) -> Self {
        Self {
            wallet_path: default_wallet_path(network).to_string_lossy().into_owned(),
            wallet_password: String::new(),
            show_password: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WalletActivityRow {
    pub(crate) when: String,
    pub(crate) kind: WalletActivityKind,
    pub(crate) label: String,
    pub(crate) amount_atoms: i128,
    pub(crate) reference: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalletActivityKind {
    Mined,
    Received,
    Sent,
}

impl WalletActivityKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            WalletActivityKind::Mined => "Mined",
            WalletActivityKind::Received => "Received",
            WalletActivityKind::Sent => "Sent",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WalletBalanceSummary {
    pub(crate) available_atoms: u64,
    pub(crate) pending_atoms: u64,
    pub(crate) total_atoms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ReceiveRequestRecord {
    pub(crate) sequence: usize,
    pub(crate) label: String,
    pub(crate) message: String,
    pub(crate) amount_atoms: Option<u64>,
    pub(crate) address: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiveAddressRow {
    pub(crate) address: WalletAddress,
    pub(crate) used: bool,
    pub(crate) utxo_count: usize,
    pub(crate) total_atoms: u64,
    pub(crate) is_current: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct MiningOutcome {
    pub(crate) height: u64,
    pub(crate) block_hash: [u8; 48],
    pub(crate) accepted: bool,
    pub(crate) message: String,
    pub(crate) backend_used: String,
    pub(crate) accelerator_label: Option<String>,
    pub(crate) fallback_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct MiningStaleTemplate {
    pub(crate) height: u64,
    pub(crate) previous_block_hash: [u8; 48],
    pub(crate) current_height: Option<u64>,
    pub(crate) current_tip_hash: Option<[u8; 48]>,
    pub(crate) solved_block_hash: Option<[u8; 48]>,
}

#[derive(Debug, Clone)]
pub(crate) enum MiningJobResult {
    Completed(MiningOutcome),
    StaleTemplate(MiningStaleTemplate),
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
pub(crate) struct MiningJob {
    pub(crate) started_at: Instant,
    pub(crate) stop_requested: Arc<AtomicBool>,
    pub(crate) mining_stop_requested: Arc<AtomicBool>,
    pub(crate) receiver: mpsc::Receiver<MiningJobResult>,
}

#[derive(Debug, Clone)]
pub(crate) struct SendOutcome {
    pub(crate) fee_atoms: u64,
    pub(crate) txid: [u8; 48],
    pub(crate) tx_pow_nonce: u64,
    pub(crate) tx_pow_bits: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendProgressStage {
    Preparing,
    Signing,
    FinalizingProof,
    Broadcasting,
}

impl SendProgressStage {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing transaction…",
            Self::Signing => "Signing transaction…",
            Self::FinalizingProof => "Finalizing anti-spam proof…",
            Self::Broadcasting => "Submitting transaction to node…",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum SendJobEvent {
    Progress { stage: SendProgressStage },
    Finished(Result<SendOutcome, String>),
}

#[derive(Debug)]
pub(crate) struct SendJob {
    pub(crate) started_at: Instant,
    pub(crate) stage: SendProgressStage,
    pub(crate) receiver: mpsc::Receiver<SendJobEvent>,
}
