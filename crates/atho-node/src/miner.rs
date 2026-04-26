use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use atho_core::block::{merkle_root, Block, BlockHeader};
use atho_core::address::internal_hpk_bytes;
use atho_core::genesis;
use atho_core::crypto::hash::sha3_384;
use atho_core::consensus::{pow, subsidy};
use atho_core::transaction::{Transaction, TxOutput};

#[derive(Debug, Default)]
pub struct Miner {
    pub cores: u32,
}

impl Miner {
    pub fn new(cores: u32) -> Self {
        Self { cores }
    }

    fn mine_nonce(mut header: BlockHeader) -> BlockHeader {
        let prefix = header.canonical_bytes_without_nonce();
        let mut nonce = 0u64;
        loop {
            let mut bytes = prefix.clone();
            bytes.extend_from_slice(&nonce.to_le_bytes());
            if sha3_384(&bytes) <= header.target {
                header.nonce = nonce;
                return header;
            }
            nonce = nonce.wrapping_add(1);
        }
    }

    pub fn assemble_candidate_block(&self, node: &Node) -> Result<Block, NodeError> {
        let fees_atoms = node.mempool.total_fee_atoms();
        let subsidy_atoms = subsidy::block_subsidy_atho(node.chainstate.height.saturating_add(1));
        let reward_address = genesis::genesis_reward_address(node.config.network);
        let reward_script = internal_hpk_bytes(node.config.network, &reward_address)
            .unwrap_or_else(|| reward_address.as_bytes().to_vec());
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy_atoms.saturating_add(fees_atoms),
                locking_script: reward_script,
            }],
            lock_time: 0,
            witness: vec![],
        };
        let mut transactions = Vec::with_capacity(node.mempool.len().saturating_add(1));
        transactions.push(coinbase);
        transactions.extend(node.mempool.transactions());
        let previous_block_hash = node.chainstate.tip_hash;
        let header = BlockHeader {
            version: 1,
            previous_block_hash,
            merkle_root: merkle_root(&transactions),
            timestamp: node.chainstate.height.saturating_add(1) * 75,
            target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        let header = if cfg!(test) {
            header
        } else {
            Self::mine_nonce(header)
        };
        let mut block = Block::new(header, transactions);
        block.fees_total_atoms = fees_atoms;
        block.fees_miner_atoms = fees_atoms;
        block.fees_burned_atoms = 0;
        block.fees_pool_atoms = 0;
        block.cumulative_burned_atoms = 0;
        let _ = dev::append_log(
            "miner",
            &format!(
                "assembled candidate block prev={} txs={} cores={}",
                hex::encode(previous_block_hash),
                block.transactions.len(),
                self.cores
            ),
        );
        Ok(block)
    }
}
