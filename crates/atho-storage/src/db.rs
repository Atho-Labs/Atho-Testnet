use crate::error::StorageError;
use crate::path;
use crate::utxo::UtxoEntry;
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::rules::STORAGE_SCHEMA_VERSION;
use atho_core::network::Network;
use atho_core::transaction::Transaction as CoreTransaction;
use lmdb::{
    Cursor, Database as LmdbDatabase, DatabaseFlags, Environment, Error as LmdbError,
    RwTransaction, Transaction as LmdbTransaction, WriteFlags,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
#[cfg(test)]
use std::sync::OnceLock;

const INITIAL_MAP_SIZE: usize = 1 << 30;
const MAX_MAP_SIZE: usize = 1 << 40;
const MAX_DBS: u32 = 8;

const META_DB: &str = "meta";
const BLOCKS_DB: &str = "blocks";
const TRANSACTIONS_DB: &str = "transactions";
const UTXOS_DB: &str = "utxos";
const PEERS_DB: &str = "peers";
const ADDRESSES_DB: &str = "addresses";

const LEGACY_META_DIR: &str = "meta";
const LEGACY_BLOCKS_DIR: &str = "blocks";
const LEGACY_TRANSACTIONS_DIR: &str = "transactions";
const LEGACY_UTXOS_DIR: &str = "utxos";
const LEGACY_PEERS_DIR: &str = "peers";
const LEGACY_ADDRESSES_DIR: &str = "addresses";

const SNAPSHOT_KEY: &[u8; 10] = b"chainstate";
const SCHEMA_VERSION_KEY: &[u8; 14] = b"schema_version";
const SCHEMA_MIGRATION_LOG_KEY: &[u8; 20] = b"schema_migration_log";

#[cfg(test)]
static COMMIT_FAULT: OnceLock<Mutex<Option<CommitFault>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainstateSnapshot {
    pub height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub tip_hash: [u8; 48],
    pub tip_header: Option<BlockHeader>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockArchiveRecord {
    pub height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub block_hash: [u8; 48],
    pub block: Block,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRecord {
    pub network: Network,
    pub remote_addr: String,
    pub first_seen_height: u64,
    pub last_seen_height: u64,
    pub last_seen_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressRecord {
    pub network: Network,
    pub address: String,
    pub label: Option<String>,
    pub first_seen_height: u64,
    pub last_seen_height: u64,
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
    Transactions,
    Utxos,
    Peers,
    Addresses,
}

#[derive(Debug)]
pub struct Database {
    network: Network,
    path: PathBuf,
    state: Mutex<DatabaseState>,
}

#[derive(Debug)]
struct DatabaseState {
    env: Environment,
    map_size: usize,
    meta: LmdbDatabase,
    blocks: LmdbDatabase,
    transactions: LmdbDatabase,
    utxos: LmdbDatabase,
    peers: LmdbDatabase,
    addresses: LmdbDatabase,
}

impl Database {
    pub fn open(network: Network) -> Result<Self, StorageError> {
        let root = path::database_dir(network);
        fs::create_dir_all(&root)?;
        if legacy_layout_present(&root) {
            return Err(StorageError::LegacyStorageLayout);
        }
        let state = Self::open_state(&root, INITIAL_MAP_SIZE)?;
        let database = Self {
            network,
            path: root,
            state: Mutex::new(state),
        };
        database.ensure_schema_version()?;
        Ok(database)
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn load_chainstate_snapshot(&self) -> Result<Option<ChainstateSnapshot>, StorageError> {
        let snapshot_bytes = match self.get(Dataset::Meta, SNAPSHOT_KEY)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let snapshot: ChainstateSnapshot =
            bincode::deserialize(&snapshot_bytes).map_err(|_| StorageError::CorruptData)?;
        Ok(Some(snapshot))
    }

    pub fn load_block(&self, block_hash: [u8; 48]) -> Result<Option<Block>, StorageError> {
        match self.get(Dataset::Blocks, &block_hash)? {
            Some(bytes) => {
                let record: BlockArchiveRecord =
                    bincode::deserialize(&bytes).map_err(|_| StorageError::CorruptData)?;
                Ok(Some(record.block))
            }
            None => Ok(None),
        }
    }

    pub fn load_transaction(
        &self,
        txid: [u8; 48],
    ) -> Result<Option<TransactionArchiveRecord>, StorageError> {
        match self.get(Dataset::Transactions, &txid)? {
            Some(bytes) => {
                let record: TransactionArchiveRecord =
                    bincode::deserialize(&bytes).map_err(|_| StorageError::CorruptData)?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    pub fn load_utxos(&self) -> Result<Vec<UtxoEntry>, StorageError> {
        let entries = self.entries(Dataset::Utxos)?;
        let mut utxos = Vec::with_capacity(entries.len());
        for (_, value) in entries {
            let entry: UtxoEntry =
                bincode::deserialize(&value).map_err(|_| StorageError::CorruptData)?;
            utxos.push(entry);
        }
        Ok(utxos)
    }

    pub fn save_chainstate_snapshot(
        &self,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
    ) -> Result<(), StorageError> {
        self.commit_chainstate(snapshot, utxos, None)
    }

    pub fn append_block(&self, height: u64, block: &Block) -> Result<(), StorageError> {
        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            write_block_archive(&mut txn, state, height, block)?;
            txn.commit()?;
            Ok(())
        })
    }

    pub fn commit_chainstate(
        &self,
        snapshot: &ChainstateSnapshot,
        utxos: &[UtxoEntry],
        appended_block: Option<(u64, &Block)>,
    ) -> Result<(), StorageError> {
        // Consensus-visible persistence must move as one unit. The snapshot, canonical block
        // archive, transaction archive, and UTXO set are written in one LMDB transaction so a
        // crash cannot expose a mixed-height state.
        let snapshot_value = bincode::serialize(snapshot).map_err(|_| StorageError::CorruptData)?;
        let mut serialized_utxos = Vec::with_capacity(utxos.len());
        for utxo in utxos {
            let key = utxo_key(utxo.txid, utxo.output_index);
            let value = bincode::serialize(utxo).map_err(|_| StorageError::CorruptData)?;
            serialized_utxos.push((key, value));
        }

        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            if let Some((height, block)) = appended_block {
                write_block_archive(&mut txn, state, height, block)?;
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
            #[cfg(test)]
            maybe_inject_commit_fault(CommitFaultPoint::BeforeCommit)?;
            txn.commit()?;
            Ok(())
        })
    }

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

        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            clear_db(&mut txn, state.blocks)?;
            clear_db(&mut txn, state.transactions)?;
            clear_db(&mut txn, state.utxos)?;
            for block in blocks {
                write_block_archive(&mut txn, state, block.header.height, block)?;
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

    pub fn list_peers(&self) -> Result<Vec<PeerRecord>, StorageError> {
        let mut peers = Vec::new();
        for (_, value) in self.entries(Dataset::Peers)? {
            let record: PeerRecord =
                bincode::deserialize(&value).map_err(|_| StorageError::CorruptData)?;
            peers.push(record);
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
            let record: AddressRecord =
                bincode::deserialize(&value).map_err(|_| StorageError::CorruptData)?;
            addresses.push(record);
        }
        Ok(addresses)
    }

    fn ensure_schema_version(&self) -> Result<(), StorageError> {
        match self.get(Dataset::Meta, SCHEMA_VERSION_KEY)? {
            Some(bytes) => {
                let bytes: [u8; 4] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| StorageError::CorruptData)?;
                let found = u32::from_le_bytes(bytes);
                if found == STORAGE_SCHEMA_VERSION {
                    return Ok(());
                }
                if !self.try_migrate_schema(found)? {
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

    fn try_migrate_schema(&self, found: u32) -> Result<bool, StorageError> {
        match found {
            2 => {
                self.write_with_retry(|state| {
                    let mut txn = state.env.begin_rw_txn()?;
                    let migration_log = format!("{found}->{STORAGE_SCHEMA_VERSION}").into_bytes();
                    txn.put(
                        state.meta,
                        &SCHEMA_VERSION_KEY,
                        &STORAGE_SCHEMA_VERSION.to_le_bytes(),
                        WriteFlags::empty(),
                    )?;
                    txn.put(
                        state.meta,
                        &SCHEMA_MIGRATION_LOG_KEY,
                        &migration_log,
                        WriteFlags::empty(),
                    )?;
                    txn.commit()?;
                    Ok(())
                })?;
                Ok(true)
            }
            _ => Ok(false),
        }
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
        let transactions = env.create_db(Some(TRANSACTIONS_DB), DatabaseFlags::empty())?;
        let utxos = env.create_db(Some(UTXOS_DB), DatabaseFlags::empty())?;
        let peers = env.create_db(Some(PEERS_DB), DatabaseFlags::empty())?;
        let addresses = env.create_db(Some(ADDRESSES_DB), DatabaseFlags::empty())?;
        Ok(DatabaseState {
            env,
            map_size,
            meta,
            blocks,
            transactions,
            utxos,
            peers,
            addresses,
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
}

impl Dataset {
    fn db(self, state: &DatabaseState) -> LmdbDatabase {
        match self {
            Dataset::Meta => state.meta,
            Dataset::Blocks => state.blocks,
            Dataset::Transactions => state.transactions,
            Dataset::Utxos => state.utxos,
            Dataset::Peers => state.peers,
            Dataset::Addresses => state.addresses,
        }
    }
}

fn write_block_archive(
    txn: &mut RwTransaction<'_>,
    state: &DatabaseState,
    height: u64,
    block: &Block,
) -> Result<(), StorageError> {
    let block_hash = block.header.block_hash();
    let block_record = BlockArchiveRecord {
        height,
        block_hash,
        block: block.clone(),
    };
    let block_value = bincode::serialize(&block_record).map_err(|_| StorageError::CorruptData)?;
    txn.put(state.blocks, &block_hash, &block_value, WriteFlags::empty())?;

    for (tx_index, tx) in block.transactions.iter().enumerate() {
        let tx_record = TransactionArchiveRecord {
            height,
            block_hash,
            tx_index: tx_index as u32,
            txid: tx.txid(),
            transaction: tx.clone(),
        };
        let tx_value = bincode::serialize(&tx_record).map_err(|_| StorageError::CorruptData)?;
        txn.put(
            state.transactions,
            &tx_record.txid,
            &tx_value,
            WriteFlags::empty(),
        )?;
    }
    Ok(())
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
    [
        LEGACY_META_DIR,
        LEGACY_BLOCKS_DIR,
        LEGACY_TRANSACTIONS_DIR,
        LEGACY_UTXOS_DIR,
        LEGACY_PEERS_DIR,
        LEGACY_ADDRESSES_DIR,
    ]
    .iter()
    .any(|dataset| root.join(dataset).exists())
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
    use std::ffi::OsString;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &Path) -> Self {
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

    #[test]
    fn schema_version_two_migrates_forward_in_place() {
        let root = temp_data_dir("schema-migrate");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open current db");
        database
            .put(Dataset::Meta, SCHEMA_VERSION_KEY, &2u32.to_le_bytes())
            .expect("force schema v2");
        drop(database);

        let reopened = Database::open(Network::Regnet).expect("reopen migrated db");
        let version = reopened
            .get(Dataset::Meta, SCHEMA_VERSION_KEY)
            .expect("schema bytes")
            .expect("schema present");
        assert_eq!(
            u32::from_le_bytes(version.try_into().expect("u32 bytes")),
            3
        );
        let migration_log = reopened
            .get(Dataset::Meta, SCHEMA_MIGRATION_LOG_KEY)
            .expect("migration log")
            .expect("migration log present");
        assert_eq!(migration_log, b"2->3");
    }

    #[test]
    fn unsupported_schema_version_still_fails_closed() {
        let root = temp_data_dir("schema-reject");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let database = Database::open(Network::Regnet).expect("open current db");
        database
            .put(Dataset::Meta, SCHEMA_VERSION_KEY, &99u32.to_le_bytes())
            .expect("force unknown schema");
        drop(database);

        let err = Database::open(Network::Regnet).unwrap_err();
        assert!(matches!(
            err,
            StorageError::SchemaVersionMismatch {
                expected: STORAGE_SCHEMA_VERSION,
                found: 99
            }
        ));
    }
}
