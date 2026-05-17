// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! In-memory mempool admission and conflict tracking.
//!
//! The mempool keeps policy-accepted transactions that are not yet mined and
//! tracks spent inputs so unconfirmed double spends are rejected locally.
//!
//! POLICY: Relay policy is intentionally stricter than bare consensus. Dust and
//! fee-floor checks happen here before transactions are mined into templates.
use crate::config::{DEFAULT_MAX_MEMPOOL_TRANSACTIONS, DEFAULT_MAX_MEMPOOL_VBYTES};
use crate::dev;
use crate::validation::{validate_transaction_with_context_for_mempool, ValidationError};
#[cfg(test)]
use atho_core::constants::ADDRESS_DIGEST_BYTES;
use atho_core::crypto::hash::sha3_256;
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use atho_storage::utxo::UtxoEntry;
use std::cell::RefCell;
use std::cmp::Reverse;
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
#[derive(Debug, Clone)]
struct MempoolFingerprintCache {
    network: Network,
    fingerprint: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MiningOrderKey {
    feerate_atoms_per_vbyte: Reverse<u64>,
    fee_atoms: Reverse<u64>,
    txid: [u8; 48],
}

impl From<&MempoolEntry> for MiningOrderKey {
    fn from(entry: &MempoolEntry) -> Self {
        Self {
            feerate_atoms_per_vbyte: Reverse(entry.feerate_atoms_per_vbyte()),
            fee_atoms: Reverse(entry.fee_atoms),
            txid: entry.txid(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Mempool {
    entries: BTreeMap<[u8; 48], MempoolEntry>,
    mining_order: BTreeSet<MiningOrderKey>,
    parents_by_txid: BTreeMap<[u8; 48], BTreeSet<[u8; 48]>>,
    children_by_txid: BTreeMap<[u8; 48], BTreeSet<[u8; 48]>>,
    spent_inputs: BTreeSet<([u8; 48], u32)>,
    total_fee_atoms: u64,
    total_vbytes: usize,
    limits: MempoolLimits,
    fingerprint_cache: RefCell<Option<MempoolFingerprintCache>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MempoolLimits {
    pub max_transactions: usize,
    pub max_vbytes: usize,
}

impl Default for MempoolLimits {
    fn default() -> Self {
        Self {
            max_transactions: DEFAULT_MAX_MEMPOOL_TRANSACTIONS,
            max_vbytes: DEFAULT_MAX_MEMPOOL_VBYTES,
        }
    }
}

impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(limits: MempoolLimits) -> Self {
        Self {
            limits: limits.normalized(),
            ..Self::default()
        }
    }

    pub fn limits(&self) -> MempoolLimits {
        self.limits
    }

    pub fn total_vbytes(&self) -> usize {
        self.total_vbytes
    }

    fn would_exceed_limits(&self, entry: &MempoolEntry) -> bool {
        self.entries.len() >= self.limits.max_transactions
            || self.total_vbytes.saturating_add(entry.vsize_bytes()) > self.limits.max_vbytes
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

    fn invalidate_fingerprint(&self) {
        self.fingerprint_cache.replace(None);
    }

    fn reset_derived_state(&mut self) {
        self.mining_order.clear();
        self.parents_by_txid.clear();
        self.children_by_txid.clear();
        self.spent_inputs.clear();
        self.total_fee_atoms = 0;
        self.total_vbytes = 0;
    }

    fn rebuild_derived_state_from_entries(&mut self) {
        self.reset_derived_state();
        let txids = self.entries.keys().copied().collect::<Vec<_>>();
        for txid in txids {
            let Some(entry) = self.entries.get(&txid) else {
                continue;
            };
            self.total_fee_atoms = self.total_fee_atoms.saturating_add(entry.fee_atoms);
            self.total_vbytes = self.total_vbytes.saturating_add(entry.vsize_bytes());
            self.mining_order.insert(MiningOrderKey::from(entry));
            for key in Self::input_keys(&entry.transaction) {
                self.spent_inputs.insert(key);
            }
        }

        for txid in self.entries.keys().copied().collect::<Vec<_>>() {
            let Some(entry) = self.entries.get(&txid) else {
                continue;
            };
            let mut parents = BTreeSet::new();
            for input in &entry.transaction.inputs {
                if self.entries.contains_key(&input.previous_txid) {
                    parents.insert(input.previous_txid);
                    self.children_by_txid
                        .entry(input.previous_txid)
                        .or_default()
                        .insert(txid);
                }
            }
            if !parents.is_empty() {
                self.parents_by_txid.insert(txid, parents);
            }
        }
    }

    fn link_relations(&mut self, txid: [u8; 48], tx: &Transaction) {
        let mut parents = BTreeSet::new();
        for input in &tx.inputs {
            if self.entries.contains_key(&input.previous_txid) {
                parents.insert(input.previous_txid);
                self.children_by_txid
                    .entry(input.previous_txid)
                    .or_default()
                    .insert(txid);
            }
        }
        if !parents.is_empty() {
            self.parents_by_txid.insert(txid, parents);
        }
    }

    fn unlink_relations(&mut self, txid: [u8; 48]) {
        if let Some(parents) = self.parents_by_txid.remove(&txid) {
            let parent_ids = parents.into_iter().collect::<Vec<_>>();
            for parent in parent_ids {
                let should_remove = if let Some(children) = self.children_by_txid.get_mut(&parent) {
                    children.remove(&txid);
                    children.is_empty()
                } else {
                    false
                };
                if should_remove {
                    self.children_by_txid.remove(&parent);
                }
            }
        }

        if let Some(children) = self.children_by_txid.remove(&txid) {
            for child in children {
                let should_remove = if let Some(parents) = self.parents_by_txid.get_mut(&child) {
                    parents.remove(&txid);
                    parents.is_empty()
                } else {
                    false
                };
                if should_remove {
                    self.parents_by_txid.remove(&child);
                }
            }
        }
    }

    fn insert_entry(&mut self, txid: [u8; 48], entry: MempoolEntry) {
        self.total_fee_atoms = self.total_fee_atoms.saturating_add(entry.fee_atoms);
        self.total_vbytes = self.total_vbytes.saturating_add(entry.vsize_bytes());
        self.mining_order.insert(MiningOrderKey::from(&entry));
        self.link_relations(txid, &entry.transaction);
        self.entries.insert(txid, entry);
    }

    fn remove_entry(&mut self, txid: &[u8; 48]) -> Option<MempoolEntry> {
        let entry = self.entries.remove(txid)?;
        self.unlink_relations(entry.txid());
        self.mining_order.remove(&MiningOrderKey::from(&entry));
        self.total_fee_atoms = self.total_fee_atoms.saturating_sub(entry.fee_atoms);
        self.total_vbytes = self.total_vbytes.saturating_sub(entry.vsize_bytes());
        Some(entry)
    }

    fn ordered_entries_iter(&self) -> impl Iterator<Item = &MempoolEntry> + '_ {
        self.mining_order
            .iter()
            .filter_map(|key| self.entries.get(&key.txid))
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
        if self.would_exceed_limits(&entry) {
            return Err(ValidationError::MempoolConflict);
        }
        self.validate_entry(&entry, network, spend_height, lookup)?;
        self.reserve_inputs(&entry.transaction)?;
        self.insert_entry(txid, entry);
        self.invalidate_fingerprint();
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
        self.reset_derived_state();
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
        self.rebuild_derived_state_from_entries();
        self.invalidate_fingerprint();
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
        let mut entries = Vec::with_capacity(self.mining_order.len());
        let mut fees = 0u64;
        for entry in self.ordered_entries_iter() {
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
        let mut entries = Vec::with_capacity(self.mining_order.len());
        let mut fees = 0u64;
        let mut skipped = 0usize;
        for entry in self.ordered_entries_iter() {
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
        self.total_fee_atoms
    }

    pub fn dependency_txids(&self, target: &[u8; 48]) -> Option<Vec<[u8; 48]>> {
        if !self.entries.contains_key(target) {
            return None;
        }
        let mut visited = BTreeSet::new();
        let mut stack = self
            .parents_by_txid
            .get(target)
            .map(|parents| parents.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        while let Some(txid) = stack.pop() {
            if !visited.insert(txid) {
                continue;
            }
            if let Some(parents) = self.parents_by_txid.get(&txid) {
                stack.extend(parents.iter().copied());
            }
        }
        Some(visited.into_iter().collect())
    }

    pub fn descendant_txids(&self, target: &[u8; 48]) -> Option<Vec<[u8; 48]>> {
        if !self.entries.contains_key(target) {
            return None;
        }
        let mut visited = BTreeSet::new();
        let mut stack = self
            .children_by_txid
            .get(target)
            .map(|children| children.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        while let Some(txid) = stack.pop() {
            if !visited.insert(txid) {
                continue;
            }
            if let Some(children) = self.children_by_txid.get(&txid) {
                stack.extend(children.iter().copied());
            }
        }
        Some(visited.into_iter().collect())
    }

    pub fn spent_inputs_snapshot(&self) -> Vec<([u8; 48], u32)> {
        self.spent_inputs.iter().cloned().collect()
    }

    pub fn fingerprint(&self, network: Network) -> [u8; 32] {
        if let Some(cache) = self.fingerprint_cache.borrow().as_ref() {
            if cache.network == network {
                return cache.fingerprint;
            }
        }

        let mut preimage = Vec::with_capacity(
            network.id().len()
                + 16
                + self.entries.len().saturating_mul(48)
                + self.spent_inputs.len().saturating_mul(52),
        );
        preimage.extend_from_slice(network.id().as_bytes());
        preimage.extend_from_slice(&(self.entries.len() as u64).to_be_bytes());
        preimage.extend_from_slice(&self.total_fee_atoms.to_be_bytes());
        for txid in self.entries.keys() {
            preimage.extend_from_slice(txid);
        }
        for (txid, output_index) in &self.spent_inputs {
            preimage.extend_from_slice(txid);
            preimage.extend_from_slice(&output_index.to_be_bytes());
        }
        let fingerprint = sha3_256(&preimage);
        self.fingerprint_cache
            .replace(Some(MempoolFingerprintCache {
                network,
                fingerprint,
            }));
        fingerprint
    }

    pub fn remove_block_transactions(&mut self, block: &atho_core::block::Block) {
        let mut removed_any = false;
        for tx in &block.transactions {
            let txid = tx.txid();
            if let Some(entry) = self.remove_entry(&txid) {
                self.release_inputs(&entry.transaction);
                removed_any = true;
            }
        }
        if removed_any {
            self.invalidate_fingerprint();
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_unchecked(&mut self, entry: MempoolEntry) {
        let txid = entry.txid();
        if let Some(previous) = self.entries.insert(txid, entry) {
            self.release_inputs(&previous.transaction);
        }
        self.rebuild_derived_state_from_entries();
        self.invalidate_fingerprint();
    }
}

impl MempoolLimits {
    fn normalized(self) -> Self {
        Self {
            max_transactions: self.max_transactions.max(1),
            max_vbytes: self.max_vbytes.max(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::address::public_key_digest;
    use atho_core::consensus::rules::TRANSACTION_VERSION_V2_PLACEHOLDER;
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::consensus::tx_policy::solve_transaction_pow;
    use atho_core::constants::DUST_RELAY_VALUE_ATOMS;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};

    fn test_lock(network: Network) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-mempool-test").expect("falcon keypair");
        public_key_digest(network, &keypair.public_key.0).to_vec()
    }

    fn alternate_lock(network: Network) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-mempool-output").expect("falcon keypair");
        public_key_digest(network, &keypair.public_key.0).to_vec()
    }

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
        let spend_lock = test_lock(Network::Mainnet);
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [2; 48],
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: alternate_lock(Network::Mainnet),
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
            .admit(
                MempoolEntry::new(tx, 10_000),
                Network::Mainnet,
                0,
                |_, _| None,
            )
            .expect_err("missing utxo should fail");

        assert_eq!(txid, ValidationError::MissingUtxo);
    }

    #[test]
    fn mempool_rejects_conflicts() {
        let mut mempool = Mempool::new();
        let spend_lock = test_lock(Network::Mainnet);
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [2; 48],
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: alternate_lock(Network::Mainnet),
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
    fn mempool_rejects_new_entries_when_transaction_cap_is_reached() {
        let mut mempool = Mempool::with_limits(MempoolLimits {
            max_transactions: 1,
            max_vbytes: usize::MAX,
        });
        let spend_lock = test_lock(Network::Mainnet);
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [2; 48],
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: DUST_RELAY_VALUE_ATOMS,
                locking_script: alternate_lock(Network::Mainnet),
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        mempool.insert_unchecked(MempoolEntry::new(tx.clone(), 1_000));

        let err = mempool
            .admit(MempoolEntry::new(tx, 1_000), Network::Mainnet, 0, |_, _| {
                None
            })
            .expect_err("full mempool should reject before expensive validation");
        assert_eq!(err, ValidationError::MempoolConflict);
    }

    #[test]
    fn mempool_total_fee_and_fingerprint_update_from_cached_state() {
        let mut mempool = Mempool::new();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [0x55; 48],
                output_index: 0,
                unlocking_script: vec![0x55; ADDRESS_DIGEST_BYTES],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![0x56; ADDRESS_DIGEST_BYTES],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let empty = mempool.fingerprint(Network::Regnet);
        let entry = MempoolEntry::new(tx.clone(), 17);
        let txid = entry.txid();

        mempool.insert_unchecked(entry);
        assert_eq!(mempool.total_fee_atoms(), 17);
        let populated = mempool.fingerprint(Network::Regnet);
        assert_ne!(empty, populated);
        assert_eq!(mempool.fingerprint(Network::Regnet), populated);

        let block = atho_core::block::Block {
            header: atho_core::block::BlockHeader {
                version: 1,
                network_id: Network::Regnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: [0; 48],
                witness_root: [0; 48],
                founders_hash_sha3_384:
                    atho_core::block::BlockHeader::consensus_founders_hash_sha3_384(),
                founders_hash_sha3_512:
                    atho_core::block::BlockHeader::consensus_founders_hash_sha3_512(),
                timestamp: 0,
                difficulty_target_or_bits: [0xff; 48],
                nonce: 0,
            },
            transactions: vec![tx],
            witnesses: Default::default(),
            fees_total_atoms: 0,
            fees_miner_atoms: 0,
        };
        assert!(mempool.contains(&txid));
        mempool.remove_block_transactions(&block);
        assert_eq!(mempool.total_fee_atoms(), 0);
        assert_eq!(mempool.fingerprint(Network::Regnet), empty);
    }

    #[test]
    fn mempool_rejects_sub_dust_outputs() {
        let mut mempool = Mempool::new();
        let spend_lock = test_lock(Network::Regnet);
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [6; 48],
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: DUST_RELAY_VALUE_ATOMS - 1,
                locking_script: alternate_lock(Network::Regnet),
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
        let spend_lock = test_lock(Network::Regnet);
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [16; 48],
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: DUST_RELAY_VALUE_ATOMS - 1,
                locking_script: alternate_lock(Network::Regnet),
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
        let spend_lock = test_lock(Network::Mainnet);
        let output_lock = alternate_lock(Network::Mainnet);
        let low = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [4; 48],
                output_index: 0,
                unlocking_script: spend_lock.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_500,
                locking_script: output_lock.clone(),
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
                unlocking_script: spend_lock.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: output_lock.clone(),
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
        mempool.insert_unchecked(MempoolEntry::new(low.clone(), 2_500));
        mempool.insert_unchecked(MempoolEntry::new(high.clone(), 3_000));

        let mut utxos = std::collections::BTreeMap::new();
        utxos.insert(
            ([4; 48], 0),
            UtxoEntry::new(
                atho_core::network::Network::Mainnet,
                [4; 48],
                0,
                10_000,
                spend_lock.clone(),
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
                spend_lock.clone(),
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
        let spend_lock = test_lock(Network::Mainnet);
        let tx = Transaction {
            version: TRANSACTION_VERSION_V2_PLACEHOLDER,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_500,
                locking_script: alternate_lock(Network::Mainnet),
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
        let spend_lock = test_lock(Network::Mainnet);
        let output_lock = alternate_lock(Network::Mainnet);
        let valid = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [11; 48],
                output_index: 0,
                unlocking_script: spend_lock.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: output_lock.clone(),
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
                unlocking_script: spend_lock.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: output_lock,
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
        mempool.insert_unchecked(valid_entry);
        mempool.insert_unchecked(invalid_entry);

        let mut utxos = std::collections::BTreeMap::new();
        utxos.insert(
            ([11; 48], 0),
            UtxoEntry::new(
                atho_core::network::Network::Mainnet,
                [11; 48],
                0,
                10_000,
                spend_lock,
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

    #[test]
    fn unchecked_rebuild_tracks_dependency_and_descendant_indexes() {
        let mut mempool = Mempool::new();
        let spend_lock = test_lock(Network::Mainnet);
        let output_lock = alternate_lock(Network::Mainnet);

        let parent = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [0x11; 48],
                output_index: 0,
                unlocking_script: spend_lock.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 8_000,
                locking_script: output_lock.clone(),
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let child = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: parent.txid(),
                output_index: 0,
                unlocking_script: output_lock.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: spend_lock.clone(),
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let grandchild = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: child.txid(),
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: 6_000,
                locking_script: output_lock,
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };

        // Insert in reverse topological order to ensure the unchecked rebuild
        // path reconstructs the relation indexes from the full entry set.
        mempool.insert_unchecked(MempoolEntry::new(grandchild.clone(), 1_000));
        mempool.insert_unchecked(MempoolEntry::new(child.clone(), 1_000));
        mempool.insert_unchecked(MempoolEntry::new(parent.clone(), 1_000));

        assert_eq!(
            mempool.dependency_txids(&parent.txid()),
            Some(Vec::new()),
            "parent has no in-mempool ancestors"
        );
        let dependencies = mempool
            .dependency_txids(&grandchild.txid())
            .expect("grandchild should be indexed");
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies.contains(&parent.txid()));
        assert!(dependencies.contains(&child.txid()));

        let descendants = mempool
            .descendant_txids(&parent.txid())
            .expect("parent should be indexed");
        assert_eq!(descendants.len(), 2);
        assert!(descendants.contains(&child.txid()));
        assert!(descendants.contains(&grandchild.txid()));
    }
}
