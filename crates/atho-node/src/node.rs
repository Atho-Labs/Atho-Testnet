// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Core in-process full-node state machine.
use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::mempool::{Mempool, MempoolEntry, MempoolLimits};
use crate::miner::Miner;
#[cfg(test)]
use crate::test_support::acquire_global_test_lock;
use crate::validation::ValidationError;
use atho_core::block::Block;
use atho_core::block::BlockHeader;
use atho_core::consensus::pow;
#[cfg(test)]
use atho_core::constants::ADDRESS_DIGEST_BYTES;
use atho_core::genesis;
use atho_core::transaction::Transaction;
use atho_p2p::address_manager::{format_remote_addr, parse_remote_addr};
use atho_p2p::protocol::PeerAddress;
use atho_storage::chainstate::{
    ChainSelectionOutcome, ChainSelectionResult, ChainSnapshotBundle,
    Chainstate as StorageChainstate, FinalizedCheckpoint,
};
use atho_storage::db::{
    BlockArchiveRecord, BlockPruneReport, PeerHealthRecord, PeerRecord, TransactionArchiveRecord,
};
use atho_storage::error::StorageError;

fn chain_trace_enabled() -> bool {
    matches!(
        std::env::var("ATHO_DEV_CHAIN_TRACE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

fn should_log_chain_progress(height: u64) -> bool {
    chain_trace_enabled() || height <= 10 || height % 100 == 0
}

#[derive(Debug)]
pub struct Node {
    pub config: NodeConfig,
    chainstate: StorageChainstate,
    pub mempool: Mempool,
    #[cfg(test)]
    _test_lock: crate::test_support::TestLockGuard,
}

impl Node {
    fn mempool_for_config(config: &NodeConfig) -> Mempool {
        Mempool::with_limits(MempoolLimits {
            max_transactions: config.mempool.max_transactions,
            max_vbytes: config.mempool.max_vbytes,
        })
    }

    pub fn new(config: NodeConfig) -> Self {
        #[cfg(test)]
        let test_lock = acquire_global_test_lock();
        let network = config.network;
        #[cfg(not(test))]
        config.apply_process_overrides();
        let mempool = Self::mempool_for_config(&config);
        Self {
            config,
            chainstate: StorageChainstate::new(network),
            mempool,
            #[cfg(test)]
            _test_lock: test_lock,
        }
    }

    pub fn network(&self) -> atho_core::network::Network {
        self.config.network
    }

    pub fn height(&self) -> u64 {
        self.chainstate.height
    }

    pub fn tip_hash(&self) -> [u8; 48] {
        self.chainstate.tip_hash
    }

    pub fn utxo_snapshot(&self) -> atho_storage::utxo::UtxoSet {
        self.chainstate.utxo_snapshot()
    }

    pub fn utxo_entries(&self) -> impl Iterator<Item = &atho_storage::utxo::UtxoEntry> + '_ {
        self.chainstate.utxo_entries()
    }

    pub fn utxo_count(&self) -> usize {
        self.chainstate.utxo_count()
    }

    pub fn utxo_entry(
        &self,
        txid: [u8; 48],
        output_index: u32,
    ) -> Option<atho_storage::utxo::UtxoEntry> {
        self.chainstate.utxo_entry(txid, output_index)
    }

    pub fn blocks_len(&self) -> usize {
        self.chainstate.blocks().len()
    }

    pub fn blocks(&self) -> &[Block] {
        self.chainstate.blocks()
    }

    pub fn block_locator_hashes(&self) -> Vec<[u8; 48]> {
        const MAX_LOCATOR_HASHES: usize = 32;
        const LINEAR_RECENT_HASHES: usize = 10;

        let mut locator = Vec::new();
        let mut height = self.height();
        let mut step = 1u64;

        loop {
            if let Some(record) = self.block_record_by_height(height) {
                if locator.last().copied() != Some(record.block_hash) {
                    locator.push(record.block_hash);
                }
            } else if height == self.height() {
                let tip_hash = self.tip_hash();
                if locator.last().copied() != Some(tip_hash) {
                    locator.push(tip_hash);
                }
            }

            if height == 0 {
                break;
            }
            if locator.len().saturating_add(1) >= MAX_LOCATOR_HASHES {
                break;
            }
            if locator.len() >= LINEAR_RECENT_HASHES {
                step = step.saturating_mul(2);
            }
            height = height.saturating_sub(step);
        }

        let genesis_hash = genesis::genesis_hash(self.network());
        if locator.last().copied() != Some(genesis_hash) {
            if locator.len() >= MAX_LOCATOR_HASHES {
                let _ = locator.pop();
            }
            locator.push(genesis_hash);
        }
        locator
    }

    pub fn chainwork_bytes(&self) -> [u8; 48] {
        let chainwork = self
            .block_record_by_height(self.height())
            .map(|record| record.chainwork)
            .unwrap_or_else(|| pow::accumulated_chain_work(self.blocks()).to_bytes_be());
        fixed_hash48_from_be_bytes(&chainwork)
    }

    pub fn canonical_blocks(&self) -> Result<Vec<Block>, NodeError> {
        self.chainstate.canonical_blocks().map_err(NodeError::from)
    }

    pub fn export_snapshot_bundle(&self) -> Result<ChainSnapshotBundle, NodeError> {
        self.chainstate
            .export_snapshot_bundle()
            .map_err(NodeError::from)
    }

    pub fn import_snapshot_bundle(&mut self, bundle: ChainSnapshotBundle) -> Result<(), NodeError> {
        self.chainstate
            .import_snapshot_bundle(bundle)
            .map_err(NodeError::from)?;
        self.revalidate_mempool_if_needed();
        Ok(())
    }

    pub fn contains_block(&self, block_hash: &[u8; 48]) -> bool {
        self.chainstate.contains_block(*block_hash).unwrap_or(false)
    }

    pub fn is_canonical_block(&self, block_hash: &[u8; 48]) -> bool {
        self.chainstate
            .is_canonical_block(*block_hash)
            .unwrap_or(false)
    }

    pub fn known_block_height(&self, block_hash: &[u8; 48]) -> Option<u64> {
        self.chainstate.known_block_height(*block_hash)
    }

    pub fn block_by_hash(&self, block_hash: [u8; 48]) -> Option<Block> {
        self.chainstate.block_by_hash(block_hash).ok().flatten()
    }

    pub fn block_by_height(&self, height: u64) -> Option<Block> {
        self.chainstate.block_by_height(height).ok().flatten()
    }

    pub fn branch_is_preferred_over_current(&self, branch: &[Block]) -> bool {
        let Some(first) = branch.first() else {
            return false;
        };
        let Some(fork_height) = self.known_block_height(&first.header.previous_block_hash) else {
            return false;
        };
        let Some(fork_block) = self.block_by_height(fork_height) else {
            return false;
        };
        if fork_block.header.block_hash() != first.header.previous_block_hash {
            return false;
        }

        let mut current_branch =
            Vec::with_capacity(self.height().saturating_sub(fork_height) as usize);
        for height in fork_height.saturating_add(1)..=self.height() {
            let Some(block) = self.block_by_height(height) else {
                return false;
            };
            current_branch.push(block);
        }
        pow::branch_is_preferred(branch, &current_branch)
    }

    pub fn block_record_by_hash(&self, block_hash: [u8; 48]) -> Option<BlockArchiveRecord> {
        self.chainstate
            .block_record_by_hash(block_hash)
            .ok()
            .flatten()
    }

    pub fn block_record_by_height(&self, height: u64) -> Option<BlockArchiveRecord> {
        self.chainstate
            .block_record_by_height(height)
            .ok()
            .flatten()
    }

    pub fn transaction_record_by_txid(&self, txid: [u8; 48]) -> Option<TransactionArchiveRecord> {
        self.chainstate
            .transaction_record_by_txid(txid)
            .ok()
            .flatten()
    }

    pub fn prune_depth(&self) -> u64 {
        self.chainstate.prune_depth()
    }

    pub fn max_reorg_depth(&self) -> u64 {
        self.chainstate.max_reorg_depth()
    }

    pub fn finalized_checkpoint(&self) -> Result<Option<FinalizedCheckpoint>, NodeError> {
        self.chainstate
            .finalized_checkpoint()
            .map_err(NodeError::from)
    }

    pub fn last_prune_report(&self) -> Option<&BlockPruneReport> {
        self.chainstate.last_prune_report()
    }

    pub fn last_prune_error(&self) -> Option<&str> {
        self.chainstate.last_prune_error()
    }

    pub fn has_pruned_history(&self) -> bool {
        self.chainstate.has_pruned_history().unwrap_or(false)
    }

    pub fn load_peer_health(
        &self,
        remote_addr: &str,
    ) -> Result<Option<PeerHealthRecord>, NodeError> {
        self.chainstate
            .load_peer_health(remote_addr)
            .map_err(NodeError::from)
    }

    pub fn load_peer_record(&self, remote_addr: &str) -> Result<Option<PeerRecord>, NodeError> {
        self.chainstate
            .load_peer(remote_addr)
            .map_err(NodeError::from)
    }

    pub fn list_peer_records(&self) -> Result<Vec<PeerRecord>, NodeError> {
        self.chainstate.list_peers().map_err(NodeError::from)
    }

    pub fn save_peer_health(&self, record: &PeerHealthRecord) -> Result<(), NodeError> {
        self.chainstate
            .save_peer_health(record)
            .map_err(NodeError::from)
    }

    pub fn save_peer_record(&self, record: &PeerRecord) -> Result<(), NodeError> {
        self.chainstate.save_peer(record).map_err(NodeError::from)
    }

    pub fn observe_peer(
        &self,
        remote_addr: impl Into<String>,
        observed_height: u64,
        observed_unix: u64,
    ) -> Result<(), NodeError> {
        let remote_addr = remote_addr.into();
        let record = PeerRecord {
            network: self.network(),
            remote_addr: remote_addr.clone(),
            first_seen_height: observed_height,
            last_seen_height: observed_height,
            last_seen_unix: observed_unix,
        };
        let merged = match self.load_peer_record(&remote_addr)? {
            Some(existing) => PeerRecord {
                network: self.network(),
                remote_addr,
                first_seen_height: existing.first_seen_height,
                last_seen_height: record.last_seen_height,
                last_seen_unix: existing.last_seen_unix.max(record.last_seen_unix),
            },
            None => record,
        };
        self.save_peer_record(&merged)
    }

    pub fn observe_peer_address(
        &self,
        address: &PeerAddress,
        observed_height: u64,
        observed_unix: u64,
    ) -> Result<(), NodeError> {
        let mut record = PeerRecord {
            network: self.network(),
            remote_addr: format_remote_addr(address),
            first_seen_height: observed_height,
            last_seen_height: observed_height,
            last_seen_unix: observed_unix.max(address.last_seen_unix),
        };
        if let Some(existing) = self.load_peer_record(&record.remote_addr)? {
            record.first_seen_height = existing.first_seen_height;
            record.last_seen_unix = existing.last_seen_unix.max(record.last_seen_unix);
        }
        self.save_peer_record(&record)
    }

    pub fn peer_addresses(&self) -> Result<Vec<PeerAddress>, NodeError> {
        let mut records = self.list_peer_records()?;
        records.sort_by(|left, right| {
            right
                .last_seen_unix
                .cmp(&left.last_seen_unix)
                .then(right.last_seen_height.cmp(&left.last_seen_height))
                .then(left.remote_addr.cmp(&right.remote_addr))
        });
        Ok(records
            .into_iter()
            .filter(|record| record.network == self.network())
            .filter_map(|record| {
                let mut address =
                    parse_remote_addr(&record.remote_addr, self.network().p2p_port())?;
                if address.port == 0 {
                    return None;
                }
                address.last_seen_unix = record.last_seen_unix;
                Some(address)
            })
            .collect())
    }

    pub fn difficulty_target_for_next_block(&self) -> [u8; 48] {
        pow::target_for_next_block(self.network(), self.chainstate.blocks())
    }

    pub fn difficulty_target_for_next_block_at(&self, timestamp: u64) -> [u8; 48] {
        pow::target_for_next_block_with_timestamp(
            self.network(),
            self.chainstate.blocks(),
            timestamp,
        )
    }

    pub fn mempool_len(&self) -> usize {
        self.mempool.len()
    }

    pub fn mempool_total_fee_atoms(&self) -> u64 {
        self.mempool.total_fee_atoms()
    }

    pub fn mempool_fingerprint(&self) -> [u8; 32] {
        self.mempool.fingerprint(self.network())
    }

    pub fn mempool_contains(&self, txid: &[u8; 48]) -> bool {
        self.mempool.contains(txid)
    }

    pub fn mempool_transaction(&self, txid: &[u8; 48]) -> Option<Transaction> {
        self.mempool.transaction(txid)
    }

    pub fn mempool_entry(&self, txid: &[u8; 48]) -> Option<MempoolEntry> {
        self.mempool.entry(txid)
    }

    pub fn mempool_entry_ref(&self, txid: &[u8; 48]) -> Option<&MempoolEntry> {
        self.mempool.entry_ref(txid)
    }

    pub fn mempool_entries(&self) -> Vec<MempoolEntry> {
        self.mempool.entries()
    }

    pub fn mempool_entries_iter(&self) -> impl Iterator<Item = &MempoolEntry> + '_ {
        self.mempool.entries_iter()
    }

    pub fn mempool_dependency_txids(&self, txid: &[u8; 48]) -> Option<Vec<[u8; 48]>> {
        self.mempool.dependency_txids(txid)
    }

    pub fn mempool_descendant_txids(&self, txid: &[u8; 48]) -> Option<Vec<[u8; 48]>> {
        self.mempool.descendant_txids(txid)
    }

    pub fn mempool_transactions(&self) -> Vec<Transaction> {
        self.mempool.transactions()
    }

    pub fn mempool_txids(&self) -> Vec<[u8; 48]> {
        self.mempool.txids()
    }

    pub fn mempool_total_raw_bytes(&self) -> usize {
        self.mempool.total_raw_bytes()
    }

    pub fn mempool_total_vbytes(&self) -> usize {
        self.mempool.total_vbytes()
    }

    #[doc(hidden)]
    pub fn dev_seed_chainstate(
        &mut self,
        height: u64,
        tip_hash: [u8; 48],
        utxos: impl IntoIterator<Item = atho_storage::utxo::UtxoEntry>,
    ) -> Result<(), NodeError> {
        self.chainstate.height = height;
        self.chainstate.tip_hash = tip_hash;
        for utxo in utxos {
            self.chainstate.insert_utxo(utxo)?;
        }
        Ok(())
    }

    pub fn load_or_new(config: NodeConfig) -> Self {
        #[cfg(test)]
        let test_lock = acquire_global_test_lock();
        let network = config.network;
        #[cfg(not(test))]
        config.apply_process_overrides();
        let mempool = Self::mempool_for_config(&config);
        Self {
            config,
            chainstate: StorageChainstate::load_or_new(network),
            mempool,
            #[cfg(test)]
            _test_lock: test_lock,
        }
    }

    pub fn try_load_or_new(config: NodeConfig) -> Result<Self, NodeError> {
        #[cfg(test)]
        let test_lock = acquire_global_test_lock();
        let network = config.network;
        #[cfg(not(test))]
        config.apply_process_overrides();
        let mempool = Self::mempool_for_config(&config);
        Ok(Self {
            config,
            chainstate: StorageChainstate::try_load_or_new(network)?,
            mempool,
            #[cfg(test)]
            _test_lock: test_lock,
        })
    }

    pub fn try_load_or_recover(config: NodeConfig) -> Result<Self, NodeError> {
        #[cfg(test)]
        let test_lock = acquire_global_test_lock();
        let network = config.network;
        #[cfg(not(test))]
        config.apply_process_overrides();
        let mempool = Self::mempool_for_config(&config);
        Ok(Self {
            config,
            chainstate: StorageChainstate::try_load_or_recover(network)?,
            mempool,
            #[cfg(test)]
            _test_lock: test_lock,
        })
    }

    pub fn connect_block(&mut self, block: &Block) -> Result<(), NodeError> {
        if let Err(err) = self.chainstate.connect_block(block) {
            let _ = dev::append_log(
                "athod",
                &format!(
                    "block connect failed height={} error={} {}",
                    block.header.height,
                    err,
                    dev::summarize_block(block)
                ),
            );
            return Err(match err {
                StorageError::Validation(validation) => NodeError::Validation(validation),
                other => NodeError::Storage(other),
            });
        }
        self.mempool.remove_block_transactions(block);
        self.revalidate_mempool_if_needed();
        let mempool_count = self.mempool.len();
        if chain_trace_enabled() {
            let _ = dev::record_block(self.chainstate.height, block);
        } else if should_log_chain_progress(self.chainstate.height) {
            let _ = dev::append_log(
                "chain",
                &format!(
                    "connected height={} hash={} txs={} mempool={mempool_count}",
                    self.chainstate.height,
                    hex::encode(block.header.block_hash()),
                    block.transactions.len()
                ),
            );
        }
        Ok(())
    }

    pub fn admit_transaction(&mut self, entry: MempoolEntry) -> Result<[u8; 48], NodeError> {
        let chainstate = &self.chainstate;
        let txid = self.mempool.admit(
            entry,
            self.network(),
            self.chainstate.height,
            |txid, output_index| chainstate.utxo_entry(*txid, output_index),
        )?;
        Ok(txid)
    }

    pub fn accept_relayed_transaction(
        &mut self,
        transaction: Transaction,
    ) -> Result<[u8; 48], NodeError> {
        if transaction.is_coinbase() {
            return Err(NodeError::Validation(ValidationError::InvalidCoinbase));
        }
        let chainstate = &self.chainstate;
        let fee_atoms = transaction_fee_from_lookup(&transaction, |txid, output_index| {
            chainstate.utxo_entry(*txid, output_index)
        })
        .ok_or(NodeError::Validation(ValidationError::MissingUtxo))?;
        self.submit_transaction(MempoolEntry::new(transaction, fee_atoms))
    }

    pub fn mine_candidate_block(&self, miner: &Miner) -> Result<Block, NodeError> {
        Ok(miner.solve_block(self.build_candidate_block()?))
    }

    pub fn build_candidate_block(&self) -> Result<Block, NodeError> {
        crate::mining::build_candidate_block(self)
    }

    pub fn mine_and_connect_candidate_block(&mut self, miner: &Miner) -> Result<Block, NodeError> {
        let block = self.mine_candidate_block(miner)?;
        self.connect_block(&block)?;
        Ok(block)
    }

    pub fn submit_block(&mut self, block: &Block) -> Result<(), NodeError> {
        self.connect_block(block)
    }

    pub fn consider_branch(&mut self, branch: &[Block]) -> Result<ChainSelectionResult, NodeError> {
        let selection = match self.chainstate.select_branch(branch) {
            Ok(selection) => selection,
            Err(StorageError::ReorgTooDeep {
                depth,
                max_depth,
                fork_height,
                current_height,
            }) => {
                let _ = dev::append_log(
                    "p2p",
                    &format!(
                        "deep reorg rejected depth={} max_depth={} fork_height={} current_height={}",
                        depth, max_depth, fork_height, current_height
                    ),
                );
                return Err(StorageError::ReorgTooDeep {
                    depth,
                    max_depth,
                    fork_height,
                    current_height,
                }
                .into());
            }
            Err(err) => return Err(err.into()),
        };
        match selection.outcome {
            ChainSelectionOutcome::KeptCurrent => return Ok(selection),
            ChainSelectionOutcome::Extended | ChainSelectionOutcome::Reorged => {}
        }

        for block in branch {
            self.mempool.remove_block_transactions(block);
        }

        if selection.outcome == ChainSelectionOutcome::Reorged {
            let chainstate = &self.chainstate;
            for tx in selection
                .disconnected
                .iter()
                .flat_map(|block| block.transactions.iter().skip(1))
            {
                let Some(fee_atoms) = transaction_fee_from_lookup(tx, |txid, output_index| {
                    chainstate.utxo_entry(*txid, output_index)
                }) else {
                    continue;
                };
                let _ = self.mempool.admit(
                    MempoolEntry::new(tx.clone(), fee_atoms),
                    self.network(),
                    self.chainstate.height,
                    |txid, output_index| chainstate.utxo_entry(*txid, output_index),
                );
            }
        }

        self.revalidate_mempool_if_needed();
        Ok(selection)
    }

    pub fn submit_transaction(&mut self, entry: MempoolEntry) -> Result<[u8; 48], NodeError> {
        let txid = self.admit_transaction(entry)?;
        let _ = dev::append_log(
            "chain",
            &format!(
                "accepted transaction txid={} mempool={}",
                hex::encode(txid),
                self.mempool.len()
            ),
        );
        Ok(txid)
    }

    pub fn mempool_spent_inputs(&self) -> Vec<([u8; 48], u32)> {
        self.mempool.spent_inputs_snapshot()
    }

    pub fn headers_after_locator(
        &self,
        locator_hashes: &[[u8; 48]],
        stop_hash: [u8; 48],
        max_headers: usize,
    ) -> Vec<BlockHeader> {
        self.chainstate
            .headers_after_locator(locator_hashes, stop_hash, max_headers)
            .unwrap_or_default()
    }

    fn revalidate_mempool_if_needed(&mut self) {
        if self.mempool.is_empty() {
            return;
        }
        let chainstate = &self.chainstate;
        self.mempool.revalidate(
            self.network(),
            self.chainstate.height,
            |txid, output_index| chainstate.utxo_entry(*txid, output_index),
        );
    }
}

fn transaction_fee_from_lookup<F>(
    tx: &atho_core::transaction::Transaction,
    mut lookup: F,
) -> Option<u64>
where
    F: FnMut(&[u8; 48], u32) -> Option<atho_storage::utxo::UtxoEntry>,
{
    if tx.is_coinbase() {
        return None;
    }
    let mut input_total = 0u64;
    for input in &tx.inputs {
        let utxo = lookup(&input.previous_txid, input.output_index)?;
        input_total = input_total.checked_add(utxo.value_atoms)?;
    }
    input_total.checked_sub(tx.checked_output_value_atoms()?)
}

fn fixed_hash48_from_be_bytes(bytes: &[u8]) -> [u8; 48] {
    let mut out = [0u8; 48];
    let copy_len = bytes.len().min(out.len());
    let out_start = out.len() - copy_len;
    let bytes_start = bytes.len() - copy_len;
    out[out_start..].copy_from_slice(&bytes[bytes_start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use crate::test_support::acquire_global_test_lock;
    use crate::validation::{derive_sig_ref_short, derive_witness_commit_ref};
    use atho_core::address::public_key_digest;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::consensus::tx_policy::{minimum_required_fee_atoms, solve_transaction_pow};
    use atho_core::consensus::{pow, subsidy};
    use atho_core::constants::DUST_RELAY_VALUE_ATOMS;
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};
    use atho_storage::db::{ChainstateSnapshot, Database};
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use atho_storage::utxo::UtxoEntry;
    use std::ffi::OsString;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_lock(network: Network) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-node-test").expect("falcon keypair");
        public_key_digest(network, &keypair.public_key.0).to_vec()
    }

    fn alternate_lock(network: Network) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-node-output").expect("falcon keypair");
        public_key_digest(network, &keypair.public_key.0).to_vec()
    }

    fn witness_bytes_for_tx(network: Network, tx: &Transaction) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-node-test").expect("falcon keypair");
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
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
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
        let sig_bytes = signature.clone();
        TxWitness {
            signature: sig_bytes.clone(),
            pubkey: pubkey.clone(),
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

    fn temp_data_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "atho-node-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn synthetic_coinbase_block(
        network: Network,
        height: u64,
        previous_block_hash: [u8; 48],
        salt: u8,
    ) -> Block {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms_for_network(network, height),
                locking_script: vec![salt ^ height as u8; ADDRESS_DIGEST_BYTES],
            }],
            lock_time: height as u32,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        Block::new(
            BlockHeader {
                version: 1,
                network_id: network,
                height,
                previous_block_hash,
                merkle_root: merkle_root(std::slice::from_ref(&tx)),
                witness_root: witness_root(std::slice::from_ref(&tx)),
                founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
                founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
                timestamp: 1_700_000_000 + u64::from(salt) * 10_000 + height * 75,
                difficulty_target_or_bits: pow::target_for_height(network, height),
                nonce: u64::from(salt) << 32 | height,
            },
            vec![tx],
        )
    }

    fn persist_synthetic_chain(network: Network, height: u64, salt: u8) -> [u8; 48] {
        let genesis = atho_core::genesis::genesis_state(network).block;
        let mut blocks = vec![genesis];
        let mut previous_hash = blocks[0].header.block_hash();
        for next_height in 1..=height {
            let block = synthetic_coinbase_block(network, next_height, previous_hash, salt);
            previous_hash = block.header.block_hash();
            blocks.push(block);
        }
        let snapshot = ChainstateSnapshot {
            height,
            tip_hash: previous_hash,
            tip_header: blocks.last().map(|block| block.header.clone()),
        };
        Database::open(network)
            .expect("database")
            .replace_chainstate(&snapshot, &[], &blocks)
            .expect("replace synthetic chainstate");
        previous_hash
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

    #[test]
    fn node_connect_block_surfaces_storage_errors() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        let genesis_timestamp = atho_core::genesis::genesis_state(Network::Mainnet)
            .block
            .header
            .timestamp;
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![0; ADDRESS_DIGEST_BYTES],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1; ADDRESS_DIGEST_BYTES],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_500,
                locking_script: vec![2; ADDRESS_DIGEST_BYTES],
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
        let transactions = vec![coinbase, tx];
        let header = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: node.chainstate.tip_hash,
            merkle_root: merkle_root(&transactions),
            witness_root: witness_root(&transactions),
            founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
            founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
            timestamp: genesis_timestamp.saturating_add(1),
            difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
            nonce: 0,
        };
        let block = Miner::new(1).solve_block(Block::new(header, transactions));

        let err = node.connect_block(&block).unwrap_err();
        assert!(matches!(
            err,
            NodeError::Validation(crate::validation::ValidationError::MissingUtxo)
        ));
    }

    #[test]
    fn node_rejects_wrong_parent_hash() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        let genesis_timestamp = atho_core::genesis::genesis_state(Network::Mainnet)
            .block
            .header
            .timestamp;
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![0; ADDRESS_DIGEST_BYTES],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1; ADDRESS_DIGEST_BYTES],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![2; ADDRESS_DIGEST_BYTES],
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
        let transactions = vec![coinbase, tx];
        let header = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [1; 48],
            merkle_root: merkle_root(&transactions),
            witness_root: witness_root(&transactions),
            founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
            founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
            timestamp: genesis_timestamp.saturating_add(1),
            difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
            nonce: 0,
        };
        let block = Miner::new(1).solve_block(Block::new(header, transactions));

        let err = node.connect_block(&block).unwrap_err();
        assert!(matches!(
            err,
            NodeError::Validation(crate::validation::ValidationError::BlockParentHashMismatch)
        ));
    }

    #[test]
    fn node_mines_and_connects_candidate_block() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        node.chainstate.height = 6;
        let spend_lock = test_lock(Network::Mainnet);
        let output_lock = alternate_lock(Network::Mainnet);
        node.chainstate
            .insert_utxo(UtxoEntry::new(
                Network::Mainnet,
                [9; 48],
                0,
                2_000,
                spend_lock.clone(),
                0,
                false,
            ))
            .unwrap();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
                output_index: 0,
                unlocking_script: spend_lock,
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: output_lock.clone(),
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
        let fee_atoms = minimum_required_fee_atoms(Network::Mainnet, &tx);
        let tx = Transaction {
            outputs: vec![TxOutput {
                value_atoms: 2_000 - fee_atoms,
                locking_script: output_lock,
            }],
            ..Transaction {
                witness: vec![],
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
                ..tx
            }
        };
        let mut tx = Transaction {
            witness: witness_bytes_for_tx(Network::Mainnet, &tx),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx
        };
        solve_transaction_pow(Network::Mainnet, &mut tx, fee_atoms);
        node.admit_transaction(MempoolEntry::new(tx.clone(), fee_atoms))
            .unwrap();

        let miner = Miner::new(4);
        let block = node.mine_and_connect_candidate_block(&miner).unwrap();
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(node.chainstate.height, 7);
        assert_eq!(node.mempool.len(), 0);
    }

    #[test]
    fn node_rejects_sub_dust_transaction_submission() {
        let mut node = Node::new(NodeConfig::new(Network::Regnet));
        let spend_lock = test_lock(Network::Regnet);
        node.chainstate
            .insert_utxo(UtxoEntry::new(
                Network::Regnet,
                [0x19; 48],
                0,
                2_000,
                spend_lock.clone(),
                0,
                false,
            ))
            .unwrap();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [0x19; 48],
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
        let fee_atoms = 2_000 - (DUST_RELAY_VALUE_ATOMS - 1);

        let err = node
            .submit_transaction(MempoolEntry::new(tx, fee_atoms))
            .unwrap_err();
        assert!(matches!(
            err,
            NodeError::Validation(crate::validation::ValidationError::DustOutput)
        ));
    }

    #[test]
    fn node_mines_two_blocks_in_sequence() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        node.chainstate
            .insert_utxo(UtxoEntry::new(
                Network::Mainnet,
                [9; 48],
                0,
                2_000,
                vec![1],
                0,
                false,
            ))
            .unwrap();
        let miner = Miner::new(2);
        let first = node.mine_and_connect_candidate_block(&miner).unwrap();
        let second = node.mine_and_connect_candidate_block(&miner).unwrap();
        assert_eq!(first.header.height, 1);
        assert_eq!(second.header.height, 2);
        assert_eq!(node.chainstate.height, 2);
    }

    #[test]
    fn candidate_block_rejects_fee_total_overflow() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        node.chainstate.height = 6;
        let locking_script = test_lock(Network::Mainnet);
        let output_lock = alternate_lock(Network::Mainnet);
        for txid in [[0x31; 48], [0x32; 48]] {
            let utxo = UtxoEntry::new(
                Network::Mainnet,
                txid,
                0,
                u64::MAX,
                locking_script.clone(),
                0,
                false,
            );
            node.chainstate.insert_utxo(utxo.clone()).unwrap();
            let mut tx = Transaction {
                version: 1,
                inputs: vec![TxInput {
                    previous_txid: utxo.txid,
                    output_index: utxo.output_index,
                    unlocking_script: locking_script.clone(),
                }],
                outputs: vec![TxOutput {
                    value_atoms: DUST_RELAY_VALUE_ATOMS,
                    locking_script: output_lock.clone(),
                }],
                lock_time: 0,
                witness: vec![],
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
            };
            tx.witness = witness_bytes_for_tx(Network::Mainnet, &tx);
            let fee_atoms = transaction_fee_from_lookup(&tx, |txid, output_index| {
                node.chainstate.utxo_entry(*txid, output_index)
            })
            .expect("fee atoms");
            solve_transaction_pow(Network::Mainnet, &mut tx, fee_atoms);
            node.admit_transaction(MempoolEntry::new(tx, fee_atoms))
                .unwrap();
        }

        let err = node.build_candidate_block().unwrap_err();
        assert!(matches!(
            err,
            NodeError::Validation(crate::validation::ValidationError::FeeMismatch)
        ));
    }

    #[test]
    fn node_merges_peer_observations_without_losing_first_seen_height() {
        let root = temp_data_dir("peer-observation");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let node = Node::load_or_new(NodeConfig::new(Network::Mainnet));
        node.observe_peer("127.0.0.1:56000", 12, 1_700_000_100)
            .expect("first peer observation");
        node.observe_peer("127.0.0.1:56000", 34, 1_700_000_200)
            .expect("second peer observation");

        let record = node
            .load_peer_record("127.0.0.1:56000")
            .expect("load peer record")
            .expect("peer record present");
        assert_eq!(record.first_seen_height, 12);
        assert_eq!(record.last_seen_height, 34);
        assert_eq!(record.remote_addr, "127.0.0.1:56000");
    }

    #[test]
    fn node_peer_addresses_filter_wrong_network_records() {
        let root = temp_data_dir("peer-network-filter");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let node = Node::load_or_new(NodeConfig::new(Network::Mainnet));
        node.save_peer_record(&PeerRecord {
            network: Network::Mainnet,
            remote_addr: String::from("8.8.8.8:56000"),
            first_seen_height: 1,
            last_seen_height: 1,
            last_seen_unix: 1_700_000_100,
        })
        .expect("save mainnet peer");
        node.save_peer_record(&PeerRecord {
            network: Network::Testnet,
            remote_addr: String::from("8.8.4.4:9100"),
            first_seen_height: 1,
            last_seen_height: 1,
            last_seen_unix: 1_700_000_200,
        })
        .expect("save wrong-network peer");

        let peers = node.peer_addresses().expect("peer addresses");
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].host, "8.8.8.8");
        assert_eq!(peers[0].port, 56000);
    }

    #[test]
    fn node_restart_reloads_persisted_chainstate() {
        let root = temp_data_dir("restart");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut node = Node::load_or_new(NodeConfig::new(Network::Mainnet));
        let block = node
            .mine_and_connect_candidate_block(&Miner::new(1))
            .expect("mine");
        let tip_hash = block.header.block_hash();
        let next_target = node.difficulty_target_for_next_block();
        let database = Database::open(Network::Mainnet).expect("database");
        let snapshot = database
            .load_chainstate_snapshot()
            .expect("snapshot")
            .expect("present snapshot");
        assert_eq!(snapshot.height, 1);
        assert_eq!(snapshot.tip_hash, tip_hash);
        let tip_header = snapshot.tip_header.expect("tip header");
        assert_eq!(tip_header.height, 1);
        assert_eq!(tip_header.block_hash(), tip_hash);
        let stored_tip = database
            .load_block(tip_hash)
            .expect("tip block")
            .expect("stored tip");
        let genesis_hash = atho_core::genesis::genesis_hash(Network::Mainnet);
        assert_eq!(stored_tip.header.height, 1);
        assert_eq!(stored_tip.header.previous_block_hash, genesis_hash);
        let stored_genesis = database
            .load_block(genesis_hash)
            .expect("genesis block")
            .expect("stored genesis");
        assert_eq!(stored_genesis.header.height, 0);
        assert_eq!(stored_genesis.header.block_hash(), genesis_hash);
        drop(node);

        let reopened_database = Database::open(Network::Mainnet).expect("reopened database");
        let reopened_snapshot = reopened_database
            .load_chainstate_snapshot()
            .expect("reopened snapshot")
            .expect("reopened snapshot present");
        assert_eq!(reopened_snapshot.height, 1);
        assert_eq!(reopened_snapshot.tip_hash, tip_hash);
        assert!(reopened_database
            .load_block(tip_hash)
            .expect("reopened tip")
            .is_some());
        assert!(reopened_database
            .load_block(genesis_hash)
            .expect("reopened genesis")
            .is_some());

        let reloaded = Node::load_or_new(NodeConfig::new(Network::Mainnet));
        assert_eq!(reloaded.height(), 1);
        assert_eq!(reloaded.tip_hash(), tip_hash);
        assert_eq!(reloaded.difficulty_target_for_next_block(), next_target);
        assert_eq!(reloaded.utxo_count(), 2);
    }

    #[test]
    fn reloaded_node_locator_uses_persisted_history_beyond_recent_window() {
        let root = temp_data_dir("persisted-locator");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let tip_hash = persist_synthetic_chain(Network::Regnet, 30, 11);

        let reloaded = Node::load_or_new(NodeConfig::new(Network::Regnet));
        assert_eq!(reloaded.height(), 30);
        assert_eq!(reloaded.tip_hash(), tip_hash);
        assert!(
            reloaded.blocks_len() < reloaded.height() as usize,
            "test must cover a chain reloaded with only recent blocks in memory"
        );

        let genesis_hash = atho_core::genesis::genesis_hash(Network::Regnet);
        let recent_only_locator = atho_p2p::sync::block_locator(reloaded.blocks());
        assert!(
            !recent_only_locator
                .iter()
                .any(|hash| (*hash).into_inner() == genesis_hash),
            "the old recent-only locator would not find a fork older than the reload window"
        );

        let locator = reloaded.block_locator_hashes();
        assert_eq!(locator.first().copied(), Some(tip_hash));
        assert!(
            locator.contains(&genesis_hash),
            "persistent locators must retain a guaranteed common ancestor"
        );
    }

    #[test]
    fn prunetest_node_mines_and_restarts_in_an_isolated_database_root() {
        let root = temp_data_dir("prunetest-restart");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut node = Node::load_or_new(NodeConfig::new(Network::Prunetest));
        let first = node
            .mine_and_connect_candidate_block(&Miner::new(1))
            .expect("mine first prune block");
        let second = node
            .mine_and_connect_candidate_block(&Miner::new(1))
            .expect("mine second prune block");
        let tip_hash = second.header.block_hash();
        assert_eq!(first.header.network_id, Network::Prunetest);
        assert_eq!(second.header.network_id, Network::Prunetest);
        assert_eq!(node.height(), 2);
        assert_eq!(
            atho_storage::path::database_dir(Network::Prunetest),
            root.join("prunetest")
        );
        drop(node);

        let reloaded = Node::load_or_new(NodeConfig::new(Network::Prunetest));
        assert_eq!(reloaded.height(), 2);
        assert_eq!(reloaded.tip_hash(), tip_hash);
        assert_eq!(reloaded.canonical_blocks().expect("canonical").len(), 3);
    }

    #[test]
    fn node_imports_snapshot_bundle_and_keeps_mining_after_restart() {
        let donor_root = temp_data_dir("snapshot-donor");
        fs::create_dir_all(&donor_root).expect("donor root");
        let donor_bundle = {
            let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &donor_root);
            let mut donor = Node::load_or_new(NodeConfig::new(Network::Regnet));
            donor
                .mine_and_connect_candidate_block(&Miner::new(1))
                .expect("mine donor 1");
            donor
                .mine_and_connect_candidate_block(&Miner::new(1))
                .expect("mine donor 2");
            donor
                .mine_and_connect_candidate_block(&Miner::new(1))
                .expect("mine donor 3");
            donor
                .export_snapshot_bundle()
                .expect("export snapshot bundle")
        };

        let receiver_root = temp_data_dir("snapshot-receiver");
        fs::create_dir_all(&receiver_root).expect("receiver root");
        let imported_tip = donor_bundle.snapshot.tip_hash;
        {
            let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &receiver_root);
            let mut receiver = Node::load_or_new(NodeConfig::new(Network::Regnet));
            receiver
                .import_snapshot_bundle(donor_bundle.clone())
                .expect("import snapshot bundle");
            assert_eq!(receiver.height(), 3);
            assert_eq!(receiver.tip_hash(), imported_tip);
            receiver
                .mine_and_connect_candidate_block(&Miner::new(1))
                .expect("mine after import");
            assert_eq!(receiver.height(), 4);
        }

        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &receiver_root);
        let reloaded = Node::load_or_new(NodeConfig::new(Network::Regnet));
        assert_eq!(reloaded.height(), 4);
        assert_eq!(reloaded.canonical_blocks().expect("canonical").len(), 5);
    }
}
