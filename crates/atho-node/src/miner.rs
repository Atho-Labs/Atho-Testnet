use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use atho_core::address::internal_hpk_bytes;
use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::BLOCK_TIME_SECONDS;
use atho_core::crypto::hash::sha3_384;
use atho_core::genesis;
use atho_core::transaction::{Transaction, TxOutput};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Miner {
    pub cores: u32,
}

impl Miner {
    pub fn new(cores: u32) -> Self {
        Self { cores }
    }

    fn mine_nonce(mut header: BlockHeader, cores: usize) -> BlockHeader {
        let prefix = header.canonical_bytes_without_nonce();
        let target = header.difficulty_target_or_bits;
        let worker_count = cores.max(1);
        if worker_count == 1 {
            let mut nonce = 0u64;
            loop {
                let mut bytes = Vec::with_capacity(prefix.len().saturating_add(8));
                bytes.extend_from_slice(&prefix);
                bytes.extend_from_slice(&nonce.to_le_bytes());
                if pow::meets_target(&sha3_384(&bytes), &target) {
                    header.nonce = nonce;
                    return header;
                }
                nonce = nonce.wrapping_add(1);
            }
        }

        let found = AtomicBool::new(false);
        let nonce = AtomicU64::new(0);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .build()
            .expect("rayon pool");
        pool.install(|| {
            (0..worker_count).into_par_iter().for_each(|worker_index| {
                let mut candidate = worker_index as u64;
                while !found.load(Ordering::Relaxed) {
                    let mut bytes = Vec::with_capacity(prefix.len().saturating_add(8));
                    bytes.extend_from_slice(&prefix);
                    bytes.extend_from_slice(&candidate.to_le_bytes());
                    if pow::meets_target(&sha3_384(&bytes), &target) {
                        if found
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
                            .is_ok()
                        {
                            nonce.store(candidate, Ordering::Release);
                        }
                        break;
                    }
                    candidate = candidate.wrapping_add(worker_count as u64);
                }
            });
        });
        header.nonce = nonce.load(Ordering::Acquire);
        header
    }

    pub fn solve_block(&self, mut block: Block) -> Block {
        let header = if cfg!(test) {
            block.header.clone()
        } else {
            Self::mine_nonce(block.header.clone(), self.cores as usize)
        };
        block.header = header;
        block
    }

    pub fn build_candidate_block(&self, node: &Node) -> Result<Block, NodeError> {
        let utxos = node.chainstate.utxo_snapshot();
        let (pending_transactions, fees_atoms) = node
            .mempool
            .valid_transactions(|txid, output_index| utxos.get(*txid, output_index).cloned())?;
        let subsidy_atoms = subsidy::block_subsidy_atoms(node.chainstate.height.saturating_add(1));
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
        let mut transactions = Vec::with_capacity(pending_transactions.len().saturating_add(1));
        transactions.push(coinbase);
        transactions.extend(pending_transactions);
        let previous_block_hash = node.chainstate.tip_hash;
        let height = node.chainstate.height.saturating_add(1);
        let witness_root = witness_root(&transactions);
        let header = BlockHeader {
            version: 1,
            network_id: node.config.network,
            height,
            previous_block_hash,
            merkle_root: merkle_root(&transactions),
            witness_root,
            timestamp: node
                .chainstate
                .height
                .saturating_add(1)
                .saturating_mul(BLOCK_TIME_SECONDS),
            difficulty_target_or_bits: pow::target_for_height(node.config.network, height),
            nonce: 0,
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

    pub fn mine_candidate_block(&self, node: &Node) -> Result<Block, NodeError> {
        Ok(self.solve_block(self.build_candidate_block(node)?))
    }
}
