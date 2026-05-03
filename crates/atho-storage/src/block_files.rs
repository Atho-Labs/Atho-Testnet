//! Bitcoin-style flat block file storage for canonical raw Atho blocks.
//!
//! Raw block payloads are stored as append-only records:
//! `[network magic][payload length][canonical block bytes]`.
//!
//! STORAGE: These files are the archival source of truth for full raw block
//! bytes. Indexed lookup, chain metadata, and UTXO state remain in LMDB.

use crate::error::StorageError;
use crate::path;
use atho_core::block::Block;
use atho_core::constants::{
    BLOCK_FILE_RECORD_OVERHEAD_BYTES, BLOCK_FILE_ROTATE_BYTES, MAX_BLOCK_SERIALIZED_BYTES,
};
use atho_core::network::Network;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
static ROTATION_OVERRIDE_BYTES: OnceLock<Mutex<Option<u64>>> = OnceLock::new();

/// LMDB-persisted location of one raw block record within the flat-file archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockFileLocation {
    pub file_number: u64,
    pub record_offset: u64,
    pub payload_length: u32,
}

impl BlockFileLocation {
    pub fn payload_offset(self) -> u64 {
        self.record_offset + BLOCK_FILE_RECORD_OVERHEAD_BYTES
    }

    pub fn record_length(self) -> u64 {
        BLOCK_FILE_RECORD_OVERHEAD_BYTES + self.payload_length as u64
    }
}

/// Flat-file archive manager for one network's raw block history.
#[derive(Debug, Clone)]
pub struct BlockFileStore {
    network: Network,
    root: PathBuf,
}

impl BlockFileStore {
    /// Opens the block archive for the selected network and repairs any
    /// partially written tail record in the active file.
    pub fn open(network: Network) -> Result<Self, StorageError> {
        let root = path::block_storage_dir(network);
        fs::create_dir_all(&root)?;
        let store = Self { network, root };
        store.repair_last_file_tail()?;
        Ok(store)
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn append_block(&self, block: &Block) -> Result<BlockFileLocation, StorageError> {
        self.append_block_with_minimum_file_number(block, None)
    }

    pub fn append_block_with_minimum_file_number(
        &self,
        block: &Block,
        minimum_file_number: Option<u64>,
    ) -> Result<BlockFileLocation, StorageError> {
        // Recreate the archive path on every append. A running node can keep
        // LMDB handles alive even after an operator deletes the network root,
        // but raw block persistence must not fail just because the archive
        // directory vanished from the filesystem namespace.
        fs::create_dir_all(&self.root)?;

        let payload = block.canonical_bytes();
        let payload_length = u32::try_from(payload.len()).map_err(|_| StorageError::CorruptData)?;
        if payload_length == 0 || payload.len() > MAX_BLOCK_SERIALIZED_BYTES {
            return Err(StorageError::CorruptData);
        }

        let record_length = BLOCK_FILE_RECORD_OVERHEAD_BYTES + payload.len() as u64;
        let rotation_bytes = rotation_bytes();
        let minimum_file_number = minimum_file_number.unwrap_or(0);
        let mut file_number = self
            .highest_file_number()?
            .unwrap_or(minimum_file_number)
            .max(minimum_file_number);
        let mut record_offset = match self.file_path(file_number).metadata() {
            Ok(metadata) => metadata.len(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
            Err(err) => return Err(StorageError::Io(err)),
        };
        if record_offset > 0 && record_offset.saturating_add(record_length) > rotation_bytes {
            file_number = file_number.saturating_add(1);
            record_offset = 0;
        }

        let path = self.file_path(file_number);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        if record_offset != file.metadata()?.len() {
            record_offset = file.metadata()?.len();
        }
        if record_offset > 0 && record_offset.saturating_add(record_length) > rotation_bytes {
            drop(file);
            file_number = file_number.saturating_add(1);
            record_offset = 0;
            let mut rotated = OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .open(self.file_path(file_number))?;
            self.write_record(&mut rotated, payload_length, &payload)?;
            return Ok(BlockFileLocation {
                file_number,
                record_offset,
                payload_length,
            });
        }

        self.write_record(&mut file, payload_length, &payload)?;
        Ok(BlockFileLocation {
            file_number,
            record_offset,
            payload_length,
        })
    }

    pub fn read_block(&self, location: BlockFileLocation) -> Result<Block, StorageError> {
        let mut file = File::open(self.file_path(location.file_number))?;
        file.seek(SeekFrom::Start(location.record_offset))?;
        let mut wrapper = [0u8; 8];
        file.read_exact(&mut wrapper)?;
        let magic: [u8; 4] = wrapper[..4].try_into().expect("wrapper magic");
        if magic != self.network.p2p_magic() {
            return Err(StorageError::CrossNetworkReplay);
        }
        let payload_length = u32::from_le_bytes(wrapper[4..8].try_into().expect("wrapper len"));
        if payload_length != location.payload_length
            || payload_length as usize > MAX_BLOCK_SERIALIZED_BYTES
        {
            return Err(StorageError::CorruptData);
        }
        let mut payload = vec![0u8; payload_length as usize];
        file.read_exact(&mut payload)?;
        let block = Block::from_canonical_bytes(&payload).ok_or(StorageError::CorruptData)?;
        if block.header.network_id != self.network {
            return Err(StorageError::CrossNetworkReplay);
        }
        Ok(block)
    }

    pub fn delete_file(&self, file_number: u64) -> Result<(), StorageError> {
        let path = self.file_path(file_number);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StorageError::Io(err)),
        }
    }

    pub fn reset(&self) -> Result<(), StorageError> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    pub fn file_path(&self, file_number: u64) -> PathBuf {
        self.root.join(format!("blk{file_number:05}.dat"))
    }

    pub fn highest_file_number(&self) -> Result<Option<u64>, StorageError> {
        let mut highest = None;
        if !self.root.exists() {
            return Ok(None);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Some(number) = parse_file_number(&name) {
                highest = Some(highest.map_or(number, |current: u64| current.max(number)));
            }
        }
        Ok(highest)
    }

    pub fn existing_file_numbers(&self) -> Result<Vec<u64>, StorageError> {
        let mut files = Vec::new();
        if !self.root.exists() {
            return Ok(files);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Some(number) = parse_file_number(&name) {
                files.push(number);
            }
        }
        files.sort_unstable();
        Ok(files)
    }

    fn repair_last_file_tail(&self) -> Result<(), StorageError> {
        let Some(file_number) = self.highest_file_number()? else {
            return Ok(());
        };
        let path = self.file_path(file_number);
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let file_len = file.metadata()?.len();
        let mut offset = 0u64;
        let mut last_good = 0u64;

        while offset < file_len {
            let remaining = file_len.saturating_sub(offset);
            if remaining < BLOCK_FILE_RECORD_OVERHEAD_BYTES {
                file.set_len(last_good)?;
                break;
            }

            file.seek(SeekFrom::Start(offset))?;
            let mut wrapper = [0u8; 8];
            file.read_exact(&mut wrapper)?;
            let magic: [u8; 4] = wrapper[..4].try_into().expect("wrapper magic");
            let payload_length = u32::from_le_bytes(wrapper[4..8].try_into().expect("wrapper len"));
            let payload_length = payload_length as u64;
            if magic != self.network.p2p_magic()
                || payload_length == 0
                || payload_length > MAX_BLOCK_SERIALIZED_BYTES as u64
                || remaining < BLOCK_FILE_RECORD_OVERHEAD_BYTES + payload_length
            {
                file.set_len(last_good)?;
                break;
            }

            let mut payload = vec![0u8; payload_length as usize];
            file.read_exact(&mut payload)?;
            let Some(block) = Block::from_canonical_bytes(&payload) else {
                file.set_len(last_good)?;
                break;
            };
            if block.header.network_id != self.network {
                file.set_len(last_good)?;
                break;
            }
            last_good = offset + BLOCK_FILE_RECORD_OVERHEAD_BYTES + payload_length;
            offset = last_good;
        }
        Ok(())
    }

    fn write_record(
        &self,
        file: &mut File,
        payload_length: u32,
        payload: &[u8],
    ) -> Result<(), StorageError> {
        file.write_all(&self.network.p2p_magic())?;
        file.write_all(&payload_length.to_le_bytes())?;
        file.write_all(payload)?;
        file.sync_data()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn set_rotation_override_for_test(bytes: Option<u64>) {
        let override_bytes = ROTATION_OVERRIDE_BYTES.get_or_init(|| Mutex::new(None));
        *override_bytes
            .lock()
            .expect("rotation override mutex poisoned") = bytes;
    }
}

fn parse_file_number(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("blk")?.strip_suffix(".dat")?;
    digits.parse::<u64>().ok()
}

fn rotation_bytes() -> u64 {
    #[cfg(test)]
    {
        if let Some(override_bytes) = ROTATION_OVERRIDE_BYTES.get() {
            if let Some(bytes) = *override_bytes
                .lock()
                .expect("rotation override mutex poisoned")
            {
                return bytes;
            }
        }
    }
    BLOCK_FILE_ROTATE_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::acquire_global_test_lock;
    use atho_core::block::{merkle_root, witness_root, BlockHeader};
    use atho_core::transaction::{Transaction, TxInput, TxOutput};
    use std::ffi::OsString;
    use std::sync::MutexGuard;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
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
            BlockFileStore::set_rotation_override_for_test(None);
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atho-block-files-{label}-{}-{}",
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
    fn appends_and_reads_block_records() {
        let root = temp_root("roundtrip");
        let _guard = EnvVarGuard::set_path(path::ATHO_DATA_DIR_ENV, &root);
        let store = BlockFileStore::open(Network::Regnet).expect("open store");
        let block = sample_block(Network::Regnet, 1, [0; 48]);
        let location = store.append_block(&block).expect("append block");
        let loaded = store.read_block(location).expect("read block");
        assert_eq!(loaded.header, block.header);
        assert_eq!(loaded.transactions, block.transactions);
    }

    #[test]
    fn rotates_files_when_size_limit_is_reached() {
        let root = temp_root("rotate");
        let _guard = EnvVarGuard::set_path(path::ATHO_DATA_DIR_ENV, &root);
        BlockFileStore::set_rotation_override_for_test(Some(512));
        let store = BlockFileStore::open(Network::Regnet).expect("open store");
        let mut previous = [0; 48];
        let mut locations = Vec::new();
        for height in 1..6 {
            let block = sample_block(Network::Regnet, height, previous);
            previous = block.header.block_hash();
            locations.push(store.append_block(&block).expect("append block"));
        }
        assert!(locations.iter().any(|location| location.file_number > 0));
        assert!(store.file_path(0).exists());
        assert!(store.file_path(1).exists());
    }

    #[test]
    fn truncates_incomplete_tail_record_on_reopen() {
        let root = temp_root("truncate-tail");
        let _guard = EnvVarGuard::set_path(path::ATHO_DATA_DIR_ENV, &root);
        let store = BlockFileStore::open(Network::Regnet).expect("open store");
        let block = sample_block(Network::Regnet, 1, [0; 48]);
        let location = store.append_block(&block).expect("append block");
        let path = store.file_path(location.file_number);
        let original_len = path.metadata().expect("metadata").len();
        let mut file = OpenOptions::new().append(true).open(&path).expect("append");
        file.write_all(&[0u8; 3]).expect("write torn tail");
        drop(file);

        let repaired = BlockFileStore::open(Network::Regnet).expect("reopen store");
        assert_eq!(path.metadata().expect("metadata").len(), original_len);
        let loaded = repaired.read_block(location).expect("read block");
        assert_eq!(loaded.header.block_hash(), block.header.block_hash());
    }

    #[test]
    fn append_recreates_missing_archive_dir_after_external_deletion() {
        let root = temp_root("recreate-archive-dir");
        let _guard = EnvVarGuard::set_path(path::ATHO_DATA_DIR_ENV, &root);
        let store = BlockFileStore::open(Network::Regnet).expect("open store");
        let first = sample_block(Network::Regnet, 1, [0; 48]);
        store.append_block(&first).expect("append first block");

        fs::remove_dir_all(store.root()).expect("remove archive dir");
        assert!(!store.root().exists());

        let second = sample_block(Network::Regnet, 2, first.header.block_hash());
        let location = store
            .append_block_with_minimum_file_number(&second, Some(1))
            .expect("append after directory recreation");
        assert!(store.root().exists());
        assert_eq!(location.file_number, 1);

        let loaded = store.read_block(location).expect("read recreated block");
        assert_eq!(loaded.header, second.header);
        assert_eq!(loaded.transactions, second.transactions);
    }

    #[test]
    fn rejects_wrong_network_magic_when_reading_archived_block() {
        let root = temp_root("wrong-magic");
        let _guard = EnvVarGuard::set_path(path::ATHO_DATA_DIR_ENV, &root);
        let regnet_store = BlockFileStore::open(Network::Regnet).expect("open regnet store");
        let block = sample_block(Network::Regnet, 1, [0; 48]);
        let location = regnet_store.append_block(&block).expect("append block");

        let mainnet_view = BlockFileStore {
            network: Network::Mainnet,
            root: regnet_store.root.clone(),
        };
        let err = mainnet_view.read_block(location).unwrap_err();
        assert!(matches!(err, StorageError::CrossNetworkReplay));
    }
}
