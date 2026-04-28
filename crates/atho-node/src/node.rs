use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::mempool::{Mempool, MempoolEntry};
use crate::miner::Miner;
use crate::validation::ValidationError;
use atho_core::block::Block;
use atho_core::block::BlockHeader;
use atho_core::consensus::pow;
use atho_core::transaction::Transaction;
use atho_storage::chainstate::{
    ChainSelectionOutcome, ChainSelectionResult, ChainSnapshotBundle,
    Chainstate as StorageChainstate,
};
use atho_storage::error::StorageError;

#[derive(Debug)]
pub struct Node {
    pub config: NodeConfig,
    chainstate: StorageChainstate,
    pub mempool: Mempool,
}

impl Node {
    pub fn new(config: NodeConfig) -> Self {
        Self {
            config,
            chainstate: StorageChainstate::new(config.network),
            mempool: Mempool::new(),
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

    pub fn canonical_blocks(&self) -> Result<Vec<Block>, NodeError> {
        self.chainstate.canonical_blocks().map_err(NodeError::from)
    }

    pub fn export_snapshot_bundle(&self) -> Result<ChainSnapshotBundle, NodeError> {
        self.chainstate.export_snapshot_bundle().map_err(NodeError::from)
    }

    pub fn import_snapshot_bundle(
        &mut self,
        bundle: ChainSnapshotBundle,
    ) -> Result<(), NodeError> {
        self.chainstate
            .import_snapshot_bundle(bundle)
            .map_err(NodeError::from)?;
        let updated_utxos = self.chainstate.utxo_snapshot();
        self.mempool
            .revalidate(self.chainstate.height, |txid, output_index| {
                updated_utxos.get(*txid, output_index).cloned()
            });
        Ok(())
    }

    pub fn contains_block(&self, block_hash: &[u8; 48]) -> bool {
        self.chainstate.contains_block(*block_hash).unwrap_or(false)
    }

    pub fn block_by_hash(&self, block_hash: [u8; 48]) -> Option<Block> {
        self.chainstate.block_by_hash(block_hash).ok().flatten()
    }

    pub fn difficulty_target_for_next_block(&self) -> [u8; 48] {
        pow::target_for_next_block(self.network(), self.chainstate.blocks())
    }

    pub fn mempool_len(&self) -> usize {
        self.mempool.len()
    }

    pub fn mempool_total_fee_atoms(&self) -> u64 {
        self.mempool.total_fee_atoms()
    }

    pub fn mempool_contains(&self, txid: &[u8; 48]) -> bool {
        self.mempool.contains(txid)
    }

    pub fn mempool_transaction(&self, txid: &[u8; 48]) -> Option<Transaction> {
        self.mempool.transaction(txid)
    }

    pub fn mempool_transactions(&self) -> Vec<Transaction> {
        self.mempool.transactions()
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
        Self {
            config,
            chainstate: StorageChainstate::load_or_new(config.network),
            mempool: Mempool::new(),
        }
    }

    pub fn try_load_or_new(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            config,
            chainstate: StorageChainstate::try_load_or_new(config.network)?,
            mempool: Mempool::new(),
        })
    }

    pub fn try_load_or_recover(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            config,
            chainstate: StorageChainstate::try_load_or_recover(config.network)?,
            mempool: Mempool::new(),
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
        let updated_utxos = self.chainstate.utxo_snapshot();
        self.mempool
            .revalidate(self.chainstate.height, |txid, output_index| {
                updated_utxos.get(*txid, output_index).cloned()
            });
        let mempool_count = self.mempool.len();
        let _ = dev::record_block(self.chainstate.height, block);
        let _ = dev::append_log(
            "chain",
            &format!(
                "connected mempool={mempool_count} {}",
                dev::summarize_block(block)
            ),
        );
        Ok(())
    }

    pub fn admit_transaction(&mut self, entry: MempoolEntry) -> Result<[u8; 48], NodeError> {
        let utxos = self.chainstate.utxo_snapshot();
        let txid = self
            .mempool
            .admit(entry, self.chainstate.height, |txid, output_index| {
                utxos.get(*txid, output_index).cloned()
            })?;
        Ok(txid)
    }

    pub fn accept_relayed_transaction(
        &mut self,
        transaction: Transaction,
    ) -> Result<[u8; 48], NodeError> {
        if transaction.is_coinbase() {
            return Err(NodeError::Validation(ValidationError::InvalidCoinbase));
        }
        let utxos = self.chainstate.utxo_snapshot();
        let fee_atoms = transaction_fee_from_utxos(&transaction, &utxos)
            .ok_or(NodeError::Validation(ValidationError::MissingUtxo))?;
        self.submit_transaction(MempoolEntry::new(transaction, fee_atoms))
    }

    pub fn mine_candidate_block(&self, miner: &Miner) -> Result<Block, NodeError> {
        miner.mine_candidate_block(self)
    }

    pub fn build_candidate_block(&self, miner: &Miner) -> Result<Block, NodeError> {
        miner.build_candidate_block(self)
    }

    pub fn mine_and_connect_candidate_block(&mut self, miner: &Miner) -> Result<Block, NodeError> {
        let block = miner.mine_candidate_block(self)?;
        self.connect_block(&block)?;
        Ok(block)
    }

    pub fn submit_block(&mut self, block: &Block) -> Result<(), NodeError> {
        self.connect_block(block)
    }

    pub fn consider_branch(&mut self, branch: &[Block]) -> Result<ChainSelectionResult, NodeError> {
        let selection = self.chainstate.select_branch(branch)?;
        match selection.outcome {
            ChainSelectionOutcome::KeptCurrent => return Ok(selection),
            ChainSelectionOutcome::Extended | ChainSelectionOutcome::Reorged => {}
        }

        for block in branch {
            self.mempool.remove_block_transactions(block);
        }

        if selection.outcome == ChainSelectionOutcome::Reorged {
            let utxos = self.chainstate.utxo_snapshot();
            for tx in selection
                .disconnected
                .iter()
                .flat_map(|block| block.transactions.iter().skip(1))
            {
                let Some(fee_atoms) = transaction_fee_from_utxos(tx, &utxos) else {
                    continue;
                };
                let _ = self.mempool.admit(
                    MempoolEntry::new(tx.clone(), fee_atoms),
                    self.chainstate.height,
                    |txid, output_index| utxos.get(*txid, output_index).cloned(),
                );
            }
        }

        let updated_utxos = self.chainstate.utxo_snapshot();
        self.mempool
            .revalidate(self.chainstate.height, |txid, output_index| {
                updated_utxos.get(*txid, output_index).cloned()
            });
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
        let blocks = self.chainstate.blocks();
        if blocks.is_empty() || max_headers == 0 {
            return Vec::new();
        }

        let start_index = locator_hashes
            .iter()
            .find_map(|hash| {
                blocks
                    .iter()
                    .position(|block| block.header.block_hash() == *hash)
                    .map(|index| index.saturating_add(1))
            })
            .unwrap_or(0);

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
        headers
    }
}

fn transaction_fee_from_utxos(
    tx: &atho_core::transaction::Transaction,
    utxos: &atho_storage::utxo::UtxoSet,
) -> Option<u64> {
    if tx.is_coinbase() {
        return None;
    }
    let mut input_total = 0u64;
    for input in &tx.inputs {
        let utxo = utxos.get(input.previous_txid, input.output_index)?;
        input_total = input_total.checked_add(utxo.value_atoms)?;
    }
    input_total.checked_sub(tx.output_value_atoms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use crate::validation::{derive_sig_ref_short, derive_witness_commit_ref};
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::consensus::{pow, subsidy};
    use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};
    use atho_storage::db::Database;
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use atho_storage::utxo::UtxoEntry;
    use std::ffi::OsString;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn witness_bytes_for_tx(tx: &Transaction) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-node-test").expect("falcon keypair");
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &transaction_signing_digest(tx),
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
            pubkey: pubkey.clone(),
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

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
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
                locking_script: vec![0],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_500,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
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
                locking_script: vec![0],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
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
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let fee_atoms = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let tx = Transaction {
            outputs: vec![TxOutput {
                value_atoms: 2_000 - fee_atoms,
                locking_script: vec![2],
            }],
            ..Transaction {
                witness: vec![],
                ..tx
            }
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        node.admit_transaction(MempoolEntry::new(tx.clone(), fee_atoms))
            .unwrap();

        let miner = Miner::new(4);
        let block = node.mine_and_connect_candidate_block(&miner).unwrap();
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(node.chainstate.height, 7);
        assert_eq!(node.mempool.len(), 0);
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
            donor.export_snapshot_bundle().expect("export snapshot bundle")
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
