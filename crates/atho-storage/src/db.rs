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

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFaultPoint {
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
        let state = Self::open_state(&root, INITIAL_MAP_SIZE)?;
        let database = Self {
            network,
            path: root,
            block_store,
            state: Mutex::new(state),
        };
        database.ensure_schema_version()?;
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
                        return Err(StorageError::CorruptData);
                    }
                    block.fees_total_atoms = record.fees_total_atoms;
                    block.fees_miner_atoms = record.fees_miner_atoms;
                    return Ok(Some(block));
                }
                Err(StorageError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
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

        let mut prune_files: Vec<(u64, Vec<BlockArchiveRecord>)> = Vec::new();
        for (file_number, file_records) in files {
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

            report.pruned_blocks += pending_prune.len();
            report.reclaimed_bytes += pending_prune
                .iter()
                .map(|record| record.file_location().record_length())
                .sum::<u64>();
            prune_files.push((file_number, pending_prune));
        }

        if prune_files.is_empty() {
            return Ok(report);
        }

        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            for (_, file_records) in &prune_files {
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

        for (file_number, _) in &prune_files {
            self.block_store.delete_file(*file_number)?;
            report.pruned_files.push(*file_number);
        }
        Ok(report)
    }

    /// Commits the tip snapshot, UTXO set, and optional appended block together.
    ///
    /// STORAGE: This transaction must remain atomic. Writing the tip without the
    /// matching UTXO image would let the node restart into a corrupt state.
    pub fn commit_chainstate(
        &self,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
        appended_block: Option<(u64, &Block)>,
    ) -> Result<(), StorageError> {
        let snapshot_value = bincode::serialize(snapshot).map_err(|_| StorageError::CorruptData)?;
        let mut serialized_utxos = Vec::with_capacity(utxos.len());
        for utxo in utxos {
            let key = utxo_key(utxo.txid, utxo.output_index);
            let value = bincode::serialize(utxo).map_err(|_| StorageError::CorruptData)?;
            serialized_utxos.push((key, value));
        }

        let appended = if let Some((height, block)) = appended_block {
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
            if let Some((height, block, location)) = appended {
                write_block_archive(&mut txn, state, self.network, height, block, location)?;
            }
            txn.put(
                state.meta,
                &SNAPSHOT_KEY,
                &snapshot_value,
                WriteFlags::empty(),
            )?;
            clear_db(&mut txn, state.utxos)?;
            for (key, value) in &serialized_utxos {
                txn.put(
                    state.utxos,
                    &key.as_slice(),
                    &value.as_slice(),
                    WriteFlags::empty(),
                )?;
            }
            if appended.is_none() {
                rebuild_height_index(&mut txn, state, snapshot.tip_hash)?;
            }
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::BeforeCommit)?;
            txn.commit()?;
            Ok(())
        })
    }

    /// Replaces the canonical chainstate image and rebuilds the raw archive from
    /// the supplied canonical block list.
    pub fn replace_chainstate(
        &self,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
        blocks: &[Block],
    ) -> Result<(), StorageError> {
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
            for (key, value) in &serialized_utxos {
                txn.put(
                    state.utxos,
                    &key.as_slice(),
                    &value.as_slice(),
                    WriteFlags::empty(),
                )?;
            }
            txn.commit()?;
            Ok(())
        })
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

    fn get(&self, dataset: Dataset, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let state = self.state.lock().expect("database lock poisoned");
        let txn = state.env.begin_ro_txn()?;
        match txn.get(dataset.db(&state), &key) {
            Ok(bytes) => Ok(Some(bytes.to_vec())),
            Err(LmdbError::NotFound) => Ok(None),
            Err(err) => Err(StorageError::Lmdb(err)),
        }
    }

    fn entries(&self, dataset: Dataset) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
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
        raw_block_size: block.canonical_bytes().len() as u32,
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
    Err(StorageError::Io(std::io::Error::other(
        "fault injected before LMDB commit",
    )))
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
            inputs: vec![TxInput {
                previous_txid: [height as u8; 48],
                output_index: 0,
                unlocking_script: vec![height as u8, 1, 2],
            }],
            outputs: vec![TxOutput {
                value_atoms: 5_000_000_000,
                locking_script: vec![3, 4, height as u8],
            }],
            lock_time: height as u32,
            witness: vec![],
        };
        let header = BlockHeader {
            version: 1,
            network_id: network,
            height,
            previous_block_hash,
            merkle_root: merkle_root(&[tx.clone()]),
            witness_root: witness_root(&[tx.clone()]),
            timestamp: 1_700_000_000 + height,
            difficulty_target_or_bits: [7; 48],
            nonce: 42 + height,
        };
        Block::new(header, vec![tx])
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
}
