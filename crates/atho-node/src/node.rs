use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::mempool::{Mempool, MempoolEntry};
use crate::miner::Miner;
use crate::validation::validate_block_with_context;
use atho_core::block::Block;
use atho_storage::chainstate::Chainstate as StorageChainstate;

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

    pub fn mempool_len(&self) -> usize {
        self.mempool.len()
    }

    pub fn mempool_total_fee_atoms(&self) -> u64 {
        self.mempool.total_fee_atoms()
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

    pub fn connect_block(&mut self, block: &Block) -> Result<(), NodeError> {
        let working_utxos = self.chainstate.utxo_snapshot();
        validate_block_with_context(
            block,
            self.chainstate.height.saturating_add(1),
            self.config.network,
            self.chainstate.tip_hash,
            working_utxos,
        )?;
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
            return Err(err.into());
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use crate::validation::{derive_sig_ref_short, derive_witness_commit_ref};
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::{pow, subsidy};
    use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_MIN_BYTES};
    use atho_storage::utxo::UtxoEntry;

    fn witness_bytes_for_tx(tx: &Transaction) -> Vec<u8> {
        let signature = vec![9; FALCON_512_SIGNATURE_MIN_BYTES];
        let pubkey = vec![8; FALCON_512_PUBLIC_KEY_BYTES];
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
    fn node_connect_block_surfaces_storage_errors() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(0),
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
            timestamp: 75,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        let block = Block::new(header, transactions);

        let err = node.connect_block(&block).unwrap_err();
        assert!(matches!(
            err,
            NodeError::Validation(crate::validation::ValidationError::MissingUtxo)
        ));
    }

    #[test]
    fn node_rejects_wrong_parent_hash() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(0),
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
            timestamp: 75,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        let block = Block::new(header, transactions);

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
        node.admit_transaction(MempoolEntry {
            transaction: tx.clone(),
            fee_atoms,
        })
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
}
