use crate::error::StorageError;
use crate::utxo::UtxoEntry;
use atho_core::block::{Block, BlockHeader};
use atho_core::network::Network;
use atho_core::transaction::Transaction as CoreTransaction;
use lmdb::{
    Cursor, Database as LmdbDatabase, DatabaseFlags, Environment, Error as LmdbError,
    RwTransaction, Transaction as LmdbTransaction, WriteFlags,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const INITIAL_MAP_SIZE: usize = 1 << 30;
const MAX_MAP_SIZE: usize = 1 << 40;
const MAX_DBS: u32 = 4;
const MAIN_DB: &str = "main";

const META_DIR: &str = "meta";
const BLOCKS_DIR: &str = "blocks";
const TRANSACTIONS_DIR: &str = "transactions";
const UTXOS_DIR: &str = "utxos";
const PEERS_DIR: &str = "peers";
const ADDRESSES_DIR: &str = "addresses";

const SNAPSHOT_KEY: &[u8; 10] = b"chainstate";

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

#[derive(Debug)]
pub struct Database {
    network: Network,
    meta: LmdbCollection,
    blocks: LmdbCollection,
    transactions: LmdbCollection,
    utxos: LmdbCollection,
    peers: LmdbCollection,
    addresses: LmdbCollection,
}

#[derive(Debug)]
struct LmdbCollection {
    path: PathBuf,
    state: Mutex<LmdbCollectionState>,
}

#[derive(Debug)]
struct LmdbCollectionState {
    env: Environment,
    db: LmdbDatabase,
    map_size: usize,
}

impl Database {
    pub fn open(network: Network) -> Result<Self, StorageError> {
        let root = database_dir(network);
        fs::create_dir_all(&root)?;
        Ok(Self {
            network,
            meta: LmdbCollection::open(dataset_dir(&root, META_DIR))?,
            blocks: LmdbCollection::open(dataset_dir(&root, BLOCKS_DIR))?,
            transactions: LmdbCollection::open(dataset_dir(&root, TRANSACTIONS_DIR))?,
            utxos: LmdbCollection::open(dataset_dir(&root, UTXOS_DIR))?,
            peers: LmdbCollection::open(dataset_dir(&root, PEERS_DIR))?,
            addresses: LmdbCollection::open(dataset_dir(&root, ADDRESSES_DIR))?,
        })
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn load_chainstate_snapshot(&self) -> Result<Option<ChainstateSnapshot>, StorageError> {
        let snapshot_bytes = match self.meta.get(SNAPSHOT_KEY)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let snapshot: ChainstateSnapshot =
            bincode::deserialize(&snapshot_bytes).map_err(|_| StorageError::CorruptData)?;
        Ok(Some(snapshot))
    }

    pub fn load_block(&self, block_hash: [u8; 48]) -> Result<Option<Block>, StorageError> {
        match self.blocks.get(&block_hash)? {
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
        match self.transactions.get(&txid)? {
            Some(bytes) => {
                let record: TransactionArchiveRecord =
                    bincode::deserialize(&bytes).map_err(|_| StorageError::CorruptData)?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    pub fn load_utxos(&self) -> Result<Vec<UtxoEntry>, StorageError> {
        let entries = self.utxos.entries()?;
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
        let payload = bincode::serialize(snapshot).map_err(|_| StorageError::CorruptData)?;
        self.meta.put(SNAPSHOT_KEY, &payload)?;

        let mut serialized_utxos = Vec::with_capacity(utxos.len());
        for utxo in utxos {
            let key = utxo_key(utxo.txid, utxo.output_index);
            let value = bincode::serialize(utxo).map_err(|_| StorageError::CorruptData)?;
            serialized_utxos.push((key, value));
        }
        self.utxos.replace_all(serialized_utxos)
    }

    pub fn append_block(&self, height: u64, block: &Block) -> Result<(), StorageError> {
        let block_hash = block.header.block_hash();
        let block_record = BlockArchiveRecord {
            height,
            block_hash,
            block: block.clone(),
        };
        let block_value =
            bincode::serialize(&block_record).map_err(|_| StorageError::CorruptData)?;
        self.blocks.put(&block_hash, &block_value)?;

        for (tx_index, tx) in block.transactions.iter().enumerate() {
            let tx_record = TransactionArchiveRecord {
                height,
                block_hash,
                tx_index: tx_index as u32,
                txid: tx.txid(),
                transaction: tx.clone(),
            };
            let tx_value = bincode::serialize(&tx_record).map_err(|_| StorageError::CorruptData)?;
            self.transactions.put(&tx_record.txid, &tx_value)?;
        }
        Ok(())
    }

    pub fn upsert_peer(&self, record: &PeerRecord) -> Result<(), StorageError> {
        let key = record.remote_addr.as_bytes().to_vec();
        let value = bincode::serialize(record).map_err(|_| StorageError::CorruptData)?;
        self.peers.put(&key, &value)
    }

    pub fn list_peers(&self) -> Result<Vec<PeerRecord>, StorageError> {
        let mut peers = Vec::new();
        for (_, value) in self.peers.entries()? {
            let record: PeerRecord =
                bincode::deserialize(&value).map_err(|_| StorageError::CorruptData)?;
            peers.push(record);
        }
        Ok(peers)
    }

    pub fn upsert_address(&self, record: &AddressRecord) -> Result<(), StorageError> {
        let key = record.address.as_bytes().to_vec();
        let value = bincode::serialize(record).map_err(|_| StorageError::CorruptData)?;
        self.addresses.put(&key, &value)
    }

    pub fn list_addresses(&self) -> Result<Vec<AddressRecord>, StorageError> {
        let mut addresses = Vec::new();
        for (_, value) in self.addresses.entries()? {
            let record: AddressRecord =
                bincode::deserialize(&value).map_err(|_| StorageError::CorruptData)?;
            addresses.push(record);
        }
        Ok(addresses)
    }
}

impl LmdbCollection {
    fn open(path: PathBuf) -> Result<Self, StorageError> {
        fs::create_dir_all(&path)?;
        let state = Self::open_state(&path, INITIAL_MAP_SIZE)?;
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let state = self.state.lock().expect("database lock poisoned");
        let txn = state.env.begin_ro_txn()?;
        match txn.get(state.db, &key) {
            Ok(bytes) => Ok(Some(bytes.to_vec())),
            Err(LmdbError::NotFound) => Ok(None),
            Err(err) => Err(StorageError::Lmdb(err)),
        }
    }

    fn entries(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
        let state = self.state.lock().expect("database lock poisoned");
        let txn = state.env.begin_ro_txn()?;
        let mut cursor = txn.open_ro_cursor(state.db)?;
        let mut entries = Vec::new();
        for (key, value) in cursor.iter() {
            entries.push((key.to_vec(), value.to_vec()));
        }
        Ok(entries)
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            txn.put(state.db, &key, &value, WriteFlags::empty())?;
            txn.commit()?;
            Ok(())
        })
    }

    fn replace_all(&self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), StorageError> {
        self.write_with_retry(|state| {
            let mut txn = state.env.begin_rw_txn()?;
            clear_db(&mut txn, state.db)?;
            for (key, value) in &entries {
                txn.put(
                    state.db,
                    &key.as_slice(),
                    &value.as_slice(),
                    WriteFlags::empty(),
                )?;
            }
            txn.commit()?;
            Ok(())
        })
    }

    fn write_with_retry<T, F>(&self, mut op: F) -> Result<T, StorageError>
    where
        F: FnMut(&mut LmdbCollectionState) -> Result<T, StorageError>,
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

    fn open_state(path: &PathBuf, map_size: usize) -> Result<LmdbCollectionState, StorageError> {
        let mut builder = Environment::new();
        builder
            .set_max_readers(128)
            .set_max_dbs(MAX_DBS)
            .set_map_size(map_size);
        let env = builder.open(path)?;
        let db = env.create_db(Some(MAIN_DB), DatabaseFlags::empty())?;
        Ok(LmdbCollectionState { env, db, map_size })
    }
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

fn database_dir(network: Network) -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("dev")
        .join("db")
        .join(network.id())
}

fn dataset_dir(root: &PathBuf, dataset: &str) -> PathBuf {
    root.join(dataset)
}

fn utxo_key(txid: [u8; 48], output_index: u32) -> Vec<u8> {
    let mut key = Vec::with_capacity(52);
    key.extend_from_slice(&txid);
    key.extend_from_slice(&output_index.to_be_bytes());
    key
}
