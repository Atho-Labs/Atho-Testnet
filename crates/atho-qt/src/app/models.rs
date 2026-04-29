use super::default_wallet_path;
use atho_core::network::Network;
use atho_wallet::mnemonic::DEFAULT_MNEMONIC_WORD_COUNT;
use atho_wallet::wallet::WalletAddress;
use atho_wallet::wallet::DEFAULT_RESTORE_GAP_LIMIT;
use std::sync::mpsc;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavTab {
    Overview,
    Send,
    Receive,
    Transactions,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceivePageTab {
    RequestPayment,
    AddressPool,
}

impl ReceivePageTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ReceivePageTab::RequestPayment => "Request payment",
            ReceivePageTab::AddressPool => "Address pool",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressPoolFilter {
    Unused,
    Used,
    All,
}

impl AddressPoolFilter {
    pub(crate) fn label(self) -> &'static str {
        match self {
            AddressPoolFilter::Unused => "Unused",
            AddressPoolFilter::Used => "Used",
            AddressPoolFilter::All => "All",
        }
    }

    pub(crate) fn matches(self, used: bool) -> bool {
        match self {
            AddressPoolFilter::Unused => !used,
            AddressPoolFilter::Used => used,
            AddressPoolFilter::All => true,
        }
    }

    pub(crate) fn variants() -> [AddressPoolFilter; 3] {
        [
            AddressPoolFilter::Unused,
            AddressPoolFilter::Used,
            AddressPoolFilter::All,
        ]
    }
}

impl NavTab {
    pub(crate) fn toolbar_tabs() -> [NavTab; 4] {
        [
            NavTab::Overview,
            NavTab::Send,
            NavTab::Receive,
            NavTab::Transactions,
        ]
    }

    pub(crate) fn label(self) -> &'static str {
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
pub(crate) enum LaunchPage {
    Welcome,
    CreateWallet,
    ImportWallet,
    OpenWallet,
}

#[derive(Debug)]
pub(crate) struct CreateWalletForm {
    pub(crate) wallet_path: String,
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
            wallet_path: default_wallet_path(network).to_string_lossy().into_owned(),
            encrypt_wallet: false,
            wallet_password: String::new(),
            wallet_password_confirm: String::new(),
            mnemonic_passphrase: String::new(),
            mnemonic_words: vec![String::new(); DEFAULT_MNEMONIC_WORD_COUNT],
            acknowledged_backup: false,
            show_passwords: false,
        }
    }

    pub(crate) fn reset_phrase(&mut self) {
        self.mnemonic_words = vec![String::new(); DEFAULT_MNEMONIC_WORD_COUNT];
        self.acknowledged_backup = false;
    }
}

#[derive(Debug)]
pub(crate) struct WalletManagementForm {
    pub(crate) backup_path: String,
    pub(crate) backup_password: String,
    pub(crate) backup_password_confirm: String,
    pub(crate) restore_gap_limit_input: String,
    pub(crate) show_passwords: bool,
}

impl WalletManagementForm {
    pub(crate) fn new(network: Network) -> Self {
        Self {
            backup_path: default_backup_wallet_path(network),
            backup_password: String::new(),
            backup_password_confirm: String::new(),
            restore_gap_limit_input: DEFAULT_RESTORE_GAP_LIMIT.to_string(),
            show_passwords: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ImportWalletForm {
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
            wallet_path: default_wallet_path(network).to_string_lossy().into_owned(),
            encrypt_wallet: false,
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
}

#[derive(Debug, Clone)]
pub(crate) enum MiningJobResult {
    Completed(MiningOutcome),
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
pub(crate) struct MiningJob {
    pub(crate) started_at: Instant,
    pub(crate) stop_requested: Arc<AtomicBool>,
    pub(crate) receiver: mpsc::Receiver<MiningJobResult>,
}

#[derive(Debug, Clone)]
pub(crate) struct SendOutcome {
    pub(crate) fee_atoms: u64,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) struct SendJob {
    pub(crate) started_at: Instant,
    pub(crate) receiver: mpsc::Receiver<Result<SendOutcome, String>>,
}
