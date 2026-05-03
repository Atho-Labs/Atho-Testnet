//! In-memory chainstate helpers layered on top of persisted storage.
use crate::db::{
    BlockArchiveRecord, BlockPruneReport, ChainstateSnapshot, Database, PeerHealthRecord,
    PeerRecord,
};
use crate::error::StorageError;
use crate::utxo::{BlockUndo, UtxoEntry, UtxoSet};
use crate::validation;
use atho_core::address::internal_hpk_bytes;
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::constants::{GENESIS_COINBASE_ATOMS, PRUNE_DEPTH_BLOCKS};
use atho_core::genesis;
use atho_core::network::Network;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
struct ChainUndo {
    previous_tip: Option<BlockHeader>,
    previous_tip_hash: [u8; 48],
    block_undo: BlockUndo,
}

#[derive(Debug, Clone)]
struct PersistedChainstate {
    height: u64,
    tip_hash: [u8; 48],
    tip_header: Option<BlockHeader>,
    utxos: Vec<UtxoEntry>,
}

#[derive(Debug, Clone)]
pub struct ChainSnapshotBundle {
    pub snapshot: ChainstateSnapshot,
    pub utxos: Vec<UtxoEntry>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainSelectionOutcome {
    Extended,
    Reorged,
    KeptCurrent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSelectionResult {
    pub outcome: ChainSelectionOutcome,
    pub disconnected: Vec<Block>,
}

#[derive(Debug)]
pub struct Chainstate {
    pub network: Network,
    pub tip: Option<BlockHeader>,
    pub tip_hash: [u8; 48],
    pub height: u64,
    blocks: Vec<Block>,
    utxos: UtxoSet,
    undo_stack: Vec<ChainUndo>,
    storage: Option<Database>,
    last_prune_report: Option<BlockPruneReport>,
    last_prune_error: Option<String>,
}

impl Chainstate {
    pub fn new(network: Network) -> Self {
        Self::fresh(network)
    }

    pub fn fresh(network: Network) -> Self {
        Self::fresh_with_storage(network, None)
    }

    fn fresh_with_storage(network: Network, storage: Option<Database>) -> Self {
        let genesis = genesis::genesis_state(network);
        let genesis_block = genesis.block;
        let genesis_header = genesis_block.header.clone();
        let locking_script = internal_hpk_bytes(network, &genesis.reward_address)
            .unwrap_or_else(|| genesis.reward_address.as_bytes().to_vec());
        let mut utxos = UtxoSet::new(network);
        utxos
            .insert(UtxoEntry::coinbase(
                network,
                genesis.coinbase_txid,
                0,
                GENESIS_COINBASE_ATOMS,
                locking_script,
                0,
            ))
            .expect("genesis utxo is network-local and unique");
        Self {
            network,
            tip: Some(genesis_header),
            tip_hash: genesis.block_hash,
            height: 0,
            blocks: vec![genesis_block],
            utxos,
            undo_stack: Vec::new(),
            storage,
            last_prune_report: None,
            last_prune_error: None,
        }
    }

    pub fn load_or_new(network: Network) -> Self {
        Self::try_load_or_recover(network)
            .unwrap_or_else(|err| panic!("failed to load chainstate for {}: {}", network.id(), err))
    }

    pub fn try_load_or_recover(network: Network) -> Result<Self, StorageError> {
        match Self::try_load_or_new(network) {
            Ok(chainstate) => Ok(chainstate),
            Err(err) if err.is_recoverable_local_state() => {
                quarantine_persisted_state(network, &err)?;
                Self::try_load_or_new(network)
            }
            Err(err) => Err(err),
        }
    }

    pub fn try_load_or_new(network: Network) -> Result<Self, StorageError> {
        let storage = Database::open(network)?;
        if let Some(persisted) = load_persisted_chainstate(network, &storage)? {
            let chainstate = persisted.try_into_chainstate(network, Some(storage))?;
            chainstate.save_persisted_chainstate()?;
            return Ok(chainstate);
        }

        let chainstate = Self::fresh_with_storage(network, Some(storage));
        if let Some(genesis_block) = chainstate.blocks.first().cloned() {
            if let Some(storage) = chainstate.storage.as_ref() {
                let snapshot = ChainstateSnapshot {
                    height: 0,
                    tip_hash: chainstate.tip_hash,
                    tip_header: chainstate.tip.clone(),
                };
                let utxos: Vec<_> = chainstate.utxos.entries().cloned().collect();
                storage.commit_chainstate(&snapshot, &utxos, Some((0, &genesis_block)))?;
            }
        }
        Ok(chainstate)
    }

    pub fn connect_header(&mut self, header: BlockHeader) {
        self.tip_hash = header.block_hash();
        self.tip = Some(header);
        self.height = self.tip.as_ref().map(|header| header.height).unwrap_or(0);
    }

    pub fn next_difficulty_target(&self) -> [u8; 48] {
        pow::target_for_next_block(self.network, &self.blocks)
    }

    pub fn connect_block(&mut self, block: &Block) -> Result<(), StorageError> {
        let working_utxos = self.utxos.clone();
        validation::validate_block_with_context(
            block,
            self.height.saturating_add(1),
            self.network,
            self.tip_hash,
            self.next_difficulty_target(),
            &self.blocks,
            working_utxos,
        )?;

        let undo = self.utxos.apply_block(block)?;
        let previous_tip = self.tip.clone();
        let previous_tip_hash = self.tip_hash;
        let next_tip_hash = block.header.block_hash();
        if let Some(storage) = &self.storage {
            let snapshot = ChainstateSnapshot {
                height: block.header.height,
                tip_hash: next_tip_hash,
                tip_header: Some(block.header.clone()),
            };
            let utxos: Vec<_> = self.utxos.entries().cloned().collect();
            if let Err(err) =
                storage.commit_chainstate(&snapshot, &utxos, Some((block.header.height, block)))
            {
                self.utxos.disconnect_block(undo);
                return Err(err);
            }
        }

        self.tip = Some(block.header.clone());
        self.tip_hash = next_tip_hash;
        self.height = block.header.height;
        self.blocks.push(block.clone());
        self.undo_stack.push(ChainUndo {
            previous_tip,
            previous_tip_hash,
            block_undo: undo,
        });
        self.prune_history();
        Ok(())
    }

    pub fn select_branch(
        &mut self,
        branch: &[Block],
    ) -> Result<ChainSelectionResult, StorageError> {
        if branch.is_empty() {
            return Err(StorageError::EmptyBranch);
        }
        validate_branch_sequence(branch)?;
        let fork_hash = branch[0].header.previous_block_hash;
        let Some(fork_index) = self
            .blocks
            .iter()
            .position(|block| block.header.block_hash() == fork_hash)
        else {
            return Err(StorageError::ForkPointUnavailable);
        };
        let fork_height = self.blocks[fork_index].header.height;
        if branch[0].header.height != fork_height.saturating_add(1) {
            return Err(StorageError::InvalidBranchSequence);
        }

        let disconnected = self.blocks[fork_index + 1..].to_vec();
        if !disconnected.is_empty() && !pow::branch_is_preferred(branch, &disconnected) {
            return Ok(ChainSelectionResult {
                outcome: ChainSelectionOutcome::KeptCurrent,
                disconnected: Vec::new(),
            });
        }

        let original_tip = self.tip.clone();
        let original_tip_hash = self.tip_hash;
        let original_height = self.height;
        let original_blocks = self.blocks.clone();
        let original_utxos = self.utxos.clone();
        let original_undo_stack = self.undo_stack.clone();

        for _ in 0..disconnected.len() {
            self.disconnect_last_block()?;
        }

        for block in branch {
            if let Err(err) = self.connect_block(block) {
                self.restore_chainstate_state(
                    original_tip,
                    original_tip_hash,
                    original_height,
                    original_blocks,
                    original_utxos,
                    original_undo_stack,
                )?;
                return Err(err);
            }
        }

        Ok(ChainSelectionResult {
            outcome: if disconnected.is_empty() {
                ChainSelectionOutcome::Extended
            } else {
                ChainSelectionOutcome::Reorged
            },
            disconnected,
        })
    }

    pub fn utxo_snapshot(&self) -> UtxoSet {
        self.utxos.clone()
    }

    pub fn utxo_entry(&self, txid: [u8; 48], output_index: u32) -> Option<UtxoEntry> {
        self.utxos.get(txid, output_index).cloned()
    }

    pub fn block_by_hash(&self, block_hash: [u8; 48]) -> Result<Option<Block>, StorageError> {
        if let Some(block) = self
            .blocks
            .iter()
            .find(|block| block.header.block_hash() == block_hash)
            .cloned()
        {
            return Ok(Some(block));
        }
        let Some(storage) = &self.storage else {
            return Ok(None);
        };
        storage.load_block(block_hash)
    }

    pub fn contains_block(&self, block_hash: [u8; 48]) -> Result<bool, StorageError> {
        Ok(self.block_by_hash(block_hash)?.is_some())
    }

    pub fn block_record_by_hash(
        &self,
        block_hash: [u8; 48],
    ) -> Result<Option<BlockArchiveRecord>, StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(self
                .blocks
                .iter()
                .position(|block| block.header.block_hash() == block_hash)
                .map(|index| {
                    let block = &self.blocks[index];
                    BlockArchiveRecord {
                        height: block.header.height,
                        block_hash,
                        previous_block_hash: block.header.previous_block_hash,
                        network: block.header.network_id,
                        version: block.header.version,
                        merkle_root: block.header.merkle_root,
                        witness_root: block.header.witness_root,
                        timestamp: block.header.timestamp,
                        difficulty_target_or_bits: block.header.difficulty_target_or_bits,
                        nonce: block.header.nonce,
                        file_number: 0,
                        record_offset: 0,
                        payload_length: 0,
                        raw_block_size: block.canonical_bytes().len() as u32,
                        weight_bytes: block.weight_bytes() as u32,
                        vsize_bytes: block.vsize_bytes() as u32,
                        tx_count: block.transactions.len() as u32,
                        fees_total_atoms: block.fees_total_atoms,
                        fees_miner_atoms: block.fees_miner_atoms,
                        chainwork: pow::accumulated_chain_work(&self.blocks[..=index])
                            .to_bytes_be(),
                        fully_validated: true,
                        main_chain: true,
                        pruned: false,
                        persisted_unix: 0,
                    }
                }));
        };
        storage.load_block_record(block_hash)
    }

    pub fn block_record_by_height(
        &self,
        height: u64,
    ) -> Result<Option<BlockArchiveRecord>, StorageError> {
        if let Some(block) = self
            .blocks
            .iter()
            .find(|block| block.header.height == height)
        {
            return self.block_record_by_hash(block.header.block_hash());
        }
        let Some(storage) = &self.storage else {
            return Ok(None);
        };
        storage.load_block_record_by_height(height)
    }

    pub fn export_snapshot_bundle(&self) -> Result<ChainSnapshotBundle, StorageError> {
        Ok(ChainSnapshotBundle {
            snapshot: ChainstateSnapshot {
                height: self.height,
                tip_hash: self.tip_hash,
                tip_header: self.tip.clone(),
            },
            utxos: self.utxos.entries().cloned().collect(),
            blocks: self.canonical_blocks()?,
        })
    }

    pub fn import_snapshot_bundle(
        &mut self,
        bundle: ChainSnapshotBundle,
    ) -> Result<(), StorageError> {
        let Some(last) = bundle.blocks.last() else {
            return Err(StorageError::IncompleteBlockHistory);
        };
        if last.header.height != bundle.snapshot.height
            || last.header.block_hash() != bundle.snapshot.tip_hash
        {
            return Err(StorageError::PersistedTipMismatch);
        }
        let Some(first) = bundle.blocks.first() else {
            return Err(StorageError::IncompleteBlockHistory);
        };
        if first.header.height != 0
            || first.header.block_hash() != genesis::genesis_hash(self.network)
        {
            return Err(StorageError::PersistedGenesisMismatch);
        }

        let mut utxos = UtxoSet::new(self.network);
        for entry in &bundle.utxos {
            utxos.insert(entry.clone())?;
        }

        if let Some(storage) = &self.storage {
            storage.replace_chainstate(&bundle.snapshot, &bundle.utxos, &bundle.blocks)?;
        }

        self.tip = bundle.snapshot.tip_header;
        self.tip_hash = bundle.snapshot.tip_hash;
        self.height = bundle.snapshot.height;
        self.blocks = bundle.blocks;
        self.utxos = utxos;
        self.undo_stack.clear();
        self.prune_history();
        Ok(())
    }

    pub fn load_peer_health(
        &self,
        remote_addr: &str,
    ) -> Result<Option<PeerHealthRecord>, StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(None);
        };
        storage.load_peer_health(remote_addr)
    }

    pub fn load_peer(&self, remote_addr: &str) -> Result<Option<PeerRecord>, StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(None);
        };
        storage.load_peer(remote_addr)
    }

    pub fn list_peers(&self) -> Result<Vec<PeerRecord>, StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(Vec::new());
        };
        storage.list_peers()
    }

    pub fn save_peer_health(&self, record: &PeerHealthRecord) -> Result<(), StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        storage.upsert_peer_health(record)
    }

    pub fn save_peer(&self, record: &PeerRecord) -> Result<(), StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        storage.upsert_peer(record)
    }

    pub fn canonical_blocks(&self) -> Result<Vec<Block>, StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(self.blocks.clone());
        };

        let mut remaining_height = self.height;
        let mut next_hash = self.tip_hash;
        let mut reversed = Vec::with_capacity(self.height.saturating_add(1) as usize);

        loop {
            let block = storage
                .load_block(next_hash)?
                .ok_or(StorageError::IncompleteBlockHistory)?;
            if block.header.block_hash() != next_hash || block.header.height != remaining_height {
                return Err(StorageError::IncompleteBlockHistory);
            }
            let is_genesis = block.header.height == 0;
            next_hash = block.header.previous_block_hash;
            reversed.push(block);
            if is_genesis {
                break;
            }
            remaining_height = remaining_height.saturating_sub(1);
        }

        reversed.reverse();
        Ok(reversed)
    }

    pub fn block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError> {
        if let Some(block) = self
            .blocks
            .iter()
            .find(|block| block.header.height == height)
            .cloned()
        {
            return Ok(Some(block));
        }
        let Some(storage) = &self.storage else {
            return Ok(None);
        };
        let Some(block_hash) = storage.load_block_hash_by_height(height)? else {
            return Ok(None);
        };
        storage.load_block(block_hash)
    }

    pub fn headers_after_locator(
        &self,
        locator_hashes: &[[u8; 48]],
        stop_hash: [u8; 48],
        max_headers: usize,
    ) -> Result<Vec<BlockHeader>, StorageError> {
        if max_headers == 0 {
            return Ok(Vec::new());
        }

        let start_height = match locator_hashes.iter().find_map(|hash| {
            self.height_for_known_block(*hash)
                .map(|height| height.saturating_add(1))
        }) {
            Some(height) => height,
            None if locator_hashes.is_empty() => 0,
            None => return Ok(Vec::new()),
        };

        let Some(storage) = &self.storage else {
            let blocks = &self.blocks;
            if blocks.is_empty() {
                return Ok(Vec::new());
            }
            let start_index = match locator_hashes.iter().find_map(|hash| {
                blocks
                    .iter()
                    .position(|block| block.header.block_hash() == *hash)
                    .map(|index| index.saturating_add(1))
            }) {
                Some(index) => index,
                None if locator_hashes.is_empty() => 0,
                None => return Ok(Vec::new()),
            };

            let mut headers = Vec::new();
            for block in blocks.iter().skip(start_index) {
                if headers.len() >= max_headers {
                    break;
                }
                headers.push(block.header.clone());
                if stop_hash != [0; 48] && block.header.block_hash() == stop_hash {
                    break;
                }
            }
            return Ok(headers);
        };

        let mut headers = Vec::new();
        let mut next_height = start_height;
        while headers.len() < max_headers && next_height <= self.height {
            let Some(record) = storage.load_block_record_by_height(next_height)? else {
                break;
            };
            headers.push(record.header());
            if stop_hash != [0; 48] && record.block_hash == stop_hash {
                break;
            }
            next_height = next_height.saturating_add(1);
        }
        Ok(headers)
    }

    pub fn prune_depth(&self) -> u64 {
        effective_prune_depth(self.network)
    }

    pub fn last_prune_report(&self) -> Option<&BlockPruneReport> {
        self.last_prune_report.as_ref()
    }

    pub fn last_prune_error(&self) -> Option<&str> {
        self.last_prune_error.as_deref()
    }

    pub fn has_pruned_history(&self) -> Result<bool, StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(false);
        };
        storage.has_pruned_blocks()
    }

    pub fn disconnect_last_block(&mut self) -> Result<(), StorageError> {
        let undo = self
            .undo_stack
            .last()
            .cloned()
            .ok_or(StorageError::NoBlockToDisconnect)?;
        let removed_block = self
            .blocks
            .last()
            .cloned()
            .ok_or(StorageError::NoBlockToDisconnect)?;

        self.utxos.disconnect_block(undo.block_undo.clone());
        let previous_height = undo
            .previous_tip
            .as_ref()
            .map(|header| header.height)
            .unwrap_or(0);
        if let Err(err) = self.persist_snapshot_for(
            previous_height,
            undo.previous_tip_hash,
            undo.previous_tip.clone(),
        ) {
            let reconnect = self.utxos.apply_block(&removed_block);
            debug_assert!(
                reconnect.is_ok(),
                "failed to restore block after snapshot failure"
            );
            return Err(err);
        }

        let _ = self.undo_stack.pop();
        let _ = self.blocks.pop();
        self.tip = undo.previous_tip;
        self.tip_hash = undo.previous_tip_hash;
        self.height = previous_height;
        Ok(())
    }

    pub fn utxo_count(&self) -> usize {
        self.utxos.len()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn insert_utxo(&mut self, entry: UtxoEntry) -> Result<(), StorageError> {
        self.utxos.insert(entry)
    }

    fn prune_history(&mut self) {
        let prune_depth = effective_prune_depth(self.network);
        let retain = usize::try_from(prune_depth.saturating_add(1)).unwrap_or(usize::MAX);
        self.prune_history_to_retain(retain);
        if let Some(storage) = &self.storage {
            match storage.prune_archived_blocks(self.height, prune_depth) {
                Ok(report) => {
                    self.last_prune_error = None;
                    self.last_prune_report = Some(report);
                }
                Err(err) => {
                    self.last_prune_error = Some(err.to_string());
                }
            }
        }
    }

    fn prune_history_to_retain(&mut self, retain: usize) {
        if self.blocks.len() <= retain || retain == 0 {
            return;
        }
        let prune_count = self.blocks.len().saturating_sub(retain);
        if prune_count == 0 {
            return;
        }
        self.blocks.drain(1..1 + prune_count);
        let undo_prune_count = prune_count.min(self.undo_stack.len());
        self.undo_stack.drain(0..undo_prune_count);
    }

    fn save_persisted_chainstate(&self) -> Result<(), StorageError> {
        self.persist_snapshot_for(self.height, self.tip_hash, self.tip.clone())
    }

    fn persist_snapshot_for(
        &self,
        height: u64,
        tip_hash: [u8; 48],
        tip_header: Option<BlockHeader>,
    ) -> Result<(), StorageError> {
        if let Some(storage) = &self.storage {
            let snapshot = ChainstateSnapshot {
                height,
                tip_hash,
                tip_header,
            };
            let utxos: Vec<_> = self.utxos.entries().cloned().collect();
            storage.save_chainstate_snapshot(&snapshot, &utxos)?;
        }
        Ok(())
    }

    fn restore_chainstate_state(
        &mut self,
        tip: Option<BlockHeader>,
        tip_hash: [u8; 48],
        height: u64,
        blocks: Vec<Block>,
        utxos: UtxoSet,
        undo_stack: Vec<ChainUndo>,
    ) -> Result<(), StorageError> {
        self.tip = tip;
        self.tip_hash = tip_hash;
        self.height = height;
        self.blocks = blocks;
        self.utxos = utxos;
        self.undo_stack = undo_stack;
        self.persist_snapshot_for(self.height, self.tip_hash, self.tip.clone())
    }

    fn height_for_known_block(&self, block_hash: [u8; 48]) -> Option<u64> {
        if let Some(block) = self
            .blocks
            .iter()
            .find(|block| block.header.block_hash() == block_hash)
        {
            return Some(block.header.height);
        }
        self.storage
            .as_ref()
            .and_then(|storage| storage.load_block_record(block_hash).ok().flatten())
            .map(|record| record.height)
    }
}

impl PersistedChainstate {
    fn try_into_chainstate(
        self,
        network: Network,
        storage: Option<Database>,
    ) -> Result<Chainstate, StorageError> {
        let genesis = genesis::genesis_state(network);
        let genesis_block = genesis.block;
        let mut utxos = UtxoSet::new(network);
        for entry in self.utxos {
            utxos.insert(entry)?;
        }

        if self.height == 0 {
            if self.tip_hash != genesis_block.header.block_hash() {
                return Err(StorageError::PersistedGenesisMismatch);
            }
            if utxos.get(genesis.coinbase_txid, 0).is_none() {
                return Err(StorageError::PersistedGenesisMismatch);
            }
        }

        let tip = match self.tip_header {
            Some(header) => {
                if header.height != self.height || header.block_hash() != self.tip_hash {
                    return Err(StorageError::PersistedTipMismatch);
                }
                Some(header)
            }
            None => storage
                .as_ref()
                .and_then(|db| db.load_block(self.tip_hash).ok().flatten())
                .map(|block| block.header),
        };

        let blocks = if self.height == 0 {
            vec![genesis_block]
        } else {
            let db = storage
                .as_ref()
                .ok_or(StorageError::IncompleteBlockHistory)?;
            let blocks = load_recent_blocks_from_storage(db, self.tip_hash, 27)?;
            let Some(last) = blocks.last() else {
                return Err(StorageError::IncompleteBlockHistory);
            };
            if last.header.height != self.height || last.header.block_hash() != self.tip_hash {
                return Err(StorageError::PersistedTipMismatch);
            }
            blocks
        };

        Ok(Chainstate {
            network,
            tip,
            tip_hash: self.tip_hash,
            height: self.height,
            blocks,
            utxos,
            undo_stack: Vec::new(),
            storage,
            last_prune_report: None,
            last_prune_error: None,
        })
    }
}

fn validate_branch_sequence(branch: &[Block]) -> Result<(), StorageError> {
    for window in branch.windows(2) {
        let [left, right] = window else {
            continue;
        };
        if right.header.previous_block_hash != left.header.block_hash()
            || right.header.height != left.header.height.saturating_add(1)
        {
            return Err(StorageError::InvalidBranchSequence);
        }
    }
    Ok(())
}

fn chain_dir() -> PathBuf {
    crate::path::chain_dir()
}

fn quarantine_persisted_state(
    network: Network,
    source_error: &StorageError,
) -> Result<(), StorageError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let label = match source_error {
        StorageError::CorruptData => "corrupt-data",
        StorageError::PersistedGenesisMismatch => "genesis-mismatch",
        StorageError::PersistedTipMismatch => "tip-mismatch",
        StorageError::IncompleteBlockHistory => "incomplete-history",
        StorageError::LegacyStorageLayout => "legacy-layout",
        StorageError::SchemaVersionMismatch { .. } => "schema-mismatch",
        _ => "recovery",
    };
    let quarantine_root = crate::path::quarantine_dir()
        .join(network.id())
        .join(format!("{timestamp}-{label}"));
    fs::create_dir_all(&quarantine_root)?;

    move_if_exists(
        crate::path::database_dir(network),
        quarantine_root.join("db").join(network.id()),
    )?;
    move_if_exists(
        chainstate_snapshot_path(network),
        quarantine_root
            .join("chain")
            .join(format!("chainstate-{}.tsv", network.id())),
    )?;
    move_if_exists(
        utxo_snapshot_path(network),
        quarantine_root
            .join("chain")
            .join(format!("utxos-{}.tsv", network.id())),
    )?;
    move_if_exists(
        blocks_ledger_path(),
        quarantine_root.join("chain").join("blocks.tsv"),
    )?;
    move_if_exists(
        transactions_ledger_path(),
        quarantine_root.join("chain").join("transactions.tsv"),
    )?;
    move_if_exists(
        transaction_inputs_ledger_path(),
        quarantine_root.join("chain").join("transaction_inputs.tsv"),
    )?;
    move_if_exists(
        transaction_outputs_ledger_path(),
        quarantine_root
            .join("chain")
            .join("transaction_outputs.tsv"),
    )?;

    let mut report = File::create(quarantine_root.join("RECOVERY.txt"))?;
    writeln!(report, "network={}", network.id())?;
    writeln!(report, "error={source_error}")?;
    writeln!(report, "timestamp={timestamp}")?;
    Ok(())
}

fn move_if_exists(source: PathBuf, destination: PathBuf) -> Result<(), StorageError> {
    if !source.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

fn chainstate_snapshot_path(network: Network) -> PathBuf {
    chain_dir().join(format!("chainstate-{}.tsv", network.id()))
}

fn utxo_snapshot_path(network: Network) -> PathBuf {
    chain_dir().join(format!("utxos-{}.tsv", network.id()))
}

fn blocks_ledger_path() -> PathBuf {
    chain_dir().join("blocks.tsv")
}

fn transactions_ledger_path() -> PathBuf {
    chain_dir().join("transactions.tsv")
}

fn transaction_inputs_ledger_path() -> PathBuf {
    chain_dir().join("transaction_inputs.tsv")
}

fn transaction_outputs_ledger_path() -> PathBuf {
    chain_dir().join("transaction_outputs.tsv")
}

fn load_persisted_chainstate(
    network: Network,
    storage: &Database,
) -> Result<Option<PersistedChainstate>, StorageError> {
    if let Some(snapshot) = storage.load_chainstate_snapshot()? {
        let utxos = storage.load_utxos()?;
        return Ok(Some(PersistedChainstate {
            height: snapshot.height,
            tip_hash: snapshot.tip_hash,
            tip_header: snapshot.tip_header,
            utxos,
        }));
    }

    // MIGRATION: legacy TSV snapshots and TSV chain exports are quarantine-only
    // recovery inputs now. The production runtime only boots from the LMDB
    // chainstate snapshot plus the flat raw-block archive.
    if legacy_persisted_state_present(network) {
        return Err(StorageError::LegacyStorageLayout);
    }
    Ok(None)
}

fn load_recent_blocks_from_storage(
    storage: &Database,
    tip_hash: [u8; 48],
    limit: usize,
) -> Result<Vec<Block>, StorageError> {
    let mut blocks = Vec::new();
    let mut next_hash = tip_hash;
    while blocks.len() < limit {
        let Some(block) = storage.load_block(next_hash)? else {
            if blocks.is_empty() {
                return Ok(Vec::new());
            }
            return Err(StorageError::IncompleteBlockHistory);
        };
        next_hash = block.header.previous_block_hash;
        blocks.push(block);
        if blocks
            .last()
            .map(|block| block.header.height)
            .unwrap_or_default()
            == 0
        {
            break;
        }
    }
    blocks.reverse();
    Ok(blocks)
}

fn effective_prune_depth(network: Network) -> u64 {
    if network == Network::Prunetest {
        if let Ok(raw) = std::env::var("ATHO_PRUNETEST_PRUNE_DEPTH") {
            if let Ok(value) = raw.parse::<u64>() {
                return value.max(1);
            }
        }
    }
    PRUNE_DEPTH_BLOCKS
}

fn legacy_persisted_state_present(network: Network) -> bool {
    let snapshot_present =
        chainstate_snapshot_path(network).exists() || utxo_snapshot_path(network).exists();
    let audit_chain_present = blocks_ledger_path().exists()
        || transactions_ledger_path().exists()
        || transaction_inputs_ledger_path().exists()
        || transaction_outputs_ledger_path().exists();
    snapshot_present || audit_chain_present
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_files::BlockFileStore;
    use crate::db::{ChainstateSnapshot, CommitFaultPoint, Database};
    use crate::error::StorageError;
    use crate::test_support::acquire_global_test_lock;
    use crate::utxo::UtxoEntry;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::subsidy;
    use atho_core::crypto::hash::sha3_384;
    use atho_core::genesis;
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxOutput};
    use lmdb::{Environment, Transaction as LmdbTransaction, WriteFlags};
    use std::ffi::OsString;
    use std::fs;
    use std::sync::MutexGuard;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "atho-chainstate-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    struct CurrentDirGuard {
        previous: std::path::PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl CurrentDirGuard {
        fn switch_to(path: &std::path::Path) -> Self {
            let lock = acquire_global_test_lock();
            let previous = std::env::current_dir().expect("cwd");
            std::env::set_current_dir(path).expect("set cwd");
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
        }
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
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

    struct RotationOverrideGuard;

    impl RotationOverrideGuard {
        fn set(bytes: u64) -> Self {
            BlockFileStore::set_rotation_override_for_test(Some(bytes));
            Self
        }
    }

    impl Drop for RotationOverrideGuard {
        fn drop(&mut self) {
            BlockFileStore::set_rotation_override_for_test(None);
        }
    }

    fn solve_block(mut block: Block) -> Block {
        let prefix = block.header.canonical_bytes_without_nonce();
        let target = block.header.difficulty_target_or_bits;
        let mut bytes = Vec::with_capacity(prefix.len() + 8);
        bytes.extend_from_slice(&prefix);
        bytes.resize(prefix.len() + 8, 0);
        for nonce in 0u64.. {
            bytes[prefix.len()..].copy_from_slice(&nonce.to_le_bytes());
            if atho_core::consensus::pow::meets_target(&sha3_384(&bytes), &target) {
                block.header.nonce = nonce;
                return block;
            }
        }
        unreachable!("u64 nonce space exhausted");
    }

    fn build_coinbase_successor(state: &Chainstate) -> Block {
        let height = state.height.saturating_add(1);
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(height),
                locking_script: vec![height as u8],
            }],
            lock_time: u32::try_from(height).unwrap_or(u32::MAX),
            witness: vec![],
        };
        let transactions = vec![coinbase];
        let previous_timestamp = state
            .tip
            .as_ref()
            .map(|header| header.timestamp)
            .unwrap_or_else(|| genesis::genesis_state(state.network).block.header.timestamp);
        solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: state.network,
                height,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: previous_timestamp.saturating_add(1),
                difficulty_target_or_bits: state.next_difficulty_target(),
                nonce: 0,
            },
            transactions,
        ))
    }

    fn fixture_utxo_key(txid: [u8; 48], output_index: u32) -> Vec<u8> {
        let mut key = Vec::with_capacity(52);
        key.extend_from_slice(&txid);
        key.extend_from_slice(&output_index.to_be_bytes());
        key
    }

    fn inject_snapshot_fixture(
        network: Network,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
    ) {
        let database = Database::open(network).expect("database");
        let db_path = crate::path::database_dir(network);
        drop(database);

        let mut builder = Environment::new();
        builder
            .set_max_readers(128)
            .set_max_dbs(10)
            .set_map_size(1 << 30);
        let env = builder.open(&db_path).expect("open env");
        let meta = env.open_db(Some("meta")).expect("meta db");
        let utxo_db = env.open_db(Some("utxos")).expect("utxos db");
        let mut txn = env.begin_rw_txn().expect("rw txn");
        let snapshot_bytes = bincode::serialize(snapshot).expect("serialize snapshot");
        txn.put(meta, b"chainstate", &snapshot_bytes, WriteFlags::empty())
            .expect("put snapshot");
        for utxo in utxos {
            let key = fixture_utxo_key(utxo.txid, utxo.output_index);
            let value = bincode::serialize(utxo).expect("serialize utxo");
            txn.put(utxo_db, &key.as_slice(), &value, WriteFlags::empty())
                .expect("put utxo");
        }
        txn.commit().expect("commit fixture");
    }

    #[test]
    fn chainstate_tracks_tip_and_height() {
        let mut state = Chainstate::new(Network::Mainnet);
        assert_eq!(state.height, 0);
        assert!(state.tip.is_some());
        assert_ne!(state.tip_hash, [0; 48]);
        assert_eq!(state.blocks().len(), 1);

        state.connect_header(BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [0; 48],
            merkle_root: [0; 48],
            witness_root: [0; 48],
            timestamp: 75,
            difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                Network::Mainnet,
            ),
            nonce: 0,
        });

        assert_eq!(state.height, 1);
        assert!(state.tip.is_some());
    }

    #[test]
    fn chainstate_connects_and_disconnects_blocks() {
        let mut state = Chainstate::new(Network::Mainnet);
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![9],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let transactions = vec![coinbase];
        let block = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: genesis::genesis_state(Network::Mainnet)
                    .block
                    .header
                    .timestamp
                    .saturating_add(1),
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            transactions,
        ));

        state.connect_block(&block).unwrap();
        assert_eq!(state.height, 1);
        assert_eq!(state.utxo_count(), 2);
        assert_eq!(state.blocks().len(), 2);

        state.disconnect_last_block().unwrap();
        assert_eq!(state.height, 0);
        assert_eq!(state.utxo_count(), 1);
        assert_eq!(state.blocks().len(), 1);
    }

    #[test]
    fn invalid_block_is_rejected_without_mutating_chainstate() {
        let mut state = Chainstate::new(Network::Mainnet);
        let before_height = state.height;
        let before_tip_hash = state.tip_hash;
        let before_utxo_count = state.utxo_count();

        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![9],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let transactions = vec![coinbase];
        let block = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [1; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: genesis::genesis_state(Network::Mainnet)
                    .block
                    .header
                    .timestamp
                    .saturating_add(1),
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            transactions,
        ));

        let err = state.connect_block(&block).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Validation(validation::ValidationError::BlockParentHashMismatch)
        ));
        assert_eq!(state.height, before_height);
        assert_eq!(state.tip_hash, before_tip_hash);
        assert_eq!(state.utxo_count(), before_utxo_count);
        assert_eq!(state.blocks().len(), 1);
    }

    #[test]
    fn commit_fault_injection_rolls_back_chainstate_mutation() {
        let root = temp_workspace("fault-injection");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);
        let mut state = Chainstate::try_load_or_new(Network::Mainnet).expect("state");
        let before_height = state.height;
        let before_tip_hash = state.tip_hash;
        let before_utxo_count = state.utxo_count();

        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![9],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let transactions = vec![coinbase];
        let block = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: genesis::genesis_state(Network::Mainnet)
                    .block
                    .header
                    .timestamp
                    .saturating_add(1),
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            transactions,
        ));

        Database::inject_commit_fault_for_test(CommitFaultPoint::BeforeCommit, 1);
        let result = state.connect_block(&block);
        Database::clear_commit_fault_for_test();

        assert!(matches!(result, Err(StorageError::Io(_))));
        assert_eq!(state.height, before_height);
        assert_eq!(state.tip_hash, before_tip_hash);
        assert_eq!(state.utxo_count(), before_utxo_count);
        assert_eq!(state.blocks().len(), 1);
    }

    #[test]
    fn chainstate_prunes_old_history_after_retention_window() {
        let mut state = Chainstate::new(Network::Mainnet);
        state.blocks = vec![
            state.blocks[0].clone(),
            state.blocks[0].clone(),
            state.blocks[0].clone(),
        ];
        state.undo_stack = vec![
            ChainUndo {
                previous_tip: None,
                previous_tip_hash: [0; 48],
                block_undo: BlockUndo::empty(),
            },
            ChainUndo {
                previous_tip: None,
                previous_tip_hash: [0; 48],
                block_undo: BlockUndo::empty(),
            },
        ];
        state.prune_history_to_retain(2);
        assert_eq!(state.blocks.len(), 2);
        assert_eq!(state.undo_stack.len(), 1);
    }

    #[test]
    fn snapshot_bundle_uses_canonical_storage_after_pruning_memory_tail() {
        let root = temp_workspace("snapshot-export");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);
        let mut state = Chainstate::try_load_or_new(Network::Regnet).expect("state");

        for height in 1..=4u64 {
            let coinbase = Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(height),
                    locking_script: vec![height as u8],
                }],
                lock_time: 0,
                witness: vec![],
            };
            let transactions = vec![coinbase];
            let block = solve_block(Block::new(
                BlockHeader {
                    version: 1,
                    network_id: Network::Regnet,
                    height,
                    previous_block_hash: state.tip_hash,
                    merkle_root: merkle_root(&transactions),
                    witness_root: witness_root(&transactions),
                    timestamp: genesis::genesis_state(Network::Regnet)
                        .block
                        .header
                        .timestamp
                        .saturating_add(height),
                    difficulty_target_or_bits: state.next_difficulty_target(),
                    nonce: 0,
                },
                transactions,
            ));
            state.connect_block(&block).expect("connect block");
        }

        state.prune_history_to_retain(2);
        assert_eq!(state.blocks().len(), 2);

        let bundle = state.export_snapshot_bundle().expect("export bundle");
        assert_eq!(bundle.snapshot.height, 4);
        assert_eq!(bundle.blocks.len(), 5);
        assert_eq!(bundle.blocks.first().expect("genesis").header.height, 0);
        assert_eq!(bundle.blocks.last().expect("tip").header.height, 4);
    }

    #[test]
    fn legacy_snapshot_files_are_rejected_as_legacy_layout() {
        let root = temp_workspace("files");
        fs::create_dir_all(root.join("dev/chain")).expect("chain dir");
        let _guard = CurrentDirGuard::switch_to(&root);

        fs::write(
            root.join("dev/chain/chainstate-atho-mainnet.tsv"),
            "height\ttip_hash\n42\tnot-hex\n",
        )
        .expect("state");
        fs::write(
            root.join("dev/chain/utxos-atho-mainnet.tsv"),
            "txid\toutput_index\tvalue_atoms\tlocking_script_hex\tcreated_height\tis_coinbase\n0101\t0\t100\t01\t1\t0\n",
        )
        .expect("utxos");

        let err = Chainstate::try_load_or_new(Network::Mainnet).unwrap_err();
        assert!(matches!(err, StorageError::LegacyStorageLayout));
    }

    #[test]
    fn incomplete_history_is_quarantined_and_rebuilt() {
        let root = temp_workspace("recover");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);
        inject_snapshot_fixture(
            Network::Mainnet,
            &ChainstateSnapshot {
                height: 1,
                tip_hash: [9; 48],
                tip_header: None,
            },
            &[],
        );

        let recovered = Chainstate::try_load_or_recover(Network::Mainnet).expect("recovered");
        assert_eq!(recovered.height, 0);
        assert_eq!(recovered.blocks().len(), 1);
        assert_eq!(recovered.tip_hash, genesis::genesis_hash(Network::Mainnet));

        let quarantine_root = crate::path::quarantine_dir().join(Network::Mainnet.id());
        let mut entries = fs::read_dir(quarantine_root)
            .expect("quarantine dir")
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        let report = entries
            .pop()
            .expect("quarantine entry")
            .path()
            .join("RECOVERY.txt");
        let report_text = fs::read_to_string(report).expect("report");
        assert!(report_text.contains("error=persisted block history is incomplete"));

        let reloaded = Database::open(Network::Mainnet).expect("database reloaded");
        let snapshot = reloaded
            .load_chainstate_snapshot()
            .expect("load snapshot")
            .expect("snapshot present");
        assert_eq!(snapshot.height, 0);
        assert_eq!(snapshot.tip_hash, genesis::genesis_hash(Network::Mainnet));
    }

    #[test]
    fn legacy_chain_logs_are_quarantined_during_recovery() {
        let root = temp_workspace("legacy-recover");
        fs::create_dir_all(root.join("dev/chain")).expect("chain dir");
        let _guard = CurrentDirGuard::switch_to(&root);

        fs::write(
            root.join("dev/chain/blocks.tsv"),
            "height\tblock_hash\n1\t090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909\n",
        )
        .expect("blocks");

        let recovered = Chainstate::try_load_or_recover(Network::Mainnet).expect("recovered");
        assert_eq!(recovered.height, 0);
        assert_eq!(recovered.blocks().len(), 1);
        assert_eq!(recovered.tip_hash, genesis::genesis_hash(Network::Mainnet));
        assert!(!root.join("dev/chain/blocks.tsv").exists());

        let quarantine_root = crate::path::quarantine_dir().join(Network::Mainnet.id());
        let mut entries = fs::read_dir(quarantine_root)
            .expect("quarantine dir")
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        let quarantined = entries.pop().expect("entry").path();
        assert!(quarantined.join("chain/blocks.tsv").exists());
    }

    #[test]
    fn legacy_multi_environment_storage_is_quarantined_during_recovery() {
        let root = temp_workspace("legacy-db-recover");
        fs::create_dir_all(root.join("dev/db/mainnet/meta")).expect("legacy meta dir");
        fs::create_dir_all(root.join("dev/db/mainnet/blocks")).expect("legacy blocks dir");
        let _guard = CurrentDirGuard::switch_to(&root);

        let recovered = Chainstate::try_load_or_recover(Network::Mainnet).expect("recovered");
        assert_eq!(recovered.height, 0);
        assert_eq!(recovered.blocks().len(), 1);
        assert_eq!(recovered.tip_hash, genesis::genesis_hash(Network::Mainnet));
        assert!(root.join("dev/db/mainnet/data.mdb").exists());

        let quarantine_root = crate::path::quarantine_dir().join(Network::Mainnet.id());
        let mut entries = fs::read_dir(quarantine_root)
            .expect("quarantine dir")
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        let quarantined = entries.pop().expect("entry").path();
        assert!(quarantined.join("db/atho-mainnet/meta").exists());
        let report_text =
            fs::read_to_string(quarantined.join("RECOVERY.txt")).expect("recovery report");
        assert!(report_text.contains("legacy multi-environment storage layout detected"));
    }

    #[test]
    fn persisted_cross_network_utxos_fail_closed() {
        let root = temp_workspace("db");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);

        {
            let snapshot = ChainstateSnapshot {
                height: 7,
                tip_hash: [3; 48],
                tip_header: None,
            };
            let utxos = vec![
                UtxoEntry::new(Network::Mainnet, [11; 48], 0, 100, vec![1], 1, false),
                UtxoEntry::new(Network::Testnet, [12; 48], 1, 200, vec![2], 2, false),
            ];
            inject_snapshot_fixture(Network::Mainnet, &snapshot, &utxos);
        }

        let err = Chainstate::try_load_or_new(Network::Mainnet).unwrap_err();
        assert!(matches!(err, StorageError::CrossNetworkReplay));
    }

    #[test]
    fn reload_preserves_next_difficulty_target_from_recent_history() {
        let root = temp_workspace("pow");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);

        let mut state = Chainstate::load_or_new(Network::Mainnet);
        let mut last_target = state.next_difficulty_target();
        let genesis_timestamp = genesis::genesis_state(Network::Mainnet)
            .block
            .header
            .timestamp;
        for height in 1u64..=4 {
            let previous_block_hash = state.tip_hash;
            let coinbase = Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(height),
                    locking_script: vec![height as u8],
                }],
                lock_time: u32::try_from(height).unwrap_or(u32::MAX),
                witness: vec![],
            };
            let transactions = vec![coinbase];
            let block = solve_block(Block::new(
                BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash,
                    merkle_root: merkle_root(&transactions),
                    witness_root: witness_root(&transactions),
                    timestamp: genesis_timestamp
                        .saturating_add(30 * 24 * 60 * 60)
                        .saturating_add(height),
                    difficulty_target_or_bits: state.next_difficulty_target(),
                    nonce: 0,
                },
                transactions,
            ));
            state.connect_block(&block).expect("connect block");
            last_target = state.next_difficulty_target();
        }

        drop(state);
        let reloaded = Chainstate::load_or_new(Network::Mainnet);
        assert_eq!(reloaded.next_difficulty_target(), last_target);
        assert_eq!(reloaded.blocks().len(), 5);
    }

    #[test]
    fn fast_bootstrap_chain_retargets_before_full_window() {
        let mut state = Chainstate::new(Network::Mainnet);
        let genesis_timestamp = genesis::genesis_state(Network::Mainnet)
            .block
            .header
            .timestamp;
        let initial_target =
            atho_core::consensus::pow::initial_target_for_network(Network::Mainnet);
        let mut next_target = initial_target;

        for height in 1u64..=3 {
            let previous_block_hash = state.tip_hash;
            let coinbase = Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(height),
                    locking_script: vec![height as u8],
                }],
                lock_time: u32::try_from(height).unwrap_or(u32::MAX),
                witness: vec![],
            };
            let transactions = vec![coinbase];
            let block = solve_block(Block::new(
                BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash,
                    merkle_root: merkle_root(&transactions),
                    witness_root: witness_root(&transactions),
                    timestamp: genesis_timestamp
                        .saturating_add(30 * 24 * 60 * 60)
                        .saturating_add(height),
                    difficulty_target_or_bits: next_target,
                    nonce: 0,
                },
                transactions,
            ));
            state.connect_block(&block).expect("connect block");
            next_target = state.next_difficulty_target();
        }

        assert!(next_target < initial_target);
    }

    #[test]
    fn single_block_reload_rehydrates_from_persisted_history() {
        let root = temp_workspace("single-reload");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);

        let mut state = Chainstate::load_or_new(Network::Mainnet);
        let genesis_timestamp = genesis::genesis_state(Network::Mainnet)
            .block
            .header
            .timestamp;
        let transactions = vec![Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![1],
            }],
            lock_time: 1,
            witness: vec![],
        }];
        let block = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: genesis_timestamp.saturating_add(1),
                difficulty_target_or_bits: state.next_difficulty_target(),
                nonce: 0,
            },
            transactions,
        ));
        let tip_hash = block.header.block_hash();
        state.connect_block(&block).expect("connect");
        drop(state);

        let db = Database::open(Network::Mainnet).expect("database");
        let persisted = load_persisted_chainstate(Network::Mainnet, &db)
            .expect("persisted")
            .expect("snapshot");
        assert_eq!(persisted.height, 1);
        assert_eq!(persisted.tip_hash, tip_hash);
        assert_eq!(
            load_recent_blocks_from_storage(&db, tip_hash, 27)
                .unwrap()
                .len(),
            2
        );

        let reloaded = Chainstate::try_load_or_new(Network::Mainnet).expect("reload");
        assert_eq!(reloaded.height, 1);
        assert_eq!(reloaded.tip_hash, tip_hash);
    }

    #[test]
    fn reloaded_chainstate_serves_headers_from_persisted_canonical_history() {
        let root = temp_workspace("headers-reload");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);
        let genesis_hash = genesis::genesis_hash(Network::Prunetest);
        let genesis = genesis::genesis_state(Network::Prunetest).block;
        let mut blocks = vec![genesis.clone()];
        let mut previous_hash = genesis.header.block_hash();
        let mut previous_timestamp = genesis.header.timestamp;
        for height in 1..=40u64 {
            let transactions = vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(height),
                    locking_script: vec![height as u8],
                }],
                lock_time: u32::try_from(height).unwrap_or(u32::MAX),
                witness: vec![],
            }];
            let block = Block::new(
                BlockHeader {
                    version: 1,
                    network_id: Network::Prunetest,
                    height,
                    previous_block_hash: previous_hash,
                    merkle_root: merkle_root(&transactions),
                    witness_root: witness_root(&transactions),
                    timestamp: previous_timestamp.saturating_add(1),
                    difficulty_target_or_bits:
                        atho_core::consensus::pow::initial_target_for_network(Network::Prunetest),
                    nonce: height,
                },
                transactions,
            );
            previous_hash = block.header.block_hash();
            previous_timestamp = block.header.timestamp;
            blocks.push(block);
        }

        let db = Database::open(Network::Prunetest).expect("database");
        let snapshot = ChainstateSnapshot {
            height: 40,
            tip_hash: previous_hash,
            tip_header: blocks.last().map(|block| block.header.clone()),
        };
        db.replace_chainstate(&snapshot, &[], &blocks)
            .expect("replace chainstate");

        let reloaded = Chainstate::try_load_or_new(Network::Prunetest).expect("reload");
        assert_eq!(reloaded.height, 40);
        assert!(reloaded.blocks().len() < 41);

        let headers = reloaded
            .headers_after_locator(&[genesis_hash], [0; 48], 64)
            .expect("headers");
        assert!(!headers.is_empty());
        assert_eq!(headers.first().map(|header| header.height), Some(1));
        assert_eq!(headers.last().map(|header| header.height), Some(40));

        let unknown_locator_headers = reloaded
            .headers_after_locator(&[[9; 48]], [0; 48], 64)
            .expect("unknown locator headers");
        assert!(unknown_locator_headers.is_empty());
    }

    #[test]
    fn in_memory_chainstate_does_not_serve_genesis_for_unknown_nonempty_locator() {
        let chainstate = Chainstate::new(Network::Regnet);
        let unknown_locator_headers = chainstate
            .headers_after_locator(&[[9; 48]], [0; 48], 64)
            .expect("unknown locator headers");
        assert!(unknown_locator_headers.is_empty());

        let empty_locator_headers = chainstate
            .headers_after_locator(&[], [0; 48], 64)
            .expect("empty locator headers");
        assert_eq!(
            empty_locator_headers.first().map(|header| header.height),
            Some(0)
        );
    }

    #[test]
    fn prunetest_archive_prunes_raw_files_and_recovers_blocks_from_metadata() {
        let root = temp_workspace("prunetest-prune-archive");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);
        let _prune_guard = EnvVarGuard::set("ATHO_PRUNETEST_PRUNE_DEPTH", "2");
        let rotation_bytes = build_coinbase_successor(&Chainstate::new(Network::Prunetest))
            .canonical_bytes()
            .len() as u64
            + atho_core::constants::BLOCK_FILE_RECORD_OVERHEAD_BYTES;
        let _rotation_guard = RotationOverrideGuard::set(rotation_bytes);

        let mut state = Chainstate::try_load_or_new(Network::Prunetest).expect("state");
        for _ in 0..4 {
            let block = build_coinbase_successor(&state);
            state.connect_block(&block).expect("connect block");
        }

        assert_eq!(state.height, 4);
        assert_eq!(state.prune_depth(), 2);
        assert!(state.last_prune_error().is_none());
        assert!(state.has_pruned_history().expect("has pruned history"));

        let prune_report = state
            .last_prune_report()
            .cloned()
            .expect("last prune report");
        assert_eq!(prune_report.tip_height, 4);
        assert_eq!(prune_report.prune_depth, 2);
        assert_eq!(prune_report.eligible_height, Some(2));
        assert_eq!(prune_report.pruned_blocks, 1);

        let genesis_record = state
            .block_record_by_height(0)
            .expect("genesis record")
            .expect("genesis present");
        let height_one_record = state
            .block_record_by_height(1)
            .expect("height one record")
            .expect("height one present");
        let height_two_record = state
            .block_record_by_height(2)
            .expect("height two record")
            .expect("height two present");
        let tip_record = state
            .block_record_by_height(4)
            .expect("tip record")
            .expect("tip present");
        assert!(genesis_record.pruned);
        assert!(height_one_record.pruned);
        assert!(height_two_record.pruned);
        assert!(!tip_record.pruned);

        let block_root = crate::path::block_storage_dir(Network::Prunetest);
        assert!(!block_root
            .join(format!("blk{:05}.dat", genesis_record.file_number))
            .exists());
        assert!(!block_root
            .join(format!("blk{:05}.dat", height_one_record.file_number))
            .exists());
        assert!(!block_root
            .join(format!("blk{:05}.dat", height_two_record.file_number))
            .exists());
        assert!(block_root
            .join(format!("blk{:05}.dat", tip_record.file_number))
            .exists());

        let reconstructed = state
            .block_by_height(1)
            .expect("load pruned block")
            .expect("block present");
        assert_eq!(reconstructed.header.height, 1);

        drop(state);
        let mut reloaded = Chainstate::try_load_or_new(Network::Prunetest).expect("reload");
        assert_eq!(reloaded.height, 4);
        assert!(reloaded
            .has_pruned_history()
            .expect("reloaded prune marker"));
        let reloaded_block = reloaded
            .block_by_height(1)
            .expect("reload pruned block")
            .expect("reloaded block present");
        assert_eq!(reloaded_block.header.height, 1);

        let next = build_coinbase_successor(&reloaded);
        reloaded
            .connect_block(&next)
            .expect("connect post-prune block");
        assert_eq!(reloaded.height, 5);
    }

    #[test]
    fn prunetest_pruning_threshold_is_exactly_inclusive() {
        let root = temp_workspace("prunetest-prune-threshold");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);
        let _prune_guard = EnvVarGuard::set("ATHO_PRUNETEST_PRUNE_DEPTH", "2");
        let rotation_bytes = build_coinbase_successor(&Chainstate::new(Network::Prunetest))
            .canonical_bytes()
            .len() as u64
            + atho_core::constants::BLOCK_FILE_RECORD_OVERHEAD_BYTES;
        let _rotation_guard = RotationOverrideGuard::set(rotation_bytes);

        let mut state = Chainstate::try_load_or_new(Network::Prunetest).expect("state");
        let block_one = build_coinbase_successor(&state);
        state.connect_block(&block_one).expect("connect height one");
        let genesis_after_one = state
            .block_record_by_height(0)
            .expect("genesis after one")
            .expect("genesis present");
        assert!(!genesis_after_one.pruned);
        assert_eq!(
            state
                .last_prune_report()
                .and_then(|report| report.eligible_height),
            None
        );

        let block_two = build_coinbase_successor(&state);
        state.connect_block(&block_two).expect("connect height two");
        let prune_report = state
            .last_prune_report()
            .cloned()
            .expect("prune report at threshold");
        assert_eq!(prune_report.tip_height, 2);
        assert_eq!(prune_report.eligible_height, Some(0));
        assert_eq!(prune_report.pruned_blocks, 1);

        let genesis_after_two = state
            .block_record_by_height(0)
            .expect("genesis after two")
            .expect("genesis present");
        let height_one_after_two = state
            .block_record_by_height(1)
            .expect("height one after two")
            .expect("height one present");
        assert!(genesis_after_two.pruned);
        assert!(!height_one_after_two.pruned);
    }

    #[test]
    fn chainstate_reorgs_to_longer_branch_and_rolls_back_current_tip() {
        let mut state = Chainstate::new(Network::Mainnet);
        let genesis_timestamp = genesis::genesis_state(Network::Mainnet)
            .block
            .header
            .timestamp;

        let main_1 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(1),
                        locking_script: vec![1],
                    }],
                    lock_time: 1,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(1),
                        locking_script: vec![1],
                    }],
                    lock_time: 1,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(1),
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(1),
                    locking_script: vec![1],
                }],
                lock_time: 1,
                witness: vec![],
            }],
        ));
        state.connect_block(&main_1).unwrap();

        let main_2 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 2,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![2],
                    }],
                    lock_time: 2,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![2],
                    }],
                    lock_time: 2,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(10_000),
                difficulty_target_or_bits: state.next_difficulty_target(),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(2),
                    locking_script: vec![2],
                }],
                lock_time: 2,
                witness: vec![],
            }],
        ));
        state.connect_block(&main_2).unwrap();
        let old_tip = state.tip_hash;

        let fork_2 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 2,
                previous_block_hash: main_1.header.block_hash(),
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![22],
                    }],
                    lock_time: 22,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![22],
                    }],
                    lock_time: 22,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(10),
                difficulty_target_or_bits: main_2.header.difficulty_target_or_bits,
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(2),
                    locking_script: vec![22],
                }],
                lock_time: 22,
                witness: vec![],
            }],
        ));
        let fork_history = vec![
            genesis::genesis_state(Network::Mainnet).block,
            main_1.clone(),
            fork_2.clone(),
        ];
        let fork_3 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 3,
                previous_block_hash: fork_2.header.block_hash(),
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(3),
                        locking_script: vec![33],
                    }],
                    lock_time: 33,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(3),
                        locking_script: vec![33],
                    }],
                    lock_time: 33,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(11),
                difficulty_target_or_bits: atho_core::consensus::pow::target_for_next_block(
                    Network::Mainnet,
                    &fork_history,
                ),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(3),
                    locking_script: vec![33],
                }],
                lock_time: 33,
                witness: vec![],
            }],
        ));

        let result = state
            .select_branch(&[fork_2.clone(), fork_3.clone()])
            .unwrap();
        assert_eq!(result.outcome, ChainSelectionOutcome::Reorged);
        assert_eq!(result.disconnected.len(), 1);
        assert_eq!(result.disconnected[0].header.block_hash(), old_tip);
        assert_eq!(state.height, 3);
        assert_eq!(state.tip_hash, fork_3.header.block_hash());
    }

    #[test]
    fn select_branch_restores_exact_state_after_candidate_commit_failure() {
        let root = temp_workspace("select-branch-rollback");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);

        let mut state = Chainstate::try_load_or_new(Network::Mainnet).expect("state");
        let genesis_timestamp = genesis::genesis_state(Network::Mainnet)
            .block
            .header
            .timestamp;

        let main_1 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(1),
                        locking_script: vec![1],
                    }],
                    lock_time: 1,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(1),
                        locking_script: vec![1],
                    }],
                    lock_time: 1,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(1),
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(1),
                    locking_script: vec![1],
                }],
                lock_time: 1,
                witness: vec![],
            }],
        ));
        state.connect_block(&main_1).unwrap();

        let main_2 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 2,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![2],
                    }],
                    lock_time: 2,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![2],
                    }],
                    lock_time: 2,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(2),
                difficulty_target_or_bits: state.next_difficulty_target(),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(2),
                    locking_script: vec![2],
                }],
                lock_time: 2,
                witness: vec![],
            }],
        ));
        state.connect_block(&main_2).unwrap();
        let main_3 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 3,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(3),
                        locking_script: vec![3],
                    }],
                    lock_time: 3,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(3),
                        locking_script: vec![3],
                    }],
                    lock_time: 3,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(20_000),
                difficulty_target_or_bits: state.next_difficulty_target(),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(3),
                    locking_script: vec![3],
                }],
                lock_time: 3,
                witness: vec![],
            }],
        ));
        state.connect_block(&main_3).unwrap();
        let main_4 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 4,
                previous_block_hash: state.tip_hash,
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(4),
                        locking_script: vec![4],
                    }],
                    lock_time: 4,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(4),
                        locking_script: vec![4],
                    }],
                    lock_time: 4,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(20_001),
                difficulty_target_or_bits: state.next_difficulty_target(),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(4),
                    locking_script: vec![4],
                }],
                lock_time: 4,
                witness: vec![],
            }],
        ));
        state.connect_block(&main_4).unwrap();
        let before = (
            state.height,
            state.tip_hash,
            state.blocks().len(),
            state.utxo_count(),
        );

        let fork_2 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 2,
                previous_block_hash: main_1.header.block_hash(),
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![22],
                    }],
                    lock_time: 22,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(2),
                        locking_script: vec![22],
                    }],
                    lock_time: 22,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(10),
                difficulty_target_or_bits: main_2.header.difficulty_target_or_bits,
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(2),
                    locking_script: vec![22],
                }],
                lock_time: 22,
                witness: vec![],
            }],
        ));
        let fork_history = vec![
            genesis::genesis_state(Network::Mainnet).block,
            main_1.clone(),
            fork_2.clone(),
        ];
        let fork_3 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 3,
                previous_block_hash: fork_2.header.block_hash(),
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(3),
                        locking_script: vec![33],
                    }],
                    lock_time: 33,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(3),
                        locking_script: vec![33],
                    }],
                    lock_time: 33,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(11),
                difficulty_target_or_bits: atho_core::consensus::pow::target_for_next_block(
                    Network::Mainnet,
                    &fork_history,
                ),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(3),
                    locking_script: vec![33],
                }],
                lock_time: 33,
                witness: vec![],
            }],
        ));
        let fork_history = vec![
            genesis::genesis_state(Network::Mainnet).block,
            main_1.clone(),
            fork_2.clone(),
            fork_3.clone(),
        ];
        let fork_4 = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 4,
                previous_block_hash: fork_3.header.block_hash(),
                merkle_root: merkle_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(4),
                        locking_script: vec![44],
                    }],
                    lock_time: 44,
                    witness: vec![],
                }]),
                witness_root: witness_root(&[Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        value_atoms: subsidy::block_subsidy_atoms(4),
                        locking_script: vec![44],
                    }],
                    lock_time: 44,
                    witness: vec![],
                }]),
                timestamp: genesis_timestamp.saturating_add(12),
                difficulty_target_or_bits: atho_core::consensus::pow::target_for_next_block(
                    Network::Mainnet,
                    &fork_history,
                ),
                nonce: 0,
            },
            vec![Transaction {
                version: 1,
                inputs: vec![],
                outputs: vec![TxOutput {
                    value_atoms: subsidy::block_subsidy_atoms(4),
                    locking_script: vec![44],
                }],
                lock_time: 44,
                witness: vec![],
            }],
        ));

        Database::inject_commit_fault_for_test(CommitFaultPoint::BeforeCommit, 1);
        assert!(atho_core::consensus::pow::branch_is_preferred(
            &[fork_2.clone(), fork_3.clone(), fork_4.clone()],
            &[main_3.clone(), main_4.clone()]
        ));
        let result = state.select_branch(&[fork_2, fork_3, fork_4]);
        Database::clear_commit_fault_for_test();

        assert!(
            matches!(result, Err(StorageError::Io(_))),
            "unexpected result: {result:?}"
        );
        assert_eq!(state.height, before.0);
        assert_eq!(state.tip_hash, before.1);
        assert_eq!(state.blocks().len(), before.2);
        assert_eq!(state.utxo_count(), before.3);

        let db = Database::open(Network::Mainnet).expect("database");
        let snapshot = db
            .load_chainstate_snapshot()
            .expect("snapshot")
            .expect("present snapshot");
        assert_eq!(snapshot.height, before.0);
        assert_eq!(snapshot.tip_hash, before.1);
    }
}
