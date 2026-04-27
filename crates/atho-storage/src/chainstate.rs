use crate::db::{ChainstateSnapshot, Database};
use crate::error::StorageError;
use crate::utxo::{BlockUndo, UtxoEntry, UtxoSet};
use atho_core::address::internal_hpk_bytes;
use atho_core::block::{Block, BlockHeader};
use atho_core::constants::{GENESIS_COINBASE_ATOMS, PRUNE_DEPTH_BLOCKS};
use atho_core::genesis;
use atho_core::network::Network;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

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
        let storage = Database::open(network).ok();
        if let Some(storage) = storage {
            if let Some(persisted) = load_persisted_chainstate(network, &storage) {
                let chainstate = persisted.into_chainstate(network, Some(storage));
                let _ = chainstate.save_persisted_chainstate();
                return chainstate;
            }

            let chainstate = Self::fresh_with_storage(network, Some(storage));
            if let Some(genesis_block) = chainstate.blocks.first().cloned() {
                if let Some(storage) = chainstate.storage.as_ref() {
                    let _ = storage.append_block(0, &genesis_block);
                }
            }
            let _ = chainstate.save_persisted_chainstate();
            return chainstate;
        }

        Self::fresh(network)
    }

    pub fn connect_header(&mut self, header: BlockHeader) {
        self.tip_hash = header.block_hash();
        self.tip = Some(header);
        self.height = self.tip.as_ref().map(|header| header.height).unwrap_or(0);
    }

    pub fn connect_block(&mut self, block: &Block) -> Result<(), StorageError> {
        let undo = self.utxos.apply_block(block)?;
        let previous_tip = self.tip.clone();
        let previous_tip_hash = self.tip_hash;
        self.tip = Some(block.header.clone());
        self.tip_hash = block.header.block_hash();
        self.height = block.header.height;
        self.blocks.push(block.clone());
        self.undo_stack.push(ChainUndo {
            previous_tip,
            previous_tip_hash,
            block_undo: undo,
        });
        self.prune_history();
        if let Some(storage) = &self.storage {
            storage.append_block(self.height, block)?;
        }
        self.save_persisted_chainstate()?;
        Ok(())
    }

    pub fn utxo_snapshot(&self) -> UtxoSet {
        self.utxos.clone()
    }

    pub fn utxo_entry(&self, txid: [u8; 48], output_index: u32) -> Option<UtxoEntry> {
        self.utxos.get(txid, output_index).cloned()
    }

    pub fn disconnect_last_block(&mut self) -> Result<(), StorageError> {
        let undo = self
            .undo_stack
            .pop()
            .ok_or(StorageError::NoBlockToDisconnect)?;
        self.utxos.disconnect_block(undo.block_undo);
        let _ = self.blocks.pop();
        self.tip = undo.previous_tip;
        self.tip_hash = undo.previous_tip_hash;
        self.height = self.tip.as_ref().map(|header| header.height).unwrap_or(0);
        self.save_persisted_chainstate()?;
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
        if let Some(storage) = &self.storage {
            let snapshot = ChainstateSnapshot {
                height: self.height,
                tip_hash: self.tip_hash,
                tip_header: self.tip.clone(),
            };
            let utxos: Vec<_> = self.utxos.entries().cloned().collect();
            storage.save_chainstate_snapshot(&snapshot, &utxos)?;
        }
        Ok(())
    }
}

impl PersistedChainstate {
    fn into_chainstate(self, network: Network, storage: Option<Database>) -> Chainstate {
        let genesis = genesis::genesis_state(network);
        let genesis_block = genesis.block;
        let mut utxos = UtxoSet::new(network);
        utxos
            .insert(UtxoEntry::coinbase(
                network,
                genesis.coinbase_txid,
                0,
                GENESIS_COINBASE_ATOMS,
                internal_hpk_bytes(network, &genesis.reward_address)
                    .unwrap_or_else(|| genesis.reward_address.as_bytes().to_vec()),
                0,
            ))
            .expect("genesis utxo is network-local and unique");
        for entry in self.utxos {
            let _ = utxos.insert(entry);
        }

        let tip = match self.tip_header {
            Some(header) => Some(header),
            None => storage
                .as_ref()
                .and_then(|db| db.load_block(self.tip_hash).ok().flatten())
                .map(|block| block.header),
        };

        Chainstate {
            network,
            tip,
            tip_hash: self.tip_hash,
            height: self.height,
            blocks: vec![genesis_block],
            utxos,
            undo_stack: Vec::new(),
            storage,
        }
    }
}

fn chain_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("dev")
        .join("chain")
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

fn load_persisted_chainstate(network: Network, storage: &Database) -> Option<PersistedChainstate> {
    if let Ok(Some(snapshot)) = storage.load_chainstate_snapshot() {
        let utxos = storage.load_utxos().ok()?;
        return Some(PersistedChainstate {
            height: snapshot.height,
            tip_hash: snapshot.tip_hash,
            tip_header: snapshot.tip_header,
            utxos,
        });
    }

    if let Some(persisted) = load_snapshot_files(network) {
        return Some(persisted);
    }
    replay_legacy_chain_logs(network).ok().flatten()
}

fn load_snapshot_files(network: Network) -> Option<PersistedChainstate> {
    let state_path = chainstate_snapshot_path(network);
    let utxo_path = utxo_snapshot_path(network);
    let state_file = File::open(state_path).ok()?;
    let utxo_file = File::open(utxo_path).ok()?;

    let mut state_reader = BufReader::new(state_file);
    let mut state_line = String::new();
    if state_reader.read_line(&mut state_line).ok()? == 0 {
        return None;
    }
    let state_line = state_line.trim();
    if state_line.is_empty() || state_line.starts_with("height\t") {
        return None;
    }
    let mut fields = state_line.split('\t');
    let height = fields.next()?.parse().ok()?;
    let tip_hash = hex::decode(fields.next()?).ok()?.try_into().ok()?;

    let mut utxos = Vec::new();
    let reader = BufReader::new(utxo_file);
    for line in reader.lines() {
        let line = line.ok()?;
        if line.starts_with("txid\t") || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let txid: [u8; 48] = hex::decode(fields.next()?).ok()?.try_into().ok()?;
        let output_index = fields.next()?.parse().ok()?;
        let value_atoms = fields.next()?.parse().ok()?;
        let locking_script = hex::decode(fields.next()?).ok()?;
        let created_height = fields.next()?.parse().ok()?;
        let is_coinbase = fields.next()?.parse::<u8>().ok()? != 0;
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

    Some(PersistedChainstate {
        height,
        tip_hash,
        tip_header: None,
        utxos,
    })
}

fn replay_legacy_chain_logs(network: Network) -> std::io::Result<Option<PersistedChainstate>> {
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

fn load_block_rows() -> std::io::Result<BTreeMap<u64, BlockRecord>> {
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
        let height = match fields.next().and_then(|value| value.parse().ok()) {
            Some(height) => height,
            None => continue,
        };
        let block_hash = match fields.next().and_then(parse_hex::<48>) {
            Some(hash) => hash,
            None => continue,
        };
        rows.insert(height, BlockRecord { height, block_hash });
    }
    Ok(rows)
}

fn load_tx_rows() -> std::io::Result<BTreeMap<(u64, [u8; 48], u32), TxRow>> {
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
        let height = match fields.next().and_then(|value| value.parse().ok()) {
            Some(height) => height,
            None => continue,
        };
        let block_hash = match fields.next().and_then(parse_hex::<48>) {
            Some(hash) => hash,
            None => continue,
        };
        let tx_index = match fields.next().and_then(|value| value.parse().ok()) {
            Some(tx_index) => tx_index,
            None => continue,
        };
        let txid = match fields.next().and_then(parse_hex::<48>) {
            Some(txid) => txid,
            None => continue,
        };
        let _wtxid = fields.next();
        let _version = fields.next();
        let _lock_time = fields.next();
        let input_count = match fields.next().and_then(|value| value.parse().ok()) {
            Some(count) => count,
            None => continue,
        };
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

fn load_input_rows() -> std::io::Result<BTreeMap<(u64, [u8; 48], u32), Vec<InputRow>>> {
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
        let height = match fields.next().and_then(|value| value.parse().ok()) {
            Some(height) => height,
            None => continue,
        };
        let block_hash = match fields.next().and_then(parse_hex::<48>) {
            Some(hash) => hash,
            None => continue,
        };
        let tx_index = match fields.next().and_then(|value| value.parse().ok()) {
            Some(tx_index) => tx_index,
            None => continue,
        };
        let _input_index = fields.next();
        let previous_txid = match fields.next().and_then(parse_hex::<48>) {
            Some(txid) => txid,
            None => continue,
        };
        let output_index = match fields.next().and_then(|value| value.parse().ok()) {
            Some(output_index) => output_index,
            None => continue,
        };
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

fn load_output_rows() -> std::io::Result<BTreeMap<(u64, [u8; 48], u32), Vec<OutputRow>>> {
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
        let height = match fields.next().and_then(|value| value.parse().ok()) {
            Some(height) => height,
            None => continue,
        };
        let block_hash = match fields.next().and_then(parse_hex::<48>) {
            Some(hash) => hash,
            None => continue,
        };
        let tx_index = match fields.next().and_then(|value| value.parse().ok()) {
            Some(tx_index) => tx_index,
            None => continue,
        };
        let _output_index = fields.next();
        let value_atoms = match fields.next().and_then(|value| value.parse().ok()) {
            Some(value_atoms) => value_atoms,
            None => continue,
        };
        let locking_script = match fields.next().and_then(|value| hex::decode(value).ok()) {
            Some(locking_script) => locking_script,
            None => continue,
        };
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
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::crypto::hash::sha3_256;
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};

    fn derive_sig_ref_short(txid: &[u8; 48], signature: &[u8], input_index: u32) -> [u8; 2] {
        let mut preimage = Vec::with_capacity(
            b"ATHO_SIG_REF_SHORT_V1".len()
                + txid.len()
                + signature.len()
                + core::mem::size_of::<u32>(),
        );
        preimage.extend_from_slice(b"ATHO_SIG_REF_SHORT_V1");
        preimage.extend_from_slice(txid);
        preimage.extend_from_slice(signature);
        preimage.extend_from_slice(&input_index.to_be_bytes());
        let digest = sha3_256(&preimage);
        [digest[0], digest[1]]
    }

    fn derive_witness_commit_ref(
        txid: &[u8; 48],
        witness_root: &[u8; 48],
        input_index: u32,
    ) -> [u8; 16] {
        let mut preimage = Vec::with_capacity(
            b"ATHO_WITNESS_COMMIT_REF_V1".len()
                + txid.len()
                + core::mem::size_of::<u32>()
                + witness_root.len(),
        );
        preimage.extend_from_slice(b"ATHO_WITNESS_COMMIT_REF_V1");
        preimage.extend_from_slice(txid);
        preimage.extend_from_slice(&input_index.to_be_bytes());
        preimage.extend_from_slice(witness_root);
        let digest = sha3_256(&preimage);
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest[..16]);
        out
    }

    fn witness_bytes_for_tx(tx: &Transaction) -> Vec<u8> {
        let signature = vec![9, 9, 9];
        let pubkey = vec![8, 8, 8];
        let txid = tx.txid();
        let staged = TxWitness {
            signature: signature.clone(),
            pubkey: pubkey.clone(),
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        };
        let staged_tx = Transaction {
            witness: staged.canonical_bytes(),
            ..tx.clone()
        };
        let witness_root = staged_tx.witness_commitment_hash();
        let sig_bytes = signature.clone();
        TxWitness {
            signature: sig_bytes.clone(),
            pubkey,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: derive_witness_commit_ref(
                        &txid,
                        &witness_root,
                        index as u32,
                    ),
                })
                .collect(),
        }
        .canonical_bytes()
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
        state
            .utxos
            .insert(crate::utxo::UtxoEntry::new(
                Network::Mainnet,
                [9; 48],
                0,
                500,
                vec![1],
                0,
                false,
            ))
            .unwrap();

        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
                output_index: 0,
                unlocking_script: vec![2],
            }],
            outputs: vec![TxOutput {
                value_atoms: 400,
                locking_script: vec![3],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: 500,
                locking_script: vec![9],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let transactions = vec![coinbase, tx];
        let mut block = Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 75,
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            transactions,
        );
        block.fees_miner_atoms = 500;

        state.connect_block(&block).unwrap();
        assert_eq!(state.height, 1);
        assert_eq!(state.utxo_count(), 3);
        assert_eq!(state.blocks().len(), 2);

        state.disconnect_last_block().unwrap();
        assert_eq!(state.height, 0);
        assert_eq!(state.utxo_count(), 2);
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
}
