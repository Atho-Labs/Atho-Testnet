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
    pub chainstate: StorageChainstate,
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

    pub fn connect_block(&mut self, block: &Block) -> Result<(), NodeError> {
        let working_utxos = self.chainstate.utxo_snapshot();
        validate_block_with_context(
            block,
            self.chainstate.height.saturating_add(1),
            self.config.network,
            self.chainstate.tip_hash,
            working_utxos,
        )?;
        self.chainstate.connect_block(block)?;
        let updated_utxos = self.chainstate.utxo_snapshot();
        self.mempool
            .revalidate(|txid, output_index| updated_utxos.get(*txid, output_index).cloned());
        let _ = dev::record_block(self.chainstate.height, block);
        let _ = dev::append_log(
            "chain",
            &format!(
                "connected block hash={} height={} txs={}",
                hex::encode(block.header.block_hash()),
                self.chainstate.height,
                block.transactions.len()
            ),
        );
        Ok(())
    }

    pub fn admit_transaction(&mut self, entry: MempoolEntry) -> Result<[u8; 48], NodeError> {
        let utxos = self.chainstate.utxo_snapshot();
        let txid = self.mempool.admit(entry, |txid, output_index| {
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
        self.admit_transaction(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use crate::validation::encode_input_reference;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::{pow, subsidy};
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
    use atho_crypto::falcon::{FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_MIN_BYTES};
    use atho_storage::utxo::UtxoEntry;

    fn witness_bytes(inputs: usize) -> Vec<u8> {
        TxWitness {
            signature: vec![9; FALCON_512_SIGNATURE_MIN_BYTES],
            pubkey: vec![8; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: (0..inputs).map(|_| vec![7, 7]).collect(),
        }
        .canonical_bytes()
    }

    fn witness_bytes_for_input(previous_txid: [u8; 48], output_index: u32) -> Vec<u8> {
        TxWitness {
            signature: vec![9; FALCON_512_SIGNATURE_MIN_BYTES],
            pubkey: vec![8; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: vec![encode_input_reference(&previous_txid, output_index)],
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
            witness: witness_bytes(1),
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
            witness: witness_bytes(1),
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
        node.chainstate
            .insert_utxo(UtxoEntry {
                network: Network::Mainnet,
                txid: [9; 48],
                output_index: 0,
                value_atoms: 2_000,
                locking_script: vec![1],
            })
            .unwrap();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_500,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: witness_bytes_for_input([9; 48], 0),
        };
        node.admit_transaction(MempoolEntry {
            transaction: tx.clone(),
            fee_atoms: 500,
        })
        .unwrap();

        let miner = Miner::new(4);
        let block = node.mine_and_connect_candidate_block(&miner).unwrap();
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(node.chainstate.height, 1);
    }
}
