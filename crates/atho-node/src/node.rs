use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::mempool::Mempool;
use crate::miner::Miner;
use crate::validation::validate_block;
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
        validate_block(block, self.chainstate.height, self.config.network)?;
        if block.header.previous_block_hash != self.chainstate.tip_hash {
            return Err(crate::validation::ValidationError::BlockParentHashMismatch.into());
        }
        self.chainstate.connect_block(block)?;
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

    pub fn mine_candidate_block(&self, miner: &Miner) -> Result<Block, NodeError> {
        miner.assemble_candidate_block(self)
    }

    pub fn mine_and_connect_candidate_block(&mut self, miner: &Miner) -> Result<Block, NodeError> {
        let block = miner.assemble_candidate_block(self)?;
        self.connect_block(&block)?;
        Ok(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::{merkle_root, Block, BlockHeader};
    use atho_core::consensus::{pow, subsidy};
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
    use atho_crypto::falcon::{FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_MIN_BYTES};
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use atho_storage::utxo::UtxoEntry;

    fn witness_bytes(inputs: usize) -> Vec<u8> {
        TxWitness {
            signature: vec![9; FALCON_512_SIGNATURE_MIN_BYTES],
            pubkey: vec![8; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: (0..inputs).map(|_| vec![7, 7]).collect(),
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
                value_atoms: subsidy::block_subsidy_atho(0),
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
                previous_block_hash: node.chainstate.tip_hash,
                merkle_root: merkle_root(&transactions),
                timestamp: 75,
                target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            };
        let block = Block::new(
            header,
            transactions,
        );

        let err = node.connect_block(&block).unwrap_err();
        assert!(matches!(err, NodeError::Storage(_)));
    }

    #[test]
    fn node_rejects_wrong_parent_hash() {
        let mut node = Node::new(NodeConfig::new(Network::Mainnet));
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atho(0),
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
                previous_block_hash: [1; 48],
                merkle_root: merkle_root(&transactions),
                timestamp: 75,
                target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            };
        let block = Block::new(
            header,
            transactions,
        );

        let err = node.connect_block(&block).unwrap_err();
        assert!(matches!(err, NodeError::Validation(crate::validation::ValidationError::BlockParentHashMismatch)));
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
                value_atoms: 1_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: witness_bytes(1),
        };
        node.mempool
            .admit(MempoolEntry {
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
