//! High-level Atho wallet model.
//!
//! The wallet combines deterministic seed derivation, HD address generation,
//! keypool reservation, address-book tracking, and encrypted datafile
//! persistence into one operator-facing model.
//!
//! WALLET SECURITY: Mnemonics and seeds remain optional so imported seed-only
//! wallets never fabricate phrase material they do not actually own.
use crate::address_book::AddressBook;
use crate::hd::{AddressKind, DerivationPath, HdWallet, WalletSeed};
use crate::keypool::Keypool;
use crate::mnemonic::MnemonicPhrase;
use crate::snapshot::WalletSnapshot;
use atho_core::address::address_parts_from_public_key;
use atho_core::consensus::signatures::{
    transaction_signing_digest_for_input_indexes, AthoSignatureDomain,
};
use atho_core::consensus::tx_policy::{minimum_required_fee_atoms, solve_transaction_pow};
use atho_core::constants::{DUST_RELAY_VALUE_ATOMS, MIN_TX_FEE_ATOMS};
use atho_core::crypto::hash::{sha3_256, sha3_384};
use atho_core::network::Network;
use atho_core::transaction::{
    Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef, WitnessSignerGroup,
};
use atho_crypto::falcon::{self, sign, FalconKeypair};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use thiserror::Error;

pub const DEFAULT_RESTORE_GAP_LIMIT: usize = 1_000;
pub const WALLET_DATAFILE_NAME: &str = ".datafile";

pub mod datafile;
pub use datafile::WalletDatafileMetadata;

/// One derived Atho address tracked by the wallet.
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

/// Minimal spendable outpoint information required to build a wallet payment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletSpendUtxo {
    pub previous_txid: [u8; 48],
    pub output_index: u32,
    pub value_atoms: u64,
    pub locking_script: Vec<u8>,
}

/// Wallet-side spend request used to build, sign, and PoW-stamp one payment transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletSpendRequest {
    pub selected_utxos: Vec<WalletSpendUtxo>,
    pub recipient_digest: [u8; 32],
    pub amount_atoms: u64,
    pub include_fee_in_total: bool,
    pub transaction_version: u16,
    pub lock_time: u32,
    pub change_address: Option<WalletAddress>,
}

/// Final wallet-built transaction ready for backend submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletBuiltSpend {
    pub transaction: Transaction,
    pub fee_atoms: u64,
    pub change_used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletSpendProgressStage {
    Preparing,
    Signing,
    FinalizingProof,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WalletSpendBuildError {
    #[error("selected inputs are empty")]
    NoInputs,
    #[error("wallet request belongs to the wrong network")]
    WrongNetwork,
    #[error("selected inputs do not belong to the requested spend address")]
    InputOwnershipMismatch,
    #[error("amount must be at least the minimum spendable output")]
    AmountBelowMinimumOutput,
    #[error("amount must exceed the network fee")]
    AmountBelowFee,
    #[error("selected inputs do not cover the requested amount and fee")]
    InsufficientFunds,
    #[error("missing change address for a spend that requires one")]
    MissingChangeAddress,
    #[error("transaction fee calculation did not converge")]
    FeeLoopDidNotConverge,
    #[error("amount arithmetic overflowed")]
    AmountOverflow,
    #[error("wallet signing failed: {0}")]
    SigningFailed(String),
}

/// User wallet state including key material, address bookkeeping, and snapshot data.
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
    /// Builds a wallet from a mnemonic phrase and optional passphrase.
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

    /// Restores a wallet from a mnemonic string.
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

    /// Builds a wallet from a raw seed instead of a mnemonic phrase.
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

    pub fn set_restore_gap_limit(&mut self, limit: usize) {
        self.restore_gap_limit = limit.max(1);
    }

    /// Persists the wallet into an encrypted `.datafile`.
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

    /// Loads a wallet from an encrypted `.datafile`.
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

    /// Returns the mnemonic phrase when this wallet was created from one.
    pub fn mnemonic_phrase(&self) -> Option<&MnemonicPhrase> {
        self.mnemonic.as_ref()
    }

    /// Returns the mnemonic as a sentence for explicit wallet export flows.
    pub fn mnemonic_sentence(&self) -> Option<String> {
        self.mnemonic.as_ref().map(MnemonicPhrase::as_sentence)
    }

    /// Reserves the next receive address from the keypool.
    pub fn checkout_receive_address(&mut self) -> WalletAddress {
        self.checkout(AddressKind::Receive, None)
    }

    /// Reserves the next change address from the keypool.
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

    /// Derives the deterministic Falcon keypair for one wallet path.
    ///
    /// WALLET SECURITY: The derivation mixes the network tag and path so keys
    /// are not silently reused across networks or address roles.
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

    pub fn build_signed_payment_transaction(
        &self,
        request: WalletSpendRequest,
    ) -> Result<WalletBuiltSpend, WalletSpendBuildError> {
        self.build_signed_payment_transaction_with_progress(request, |_| {})
    }

    pub fn build_signed_payment_transaction_with_progress<F>(
        &self,
        request: WalletSpendRequest,
        mut progress: F,
    ) -> Result<WalletBuiltSpend, WalletSpendBuildError>
    where
        F: FnMut(WalletSpendProgressStage),
    {
        progress(WalletSpendProgressStage::Preparing);
        if request.selected_utxos.is_empty() {
            return Err(WalletSpendBuildError::NoInputs);
        }
        if request
            .change_address
            .as_ref()
            .is_some_and(|address| address.network != self.network)
        {
            return Err(WalletSpendBuildError::WrongNetwork);
        }

        let min_output = DUST_RELAY_VALUE_ATOMS;
        if !request.include_fee_in_total && request.amount_atoms < min_output {
            return Err(WalletSpendBuildError::AmountBelowMinimumOutput);
        }

        let total_input_atoms = request
            .selected_utxos
            .iter()
            .try_fold(0u64, |sum, utxo| sum.checked_add(utxo.value_atoms))
            .ok_or(WalletSpendBuildError::AmountOverflow)?;
        let mut fee_atoms = MIN_TX_FEE_ATOMS;

        for _ in 0..4 {
            let (recipient_atoms, change_atoms) = recipient_and_change_amounts(
                total_input_atoms,
                request.amount_atoms,
                fee_atoms,
                request.include_fee_in_total,
                min_output,
            )?;
            progress(WalletSpendProgressStage::Signing);
            let transaction = self.build_signed_spend_transaction_from_parts(
                &request.selected_utxos,
                request.transaction_version,
                request.recipient_digest,
                recipient_atoms,
                if change_atoms > 0 {
                    Some((change_atoms, vec![0u8; 32]))
                } else {
                    None
                },
                request.lock_time,
            )?;
            let actual_fee = minimum_required_fee_atoms(self.network, &transaction);
            if actual_fee == fee_atoms {
                progress(WalletSpendProgressStage::Signing);
                let mut transaction = self.build_signed_spend_transaction_from_parts(
                    &request.selected_utxos,
                    request.transaction_version,
                    request.recipient_digest,
                    recipient_atoms,
                    if change_atoms > 0 {
                        Some((
                            change_atoms,
                            request
                                .change_address
                                .as_ref()
                                .ok_or(WalletSpendBuildError::MissingChangeAddress)?
                                .payment_digest
                                .to_vec(),
                        ))
                    } else {
                        None
                    },
                    request.lock_time,
                )?;
                let final_fee = total_input_atoms
                    .checked_sub(
                        transaction
                            .checked_output_value_atoms()
                            .ok_or(WalletSpendBuildError::AmountOverflow)?,
                    )
                    .ok_or(WalletSpendBuildError::InsufficientFunds)?;
                if final_fee < actual_fee {
                    return Err(WalletSpendBuildError::FeeLoopDidNotConverge);
                }
                if change_atoms > 0 && final_fee != actual_fee {
                    return Err(WalletSpendBuildError::FeeLoopDidNotConverge);
                }
                progress(WalletSpendProgressStage::FinalizingProof);
                solve_transaction_pow(self.network, &mut transaction, final_fee);
                return Ok(WalletBuiltSpend {
                    transaction,
                    fee_atoms: final_fee,
                    change_used: change_atoms > 0,
                });
            }
            fee_atoms = actual_fee;
        }

        Err(WalletSpendBuildError::FeeLoopDidNotConverge)
    }

    fn address_for_locking_script(&self, locking_script: &[u8]) -> Option<WalletAddress> {
        if locking_script.len() != 32 {
            return None;
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(locking_script);
        self.address_for_payment_digest(&digest)
    }

    pub fn address_for_payment_digest(&self, digest: &[u8; 32]) -> Option<WalletAddress> {
        self.all_addresses()
            .into_iter()
            .find(|address| &address.payment_digest == digest)
    }

    pub fn all_addresses(&self) -> Vec<WalletAddress> {
        self.address_book
            .snapshot()
            .into_par_iter()
            .map(|record| self.derive_address(record.path))
            .collect()
    }

    pub fn discovery_addresses(&self) -> Vec<WalletAddress> {
        self.discovery_addresses_up_to(self.restore_gap_limit)
    }

    pub fn next_indices(&self) -> (u32, u32) {
        self.hd_wallet.counters()
    }

    pub fn keypool_depths(&self) -> (usize, usize) {
        (self.keypool.receive_len(), self.keypool.change_len())
    }

    pub fn highest_reserved_indices(&self) -> (Option<u32>, Option<u32>) {
        (
            self.keypool.highest_receive_index(),
            self.keypool.highest_change_index(),
        )
    }

    pub fn highest_generated_indices(&self) -> (Option<u32>, Option<u32>) {
        let mut highest_receive = None;
        let mut highest_change = None;
        for record in self.address_book.snapshot() {
            match record.path.kind {
                AddressKind::Receive => {
                    highest_receive = Some(
                        highest_receive
                            .map(|current: u32| current.max(record.path.index))
                            .unwrap_or(record.path.index),
                    );
                }
                AddressKind::Change => {
                    highest_change = Some(
                        highest_change
                            .map(|current: u32| current.max(record.path.index))
                            .unwrap_or(record.path.index),
                    );
                }
            }
        }
        (highest_receive, highest_change)
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
        let (receive_queue, change_queue) = self.keypool.snapshot();
        let queue = match kind {
            AddressKind::Receive => receive_queue,
            AddressKind::Change => change_queue,
        };
        queue
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|path| self.derive_address(path))
            .collect()
    }

    fn generated_addresses(&self, kind: AddressKind, limit: usize) -> Vec<WalletAddress> {
        if limit == 0 {
            return Vec::new();
        }
        self.address_book
            .snapshot()
            .into_iter()
            .filter(|record| record.path.kind == kind)
            .take(limit)
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|record| self.derive_address(record.path))
            .collect()
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

fn recipient_and_change_amounts(
    total_input_atoms: u64,
    amount_atoms: u64,
    fee_atoms: u64,
    include_fee_in_total: bool,
    min_output: u64,
) -> Result<(u64, u64), WalletSpendBuildError> {
    if include_fee_in_total {
        if amount_atoms <= fee_atoms {
            return Err(WalletSpendBuildError::AmountBelowFee);
        }
        let recipient_atoms = amount_atoms
            .checked_sub(fee_atoms)
            .ok_or(WalletSpendBuildError::AmountBelowFee)?;
        if recipient_atoms < min_output {
            return Err(WalletSpendBuildError::AmountBelowMinimumOutput);
        }
        let raw_change = total_input_atoms
            .checked_sub(amount_atoms)
            .ok_or(WalletSpendBuildError::InsufficientFunds)?;
        let change_atoms = if raw_change < min_output {
            0
        } else {
            raw_change
        };
        Ok((recipient_atoms, change_atoms))
    } else {
        if amount_atoms < min_output {
            return Err(WalletSpendBuildError::AmountBelowMinimumOutput);
        }
        let raw_change = total_input_atoms
            .checked_sub(amount_atoms)
            .and_then(|value| value.checked_sub(fee_atoms))
            .ok_or(WalletSpendBuildError::InsufficientFunds)?;
        let change_atoms = if raw_change < min_output {
            0
        } else {
            raw_change
        };
        Ok((amount_atoms, change_atoms))
    }
}

impl Wallet {
    fn build_signed_spend_transaction_from_parts(
        &self,
        selected_utxos: &[WalletSpendUtxo],
        version: u16,
        recipient_digest: [u8; 32],
        recipient_atoms: u64,
        change_output: Option<(u64, Vec<u8>)>,
        lock_time: u32,
    ) -> Result<Transaction, WalletSpendBuildError> {
        let mut outputs = vec![TxOutput {
            value_atoms: recipient_atoms,
            locking_script: recipient_digest.to_vec(),
        }];
        if let Some((change_atoms, change_script)) = change_output {
            outputs.push(TxOutput {
                value_atoms: change_atoms,
                locking_script: change_script,
            });
        }

        let inputs = selected_utxos
            .iter()
            .map(|utxo| TxInput {
                previous_txid: utxo.previous_txid,
                output_index: utxo.output_index,
                unlocking_script: utxo.locking_script.clone(),
            })
            .collect::<Vec<_>>();

        let mut tx = Transaction {
            version,
            inputs,
            outputs,
            lock_time,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let txid = tx.txid();

        let mut grouped_inputs = BTreeMap::<Vec<u8>, Vec<(u32, &WalletSpendUtxo)>>::new();
        for (input_index, utxo) in selected_utxos.iter().enumerate() {
            grouped_inputs
                .entry(utxo.locking_script.clone())
                .or_default()
                .push((input_index as u32, utxo));
        }

        let mut signer_groups = Vec::with_capacity(grouped_inputs.len());
        for (locking_script, group_utxos) in grouped_inputs {
            let address = self
                .address_for_locking_script(&locking_script)
                .ok_or(WalletSpendBuildError::InputOwnershipMismatch)?;
            let keypair = self.keypair_for_path(address.path);
            let input_indexes = group_utxos
                .iter()
                .map(|(input_index, _)| *input_index)
                .collect::<Vec<_>>();
            let digest =
                transaction_signing_digest_for_input_indexes(self.network, &tx, &input_indexes);
            let signature = sign(
                AthoSignatureDomain::Transaction,
                &keypair.secret_key,
                &digest,
            )
            .map_err(|err| WalletSpendBuildError::SigningFailed(err.to_string()))?;
            let sig_bytes = signature.0.clone();
            signer_groups.push(WitnessSignerGroup {
                signature: sig_bytes.clone(),
                pubkey: keypair.public_key.0.clone(),
                input_refs: group_utxos
                    .iter()
                    .map(|(input_index, _)| WitnessInputRef {
                        input_index: *input_index,
                        sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, *input_index),
                        witness_commit_ref: [0; 16],
                    })
                    .collect(),
            });
        }
        signer_groups.sort_by_key(|group| {
            group
                .input_refs
                .first()
                .map(|input_ref| input_ref.input_index)
                .unwrap_or(u32::MAX)
        });
        let mut signer_groups = signer_groups.into_iter();
        let primary_group = signer_groups
            .next()
            .ok_or(WalletSpendBuildError::InputOwnershipMismatch)?;
        tx.witness = TxWitness {
            signature: primary_group.signature,
            pubkey: primary_group.pubkey,
            input_refs: primary_group.input_refs,
            additional_signers: signer_groups.collect(),
        }
        .canonical_bytes();
        let witness_root = tx.witness_commitment_hash();
        let mut witness = tx
            .witness_payload()
            .ok_or(WalletSpendBuildError::SigningFailed(String::from(
                "invalid staged witness payload",
            )))?;
        witness.for_each_input_ref_mut(|input_ref| {
            input_ref.witness_commit_ref =
                derive_witness_commit_ref(&txid, &witness_root, input_ref.input_index);
        });
        tx.witness = witness.canonical_bytes();
        Ok(tx)
    }
}

fn derive_sig_ref_short(txid: &[u8; 48], signature: &[u8], input_index: u32) -> [u8; 2] {
    let mut preimage = Vec::with_capacity(
        b"ATHO_SIG_REF_SHORT_V1".len() + txid.len() + signature.len() + core::mem::size_of::<u32>(),
    );
    preimage.extend_from_slice(b"ATHO_SIG_REF_SHORT_V1");
    preimage.extend_from_slice(txid);
    preimage.extend_from_slice(signature);
    preimage.extend_from_slice(&input_index.to_be_bytes());
    let digest = sha3_256(&preimage);
    [digest[0], digest[1]]
}

fn derive_witness_commit_ref(
    txid: &[u8; 48],
    block_witness_root: &[u8; 48],
    input_index: u32,
) -> [u8; 16] {
    let mut preimage = Vec::with_capacity(
        b"ATHO_WITNESS_COMMIT_REF_V1".len()
            + txid.len()
            + core::mem::size_of::<u32>()
            + block_witness_root.len(),
    );
    preimage.extend_from_slice(b"ATHO_WITNESS_COMMIT_REF_V1");
    preimage.extend_from_slice(txid);
    preimage.extend_from_slice(&input_index.to_be_bytes());
    preimage.extend_from_slice(block_witness_root);
    let digest = sha3_256(&preimage);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
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
    fn wallet_reports_generated_and_reserved_index_tips() {
        let mut wallet = Wallet::from_seed(WalletSeed([3; 32]), Network::Mainnet);

        let (receive_depth, change_depth) = wallet.keypool_depths();
        assert_eq!(receive_depth, crate::keypool::KEYPOOL_TARGET_SIZE);
        assert_eq!(change_depth, crate::keypool::KEYPOOL_TARGET_SIZE);
        assert_eq!(
            wallet.highest_reserved_indices(),
            (
                Some((crate::keypool::KEYPOOL_TARGET_SIZE - 1) as u32),
                Some((crate::keypool::KEYPOOL_TARGET_SIZE - 1) as u32)
            )
        );
        assert_eq!(wallet.highest_generated_indices(), (None, None));

        let receive = wallet.checkout_receive_address();
        let change = wallet.checkout_change_address();

        assert_eq!(
            wallet.highest_generated_indices(),
            (Some(receive.path.index), Some(change.path.index))
        );
        assert_eq!(
            wallet.next_indices(),
            (
                crate::keypool::KEYPOOL_TARGET_SIZE as u32 + 1,
                crate::keypool::KEYPOOL_TARGET_SIZE as u32 + 1
            )
        );
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
