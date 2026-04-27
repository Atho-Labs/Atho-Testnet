use super::default_wallet_path;
use atho_core::network::Network;
use std::sync::mpsc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavTab {
    Overview,
    Send,
    Receive,
    Transactions,
    Settings,
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
    pub(crate) mnemonic_text: String,
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
            mnemonic_text: String::new(),
            acknowledged_backup: false,
            show_passwords: false,
        }
    }

    pub(crate) fn reset_phrase(&mut self) {
        self.mnemonic_text.clear();
        self.acknowledged_backup = false;
    }
}

#[derive(Debug)]
pub(crate) struct ImportWalletForm {
    pub(crate) wallet_path: String,
    pub(crate) encrypt_wallet: bool,
    pub(crate) wallet_password: String,
    pub(crate) wallet_password_confirm: String,
    pub(crate) mnemonic_phrase: String,
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
            mnemonic_phrase: String::new(),
            mnemonic_passphrase: String::new(),
            show_passwords: false,
        }
    }
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

#[derive(Debug, Clone)]
pub(crate) struct WalletActivityRow {
    pub(crate) when: String,
    pub(crate) kind: &'static str,
    pub(crate) label: String,
    pub(crate) amount_atoms: u64,
    pub(crate) reference: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReceiveRequestRecord {
    pub(crate) sequence: usize,
    pub(crate) label: String,
    pub(crate) message: String,
    pub(crate) amount_atoms: Option<u64>,
    pub(crate) address: String,
}

#[derive(Debug, Clone)]
pub(crate) struct MiningOutcome {
    pub(crate) height: u64,
    pub(crate) block_hash: [u8; 48],
    pub(crate) accepted: bool,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) struct MiningJob {
    pub(crate) started_at: Instant,
    pub(crate) receiver: mpsc::Receiver<Result<MiningOutcome, String>>,
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
