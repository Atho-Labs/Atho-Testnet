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
use std::path::Path;

pub const DEFAULT_RESTORE_GAP_LIMIT: usize = 20;
pub const WALLET_DATAFILE_NAME: &str = ".datafile";

pub mod datafile;

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

#[derive(Debug)]
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
        wallet.prefill();
        wallet
    }

    pub fn restore_from_phrase(
        phrase: &str,
        passphrase: &str,
        network: Network,
    ) -> Result<Self, crate::mnemonic::MnemonicError> {
        let mnemonic = MnemonicPhrase::parse(phrase)?;
        Ok(Self::from_mnemonic(mnemonic, passphrase, network))
    }

    pub fn from_seed(seed: WalletSeed, network: Network) -> Self {
        let mut wallet = Self {
            network,
            mnemonic: None,
            hd_wallet: HdWallet::new(seed),
            keypool: Keypool::new(),
            address_book: AddressBook::new(),
            snapshot: WalletSnapshot::default(),
            restore_gap_limit: DEFAULT_RESTORE_GAP_LIMIT,
        };
        wallet.prefill();
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

    pub fn load_from_datafile(
        path: &Path,
        password: &str,
    ) -> Result<Self, datafile::WalletDatafileError> {
        datafile::WalletDataFile::load(path, password)
    }

    pub fn datafile_name() -> &'static str {
        WALLET_DATAFILE_NAME
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

    fn prefill(&mut self) {
        self.keypool.refill(
            &mut self.hd_wallet,
            self.restore_gap_limit,
            self.restore_gap_limit,
        );
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
        match kind {
            AddressKind::Receive => self.snapshot.record_receive(),
            AddressKind::Change => self.snapshot.record_change(),
        }
        address
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
            next_receive_index,
            next_change_index,
            restore_gap_limit: self.restore_gap_limit as u32,
            snapshot: self.snapshot.clone(),
            address_book: self.address_book.snapshot(),
            receive_queue,
            change_queue,
        }
    }

    pub(crate) fn from_state(
        network: Network,
        mnemonic: Option<MnemonicPhrase>,
        state: PersistedWalletState,
    ) -> Self {
        Self {
            network,
            mnemonic,
            hd_wallet: HdWallet::with_counters(
                WalletSeed(state.wallet_seed),
                state.next_receive_index,
                state.next_change_index,
            ),
            keypool: Keypool::from_snapshot(state.receive_queue, state.change_queue),
            address_book: AddressBook::from_records(state.address_book),
            snapshot: state.snapshot,
            restore_gap_limit: state.restore_gap_limit as usize,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedWalletState {
    pub wallet_seed: [u8; 32],
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
    fn wallet_passphrase_changes_root_material() {
        let a = Wallet::from_mnemonic(phrase(), "", Network::Regnet).checkout_receive_address();
        let b =
            Wallet::from_mnemonic(phrase(), "secret", Network::Regnet).checkout_receive_address();
        assert_ne!(a.payment_digest, b.payment_digest);
    }
}
