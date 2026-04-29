use crate::address_book::AddressBook;
use crate::hd::{AddressKind, DerivationPath, HdWallet, WalletSeed};
use crate::keypool::Keypool;
use crate::mnemonic::MnemonicPhrase;
use crate::snapshot::WalletSnapshot;
use atho_core::address::address_parts_from_public_key;
use atho_core::crypto::hash::{sha3_256, sha3_384};
use atho_core::network::Network;
use atho_crypto::falcon::{self, FalconKeypair};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

pub const DEFAULT_RESTORE_GAP_LIMIT: usize = 1_000;
pub const WALLET_DATAFILE_NAME: &str = ".datafile";

pub mod datafile;
pub use datafile::WalletDatafileMetadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletAddress {
    pub network: Network,
    pub path: DerivationPath,
    pub visible_prefix: char,
    pub address: String,
    pub hashed_public_key: String,
    pub public_key: Vec<u8>,
    pub payment_digest: [u8; 32],
    pub checksum: [u8; 4],
}

#[derive(Debug, Clone)]
pub struct Wallet {
    pub network: Network,
    pub mnemonic: Option<MnemonicPhrase>,
    pub hd_wallet: HdWallet,
    pub keypool: Keypool,
    pub address_book: AddressBook,
    pub snapshot: WalletSnapshot,
    pub restore_gap_limit: usize,
}

impl Wallet {
    pub fn from_mnemonic(mnemonic: MnemonicPhrase, passphrase: &str, network: Network) -> Self {
        Self::from_mnemonic_with_progress(mnemonic, passphrase, network, |_, _| {})
    }

    pub fn from_mnemonic_with_progress<F>(
        mnemonic: MnemonicPhrase,
        passphrase: &str,
        network: Network,
        progress: F,
    ) -> Self
    where
        F: FnMut(usize, usize),
    {
        let root_seed = mnemonic.root_seed(passphrase);
        let wallet_seed = WalletSeed(sha3_256(&root_seed));
        let mut wallet = Self {
            network,
            mnemonic: Some(mnemonic),
            hd_wallet: HdWallet::new(wallet_seed),
            keypool: Keypool::new(),
            address_book: AddressBook::new(),
            snapshot: WalletSnapshot::default(),
            restore_gap_limit: DEFAULT_RESTORE_GAP_LIMIT,
        };
        wallet.prefill_with_progress(progress);
        wallet
    }

    pub fn restore_from_phrase(
        phrase: &str,
        passphrase: &str,
        network: Network,
    ) -> Result<Self, crate::mnemonic::MnemonicError> {
        let mnemonic = MnemonicPhrase::parse(phrase)?;
        Ok(Self::from_mnemonic_with_progress(
            mnemonic,
            passphrase,
            network,
            |_, _| {},
        ))
    }

    pub fn from_seed(seed: WalletSeed, network: Network) -> Self {
        Self::from_seed_with_progress(seed, network, |_, _| {})
    }

    pub fn from_seed_with_progress<F>(seed: WalletSeed, network: Network, progress: F) -> Self
    where
        F: FnMut(usize, usize),
    {
        let mut wallet = Self {
            network,
            mnemonic: None,
            hd_wallet: HdWallet::new(seed),
            keypool: Keypool::new(),
            address_book: AddressBook::new(),
            snapshot: WalletSnapshot::default(),
            restore_gap_limit: DEFAULT_RESTORE_GAP_LIMIT,
        };
        wallet.prefill_with_progress(progress);
        wallet
    }

    pub fn restore_gap_limit(&self) -> usize {
        self.restore_gap_limit
    }

    pub fn save_to_datafile(
        &self,
        path: &Path,
        password: &str,
    ) -> Result<(), datafile::WalletDatafileError> {
        datafile::WalletDataFile::save(self, password, path)
    }

    #[doc(hidden)]
    pub fn save_to_datafile_with_iterations(
        &self,
        path: &Path,
        password: &str,
        iterations: u32,
    ) -> Result<(), datafile::WalletDatafileError> {
        datafile::WalletDataFile::save_with_iterations(self, password, path, iterations)
    }

    pub fn load_from_datafile(
        path: &Path,
        password: &str,
    ) -> Result<Self, datafile::WalletDatafileError> {
        datafile::WalletDataFile::load(path, password)
    }

    #[doc(hidden)]
    pub fn load_from_datafile_with_iterations(
        path: &Path,
        password: &str,
        iterations: u32,
    ) -> Result<Self, datafile::WalletDatafileError> {
        datafile::WalletDataFile::load_with_iterations(path, password, iterations)
    }

    pub fn load_from_datafile_with_progress<F>(
        path: &Path,
        password: &str,
        progress: F,
    ) -> Result<Self, datafile::WalletDatafileError>
    where
        F: FnMut(usize, usize),
    {
        datafile::WalletDataFile::load_with_progress(path, password, progress)
    }

    pub fn inspect_datafile(
        path: &Path,
    ) -> Result<WalletDatafileMetadata, datafile::WalletDatafileError> {
        datafile::WalletDataFile::inspect(path)
    }

    pub fn datafile_name() -> &'static str {
        WALLET_DATAFILE_NAME
    }

    pub fn mnemonic_phrase(&self) -> Option<&MnemonicPhrase> {
        self.mnemonic.as_ref()
    }

    pub fn mnemonic_sentence(&self) -> Option<String> {
        self.mnemonic.as_ref().map(MnemonicPhrase::as_sentence)
    }

    pub fn checkout_receive_address(&mut self) -> WalletAddress {
        self.checkout(AddressKind::Receive, None)
    }

    pub fn checkout_change_address(&mut self) -> WalletAddress {
        self.checkout(AddressKind::Change, None)
    }

    pub fn checkout_receive_address_with_label(&mut self, label: Option<String>) -> WalletAddress {
        self.checkout(AddressKind::Receive, label)
    }

    pub fn checkout_change_address_with_label(&mut self, label: Option<String>) -> WalletAddress {
        self.checkout(AddressKind::Change, label)
    }

    pub fn address_for_path(&self, path: DerivationPath) -> WalletAddress {
        self.derive_address(path)
    }

    pub fn keypair_for_path(&self, path: DerivationPath) -> FalconKeypair {
        let mut bytes = Vec::with_capacity(32 + 4 + 1 + 4 + 1 + self.network.id().len());
        bytes.extend_from_slice(self.hd_wallet.seed());
        bytes.extend_from_slice(self.network.domain_tag().as_bytes());
        bytes.extend_from_slice(&path.account.to_le_bytes());
        bytes.push(match path.kind {
            AddressKind::Receive => 0,
            AddressKind::Change => 1,
        });
        bytes.extend_from_slice(&path.index.to_le_bytes());
        falcon::generate_from_seed(&sha3_384(&bytes)).expect("falcon keygen available")
    }

    pub fn address_for_payment_digest(&self, digest: &[u8; 32]) -> Option<WalletAddress> {
        self.all_addresses()
            .into_iter()
            .find(|address| &address.payment_digest == digest)
    }

    pub fn all_addresses(&self) -> Vec<WalletAddress> {
        self.address_book
            .snapshot()
            .into_iter()
            .map(|record| self.derive_address(record.path))
            .collect()
    }

    pub fn discovery_addresses(&self) -> Vec<WalletAddress> {
        self.discovery_addresses_up_to(self.restore_gap_limit)
    }

    pub fn discovery_addresses_up_to(&self, limit: usize) -> Vec<WalletAddress> {
        if limit == 0 {
            return Vec::new();
        }

        let mut addresses = Vec::new();
        let mut seen = HashSet::new();

        for address in self.all_addresses() {
            if seen.insert(address.payment_digest) {
                addresses.push(address);
            }
        }

        let receive_preview = self.receive_addresses(limit);
        let change_preview = self.change_addresses(limit);
        let preview_len = receive_preview.len().max(change_preview.len());

        for index in 0..preview_len {
            if let Some(address) = receive_preview.get(index) {
                if seen.insert(address.payment_digest) {
                    addresses.push(address.clone());
                }
            }
            if let Some(address) = change_preview.get(index) {
                if seen.insert(address.payment_digest) {
                    addresses.push(address.clone());
                }
            }
        }

        addresses
    }

    pub fn generated_receive_addresses(&self, limit: usize) -> Vec<WalletAddress> {
        self.generated_addresses(AddressKind::Receive, limit)
    }

    pub fn generated_change_addresses(&self, limit: usize) -> Vec<WalletAddress> {
        self.generated_addresses(AddressKind::Change, limit)
    }

    pub fn receive_addresses(&self, limit: usize) -> Vec<WalletAddress> {
        self.preview_addresses(AddressKind::Receive, limit)
    }

    pub fn change_addresses(&self, limit: usize) -> Vec<WalletAddress> {
        self.preview_addresses(AddressKind::Change, limit)
    }

    #[allow(dead_code)]
    fn prefill(&mut self) {
        self.prefill_with_progress(|_, _| {});
    }

    fn prefill_with_progress<F>(&mut self, progress: F)
    where
        F: FnMut(usize, usize),
    {
        self.keypool
            .refill_to_target_with_progress(&mut self.hd_wallet, progress);
    }

    fn checkout(&mut self, kind: AddressKind, label: Option<String>) -> WalletAddress {
        let path = match kind {
            AddressKind::Receive => self
                .keypool
                .take_receive()
                .unwrap_or_else(|| self.hd_wallet.next_path(AddressKind::Receive)),
            AddressKind::Change => self
                .keypool
                .take_change()
                .unwrap_or_else(|| self.hd_wallet.next_path(AddressKind::Change)),
        };
        let address = self.derive_address(path);
        self.address_book.record(self.network, path, label);
        self.keypool.refill_to_target(&mut self.hd_wallet);
        match kind {
            AddressKind::Receive => self.snapshot.record_receive(),
            AddressKind::Change => self.snapshot.record_change(),
        }
        address
    }

    fn preview_addresses(&self, kind: AddressKind, limit: usize) -> Vec<WalletAddress> {
        if limit == 0 {
            return Vec::new();
        }
        let mut addresses = Vec::with_capacity(limit);
        let (receive_queue, change_queue) = self.keypool.snapshot();
        let queue = match kind {
            AddressKind::Receive => receive_queue,
            AddressKind::Change => change_queue,
        };
        for path in queue {
            addresses.push(self.derive_address(path));
            if addresses.len() == limit {
                break;
            }
        }

        addresses
    }

    fn generated_addresses(&self, kind: AddressKind, limit: usize) -> Vec<WalletAddress> {
        if limit == 0 {
            return Vec::new();
        }
        let mut addresses = Vec::with_capacity(limit);
        for record in self.address_book.snapshot() {
            if record.path.kind != kind {
                continue;
            }
            addresses.push(self.derive_address(record.path));
            if addresses.len() == limit {
                break;
            }
        }
        addresses
    }

    fn derive_address(&self, path: DerivationPath) -> WalletAddress {
        let keypair = self.keypair_for_path(path);
        let public_key = keypair.public_key.as_bytes().to_vec();
        let parts = address_parts_from_public_key(self.network, &public_key);
        WalletAddress {
            network: self.network,
            path,
            visible_prefix: parts.visible_prefix,
            address: parts.base56_address,
            hashed_public_key: parts.hashed_public_key,
            public_key,
            payment_digest: parts.payment_digest,
            checksum: parts.checksum,
        }
    }

    pub(crate) fn capture_state(&self) -> PersistedWalletState {
        let (next_receive_index, next_change_index) = self.hd_wallet.counters();
        let (receive_queue, change_queue) = self.keypool.snapshot();
        PersistedWalletState {
            wallet_seed: *self.hd_wallet.seed(),
            mnemonic: self.mnemonic.clone(),
            next_receive_index,
            next_change_index,
            restore_gap_limit: self.restore_gap_limit as u32,
            snapshot: self.snapshot.clone(),
            address_book: self.address_book.snapshot(),
            receive_queue,
            change_queue,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_state(
        network: Network,
        mnemonic: Option<MnemonicPhrase>,
        state: PersistedWalletState,
    ) -> Self {
        Self::from_state_with_progress(network, mnemonic, state, |_, _| {})
    }

    pub(crate) fn from_state_with_progress<F>(
        network: Network,
        mnemonic: Option<MnemonicPhrase>,
        state: PersistedWalletState,
        progress: F,
    ) -> Self
    where
        F: FnMut(usize, usize),
    {
        let mut wallet = Self {
            network,
            mnemonic: mnemonic.or(state.mnemonic),
            hd_wallet: HdWallet::with_counters(
                WalletSeed(state.wallet_seed),
                state.next_receive_index,
                state.next_change_index,
            ),
            keypool: Keypool::from_snapshot(state.receive_queue, state.change_queue),
            address_book: AddressBook::from_records(state.address_book),
            snapshot: state.snapshot,
            restore_gap_limit: state.restore_gap_limit as usize,
        };
        wallet
            .keypool
            .refill_to_target_with_progress(&mut wallet.hd_wallet, progress);
        wallet
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedWalletState {
    pub wallet_seed: [u8; 32],
    pub mnemonic: Option<MnemonicPhrase>,
    pub next_receive_index: u32,
    pub next_change_index: u32,
    pub restore_gap_limit: u32,
    pub snapshot: WalletSnapshot,
    pub address_book: Vec<crate::address_book::AddressRecord>,
    pub receive_queue: Vec<DerivationPath>,
    pub change_queue: Vec<DerivationPath>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mnemonic::{MnemonicLength, MnemonicPhrase};
    use atho_core::address::decode_base56_address;

    fn phrase() -> MnemonicPhrase {
        MnemonicPhrase::from_entropy(&[0u8; 32], MnemonicLength::Words24).unwrap()
    }

    #[test]
    fn wallet_restore_reproduces_deterministic_addresses() {
        let mut a = Wallet::from_mnemonic(phrase(), "", Network::Mainnet);
        let mut b =
            Wallet::restore_from_phrase(&phrase().as_sentence(), "", Network::Mainnet).unwrap();

        let a1 = a.checkout_receive_address();
        let b1 = b.checkout_receive_address();
        assert_eq!(a1.payment_digest, b1.payment_digest);
        assert_eq!(a1.checksum, b1.checksum);
        assert_eq!(a1.visible_prefix, 'A');
        assert_eq!(a1.address, b1.address);
        assert!(a1
            .hashed_public_key
            .starts_with(Network::Mainnet.internal_hpk_prefix()));
        let (decoded, network) = decode_base56_address(&a1.address).unwrap();
        assert_eq!(decoded, a1.payment_digest);
        assert_eq!(network, Network::Mainnet);
    }

    #[test]
    fn wallet_restore_preserves_derivation_sequence() {
        let mut original = Wallet::from_mnemonic(phrase(), "", Network::Regnet);
        let first_receive = original.checkout_receive_address();
        let second_receive = original.checkout_receive_address();
        let first_change = original.checkout_change_address();

        let mut restored =
            Wallet::restore_from_phrase(&phrase().as_sentence(), "", Network::Regnet).unwrap();
        assert_eq!(
            restored.checkout_receive_address().address,
            first_receive.address
        );
        assert_eq!(
            restored.checkout_receive_address().address,
            second_receive.address
        );
        assert_eq!(
            restored.checkout_change_address().address,
            first_change.address
        );
    }

    #[test]
    fn wallet_uses_restore_gap_limit_and_keypool_checkout() {
        let mut wallet = Wallet::from_mnemonic(phrase(), "", Network::Testnet);
        assert_eq!(wallet.restore_gap_limit(), DEFAULT_RESTORE_GAP_LIMIT);
        assert_eq!(wallet.snapshot.receive_count, 0);
        assert_eq!(wallet.snapshot.change_count, 0);

        let first = wallet.checkout_receive_address();
        let second = wallet.checkout_receive_address();
        assert_ne!(first.payment_digest, second.payment_digest);
        assert!(first
            .hashed_public_key
            .starts_with(Network::Testnet.internal_hpk_prefix()));
        assert_eq!(wallet.snapshot.receive_count, 2);
        assert_eq!(wallet.address_book.len(), 2);
        assert!(first.address.starts_with('T'));
    }

    #[test]
    fn wallet_prefills_keypool_and_exposes_receive_preview() {
        let wallet = Wallet::from_seed(WalletSeed([5; 32]), Network::Mainnet);
        let receive_preview = wallet.receive_addresses(100);
        let (receive_queue, change_queue) = wallet.keypool.snapshot();
        assert_eq!(receive_preview.len(), 100);
        assert_eq!(receive_preview[0].path.kind, AddressKind::Receive);
        assert_eq!(receive_preview[0].path.index, 0);
        assert_eq!(receive_queue.len(), crate::keypool::KEYPOOL_TARGET_SIZE);
        assert_eq!(change_queue.len(), crate::keypool::KEYPOOL_TARGET_SIZE);
    }

    #[test]
    fn wallet_discovery_addresses_include_restore_window() {
        let wallet = Wallet::from_seed(WalletSeed([7; 32]), Network::Regnet);
        let discovery = wallet.discovery_addresses_up_to(4);

        assert!(discovery.len() >= 8);
        assert_eq!(discovery[0].path.kind, AddressKind::Receive);
        assert_eq!(discovery[0].path.index, 0);
        assert_eq!(discovery[1].path.kind, AddressKind::Change);
        assert_eq!(discovery[1].path.index, 0);
        assert_eq!(discovery[2].path.kind, AddressKind::Receive);
        assert_eq!(discovery[2].path.index, 1);
        assert!(discovery
            .iter()
            .any(|address| address.path.kind == AddressKind::Change));
    }

    #[test]
    fn wallet_restore_and_preview_are_stable_for_same_phrase() {
        let phrase = phrase();
        let a = Wallet::from_mnemonic(phrase.clone(), "pass", Network::Regnet);
        let b = Wallet::restore_from_phrase(&phrase.as_sentence(), "pass", Network::Regnet)
            .expect("wallet restore");
        assert_eq!(a.receive_addresses(100), b.receive_addresses(100));
    }

    #[test]
    fn wallet_generated_receive_addresses_follow_checked_out_history() {
        let mut wallet = Wallet::from_mnemonic(phrase(), "", Network::Testnet);
        assert!(wallet.generated_receive_addresses(4).is_empty());

        let first = wallet.checkout_receive_address();
        let second = wallet.checkout_receive_address();
        let preview = wallet.generated_receive_addresses(4);

        assert_eq!(preview.len(), 2);
        assert_eq!(preview[0].address, first.address);
        assert_eq!(preview[1].address, second.address);
    }

    #[test]
    fn wallet_restore_reproduces_first_hundred_receive_and_change_addresses() {
        let phrase = phrase();
        let mut original = Wallet::from_mnemonic(phrase.clone(), "pass", Network::Mainnet);
        let mut restored =
            Wallet::restore_from_phrase(&phrase.as_sentence(), "pass", Network::Mainnet)
                .expect("wallet restore");

        let original_receive: Vec<String> = (0..100)
            .map(|_| original.checkout_receive_address().address)
            .collect();
        let restored_receive: Vec<String> = (0..100)
            .map(|_| restored.checkout_receive_address().address)
            .collect();
        assert_eq!(original_receive, restored_receive);

        let original_change: Vec<String> = (0..100)
            .map(|_| original.checkout_change_address().address)
            .collect();
        let restored_change: Vec<String> = (0..100)
            .map(|_| restored.checkout_change_address().address)
            .collect();
        assert_eq!(original_change, restored_change);
    }

    #[test]
    fn wallet_exposes_mnemonic_sentence_when_present() {
        let wallet = Wallet::from_mnemonic(phrase(), "", Network::Mainnet);
        assert!(wallet.mnemonic_phrase().is_some());
        assert_eq!(
            wallet.mnemonic_sentence(),
            Some(wallet.mnemonic_phrase().unwrap().as_sentence())
        );
    }

    #[test]
    fn wallet_passphrase_changes_root_material() {
        let a = Wallet::from_mnemonic(phrase(), "", Network::Regnet).checkout_receive_address();
        let b =
            Wallet::from_mnemonic(phrase(), "secret", Network::Regnet).checkout_receive_address();
        assert_ne!(a.payment_digest, b.payment_digest);
    }
}
