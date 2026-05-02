//! In-memory chainstate helpers layered on top of persisted storage.
use crate::db::{ChainstateSnapshot, Database, PeerHealthRecord, PeerRecord};
use crate::error::StorageError;
use crate::utxo::{BlockUndo, UtxoEntry, UtxoSet};
use crate::validation;
use atho_core::address::internal_hpk_bytes;
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::constants::{GENESIS_COINBASE_ATOMS, PRUNE_DEPTH_BLOCKS};
use atho_core::genesis;
use atho_core::network::Network;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
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
struct BlockRecord {
    height: u64,
    block_hash: [u8; 48],
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
        self.prune_history_to_retain(PRUNE_DEPTH_BLOCKS as usize + 1);
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
        self.undo_stack.drain(0..prune_count);
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

fn ensure_chain_dir() -> std::io::Result<()> {
    fs::create_dir_all(chain_dir())
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

    if let Some(persisted) = load_snapshot_files(network)? {
        return Ok(Some(persisted));
    }
    replay_legacy_chain_logs(network)
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

fn load_snapshot_files(network: Network) -> Result<Option<PersistedChainstate>, StorageError> {
    let state_path = chainstate_snapshot_path(network);
    let utxo_path = utxo_snapshot_path(network);
    let state_exists = state_path.exists();
    let utxo_exists = utxo_path.exists();
    if !state_exists && !utxo_exists {
        return Ok(None);
    }
    if state_exists != utxo_exists {
        return Err(StorageError::CorruptData);
    }
    let state_file = File::open(state_path)?;
    let utxo_file = File::open(utxo_path)?;

    let mut state_reader = BufReader::new(state_file);
    let mut header_line = String::new();
    if state_reader.read_line(&mut header_line)? == 0 {
        return Err(StorageError::CorruptData);
    }
    if header_line.trim().is_empty() || !header_line.starts_with("height\t") {
        return Err(StorageError::CorruptData);
    }
    let mut state_line = String::new();
    if state_reader.read_line(&mut state_line)? == 0 {
        return Err(StorageError::CorruptData);
    }
    let state_line = state_line.trim();
    let mut fields = state_line.split('\t');
    let height = fields
        .next()
        .ok_or(StorageError::CorruptData)?
        .parse()
        .map_err(|_| StorageError::CorruptData)?;
    let tip_hash = hex::decode(fields.next().ok_or(StorageError::CorruptData)?)
        .map_err(|_| StorageError::CorruptData)?
        .try_into()
        .map_err(|_| StorageError::CorruptData)?;

    let mut utxos = Vec::new();
    let reader = BufReader::new(utxo_file);
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("txid\t") || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let txid: [u8; 48] = hex::decode(fields.next().ok_or(StorageError::CorruptData)?)
            .map_err(|_| StorageError::CorruptData)?
            .try_into()
            .map_err(|_| StorageError::CorruptData)?;
        let output_index = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let value_atoms = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let locking_script = hex::decode(fields.next().ok_or(StorageError::CorruptData)?)
            .map_err(|_| StorageError::CorruptData)?;
        let created_height = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let is_coinbase = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse::<u8>()
            .map_err(|_| StorageError::CorruptData)?
            != 0;
        utxos.push(UtxoEntry::new(
            network,
            txid,
            output_index,
            value_atoms,
            locking_script,
            created_height,
            is_coinbase,
        ));
    }

    Ok(Some(PersistedChainstate {
        height,
        tip_hash,
        tip_header: None,
        utxos,
    }))
}

fn replay_legacy_chain_logs(network: Network) -> Result<Option<PersistedChainstate>, StorageError> {
    ensure_chain_dir()?;
    let block_rows = load_block_rows()?;
    if block_rows.is_empty() {
        return Ok(None);
    }

    let tx_rows = load_tx_rows()?;
    let mut input_rows = load_input_rows()?;
    let mut output_rows = load_output_rows()?;
    let mut utxo_set = UtxoSet::new(network);
    let tip = block_rows.values().next_back().cloned();
    for (height, canonical) in block_rows.into_iter() {
        let tx_keys: Vec<_> = tx_rows
            .keys()
            .filter(|key| key.0 == height && key.1 == canonical.block_hash)
            .copied()
            .collect();
        for key in tx_keys {
            let tx = tx_rows.get(&key).expect("tx row exists");
            let inputs = input_rows.remove(&key).unwrap_or_default();
            let outputs = output_rows.remove(&key).unwrap_or_default();
            for input in inputs {
                let _ = utxo_set.remove(input.previous_txid, input.output_index);
            }
            for (output_index, output) in outputs.into_iter().enumerate() {
                let entry = if tx.input_count == 0 && key.2 == 0 {
                    UtxoEntry::coinbase(
                        network,
                        tx.txid,
                        output_index as u32,
                        output.value_atoms,
                        output.locking_script,
                        height,
                    )
                } else {
                    UtxoEntry::new(
                        network,
                        tx.txid,
                        output_index as u32,
                        output.value_atoms,
                        output.locking_script,
                        height,
                        false,
                    )
                };
                let _ = utxo_set.insert(entry);
            }
        }
    }

    let persisted = PersistedChainstate {
        height: tip.as_ref().map(|record| record.height).unwrap_or(0),
        tip_hash: tip
            .as_ref()
            .map(|record| record.block_hash)
            .unwrap_or([0; 48]),
        tip_header: None,
        utxos: utxo_set.entries().cloned().collect(),
    };
    let _ = write_chainstate_snapshot(network, persisted.height, persisted.tip_hash);
    let _ = write_utxo_snapshot(network, persisted.utxos.iter());
    Ok(Some(persisted))
}

fn load_block_rows() -> Result<BTreeMap<u64, BlockRecord>, StorageError> {
    let path = blocks_ledger_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut rows = BTreeMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("height\t") || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let height = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let block_hash = fields
            .next()
            .and_then(parse_hex::<48>)
            .ok_or(StorageError::CorruptData)?;
        rows.insert(height, BlockRecord { height, block_hash });
    }
    Ok(rows)
}

fn load_tx_rows() -> Result<BTreeMap<(u64, [u8; 48], u32), TxRow>, StorageError> {
    let path = transactions_ledger_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut rows = BTreeMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("height\t") || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let height = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let block_hash = fields
            .next()
            .and_then(parse_hex::<48>)
            .ok_or(StorageError::CorruptData)?;
        let tx_index = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let txid = fields
            .next()
            .and_then(parse_hex::<48>)
            .ok_or(StorageError::CorruptData)?;
        let _wtxid = fields.next();
        let _version = fields.next();
        let _lock_time = fields.next();
        let input_count = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let _output_count = fields.next();
        let _size_bytes = fields.next();
        let _weight_bytes = fields.next();
        let _vsize_bytes = fields.next();
        let _witness_bytes = fields.next();
        let _output_value_atoms = fields.next();
        let _canonical_bytes_hex = fields.next();
        rows.insert((height, block_hash, tx_index), TxRow { txid, input_count });
    }
    Ok(rows)
}

fn load_input_rows() -> Result<BTreeMap<(u64, [u8; 48], u32), Vec<InputRow>>, StorageError> {
    let path = transaction_inputs_ledger_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut rows: BTreeMap<(u64, [u8; 48], u32), Vec<InputRow>> = BTreeMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("height\t") || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let height = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let block_hash = fields
            .next()
            .and_then(parse_hex::<48>)
            .ok_or(StorageError::CorruptData)?;
        let tx_index = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let _input_index = fields.next();
        let previous_txid = fields
            .next()
            .and_then(parse_hex::<48>)
            .ok_or(StorageError::CorruptData)?;
        let output_index = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let _unlocking_script_hex = fields.next();
        rows.entry((height, block_hash, tx_index))
            .or_default()
            .push(InputRow {
                previous_txid,
                output_index,
            });
    }
    Ok(rows)
}

fn load_output_rows() -> Result<BTreeMap<(u64, [u8; 48], u32), Vec<OutputRow>>, StorageError> {
    let path = transaction_outputs_ledger_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut rows: BTreeMap<(u64, [u8; 48], u32), Vec<OutputRow>> = BTreeMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("height\t") || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let height = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let block_hash = fields
            .next()
            .and_then(parse_hex::<48>)
            .ok_or(StorageError::CorruptData)?;
        let tx_index = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let _output_index = fields.next();
        let value_atoms = fields
            .next()
            .ok_or(StorageError::CorruptData)?
            .parse()
            .map_err(|_| StorageError::CorruptData)?;
        let locking_script = hex::decode(fields.next().ok_or(StorageError::CorruptData)?)
            .map_err(|_| StorageError::CorruptData)?;
        rows.entry((height, block_hash, tx_index))
            .or_default()
            .push(OutputRow {
                value_atoms,
                locking_script,
            });
    }
    Ok(rows)
}

fn write_chainstate_snapshot(
    network: Network,
    height: u64,
    tip_hash: [u8; 48],
) -> std::io::Result<()> {
    ensure_chain_dir()?;
    let path = chainstate_snapshot_path(network);
    let mut file = File::create(path)?;
    writeln!(file, "height\ttip_hash")?;
    writeln!(file, "{}\t{}", height, hex::encode(tip_hash))?;
    Ok(())
}

fn write_utxo_snapshot<'a, I>(network: Network, utxos: I) -> std::io::Result<()>
where
    I: IntoIterator<Item = &'a UtxoEntry>,
{
    ensure_chain_dir()?;
    let path = utxo_snapshot_path(network);
    let mut file = File::create(path)?;
    writeln!(
        file,
        "txid\toutput_index\tvalue_atoms\tlocking_script_hex\tcreated_height\tis_coinbase"
    )?;
    for utxo in utxos {
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}\t{}",
            hex::encode(utxo.txid),
            utxo.output_index,
            utxo.value_atoms,
            hex::encode(&utxo.locking_script),
            utxo.created_height,
            u8::from(utxo.is_coinbase)
        )?;
    }
    Ok(())
}

fn parse_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    let bytes = hex::decode(value).ok()?;
    bytes.as_slice().try_into().ok()
}

#[derive(Debug, Clone)]
struct TxRow {
    txid: [u8; 48],
    input_count: usize,
}

#[derive(Debug, Clone)]
struct InputRow {
    previous_txid: [u8; 48],
    output_index: u32,
}

#[derive(Debug, Clone)]
struct OutputRow {
    value_atoms: u64,
    locking_script: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn malformed_snapshot_files_fail_closed() {
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
        assert!(matches!(err, StorageError::CorruptData));
    }

    #[test]
    fn incomplete_history_is_quarantined_and_rebuilt() {
        let root = temp_workspace("recover");
        fs::create_dir_all(&root).expect("root");
        let _guard = CurrentDirGuard::switch_to(&root);
        let db = Database::open(Network::Mainnet).expect("database");
        db.save_chainstate_snapshot(
            &ChainstateSnapshot {
                height: 1,
                tip_hash: [9; 48],
                tip_header: None,
            },
            &[],
        )
        .expect("snapshot");

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
            let db = Database::open(Network::Mainnet).expect("database");
            let snapshot = ChainstateSnapshot {
                height: 7,
                tip_hash: [3; 48],
                tip_header: None,
            };
            let utxos = vec![
                UtxoEntry::new(Network::Mainnet, [11; 48], 0, 100, vec![1], 1, false),
                UtxoEntry::new(Network::Testnet, [12; 48], 1, 200, vec![2], 2, false),
            ];
            db.save_chainstate_snapshot(&snapshot, &utxos)
                .expect("snapshot");
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
