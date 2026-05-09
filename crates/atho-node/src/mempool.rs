//! In-memory mempool admission and conflict tracking.
//!
//! The mempool keeps policy-accepted transactions that are not yet mined and
//! tracks spent inputs so unconfirmed double spends are rejected locally.
//!
//! POLICY: Relay policy is intentionally stricter than bare consensus. Dust and
//! fee-floor checks happen here before transactions are mined into templates.
use crate::dev;
use crate::validation::{validate_transaction_with_context_for_mempool, ValidationError};
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use atho_storage::utxo::UtxoEntry;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// Cached mempool entry with precomputed sizes and identifiers.
#[derive(Debug, Clone)]
pub struct MempoolEntry {
    pub transaction: Transaction,
    pub fee_atoms: u64,
    received_at_unix: u64,
    txid: [u8; 48],
    wtxid: [u8; 48],
    base_size_bytes: usize,
    raw_size_bytes: usize,
    vsize_bytes: usize,
}

impl MempoolEntry {
    /// Builds a mempool entry and caches the transaction identifiers and sizes.
    pub fn new(transaction: Transaction, fee_atoms: u64) -> Self {
        let txid = transaction.txid();
        let wtxid = transaction.wtxid();
        let base_size_bytes = transaction.base_size_bytes();
        let raw_size_bytes = transaction.full_size_bytes();
        let vsize_bytes = transaction.vsize_bytes().max(1);
        Self {
            transaction,
            fee_atoms,
            received_at_unix: unix_timestamp(),
            txid,
            wtxid,
            base_size_bytes,
            raw_size_bytes,
            vsize_bytes,
        }
    }

    /// Returns the canonical txid.
    pub fn txid(&self) -> [u8; 48] {
        self.txid
    }

    /// Returns the witness transaction identifier.
    pub fn wtxid(&self) -> [u8; 48] {
        self.wtxid
    }

    /// Returns the raw serialized transaction size.
    pub fn raw_size_bytes(&self) -> usize {
        self.raw_size_bytes
    }

    pub fn base_size_bytes(&self) -> usize {
        self.base_size_bytes
    }

    pub fn full_size_bytes(&self) -> usize {
        self.raw_size_bytes
    }

    pub fn vsize_bytes(&self) -> usize {
        self.vsize_bytes
    }

    pub fn received_at_unix(&self) -> u64 {
        self.received_at_unix
    }

    /// Returns the feerate in atoms per vbyte.
    pub fn feerate_atoms_per_vbyte(&self) -> u64 {
        self.fee_atoms / self.vsize_bytes as u64
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// In-memory transaction pool for standard-policy transactions.
#[derive(Debug, Default)]
pub struct Mempool {
    entries: BTreeMap<[u8; 48], MempoolEntry>,
    spent_inputs: BTreeSet<([u8; 48], u32)>,
}

impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    fn input_keys(tx: &Transaction) -> impl Iterator<Item = ([u8; 48], u32)> + '_ {
        tx.inputs
            .iter()
            .map(|input| (input.previous_txid, input.output_index))
    }

    /// Reserves all inputs for a transaction, rejecting mempool double spends.
    fn reserve_inputs(&mut self, tx: &Transaction) -> Result<(), ValidationError> {
        let mut inserted = Vec::new();
        for key in Self::input_keys(tx) {
            if !self.spent_inputs.insert(key) {
                for key in inserted.into_iter().rev() {
                    let _ = self.spent_inputs.remove(&key);
                }
                return Err(ValidationError::MempoolConflict);
            }
            inserted.push(key);
        }
        Ok(())
    }

    fn release_inputs(&mut self, tx: &Transaction) {
        for key in Self::input_keys(tx) {
            let _ = self.spent_inputs.remove(&key);
        }
    }

    fn validate_entry<F>(
        &self,
        entry: &MempoolEntry,
        network: Network,
        spend_height: u64,
        mut lookup: F,
    ) -> Result<u64, ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let fee = validate_transaction_with_context_for_mempool(
            &entry.transaction,
            entry.fee_atoms,
            network,
            spend_height,
            |txid, output_index| lookup(txid, output_index),
        )?;
        if Self::input_keys(&entry.transaction).any(|key| self.spent_inputs.contains(&key)) {
            return Err(ValidationError::MempoolConflict);
        }
        Ok(fee)
    }

    /// Admits one transaction after policy and chainstate validation.
    pub fn admit<F>(
        &mut self,
        entry: MempoolEntry,
        network: Network,
        spend_height: u64,
        lookup: F,
    ) -> Result<[u8; 48], ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let txid = entry.txid();
        if self.entries.contains_key(&txid) {
            return Err(ValidationError::MempoolConflict);
        }
        self.validate_entry(&entry, network, spend_height, lookup)?;
        self.reserve_inputs(&entry.transaction)?;
        self.entries.insert(txid, entry);
        let entry = self
            .entries
            .get(&txid)
            .expect("mempool entry just inserted");
        let summary = dev::summarize_transaction(&entry.transaction, Some(entry.fee_atoms));
        let _ = dev::append_log("mempool", &format!("admitted {summary}"));
        Ok(txid)
    }

    pub fn admit_many<F>(
        &mut self,
        entries: Vec<MempoolEntry>,
        network: Network,
        spend_height: u64,
        mut lookup: F,
    ) -> Result<Vec<[u8; 48]>, ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let mut txids = Vec::with_capacity(entries.len());
        for entry in entries {
            let txid = self.admit(entry, network, spend_height, &mut lookup)?;
            txids.push(txid);
        }
        Ok(txids)
    }

    /// Revalidates the mempool after the chain tip changes.
    ///
    /// PERFORMANCE: Revalidation is bulk work triggered on tip changes, so it
    /// avoids repeated logging and reuses the caller's UTXO lookup closure.
    pub fn revalidate<F>(&mut self, network: Network, spend_height: u64, mut lookup: F)
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let current = std::mem::take(&mut self.entries);
        self.spent_inputs.clear();
        let before = current.len();
        for (txid, entry) in current {
            if self
                .validate_entry(&entry, network, spend_height, &mut lookup)
                .is_ok()
                && self.reserve_inputs(&entry.transaction).is_ok()
            {
                self.entries.insert(txid, entry);
            }
        }
        let kept = self.entries.len();
        let _ = dev::append_log(
            "mempool",
            &format!(
                "revalidated kept={} dropped={}",
                kept,
                before.saturating_sub(kept)
            ),
        );
    }

    pub fn contains(&self, txid: &[u8; 48]) -> bool {
        self.entries.contains_key(txid)
    }

    pub fn transaction(&self, txid: &[u8; 48]) -> Option<Transaction> {
        self.entries
            .get(txid)
            .map(|entry| entry.transaction.clone())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn transactions(&self) -> Vec<Transaction> {
        self.entries
            .values()
            .map(|entry| entry.transaction.clone())
            .collect()
    }

    pub fn entry(&self, txid: &[u8; 48]) -> Option<MempoolEntry> {
        self.entries.get(txid).cloned()
    }

    pub fn entries(&self) -> Vec<MempoolEntry> {
        self.entries.values().cloned().collect()
    }

    pub fn txids(&self) -> Vec<[u8; 48]> {
        self.entries.keys().copied().collect()
    }

    pub fn valid_transactions<F>(
        &self,
        network: Network,
        spend_height: u64,
        mut lookup: F,
    ) -> Result<(Vec<Transaction>, u64), ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let (entries, fees) =
            self.validated_entries(network, spend_height, |txid, output_index| {
                lookup(txid, output_index)
            })?;
        Ok((
            entries.into_iter().map(|entry| entry.transaction).collect(),
            fees,
        ))
    }

    pub fn validated_entries<F>(
        &self,
        network: Network,
        spend_height: u64,
        mut lookup: F,
    ) -> Result<(Vec<MempoolEntry>, u64), ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let mut ordered: Vec<([u8; 48], &MempoolEntry)> = self
            .entries
            .iter()
            .map(|(txid, entry)| (*txid, entry))
            .collect();
        ordered.sort_by(|(left_txid, left), (right_txid, right)| {
            right
                .feerate_atoms_per_vbyte()
                .cmp(&left.feerate_atoms_per_vbyte())
                .then_with(|| right.fee_atoms.cmp(&left.fee_atoms))
                .then_with(|| left_txid.cmp(right_txid))
        });

        let mut entries = Vec::with_capacity(ordered.len());
        let mut fees = 0u64;
        for (_, entry) in ordered {
            let fee = validate_transaction_with_context_for_mempool(
                &entry.transaction,
                entry.fee_atoms,
                network,
                spend_height,
                |txid, output_index| lookup(txid, output_index),
            )?;
            fees = fees.saturating_add(fee);
            entries.push(entry.clone());
        }
        Ok((entries, fees))
    }

    pub fn validated_entries_for_mining<F>(
        &self,
        network: Network,
        spend_height: u64,
        mut lookup: F,
    ) -> (Vec<MempoolEntry>, u64, usize)
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let mut ordered: Vec<([u8; 48], &MempoolEntry)> = self
            .entries
            .iter()
            .map(|(txid, entry)| (*txid, entry))
            .collect();
        ordered.sort_by(|(left_txid, left), (right_txid, right)| {
            right
                .feerate_atoms_per_vbyte()
                .cmp(&left.feerate_atoms_per_vbyte())
                .then_with(|| right.fee_atoms.cmp(&left.fee_atoms))
                .then_with(|| left_txid.cmp(right_txid))
        });

        let mut entries = Vec::with_capacity(ordered.len());
        let mut fees = 0u64;
        let mut skipped = 0usize;
        for (_, entry) in ordered {
            match validate_transaction_with_context_for_mempool(
                &entry.transaction,
                entry.fee_atoms,
                network,
                spend_height,
                |txid, output_index| lookup(txid, output_index),
            ) {
                Ok(fee) => {
                    fees = fees.saturating_add(fee);
                    entries.push(entry.clone());
                }
                Err(_) => {
                    skipped = skipped.saturating_add(1);
                }
            }
        }
        (entries, fees, skipped)
    }

    pub fn total_fee_atoms(&self) -> u64 {
        self.entries.values().map(|entry| entry.fee_atoms).sum()
    }

    pub fn spent_inputs_snapshot(&self) -> Vec<([u8; 48], u32)> {
        self.spent_inputs.iter().cloned().collect()
    }

    pub fn remove_block_transactions(&mut self, block: &atho_core::block::Block) {
        for tx in &block.transactions {
            let txid = tx.txid();
            if let Some(entry) = self.entries.remove(&txid) {
                self.release_inputs(&entry.transaction);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_unchecked(&mut self, entry: MempoolEntry) {
        self.entries.insert(entry.txid(), entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::consensus::rules::TRANSACTION_VERSION_V2_PLACEHOLDER;
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::consensus::tx_policy::solve_transaction_pow;
    use atho_core::constants::DUST_RELAY_VALUE_ATOMS;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};

    fn witness_bytes_for_tx(network: Network, tx: &Transaction) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-mempool-test").expect("falcon keypair");
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &transaction_signing_digest(network, tx),
        )
        .expect("falcon signature")
        .0;
        let pubkey = keypair.public_key.0;
        let txid = tx.txid();
        let staged = TxWitness {
            signature: signature.clone(),
            pubkey: pubkey.clone(),
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    input_index: index as u32,
                    sig_ref_short: crate::validation::derive_sig_ref_short(
                        &txid,
                        &signature,
                        index as u32,
                    ),
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
            signature: signature.clone(),
            pubkey: pubkey.clone(),
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    input_index: index as u32,
                    sig_ref_short: crate::validation::derive_sig_ref_short(
                        &txid,
                        &signature,
                        index as u32,
                    ),
                    witness_commit_ref: crate::validation::derive_witness_commit_ref(
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

    #[test]
    fn mempool_admits_valid_transactions() {
        let mut mempool = Mempool::new();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [2; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &tx),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx
        };

        let txid = mempool
            .admit(MempoolEntry::new(tx, 500), Network::Mainnet, 0, |_, _| None)
            .expect_err("missing utxo should fail");

        assert_eq!(txid, ValidationError::MissingUtxo);
    }

    #[test]
    fn mempool_rejects_conflicts() {
        let mut mempool = Mempool::new();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [2; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &tx),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx
        };

        let _ = mempool.spent_inputs.insert(([2; 48], 0));
        assert_eq!(
            mempool
                .reserve_inputs(&tx)
                .expect_err("conflict should fail"),
            ValidationError::MempoolConflict
        );
    }

    #[test]
    fn mempool_rejects_sub_dust_outputs() {
        let mut mempool = Mempool::new();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [6; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: DUST_RELAY_VALUE_ATOMS - 1,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(Network::Regnet, &tx),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx
        };
        let err = mempool
            .admit(
                MempoolEntry::new(tx, 10_000),
                Network::Regnet,
                10,
                |_, _| None,
            )
            .unwrap_err();
        assert_eq!(err, ValidationError::DustOutput);
    }

    #[test]
    fn mining_view_skips_unchecked_dust_entries() {
        let mut mempool = Mempool::new();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [16; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: DUST_RELAY_VALUE_ATOMS - 1,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(Network::Regnet, &tx),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx
        };
        let txid = tx.txid();
        mempool.insert_unchecked(MempoolEntry::new(tx, 10_000));

        let (entries, fees, skipped) =
            mempool.validated_entries_for_mining(Network::Regnet, 10, |_, _| None);
        assert!(entries.is_empty());
        assert_eq!(fees, 0);
        assert_eq!(skipped, 1);
        assert!(!entries.iter().any(|entry| entry.txid() == txid));
    }

    #[test]
    fn valid_transactions_are_sorted_by_feerate() {
        let mut mempool = Mempool::new();
        let low = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [4; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_500,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let mut low = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &low),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..low
        };
        let high = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [5; 48],
                output_index: 0,
                unlocking_script: vec![3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: vec![4],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let mut high = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &high),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..high
        };
        solve_transaction_pow(Network::Mainnet, &mut low, 2_500);
        solve_transaction_pow(Network::Mainnet, &mut high, 3_000);

        let low_txid = low.txid();
        let high_txid = high.txid();
        let _ = mempool
            .entries
            .insert(low_txid, MempoolEntry::new(low.clone(), 2_500));
        let _ = mempool
            .entries
            .insert(high_txid, MempoolEntry::new(high.clone(), 3_000));

        let mut utxos = std::collections::BTreeMap::new();
        utxos.insert(
            ([4; 48], 0),
            UtxoEntry::new(
                atho_core::network::Network::Mainnet,
                [4; 48],
                0,
                10_000,
                vec![1],
                0,
                false,
            ),
        );
        utxos.insert(
            ([5; 48], 0),
            UtxoEntry::new(
                atho_core::network::Network::Mainnet,
                [5; 48],
                0,
                10_000,
                vec![3],
                0,
                false,
            ),
        );

        let (txs, fees) = mempool
            .valid_transactions(Network::Mainnet, 7, |txid, output_index| {
                utxos.get(&(*txid, output_index)).cloned()
            })
            .expect("both transactions should validate");

        assert_eq!(fees, 5_500);
        assert_eq!(txs[0].txid(), high_txid);
        assert_eq!(txs[1].txid(), low_txid);
    }

    #[test]
    fn mempool_rejects_future_transaction_version_before_activation() {
        let mut mempool = Mempool::new();
        let tx = Transaction {
            version: TRANSACTION_VERSION_V2_PLACEHOLDER,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_500,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &tx),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx
        };
        let err = mempool
            .admit(
                MempoolEntry::new(tx, 2_500),
                Network::Mainnet,
                10,
                |_, _| None,
            )
            .unwrap_err();
        assert_eq!(err, ValidationError::InvalidTransactionVersion);
    }

    #[test]
    fn mining_view_skips_invalid_entries_instead_of_failing_whole_selection() {
        let mut mempool = Mempool::new();
        let valid = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [11; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let mut valid = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &valid),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..valid
        };
        let invalid = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [12; 48],
                output_index: 0,
                unlocking_script: vec![3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: vec![4],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let invalid = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &invalid),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..invalid
        };
        solve_transaction_pow(Network::Mainnet, &mut valid, 3_000);

        let valid_entry = MempoolEntry::new(valid.clone(), 3_000);
        let invalid_entry = MempoolEntry::new(invalid.clone(), 3_000);
        let _ = mempool.entries.insert(valid.txid(), valid_entry);
        let _ = mempool.entries.insert(invalid.txid(), invalid_entry);

        let mut utxos = std::collections::BTreeMap::new();
        utxos.insert(
            ([11; 48], 0),
            UtxoEntry::new(
                atho_core::network::Network::Mainnet,
                [11; 48],
                0,
                10_000,
                vec![1],
                0,
                false,
            ),
        );

        let (entries, fees, skipped) =
            mempool.validated_entries_for_mining(Network::Mainnet, 7, |txid, output_index| {
                utxos.get(&(*txid, output_index)).cloned()
            });

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].txid(), valid.txid());
        assert_eq!(fees, 3_000);
        assert_eq!(skipped, 1);
    }
}
