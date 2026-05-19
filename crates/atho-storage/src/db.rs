// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! LMDB-backed Atho storage primitives.
//!
//! This module owns the database environment, named databases, archive records,
//! and the atomic commit paths that keep chainstate and block history aligned.
//!
//! STORAGE: Changes to key names, record serialization, or transaction grouping
//! can invalidate existing databases or create unrecoverable mixed-height state.
use crate::block_files::{BlockFileLocation, BlockFileStore};
use crate::error::StorageError;
use crate::path;
use crate::utxo::UtxoEntry;
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::consensus::rules::STORAGE_SCHEMA_VERSION;
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::Transaction as CoreTransaction;
use lmdb::{
    Cursor, Database as LmdbDatabase, DatabaseFlags, Environment, Error as LmdbError,
    RwTransaction, Transaction as LmdbTransaction, WriteFlags,
};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
#[cfg(test)]
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use std::{fs::File, io::Write, os::unix::fs::PermissionsExt};

type RawEntries = Vec<(Vec<u8>, Vec<u8>)>;

const INITIAL_MAP_SIZE: usize = 1 << 30;
const MAX_MAP_SIZE: usize = 1 << 40;
const MAX_DBS: u32 = 10;

const META_DB: &str = "meta";
const BLOCKS_DB: &str = "blocks";
const BLOCK_HEIGHTS_DB: &str = "block_heights";
const BLOCK_TRANSACTIONS_DB: &str = "block_transactions";
const TRANSACTIONS_DB: &str = "transactions";
const UTXOS_DB: &str = "utxos";
const PEERS_DB: &str = "peers";
const ADDRESSES_DB: &str = "addresses";
const PEER_HEALTH_DB: &str = "peer_health";

const LEGACY_META_DIR: &str = "meta";
const LEGACY_BLOCKS_DIR: &str = "blocks";
const LEGACY_TRANSACTIONS_DIR: &str = "transactions";
const LEGACY_UTXOS_DIR: &str = "utxos";
const LEGACY_PEERS_DIR: &str = "peers";
const LEGACY_ADDRESSES_DIR: &str = "addresses";

const SNAPSHOT_KEY: &[u8; 10] = b"chainstate";
const SCHEMA_VERSION_KEY: &[u8; 14] = b"schema_version";
const STORAGE_METADATA_KEY: &[u8; 16] = b"storage_metadata";
const STORAGE_RUNTIME_STATE_KEY: &[u8; 21] = b"storage_runtime_state";
const STORAGE_MAGIC: [u8; 4] = *b"ATHO";
const SOFTWARE_STORAGE_VERSION: u32 = 1;

#[cfg(test)]
static COMMIT_FAULT: OnceLock<Mutex<Option<CommitFault>>> = OnceLock::new();

/// Canonical persisted chain tip snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainstateSnapshot {
    pub height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub tip_hash: [u8; 48],
    pub tip_header: Option<BlockHeader>,
}

/// Block metadata stored in LMDB while the full raw block bytes live in the
/// flat-file archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockArchiveRecord {
    pub height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub block_hash: [u8; 48],
    #[serde(with = "serde_big_array::BigArray")]
    pub previous_block_hash: [u8; 48],
    pub network: Network,
    pub version: u16,
    #[serde(with = "serde_big_array::BigArray")]
    pub merkle_root: [u8; 48],
    #[serde(with = "serde_big_array::BigArray")]
    pub witness_root: [u8; 48],
    pub timestamp: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub difficulty_target_or_bits: [u8; 48],
    pub nonce: u64,
    pub file_number: u64,
    pub record_offset: u64,
    pub payload_length: u32,
    pub raw_block_size: u32,
    pub weight_bytes: u32,
    pub vsize_bytes: u32,
    pub tx_count: u32,
    pub fees_total_atoms: u64,
    pub fees_miner_atoms: u64,
    pub chainwork: Vec<u8>,
    pub fully_validated: bool,
    pub main_chain: bool,
    pub pruned: bool,
    pub persisted_unix: u64,
}

impl BlockArchiveRecord {
    /// Reconstructs the canonical block header from metadata only.
    pub fn header(&self) -> BlockHeader {
        BlockHeader {
            version: self.version,
            network_id: self.network,
            height: self.height,
            previous_block_hash: self.previous_block_hash,
            merkle_root: self.merkle_root,
            witness_root: self.witness_root,
            founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
            founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
            timestamp: self.timestamp,
            difficulty_target_or_bits: self.difficulty_target_or_bits,
            nonce: self.nonce,
        }
    }

    /// Returns the flat-file location of the raw archived block payload.
    pub fn file_location(&self) -> BlockFileLocation {
        BlockFileLocation {
            file_number: self.file_number,
            record_offset: self.record_offset,
            payload_length: self.payload_length,
        }
    }
}

/// Transaction archive record stored in the transaction index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionArchiveRecord {
    pub height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub block_hash: [u8; 48],
    pub tx_index: u32,
    #[serde(with = "serde_big_array::BigArray")]
    pub txid: [u8; 48],
    pub transaction: CoreTransaction,
}

/// Ordered transaction membership for one block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockTransactionsRecord {
    pub height: u64,
    pub txids: Vec<Vec<u8>>,
}

/// Summary of one pruning pass over the flat-file archive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockPruneReport {
    pub tip_height: u64,
    pub prune_depth: u64,
    pub eligible_height: Option<u64>,
    pub pruned_files: Vec<u64>,
    pub pruned_blocks: usize,
    pub reclaimed_bytes: u64,
}

/// Persisted peer-discovery record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRecord {
    pub network: Network,
    pub remote_addr: String,
    pub first_seen_height: u64,
    pub last_seen_height: u64,
    pub last_seen_unix: u64,
}

/// Persisted address-book discovery record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressRecord {
    pub network: Network,
    pub address: String,
    pub label: Option<String>,
    pub first_seen_height: u64,
    pub last_seen_height: u64,
}

/// Persisted peer health and backoff state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerHealthRecord {
    pub network: Network,
    pub remote_addr: String,
    pub quality_score: u32,
    pub consecutive_failures: u32,
    pub backoff_until_unix: u64,
    pub last_failure_unix: Option<u64>,
    pub last_success_unix: Option<u64>,
}

/// Persisted storage identity and compatibility record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageMetadata {
    #[serde(with = "serde_big_array::BigArray")]
    pub storage_magic: [u8; 4],
    #[serde(with = "serde_big_array::BigArray")]
    pub network_magic: [u8; 4],
    pub network_name: String,
    #[serde(with = "serde_big_array::BigArray")]
    pub chain_id: [u8; 48],
    #[serde(with = "serde_big_array::BigArray")]
    pub genesis_hash: [u8; 48],
    #[serde(with = "serde_big_array::BigArray")]
    pub genesis_block_id: [u8; 48],
    pub database_schema_version: u32,
    pub software_storage_version: u32,
    pub created_at_unix: u64,
    pub last_opened_unix: u64,
}

/// Persisted runtime safety markers used to detect unclean shutdowns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageRuntimeState {
    pub last_clean_shutdown: bool,
    pub last_runtime_started_unix: u64,
    pub last_shutdown_unix: u64,
    pub last_recovery_check_unix: u64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFaultPoint {
    AfterArchiveWrite,
    AfterSnapshotWrite,
    AfterStateWrite,
    BeforeCommit,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommitFault {
    point: CommitFaultPoint,
    remaining_hits: usize,
}

#[derive(Debug, Clone, Copy)]
enum Dataset {
    Meta,
    Blocks,
    BlockHeights,
    BlockTransactions,
    Transactions,
    Utxos,
    Peers,
    Addresses,
    PeerHealth,
}

/// LMDB-backed Atho storage handle.
#[derive(Debug)]
pub struct Database {
    network: Network,
    path: PathBuf,
    block_store: BlockFileStore,
    state: Mutex<DatabaseState>,
}

#[derive(Debug)]
struct DatabaseState {
    env: Environment,
    map_size: usize,
    meta: LmdbDatabase,
    blocks: LmdbDatabase,
    block_heights: LmdbDatabase,
    block_transactions: LmdbDatabase,
    transactions: LmdbDatabase,
    utxos: LmdbDatabase,
    peers: LmdbDatabase,
    addresses: LmdbDatabase,
    peer_health: LmdbDatabase,
}

impl Database {
    /// Opens or initializes the database for the selected network.
    ///
    /// INVARIANT: Database roots and raw block archives are network-specific so
    /// one network never reuses another network's block history or chainstate.
    pub fn open(network: Network) -> Result<Self, StorageError> {
        let root = path::database_dir(network);
        fs::create_dir_all(&root)?;
        if legacy_layout_present(&root) {
            return Err(StorageError::LegacyStorageLayout);
        }
        let block_store = BlockFileStore::open(network)?;
        let state = Self::open_state(&root, configured_db_cache_bytes())?;
        let database = Self {
            network,
            path: root,
            block_store,
            state: Mutex::new(state),
        };
        database.ensure_storage_metadata()?;
        database.ensure_schema_version()?;
        database.ensure_runtime_state()?;
        database.run_startup_consistency_checks()?;
        Ok(database)
    }

    /// Returns the network this database was opened for.
    pub fn network(&self) -> Network {
        self.network
    }

    /// Returns the flat block-file directory used by this database root.
    pub fn block_storage_path(&self) -> &Path {
        self.block_store.root().as_path()
    }

    /// Returns whether any archived block metadata is marked as pruned.
    pub fn has_pruned_blocks(&self) -> Result<bool, StorageError> {
        Ok(self
            .list_block_records()?
            .into_iter()
            .any(|record| record.pruned))
    }

    /// Loads the current chainstate tip snapshot, if any.
    pub fn load_chainstate_snapshot(&self) -> Result<Option<ChainstateSnapshot>, StorageError> {
        let snapshot_bytes = match self.get(Dataset::Meta, SNAPSHOT_KEY)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let snapshot: ChainstateSnapshot =
            bincode::deserialize(&snapshot_bytes).map_err(|_| StorageError::CorruptData)?;
        Ok(Some(snapshot))
    }

    pub fn load_storage_metadata(&self) -> Result<Option<StorageMetadata>, StorageError> {
        match self.get(Dataset::Meta, STORAGE_METADATA_KEY)? {
            Some(bytes) => deserialize_record(&bytes).map(Some),
            None => Ok(None),
        }
    }

    pub fn load_runtime_state(&self) -> Result<Option<StorageRuntimeState>, StorageError> {
        match self.get(Dataset::Meta, STORAGE_RUNTIME_STATE_KEY)? {
            Some(bytes) => deserialize_record(&bytes).map(Some),
            None => Ok(None),
        }
    }

    pub fn inspect_storage_metadata(
        network: Network,
    ) -> Result<Option<StorageMetadata>, StorageError> {
        let root = path::database_dir(network);
        if !root.exists() {
            return Ok(None);
        }
        let mut builder = Environment::new();
        builder
            .set_max_readers(32)
            .set_max_dbs(MAX_DBS)
            .set_map_size(configured_db_cache_bytes());
        let env = builder.open(&root)?;
        let meta = env.create_db(Some(META_DB), DatabaseFlags::empty())?;
        let txn = env.begin_ro_txn()?;
        match txn.get(meta, &STORAGE_METADATA_KEY) {
            Ok(bytes) => deserialize_record(bytes).map(Some),
            Err(LmdbError::NotFound) => Ok(None),
            Err(err) => Err(StorageError::Lmdb(err)),
        }
    }

    /// Loads one archived block metadata record by block hash.
    pub fn load_block_record(
        &self,
        block_hash: [u8; 48],
    ) -> Result<Option<BlockArchiveRecord>, StorageError> {
        match self.get(Dataset::Blocks, &block_hash)? {
            Some(bytes) => deserialize_record(&bytes).map(Some),
            None => Ok(None),
        }
    }

    /// Loads the canonical best-chain block hash for one height.
    pub fn load_block_hash_by_height(&self, height: u64) -> Result<Option<[u8; 48]>, StorageError> {
        match self.get(Dataset::BlockHeights, &height_key(height))? {
            Some(bytes) => bytes
                .as_slice()
                .try_into()
                .map(Some)
                .map_err(|_| StorageError::CorruptData),
            None => Ok(None),
        }
    }

    /// Loads one archived block metadata record using the best-chain height map.
    pub fn load_block_record_by_height(
        &self,
        height: u64,
    ) -> Result<Option<BlockArchiveRecord>, StorageError> {
        let Some(block_hash) = self.load_block_hash_by_height(height)? else {
            return Ok(None);
        };
        self.load_block_record(block_hash)
    }

    /// Loads one archived block by block hash.
    ///
    /// The raw canonical bytes come from the flat-file archive when present.
    /// When a block has been pruned from the raw archive, the block is rebuilt
    /// from LMDB metadata plus the ordered transaction index.
    pub fn load_block(&self, block_hash: [u8; 48]) -> Result<Option<Block>, StorageError> {
        let Some(record) = self.load_block_record(block_hash)? else {
            return Ok(None);
        };
        if !record.pruned {
            match self.block_store.read_block(record.file_location()) {
                Ok(mut block) => {
                    if block.header.block_hash() != record.block_hash
                        || block.header.height != record.height
                    {
                        // Fall through to the indexed archive below. LMDB metadata
                        // remains authoritative when a raw flat-file record is stale
                        // or damaged.
                    } else {
                        block.fees_total_atoms = record.fees_total_atoms;
                        block.fees_miner_atoms = record.fees_miner_atoms;
                        return Ok(Some(block));
                    }
                }
                Err(StorageError::Io(err))
                    if Self::can_rebuild_from_index_after_raw_read_error(&err) => {}
                Err(StorageError::CorruptData | StorageError::CrossNetworkReplay) => {}
                Err(err) => return Err(err),
            }
        }

        let transactions = self.load_block_transactions(record.block_hash)?;
        if transactions.len() != record.tx_count as usize {
            return Err(StorageError::IncompleteBlockHistory);
        }
        let mut block = Block::new(
            record.header(),
            transactions
                .into_iter()
                .map(|record| record.transaction)
                .collect(),
        );
        block.fees_total_atoms = record.fees_total_atoms;
        block.fees_miner_atoms = record.fees_miner_atoms;
        Ok(Some(block))
    }

    /// Loads one indexed transaction archive record by txid.
    pub fn load_transaction(
        &self,
        txid: [u8; 48],
    ) -> Result<Option<TransactionArchiveRecord>, StorageError> {
        match self.get(Dataset::Transactions, &txid)? {
            Some(bytes) => deserialize_record(&bytes).map(Some),
            None => Ok(None),
        }
    }

    /// Loads the ordered transaction archive records for one block.
    pub fn load_block_transactions(
        &self,
        block_hash: [u8; 48],
    ) -> Result<Vec<TransactionArchiveRecord>, StorageError> {
        let Some(order_bytes) = self.get(Dataset::BlockTransactions, &block_hash)? else {
            return Ok(Vec::new());
        };
        let order: BlockTransactionsRecord = deserialize_record(&order_bytes)?;
        let mut records = Vec::with_capacity(order.txids.len());
        for (tx_index, txid_bytes) in order.txids.into_iter().enumerate() {
            let txid: [u8; 48] = txid_bytes
                .as_slice()
                .try_into()
                .map_err(|_| StorageError::CorruptData)?;
            let tx = self
                .load_transaction(txid)?
                .ok_or(StorageError::IncompleteBlockHistory)?;
            if tx.tx_index != tx_index as u32
                || tx.block_hash != block_hash
                || tx.height != order.height
            {
                return Err(StorageError::CorruptData);
            }
            records.push(tx);
        }
        Ok(records)
    }

    fn can_rebuild_from_index_after_raw_read_error(error: &std::io::Error) -> bool {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::UnexpectedEof
        )
    }

    /// Returns every archived block metadata record.
    pub fn list_block_records(&self) -> Result<Vec<BlockArchiveRecord>, StorageError> {
        let mut records: Vec<BlockArchiveRecord> = Vec::new();
        for (_, value) in self.entries(Dataset::Blocks)? {
            records.push(deserialize_record(&value)?);
        }
        records.sort_by(|left, right| {
            left.height
                .cmp(&right.height)
                .then(left.block_hash.cmp(&right.block_hash))
        });
        Ok(records)
    }

    /// Loads the entire UTXO set snapshot from storage.
    pub fn load_utxos(&self) -> Result<Vec<UtxoEntry>, StorageError> {
        let entries = self.entries(Dataset::Utxos)?;
        let mut utxos = Vec::with_capacity(entries.len());
        for (_, value) in entries {
            let entry: UtxoEntry = deserialize_record(&value)?;
            utxos.push(entry);
        }
        Ok(utxos)
    }

    /// Commits a new tip snapshot and complete UTXO image atomically.
    pub fn save_chainstate_snapshot(
        &self,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
    ) -> Result<(), StorageError> {
        self.commit_chainstate(snapshot, utxos, None)
    }

    /// Appends a block archive record without rewriting the UTXO set.
    pub fn append_block(&self, height: u64, block: &Block) -> Result<(), StorageError> {
        if block.header.network_id != self.network {
            return Err(StorageError::CrossNetworkReplay);
        }
        if self.load_block_record(block.header.block_hash())?.is_some() {
            return Ok(());
        }
        let archive_hint = self.archive_append_file_hint()?;
        let location = self
            .block_store
            .append_block_with_minimum_file_number(block, archive_hint)?;
        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            write_block_archive(&mut txn, state, self.network, height, block, location)?;
            txn.commit()?;
            Ok(())
        })
    }

    /// Prunes raw flat-file block data that is safely outside the retained
    /// rollback window while keeping all metadata and transaction indexes in
    /// LMDB.
    pub fn prune_archived_blocks(
        &self,
        tip_height: u64,
        prune_depth: u64,
    ) -> Result<BlockPruneReport, StorageError> {
        let eligible_height = tip_height.checked_sub(prune_depth);
        let mut report = BlockPruneReport {
            tip_height,
            prune_depth,
            eligible_height,
            ..BlockPruneReport::default()
        };
        let Some(cutoff_height) = eligible_height else {
            return Ok(report);
        };

        let records = self.list_block_records()?;
        let mut files: BTreeMap<u64, Vec<BlockArchiveRecord>> = BTreeMap::new();
        for record in records {
            files.entry(record.file_number).or_default().push(record);
        }

        let mut prune_files: BTreeMap<u64, Vec<BlockArchiveRecord>> = BTreeMap::new();
        for (file_number, file_records) in &files {
            let file_max_height = file_records
                .iter()
                .map(|record| record.height)
                .max()
                .unwrap_or(0);
            if file_max_height > cutoff_height {
                continue;
            }

            let pending_prune = file_records
                .iter()
                .filter(|record| !record.pruned)
                .cloned()
                .collect::<Vec<_>>();
            if pending_prune.is_empty() {
                continue;
            }

            prune_files.insert(*file_number, pending_prune);
        }

        if let Some(max_archive_bytes) = configured_prune_target_bytes() {
            let mut unpruned_bytes = files
                .values()
                .flat_map(|records| records.iter())
                .filter(|record| !record.pruned)
                .map(|record| record.file_location().record_length())
                .sum::<u64>();

            for (file_number, file_records) in &files {
                if unpruned_bytes <= max_archive_bytes {
                    break;
                }
                if prune_files.contains_key(file_number) {
                    continue;
                }
                let file_max_height = file_records
                    .iter()
                    .map(|record| record.height)
                    .max()
                    .unwrap_or(0);
                if file_max_height >= tip_height {
                    continue;
                }
                let pending_prune = file_records
                    .iter()
                    .filter(|record| !record.pruned)
                    .cloned()
                    .collect::<Vec<_>>();
                if pending_prune.is_empty() {
                    continue;
                }
                let file_bytes = pending_prune
                    .iter()
                    .map(|record| record.file_location().record_length())
                    .sum::<u64>();
                unpruned_bytes = unpruned_bytes.saturating_sub(file_bytes);
                prune_files.insert(*file_number, pending_prune);
            }
        }

        for pending_prune in prune_files.values() {
            report.pruned_blocks += pending_prune.len();
            report.reclaimed_bytes += pending_prune
                .iter()
                .map(|record| record.file_location().record_length())
                .sum::<u64>();
        }

        if prune_files.is_empty() {
            return Ok(report);
        }

        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            for file_records in prune_files.values() {
                for record in file_records {
                    let mut updated = record.clone();
                    if updated.pruned {
                        continue;
                    }
                    updated.pruned = true;
                    let bytes =
                        bincode::serialize(&updated).map_err(|_| StorageError::CorruptData)?;
                    txn.put(
                        state.blocks,
                        &updated.block_hash,
                        &bytes,
                        WriteFlags::empty(),
                    )?;
                }
            }
            txn.commit()?;
            Ok(())
        })?;

        for file_number in prune_files.keys() {
            self.block_store.delete_file(*file_number)?;
            report.pruned_files.push(*file_number);
        }
        Ok(report)
    }

    /// Commits the tip snapshot, UTXO state, and optional appended block together.
    ///
    /// STORAGE: This transaction must remain atomic. Writing the tip without the
    /// matching UTXO image would let the node restart into a corrupt state.
    ///
    /// PERFORMANCE: normal block connection applies only the block's UTXO delta.
    /// Rewriting the whole UTXO table on every block makes historical sync
    /// quadratic as the chain grows.
    pub fn commit_chainstate(
        &self,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
        appended_block: Option<(u64, &Block)>,
    ) -> Result<(), StorageError> {
        let commit_journal = CommitJournalGuard::begin(
            self.network,
            "commit",
            snapshot.height,
            appended_block.map(|(_, block)| block.header.block_hash()),
        )?;
        let snapshot_value = bincode::serialize(snapshot).map_err(|_| StorageError::CorruptData)?;
        let serialized_utxos = if appended_block.is_none() {
            let mut serialized = Vec::with_capacity(utxos.len());
            for utxo in utxos {
                let key = utxo_key(utxo.txid, utxo.output_index);
                let value = bincode::serialize(utxo).map_err(|_| StorageError::CorruptData)?;
                serialized.push((key, value));
            }
            Some(serialized)
        } else {
            None
        };

        let archive_append = if let Some((height, block)) = appended_block {
            if block.header.network_id != self.network {
                return Err(StorageError::CrossNetworkReplay);
            }
            if self.load_block_record(block.header.block_hash())?.is_some() {
                None
            } else {
                let archive_hint = self.archive_append_file_hint()?;
                Some((
                    height,
                    block,
                    self.block_store
                        .append_block_with_minimum_file_number(block, archive_hint)?,
                ))
            }
        } else {
            None
        };

        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            if let Some((height, block, location)) = archive_append {
                write_block_archive(&mut txn, state, self.network, height, block, location)?;
                #[cfg(test)]
                maybe_inject_commit_fault(CommitFaultPoint::AfterArchiveWrite)?;
            }
            txn.put(
                state.meta,
                &SNAPSHOT_KEY,
                &snapshot_value,
                WriteFlags::empty(),
            )?;
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::AfterSnapshotWrite)?;
            if let Some((_, block)) = appended_block {
                apply_utxo_delta(&mut txn, state, self.network, block)?;
            } else if let Some(serialized_utxos) = &serialized_utxos {
                clear_db(&mut txn, state.utxos)?;
                for (key, value) in serialized_utxos {
                    txn.put(
                        state.utxos,
                        &key.as_slice(),
                        &value.as_slice(),
                        WriteFlags::empty(),
                    )?;
                }
                rebuild_height_index(&mut txn, state, snapshot.tip_hash)?;
            }
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::AfterStateWrite)?;
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::BeforeCommit)?;
            txn.commit()?;
            Ok(())
        })?;
        commit_journal.finish()?;
        Ok(())
    }

    /// Replaces the canonical chainstate image and rebuilds the raw archive from
    /// the supplied canonical block list.
    pub fn replace_chainstate(
        &self,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
        blocks: &[Block],
    ) -> Result<(), StorageError> {
        let commit_journal = CommitJournalGuard::begin(
            self.network,
            "replace",
            snapshot.height,
            Some(snapshot.tip_hash),
        )?;
        let snapshot_value = bincode::serialize(snapshot).map_err(|_| StorageError::CorruptData)?;
        let mut serialized_utxos = Vec::with_capacity(utxos.len());
        for utxo in utxos {
            let key = utxo_key(utxo.txid, utxo.output_index);
            let value = bincode::serialize(utxo).map_err(|_| StorageError::CorruptData)?;
            serialized_utxos.push((key, value));
        }

        self.block_store.reset()?;
        let mut archived = Vec::with_capacity(blocks.len());
        for block in blocks {
            if block.header.network_id != self.network {
                return Err(StorageError::CrossNetworkReplay);
            }
            archived.push((block.header.height, self.block_store.append_block(block)?));
        }

        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            clear_db(&mut txn, state.blocks)?;
            clear_db(&mut txn, state.block_heights)?;
            clear_db(&mut txn, state.block_transactions)?;
            clear_db(&mut txn, state.transactions)?;
            clear_db(&mut txn, state.utxos)?;
            for ((height, location), block) in archived.iter().zip(blocks.iter()) {
                write_block_archive(&mut txn, state, self.network, *height, block, *location)?;
            }
            txn.put(
                state.meta,
                &SNAPSHOT_KEY,
                &snapshot_value,
                WriteFlags::empty(),
            )?;
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::AfterSnapshotWrite)?;
            for (key, value) in &serialized_utxos {
                txn.put(
                    state.utxos,
                    &key.as_slice(),
                    &value.as_slice(),
                    WriteFlags::empty(),
                )?;
            }
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::AfterStateWrite)?;
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::BeforeCommit)?;
            txn.commit()?;
            Ok(())
        })?;
        commit_journal.finish()?;
        Ok(())
    }

    pub fn upsert_peer(&self, record: &PeerRecord) -> Result<(), StorageError> {
        let key = record.remote_addr.as_bytes().to_vec();
        let value = bincode::serialize(record).map_err(|_| StorageError::CorruptData)?;
        self.put(Dataset::Peers, &key, &value)
    }

    pub fn load_peer(&self, remote_addr: &str) -> Result<Option<PeerRecord>, StorageError> {
        match self.get(Dataset::Peers, remote_addr.as_bytes())? {
            Some(bytes) => deserialize_record(&bytes).map(Some),
            None => Ok(None),
        }
    }

    pub fn list_peers(&self) -> Result<Vec<PeerRecord>, StorageError> {
        let mut peers = Vec::new();
        for (_, value) in self.entries(Dataset::Peers)? {
            peers.push(deserialize_record(&value)?);
        }
        Ok(peers)
    }

    pub fn upsert_address(&self, record: &AddressRecord) -> Result<(), StorageError> {
        let key = record.address.as_bytes().to_vec();
        let value = bincode::serialize(record).map_err(|_| StorageError::CorruptData)?;
        self.put(Dataset::Addresses, &key, &value)
    }

    pub fn list_addresses(&self) -> Result<Vec<AddressRecord>, StorageError> {
        let mut addresses = Vec::new();
        for (_, value) in self.entries(Dataset::Addresses)? {
            addresses.push(deserialize_record(&value)?);
        }
        Ok(addresses)
    }

    pub fn upsert_peer_health(&self, record: &PeerHealthRecord) -> Result<(), StorageError> {
        let key = record.remote_addr.as_bytes().to_vec();
        let value = bincode::serialize(record).map_err(|_| StorageError::CorruptData)?;
        self.put(Dataset::PeerHealth, &key, &value)
    }

    pub fn load_peer_health(
        &self,
        remote_addr: &str,
    ) -> Result<Option<PeerHealthRecord>, StorageError> {
        match self.get(Dataset::PeerHealth, remote_addr.as_bytes())? {
            Some(bytes) => deserialize_record(&bytes).map(Some),
            None => Ok(None),
        }
    }

    pub fn list_peer_health(&self) -> Result<Vec<PeerHealthRecord>, StorageError> {
        let mut records = Vec::new();
        for (_, value) in self.entries(Dataset::PeerHealth)? {
            records.push(deserialize_record(&value)?);
        }
        Ok(records)
    }

    pub fn mark_runtime_started(&self) -> Result<(), StorageError> {
        let now = current_unix_seconds();
        self.update_runtime_state(|state| {
            state.last_clean_shutdown = false;
            state.last_runtime_started_unix = now;
        })
        .map(|_| ())
    }

    pub fn mark_clean_shutdown(&self) -> Result<(), StorageError> {
        let now = current_unix_seconds();
        self.update_runtime_state(|state| {
            state.last_clean_shutdown = true;
            state.last_shutdown_unix = now;
        })
        .map(|_| ())
    }

    fn ensure_schema_version(&self) -> Result<(), StorageError> {
        match self.get(Dataset::Meta, SCHEMA_VERSION_KEY)? {
            Some(bytes) => {
                let bytes: [u8; 4] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| StorageError::CorruptData)?;
                let found = u32::from_le_bytes(bytes);
                if found != STORAGE_SCHEMA_VERSION {
                    return Err(StorageError::SchemaVersionMismatch {
                        expected: STORAGE_SCHEMA_VERSION,
                        found,
                    });
                }
            }
            None => {
                self.put(
                    Dataset::Meta,
                    SCHEMA_VERSION_KEY,
                    &STORAGE_SCHEMA_VERSION.to_le_bytes(),
                )?;
            }
        }
        Ok(())
    }

    fn ensure_runtime_state(&self) -> Result<(), StorageError> {
        if self.load_runtime_state()?.is_some() {
            return Ok(());
        }
        let value = bincode::serialize(&StorageRuntimeState {
            last_clean_shutdown: true,
            last_runtime_started_unix: 0,
            last_shutdown_unix: 0,
            last_recovery_check_unix: 0,
        })
        .map_err(|_| StorageError::CorruptData)?;
        self.put(Dataset::Meta, STORAGE_RUNTIME_STATE_KEY, &value)?;
        Ok(())
    }

    fn ensure_storage_metadata(&self) -> Result<(), StorageError> {
        let now = current_unix_seconds();
        let expected = expected_storage_metadata(self.network, now);
        match self.load_storage_metadata()? {
            Some(mut existing) => {
                if existing.storage_magic != expected.storage_magic {
                    return Err(StorageError::StorageMetadataMismatch {
                        field: "storage_magic",
                    });
                }
                if existing.network_magic != expected.network_magic {
                    return Err(StorageError::StorageMetadataMismatch {
                        field: "network_magic",
                    });
                }
                if existing.network_name != expected.network_name {
                    return Err(StorageError::StorageMetadataMismatch {
                        field: "network_name",
                    });
                }
                if existing.chain_id != expected.chain_id {
                    return Err(StorageError::StorageMetadataMismatch { field: "chain_id" });
                }
                if existing.genesis_hash != expected.genesis_hash {
                    return Err(StorageError::PersistedGenesisMismatch);
                }
                if existing.genesis_block_id != expected.genesis_block_id {
                    return Err(StorageError::StorageMetadataMismatch {
                        field: "genesis_block_id",
                    });
                }
                if existing.database_schema_version != expected.database_schema_version {
                    return Err(StorageError::SchemaVersionMismatch {
                        expected: expected.database_schema_version,
                        found: existing.database_schema_version,
                    });
                }
                if existing.software_storage_version != expected.software_storage_version {
                    return Err(StorageError::StorageMetadataMismatch {
                        field: "software_storage_version",
                    });
                }
                existing.last_opened_unix = now;
                let value = bincode::serialize(&existing).map_err(|_| StorageError::CorruptData)?;
                self.put(Dataset::Meta, STORAGE_METADATA_KEY, &value)?;
            }
            None => {
                let value = bincode::serialize(&expected).map_err(|_| StorageError::CorruptData)?;
                self.put(Dataset::Meta, STORAGE_METADATA_KEY, &value)?;
            }
        }
        Ok(())
    }

    fn run_startup_consistency_checks(&self) -> Result<(), StorageError> {
        let journal_path = path::storage_commit_journal_path(self.network);
        let journal_present = journal_path.exists();
        let runtime_state = self.load_runtime_state()?.unwrap_or(StorageRuntimeState {
            last_clean_shutdown: true,
            last_runtime_started_unix: 0,
            last_shutdown_unix: 0,
            last_recovery_check_unix: 0,
        });
        if runtime_state.last_clean_shutdown && !journal_present {
            return Ok(());
        }

        self.verify_persisted_chainstate_consistency()?;
        if journal_present {
            let _ = fs::remove_file(&journal_path);
        }
        let now = current_unix_seconds();
        self.update_runtime_state(|state| {
            state.last_clean_shutdown = true;
            state.last_recovery_check_unix = now;
        })?;
        Ok(())
    }

    fn verify_persisted_chainstate_consistency(&self) -> Result<(), StorageError> {
        let snapshot = self.load_chainstate_snapshot()?;
        let mut main_chain_records = self
            .list_block_records()?
            .into_iter()
            .filter(|record| record.main_chain)
            .collect::<Vec<_>>();
        main_chain_records.sort_by_key(|record| record.height);
        let tip_record = main_chain_records.last().cloned();
        match (snapshot, tip_record) {
            (None, None) => return Ok(()),
            (Some(snapshot), Some(record)) => {
                if snapshot.height != record.height || snapshot.tip_hash != record.block_hash {
                    return Err(StorageError::PersistedTipMismatch);
                }
                if main_chain_records.len() != snapshot.height as usize + 1 {
                    return Err(StorageError::IncompleteBlockHistory);
                }
                if snapshot.tip_header.as_ref().is_some_and(|header| {
                    header.block_hash() != snapshot.tip_hash || header.height != snapshot.height
                }) {
                    return Err(StorageError::PersistedTipMismatch);
                }

                // Dirty-start recovery replays the canonical block history from
                // genesis so we verify the persisted UTXO table against the
                // blocks themselves instead of trusting the snapshot alone.
                let mut replay = crate::chainstate::Chainstate::fresh(self.network);
                let hardcoded_genesis = genesis::genesis_state(self.network).block;
                for (expected_height, record) in main_chain_records.iter().enumerate() {
                    let expected_height = expected_height as u64;
                    if record.height != expected_height {
                        return Err(StorageError::IncompleteBlockHistory);
                    }
                    let indexed_hash = self
                        .load_block_hash_by_height(expected_height)?
                        .ok_or(StorageError::IncompleteBlockHistory)?;
                    if indexed_hash != record.block_hash {
                        return Err(StorageError::PersistedTipMismatch);
                    }
                    let block = self
                        .load_block(record.block_hash)?
                        .ok_or(StorageError::IncompleteBlockHistory)?;
                    if block.header.block_hash() != record.block_hash
                        || block.header.height != expected_height
                    {
                        return Err(StorageError::CorruptData);
                    }
                    if expected_height == 0 {
                        if block.header != hardcoded_genesis.header
                            || block.transactions != hardcoded_genesis.transactions
                        {
                            return Err(StorageError::PersistedGenesisMismatch);
                        }
                        continue;
                    }
                    replay
                        .connect_block(&block)
                        .map_err(|_| StorageError::CorruptData)?;
                }

                if replay.height != snapshot.height || replay.tip_hash != snapshot.tip_hash {
                    return Err(StorageError::PersistedTipMismatch);
                }
                if snapshot
                    .tip_header
                    .as_ref()
                    .is_some_and(|header| replay.tip.as_ref() != Some(header))
                {
                    return Err(StorageError::PersistedTipMismatch);
                }

                let mut persisted_utxos = self.load_utxos()?;
                if persisted_utxos
                    .iter()
                    .any(|entry| entry.network != self.network)
                {
                    return Err(StorageError::CrossNetworkReplay);
                }
                let mut rebuilt_utxos = replay.utxo_entries().cloned().collect::<Vec<_>>();
                sort_utxos_for_consistency_check(&mut persisted_utxos);
                sort_utxos_for_consistency_check(&mut rebuilt_utxos);
                if persisted_utxos != rebuilt_utxos {
                    return Err(StorageError::CorruptData);
                }
                Ok(())
            }
            _ => Err(StorageError::IncompleteBlockHistory),
        }
    }

    fn update_runtime_state<F>(&self, update: F) -> Result<StorageRuntimeState, StorageError>
    where
        F: FnOnce(&mut StorageRuntimeState),
    {
        let mut state = self.load_runtime_state()?.unwrap_or(StorageRuntimeState {
            last_clean_shutdown: true,
            last_runtime_started_unix: 0,
            last_shutdown_unix: 0,
            last_recovery_check_unix: 0,
        });
        update(&mut state);
        let value = bincode::serialize(&state).map_err(|_| StorageError::CorruptData)?;
        self.put(Dataset::Meta, STORAGE_RUNTIME_STATE_KEY, &value)?;
        Ok(state)
    }

    fn get(&self, dataset: Dataset, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let state = self.state.lock().expect("database lock poisoned");
        let txn = state.env.begin_ro_txn()?;
        match txn.get(dataset.db(&state), &key) {
            Ok(bytes) => Ok(Some(bytes.to_vec())),
            Err(LmdbError::NotFound) => Ok(None),
            Err(err) => Err(StorageError::Lmdb(err)),
        }
    }

    fn entries(&self, dataset: Dataset) -> Result<RawEntries, StorageError> {
        let state = self.state.lock().expect("database lock poisoned");
        let txn = state.env.begin_ro_txn()?;
        let mut cursor = txn.open_ro_cursor(dataset.db(&state))?;
        let mut entries = Vec::new();
        for (key, value) in cursor.iter() {
            entries.push((key.to_vec(), value.to_vec()));
        }
        Ok(entries)
    }

    fn put(&self, dataset: Dataset, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            txn.put(dataset.db(state), &key, &value, WriteFlags::empty())?;
            txn.commit()?;
            Ok(())
        })
    }

    fn write_with_retry<T, F>(&self, mut op: F) -> Result<T, StorageError>
    where
        F: FnMut(&mut DatabaseState) -> Result<T, StorageError>,
    {
        loop {
            let mut guard = self.state.lock().expect("database lock poisoned");
            match op(&mut guard) {
                Ok(value) => return Ok(value),
                Err(StorageError::Lmdb(LmdbError::MapFull)) => {
                    let next = guard.map_size.saturating_mul(2).min(MAX_MAP_SIZE);
                    if next == guard.map_size {
                        return Err(StorageError::Lmdb(LmdbError::MapFull));
                    }
                    drop(guard);
                    self.reopen(next)?;
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn reopen(&self, map_size: usize) -> Result<(), StorageError> {
        let mut guard = self.state.lock().expect("database lock poisoned");
        if map_size <= guard.map_size {
            return Ok(());
        }
        let state = Self::open_state(&self.path, map_size)?;
        *guard = state;
        Ok(())
    }

    fn open_state(path: &Path, map_size: usize) -> Result<DatabaseState, StorageError> {
        let mut builder = Environment::new();
        builder
            .set_max_readers(128)
            .set_max_dbs(MAX_DBS)
            .set_map_size(map_size);
        let env = builder.open(path)?;
        let meta = env.create_db(Some(META_DB), DatabaseFlags::empty())?;
        let blocks = env.create_db(Some(BLOCKS_DB), DatabaseFlags::empty())?;
        let block_heights = env.create_db(Some(BLOCK_HEIGHTS_DB), DatabaseFlags::empty())?;
        let block_transactions =
            env.create_db(Some(BLOCK_TRANSACTIONS_DB), DatabaseFlags::empty())?;
        let transactions = env.create_db(Some(TRANSACTIONS_DB), DatabaseFlags::empty())?;
        let utxos = env.create_db(Some(UTXOS_DB), DatabaseFlags::empty())?;
        let peers = env.create_db(Some(PEERS_DB), DatabaseFlags::empty())?;
        let addresses = env.create_db(Some(ADDRESSES_DB), DatabaseFlags::empty())?;
        let peer_health = env.create_db(Some(PEER_HEALTH_DB), DatabaseFlags::empty())?;
        Ok(DatabaseState {
            env,
            map_size,
            meta,
            blocks,
            block_heights,
            block_transactions,
            transactions,
            utxos,
            peers,
            addresses,
            peer_health,
        })
    }

    #[cfg(test)]
    pub fn inject_commit_fault_for_test(point: CommitFaultPoint, remaining_hits: usize) {
        let fault = COMMIT_FAULT.get_or_init(|| Mutex::new(None));
        *fault.lock().expect("commit fault mutex poisoned") = Some(CommitFault {
            point,
            remaining_hits,
        });
    }

    #[cfg(test)]
    pub fn clear_commit_fault_for_test() {
        if let Some(fault) = COMMIT_FAULT.get() {
            *fault.lock().expect("commit fault mutex poisoned") = None;
        }
    }

    fn archive_append_file_hint(&self) -> Result<Option<u64>, StorageError> {
        if self.block_store.highest_file_number()?.is_some() {
            return Ok(None);
        }

        Ok(self
            .list_block_records()?
            .into_iter()
            .map(|record| record.file_number)
            .max()
            .map(|file_number| file_number.saturating_add(1)))
    }
}

fn expected_storage_metadata(network: Network, now: u64) -> StorageMetadata {
    let genesis_hash = genesis::genesis_hash(network);
    StorageMetadata {
        storage_magic: STORAGE_MAGIC,
        network_magic: network.p2p_magic(),
        network_name: network.id().to_string(),
        chain_id: genesis_hash,
        genesis_hash,
        genesis_block_id: genesis_hash,
        database_schema_version: STORAGE_SCHEMA_VERSION,
        software_storage_version: SOFTWARE_STORAGE_VERSION,
        created_at_unix: now,
        last_opened_unix: now,
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn atomic_write_owner_only(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("storage.tmp");
    let tmp_path = path.with_file_name(format!("{file_name}.tmp"));
    #[cfg(unix)]
    {
        let mut file = File::create(&tmp_path)?;
        fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&tmp_path, bytes)?;
    }
    fs::rename(&tmp_path, path)?;
    #[cfg(unix)]
    {
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        if let Some(parent) = path.parent() {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }
    Ok(())
}

impl Dataset {
    fn db(self, state: &DatabaseState) -> LmdbDatabase {
        match self {
            Dataset::Meta => state.meta,
            Dataset::Blocks => state.blocks,
            Dataset::BlockHeights => state.block_heights,
            Dataset::BlockTransactions => state.block_transactions,
            Dataset::Transactions => state.transactions,
            Dataset::Utxos => state.utxos,
            Dataset::Peers => state.peers,
            Dataset::Addresses => state.addresses,
            Dataset::PeerHealth => state.peer_health,
        }
    }
}

#[derive(Debug)]
struct CommitJournalGuard {
    path: PathBuf,
    finished: bool,
}

impl CommitJournalGuard {
    fn begin(
        network: Network,
        operation: &str,
        height: u64,
        tip_hash: Option<[u8; 48]>,
    ) -> Result<Self, StorageError> {
        let path = path::storage_commit_journal_path(network);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = format!(
            "network={}\noperation={}\nheight={}\ntip_hash={}\nstarted_at_unix={}\n",
            network.id(),
            operation,
            height,
            tip_hash
                .map(hex::encode)
                .unwrap_or_else(|| String::from("none")),
            current_unix_seconds()
        );
        atomic_write_owner_only(&path, body.as_bytes())?;
        Ok(Self {
            path,
            finished: false,
        })
    }

    fn finish(mut self) -> Result<(), StorageError> {
        self.finished = true;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StorageError::Io(err)),
        }
    }
}

fn write_block_archive(
    txn: &mut RwTransaction<'_>,
    state: &DatabaseState,
    network: Network,
    height: u64,
    block: &Block,
    location: BlockFileLocation,
) -> Result<(), StorageError> {
    let block_hash = block.header.block_hash();
    if let Ok(existing) = txn.get(state.blocks, &block_hash) {
        let existing: BlockArchiveRecord = deserialize_record(existing)?;
        if existing.height == height && existing.block_hash == block_hash {
            return Ok(());
        }
        return Err(StorageError::CorruptData);
    }

    let work = pow::block_proof_work(&block.header.difficulty_target_or_bits);
    let chainwork = if height == 0 {
        work
    } else {
        let previous = load_block_record_from_txn(txn, state, block.header.previous_block_hash)?
            .ok_or(StorageError::IncompleteBlockHistory)?;
        BigUint::from_bytes_be(&previous.chainwork) + work
    };
    let persisted_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let block_record = BlockArchiveRecord {
        height,
        block_hash,
        previous_block_hash: block.header.previous_block_hash,
        network,
        version: block.header.version,
        merkle_root: block.header.merkle_root,
        witness_root: block.header.witness_root,
        timestamp: block.header.timestamp,
        difficulty_target_or_bits: block.header.difficulty_target_or_bits,
        nonce: block.header.nonce,
        file_number: location.file_number,
        record_offset: location.record_offset,
        payload_length: location.payload_length,
        raw_block_size: block.full_size_bytes() as u32,
        weight_bytes: block.weight_bytes() as u32,
        vsize_bytes: block.vsize_bytes() as u32,
        tx_count: block.transactions.len() as u32,
        fees_total_atoms: block.fees_total_atoms,
        fees_miner_atoms: block.fees_miner_atoms,
        chainwork: chainwork.to_bytes_be(),
        fully_validated: true,
        main_chain: true,
        pruned: false,
        persisted_unix,
    };
    let block_value = bincode::serialize(&block_record).map_err(|_| StorageError::CorruptData)?;
    txn.put(state.blocks, &block_hash, &block_value, WriteFlags::empty())?;
    txn.put(
        state.block_heights,
        &height_key(height),
        &block_hash,
        WriteFlags::empty(),
    )?;

    let mut txids = Vec::with_capacity(block.transactions.len());
    for (tx_index, tx) in block.transactions.iter().enumerate() {
        let tx_record = TransactionArchiveRecord {
            height,
            block_hash,
            tx_index: tx_index as u32,
            txid: tx.txid(),
            transaction: tx.clone(),
        };
        txids.push(tx_record.txid.to_vec());
        let tx_value = bincode::serialize(&tx_record).map_err(|_| StorageError::CorruptData)?;
        txn.put(
            state.transactions,
            &tx_record.txid,
            &tx_value,
            WriteFlags::empty(),
        )?;
    }
    let order_value = bincode::serialize(&BlockTransactionsRecord { height, txids })
        .map_err(|_| StorageError::CorruptData)?;
    txn.put(
        state.block_transactions,
        &block_hash,
        &order_value,
        WriteFlags::empty(),
    )?;
    Ok(())
}

fn apply_utxo_delta(
    txn: &mut RwTransaction<'_>,
    state: &DatabaseState,
    network: Network,
    block: &Block,
) -> Result<(), StorageError> {
    for tx in &block.transactions {
        for input in &tx.inputs {
            let key = utxo_key(input.previous_txid, input.output_index);
            match txn.del(state.utxos, &key.as_slice(), None) {
                Ok(()) => {}
                // Block connection has already validated the spend against the
                // in-memory chainstate. Treat an absent persisted key as an
                // idempotent delete so dev-seeded or self-repaired state can
                // still commit the correct post-block UTXO image.
                Err(LmdbError::NotFound) => {}
                Err(err) => return Err(StorageError::Lmdb(err)),
            }
        }

        let txid = tx.txid();
        for (output_index, output) in tx.outputs.iter().enumerate() {
            let entry = UtxoEntry::new(
                network,
                txid,
                output_index as u32,
                output.value_atoms,
                output.locking_script.clone(),
                block.header.height,
                tx.is_coinbase(),
            );
            let key = utxo_key(entry.txid, entry.output_index);
            match txn.get(state.utxos, &key.as_slice()) {
                Ok(_) => return Err(StorageError::DuplicateUtxo),
                Err(LmdbError::NotFound) => {}
                Err(err) => return Err(StorageError::Lmdb(err)),
            }
            let value = bincode::serialize(&entry).map_err(|_| StorageError::CorruptData)?;
            txn.put(
                state.utxos,
                &key.as_slice(),
                &value.as_slice(),
                WriteFlags::empty(),
            )?;
        }
    }
    Ok(())
}

fn rebuild_height_index(
    txn: &mut RwTransaction<'_>,
    state: &DatabaseState,
    tip_hash: [u8; 48],
) -> Result<(), StorageError> {
    let old_main_chain = current_main_chain_hashes(txn, state)?;
    clear_db(txn, state.block_heights)?;
    if tip_hash == [0; 48] {
        for hash in old_main_chain {
            update_main_chain_flag(txn, state, hash, false)?;
        }
        return Ok(());
    }

    let mut new_main_chain = BTreeSet::new();
    let mut next_hash = tip_hash;
    loop {
        let Some(record) = load_block_record_from_txn(txn, state, next_hash)? else {
            return Err(StorageError::IncompleteBlockHistory);
        };
        txn.put(
            state.block_heights,
            &height_key(record.height),
            &record.block_hash,
            WriteFlags::empty(),
        )?;
        new_main_chain.insert(record.block_hash);
        if !record.main_chain {
            update_main_chain_flag(txn, state, record.block_hash, true)?;
        }
        if record.height == 0 {
            break;
        }
        next_hash = record.previous_block_hash;
    }

    for hash in old_main_chain {
        if !new_main_chain.contains(&hash) {
            update_main_chain_flag(txn, state, hash, false)?;
        }
    }
    Ok(())
}

fn current_main_chain_hashes(
    txn: &RwTransaction<'_>,
    state: &DatabaseState,
) -> Result<BTreeSet<[u8; 48]>, StorageError> {
    let mut cursor = txn.open_ro_cursor(state.block_heights)?;
    let mut hashes = BTreeSet::new();
    for (_, value) in cursor.iter() {
        let hash: [u8; 48] = value.try_into().map_err(|_| StorageError::CorruptData)?;
        hashes.insert(hash);
    }
    Ok(hashes)
}

fn update_main_chain_flag(
    txn: &mut RwTransaction<'_>,
    state: &DatabaseState,
    block_hash: [u8; 48],
    main_chain: bool,
) -> Result<(), StorageError> {
    let Some(mut record) = load_block_record_from_txn(txn, state, block_hash)? else {
        return Err(StorageError::IncompleteBlockHistory);
    };
    if record.main_chain == main_chain {
        return Ok(());
    }
    record.main_chain = main_chain;
    let bytes = bincode::serialize(&record).map_err(|_| StorageError::CorruptData)?;
    txn.put(state.blocks, &block_hash, &bytes, WriteFlags::empty())?;
    Ok(())
}

fn load_block_record_from_txn(
    txn: &RwTransaction<'_>,
    state: &DatabaseState,
    block_hash: [u8; 48],
) -> Result<Option<BlockArchiveRecord>, StorageError> {
    match txn.get(state.blocks, &block_hash) {
        Ok(bytes) => deserialize_record(bytes).map(Some),
        Err(LmdbError::NotFound) => Ok(None),
        Err(err) => Err(StorageError::Lmdb(err)),
    }
}

fn deserialize_record<T>(bytes: &[u8]) -> Result<T, StorageError>
where
    T: for<'de> Deserialize<'de>,
{
    bincode::deserialize(bytes).map_err(|_| StorageError::CorruptData)
}

fn configured_prune_target_bytes() -> Option<u64> {
    std::env::var("ATHO_PRUNE_TARGET_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn configured_db_cache_bytes() -> usize {
    std::env::var("ATHO_DB_CACHE_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(64 * 1024 * 1024, MAX_MAP_SIZE))
        .unwrap_or(INITIAL_MAP_SIZE)
}

fn clear_db(txn: &mut RwTransaction<'_>, db: LmdbDatabase) -> Result<(), StorageError> {
    let keys: Vec<Vec<u8>> = {
        let mut cursor = txn.open_rw_cursor(db)?;
        cursor.iter().map(|(key, _)| key.to_vec()).collect()
    };
    for key in keys {
        let _ = txn.del(db, &key, None);
    }
    Ok(())
}

fn legacy_layout_present(root: &Path) -> bool {
    root.join(LEGACY_META_DIR).exists()
        || root.join(LEGACY_TRANSACTIONS_DIR).exists()
        || root.join(LEGACY_UTXOS_DIR).exists()
        || root.join(LEGACY_PEERS_DIR).exists()
        || root.join(LEGACY_ADDRESSES_DIR).exists()
        || root.join(LEGACY_BLOCKS_DIR).join("data.mdb").exists()
        || root.join(LEGACY_BLOCKS_DIR).join("lock.mdb").exists()
}

fn height_key(height: u64) -> [u8; 8] {
    height.to_be_bytes()
}

fn utxo_key(txid: [u8; 48], output_index: u32) -> Vec<u8> {
    let mut key = Vec::with_capacity(52);
    key.extend_from_slice(&txid);
    key.extend_from_slice(&output_index.to_be_bytes());
    key
}

fn sort_utxos_for_consistency_check(utxos: &mut [UtxoEntry]) {
    utxos.sort_by(|left, right| {
        left.txid
            .cmp(&right.txid)
            .then(left.output_index.cmp(&right.output_index))
            .then(left.value_atoms.cmp(&right.value_atoms))
            .then(left.created_height.cmp(&right.created_height))
            .then(left.is_coinbase.cmp(&right.is_coinbase))
            .then(left.locking_script.cmp(&right.locking_script))
            .then(left.network.id().cmp(right.network.id()))
    });
}

#[cfg(test)]
fn maybe_inject_commit_fault(point: CommitFaultPoint) -> Result<(), StorageError> {
    let Some(fault) = COMMIT_FAULT.get() else {
        return Ok(());
    };
    let mut guard = fault.lock().expect("commit fault mutex poisoned");
    let Some(active) = guard.as_mut() else {
        return Ok(());
    };
    if active.point != point || active.remaining_hits == 0 {
        return Ok(());
    }
    active.remaining_hits = active.remaining_hits.saturating_sub(1);
    if active.remaining_hits == 0 {
        *guard = None;
    }
    Err(StorageError::Io(std::io::Error::other(format!(
        "fault injected at storage commit point {point:?}"
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::ATHO_DATA_DIR_ENV;
    use crate::test_support::acquire_global_test_lock;
    use atho_core::block::{merkle_root, witness_root};
    use atho_core::transaction::{Transaction, TxInput, TxOutput};
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};
    use std::sync::MutexGuard;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &Path) -> Self {
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

    fn temp_data_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atho-db-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn sample_block(network: Network, height: u64, previous_block_hash: [u8; 48]) -> Block {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: atho_core::consensus::subsidy::block_subsidy_atoms_for_network(
                    network, height,
                ),
                locking_script: vec![3, 4, height as u8],
            }],
            lock_time: height as u32,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let header = BlockHeader {
            version: 1,
            network_id: network,
            height,
            previous_block_hash,
            merkle_root: merkle_root(std::slice::from_ref(&tx)),
            witness_root: witness_root(std::slice::from_ref(&tx)),
            founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
            founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
            timestamp: 1_700_000_000 + height,
            difficulty_target_or_bits: [7; 48],
            nonce: 42 + height,
        };
        Block::new(header, vec![tx])
    }

    fn block_with_transactions(
        network: Network,
        height: u64,
        previous_block_hash: [u8; 48],
        transactions: Vec<Transaction>,
    ) -> Block {
        let header = BlockHeader {
            version: 1,
            network_id: network,
            height,
            previous_block_hash,
            merkle_root: merkle_root(&transactions),
            witness_root: witness_root(&transactions),
            founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
            founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
            timestamp: 1_700_000_000 + height,
            difficulty_target_or_bits: [7; 48],
            nonce: 42 + height,
        };
        Block::new(header, transactions)
    }

    fn output_entry(
        network: Network,
        tx: &Transaction,
        output_index: u32,
        created_height: u64,
    ) -> UtxoEntry {
        let output = &tx.outputs[output_index as usize];
        UtxoEntry::new(
            network,
            tx.txid(),
            output_index,
            output.value_atoms,
            output.locking_script.clone(),
            created_height,
            tx.is_coinbase(),
        )
    }

    #[test]
    fn current_schema_version_initializes_cleanly() {
        let root = temp_data_dir("schema-current");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open current db");
        let version = database
            .get(Dataset::Meta, SCHEMA_VERSION_KEY)
            .expect("schema bytes")
            .expect("schema present");
        assert_eq!(
            u32::from_le_bytes(version.try_into().expect("u32 bytes")),
            STORAGE_SCHEMA_VERSION
        );
    }

    #[test]
    fn incompatible_schema_version_fails_closed() {
        let root = temp_data_dir("schema-reject");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open current db");
        database
            .put(Dataset::Meta, SCHEMA_VERSION_KEY, &3u32.to_le_bytes())
            .expect("force old schema");
        drop(database);

        let err = Database::open(Network::Regnet).unwrap_err();
        assert!(matches!(
            err,
            StorageError::SchemaVersionMismatch {
                expected: STORAGE_SCHEMA_VERSION,
                found: 3
            }
        ));
    }

    #[test]
    fn peer_health_round_trips_through_storage() {
        let root = temp_data_dir("peer-health");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open db");
        let record = PeerHealthRecord {
            network: Network::Regnet,
            remote_addr: String::from("127.0.0.1:18445"),
            quality_score: 72,
            consecutive_failures: 3,
            backoff_until_unix: 1_700_000_123,
            last_failure_unix: Some(1_700_000_120),
            last_success_unix: Some(1_700_000_000),
        };
        database
            .upsert_peer_health(&record)
            .expect("persist peer health");

        let loaded = database
            .load_peer_health(&record.remote_addr)
            .expect("load peer health")
            .expect("peer health present");
        assert_eq!(loaded.remote_addr, record.remote_addr);
        assert_eq!(loaded.quality_score, 72);
        assert_eq!(loaded.consecutive_failures, 3);
        assert_eq!(loaded.backoff_until_unix, 1_700_000_123);
    }

    #[test]
    fn append_block_uses_next_file_number_after_archive_dir_is_deleted() {
        let root = temp_data_dir("append-after-archive-loss");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open db");

        let first = sample_block(Network::Regnet, 0, [0; 48]);
        database.append_block(0, &first).expect("append first");
        let first_record = database
            .load_block_record(first.header.block_hash())
            .expect("load first record")
            .expect("first record present");
        assert_eq!(first_record.file_number, 0);

        fs::remove_dir_all(database.block_storage_path()).expect("remove archive dir");
        assert!(!database.block_storage_path().exists());

        let second = sample_block(Network::Regnet, 1, first.header.block_hash());
        database.append_block(1, &second).expect("append second");
        let second_record = database
            .load_block_record(second.header.block_hash())
            .expect("load second record")
            .expect("second record present");
        assert_eq!(second_record.file_number, 1);
        assert!(database.block_storage_path().exists());
        assert!(database
            .block_storage_path()
            .join(format!("blk{:05}.dat", second_record.file_number))
            .exists());
    }

    #[test]
    fn commit_chainstate_appended_block_applies_utxo_delta() {
        let root = temp_data_dir("incremental-utxo-delta");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open db");

        let genesis = sample_block(Network::Regnet, 0, [0; 48]);
        let snapshot = ChainstateSnapshot {
            height: 0,
            tip_hash: genesis.header.block_hash(),
            tip_header: Some(genesis.header.clone()),
        };
        database
            .commit_chainstate(&snapshot, &[], Some((0, &genesis)))
            .expect("commit genesis delta");

        let first = sample_block(Network::Regnet, 1, genesis.header.block_hash());
        let snapshot = ChainstateSnapshot {
            height: 1,
            tip_hash: first.header.block_hash(),
            tip_header: Some(first.header.clone()),
        };
        database
            .commit_chainstate(&snapshot, &[], Some((1, &first)))
            .expect("commit first delta without full utxo image");

        let first_coinbase = output_entry(Network::Regnet, &first.transactions[0], 0, 1);
        let mut utxos = database.load_utxos().expect("load utxos");
        utxos.sort_by(|left, right| left.txid.cmp(&right.txid));
        assert!(utxos.iter().any(|entry| entry.txid == first_coinbase.txid
            && entry.output_index == first_coinbase.output_index));

        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: atho_core::consensus::subsidy::block_subsidy_atoms_for_network(
                    Network::Regnet,
                    2,
                ),
                locking_script: vec![9, 9, 2],
            }],
            lock_time: 2,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let spend = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: first_coinbase.txid,
                output_index: first_coinbase.output_index,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: first_coinbase.value_atoms.saturating_sub(1),
                locking_script: vec![4, 5, 6],
            }],
            lock_time: 2,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let second = block_with_transactions(
            Network::Regnet,
            2,
            first.header.block_hash(),
            vec![coinbase.clone(), spend.clone()],
        );
        let snapshot = ChainstateSnapshot {
            height: 2,
            tip_hash: second.header.block_hash(),
            tip_header: Some(second.header.clone()),
        };
        database
            .commit_chainstate(&snapshot, &[], Some((2, &second)))
            .expect("commit spend delta without full utxo image");

        let utxos = database.load_utxos().expect("load updated utxos");
        assert!(!utxos.iter().any(|entry| entry.txid == first_coinbase.txid
            && entry.output_index == first_coinbase.output_index));
        let spend_output = output_entry(Network::Regnet, &spend, 0, 2);
        assert!(utxos.iter().any(|entry| entry.txid == spend_output.txid
            && entry.output_index == spend_output.output_index));
        let coinbase_output = output_entry(Network::Regnet, &coinbase, 0, 2);
        assert!(utxos.iter().any(|entry| entry.txid == coinbase_output.txid
            && entry.output_index == coinbase_output.output_index));
    }

    #[test]
    fn full_snapshot_commit_faults_leave_prior_snapshot_intact() {
        let root = temp_data_dir("snapshot-faults");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open db");

        let genesis = sample_block(Network::Regnet, 0, [0; 48]);
        let first = sample_block(Network::Regnet, 1, genesis.header.block_hash());
        let second = sample_block(Network::Regnet, 2, first.header.block_hash());
        database.append_block(0, &genesis).expect("append genesis");
        database.append_block(1, &first).expect("append first");
        database.append_block(2, &second).expect("append second");

        let initial_snapshot = ChainstateSnapshot {
            height: 1,
            tip_hash: first.header.block_hash(),
            tip_header: None,
        };
        let initial_utxos = vec![output_entry(Network::Regnet, &first.transactions[0], 0, 1)];
        database
            .save_chainstate_snapshot(&initial_snapshot, &initial_utxos)
            .expect("persist initial snapshot");

        let replacement_snapshot = ChainstateSnapshot {
            height: 2,
            tip_hash: second.header.block_hash(),
            tip_header: None,
        };
        let replacement_utxos = vec![
            output_entry(Network::Regnet, &first.transactions[0], 0, 1),
            output_entry(Network::Regnet, &second.transactions[0], 0, 2),
        ];

        for point in [
            CommitFaultPoint::AfterSnapshotWrite,
            CommitFaultPoint::AfterStateWrite,
            CommitFaultPoint::BeforeCommit,
        ] {
            Database::inject_commit_fault_for_test(point, 1);
            let result =
                database.save_chainstate_snapshot(&replacement_snapshot, &replacement_utxos);
            Database::clear_commit_fault_for_test();

            assert!(
                matches!(result, Err(StorageError::Io(_))),
                "fault point {point:?}"
            );
            let persisted_snapshot = database
                .load_chainstate_snapshot()
                .expect("load snapshot")
                .expect("snapshot present");
            assert_eq!(
                persisted_snapshot.height, initial_snapshot.height,
                "fault point {point:?}"
            );
            assert_eq!(
                persisted_snapshot.tip_hash, initial_snapshot.tip_hash,
                "fault point {point:?}"
            );

            let mut persisted_utxos = database.load_utxos().expect("load utxos");
            let mut expected_utxos = initial_utxos.clone();
            sort_utxos_for_consistency_check(&mut persisted_utxos);
            sort_utxos_for_consistency_check(&mut expected_utxos);
            assert_eq!(persisted_utxos, expected_utxos, "fault point {point:?}");
        }
    }

    #[test]
    fn load_block_rebuilds_when_raw_archive_has_wrong_network_magic() {
        let root = temp_data_dir("raw-cross-network");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open db");
        let block = sample_block(Network::Regnet, 0, [0; 48]);
        let block_hash = block.header.block_hash();
        database.append_block(0, &block).expect("append block");
        let record = database
            .load_block_record(block_hash)
            .expect("load record")
            .expect("record");
        let location = record.file_location();
        let raw_path = database
            .block_storage_path()
            .join(format!("blk{:05}.dat", location.file_number));
        let mut raw_file = fs::OpenOptions::new()
            .write(true)
            .open(raw_path)
            .expect("open raw block file");
        raw_file
            .seek(SeekFrom::Start(location.record_offset))
            .expect("seek raw wrapper");
        raw_file
            .write_all(&Network::Mainnet.p2p_magic())
            .expect("write wrong magic");
        raw_file.flush().expect("flush raw corruption");

        let recovered = database
            .load_block(block_hash)
            .expect("load block from indexed metadata")
            .expect("block");

        assert_eq!(recovered.header.block_hash(), block_hash);
        assert_eq!(recovered.header.network_id, Network::Regnet);
        assert_eq!(recovered.transactions, block.transactions);
    }

    #[test]
    fn runtime_state_tracks_started_and_clean_shutdown_markers() {
        let root = temp_data_dir("runtime-state");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open db");

        database.mark_runtime_started().expect("mark started");
        let started = database
            .load_runtime_state()
            .expect("runtime state")
            .expect("present runtime state");
        assert!(!started.last_clean_shutdown);
        assert!(started.last_runtime_started_unix > 0);

        database.mark_clean_shutdown().expect("mark clean");
        let stopped = database
            .load_runtime_state()
            .expect("runtime state after stop")
            .expect("present runtime state after stop");
        assert!(stopped.last_clean_shutdown);
        assert!(stopped.last_shutdown_unix > 0);
    }

    #[test]
    fn unclean_runtime_state_runs_startup_self_check_and_clears_commit_journal() {
        let root = temp_data_dir("startup-self-check");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open db");
        database.mark_runtime_started().expect("mark started");
        CommitJournalGuard::begin(Network::Regnet, "commit", 0, None)
            .expect("write journal")
            .finish()
            .expect("remove initial journal");
        fs::write(
            path::storage_commit_journal_path(Network::Regnet),
            b"stale-journal",
        )
        .expect("rewrite stale journal");
        drop(database);

        let reopened = Database::open(Network::Regnet).expect("reopen db");
        let runtime_state = reopened
            .load_runtime_state()
            .expect("runtime state after reopen")
            .expect("present runtime state after reopen");
        assert!(runtime_state.last_clean_shutdown);
        assert!(runtime_state.last_recovery_check_unix > 0);
        assert!(!path::storage_commit_journal_path(Network::Regnet).exists());
    }
}
