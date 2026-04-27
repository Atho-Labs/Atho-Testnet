use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use crate::validation::finalize_witness_commit_refs;
use atho_core::address::address_parts_from_public_key;
use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::BLOCK_TIME_SECONDS;
use atho_core::crypto::hash::sha3_384;
use atho_core::transaction::{Transaction, TxOutput};
use atho_crypto::falcon;
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
        let height = node.chainstate.height.saturating_add(1);
        let subsidy_atoms = subsidy::block_subsidy_atoms(node.chainstate.height.saturating_add(1));
        let (reward_address, reward_script) =
            Self::reward_target_for_height(node.config.network, height);
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
        let witness_root = witness_root(&transactions);
        transactions = transactions
            .into_iter()
            .map(|tx| finalize_witness_commit_refs(&tx, witness_root))
            .collect();
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
                "assembled candidate block prev={} reward={} txs={} cores={}",
                hex::encode(previous_block_hash),
                reward_address,
                block.transactions.len(),
                self.cores
            ),
        );
        Ok(block)
    }

    pub fn mine_candidate_block(&self, node: &Node) -> Result<Block, NodeError> {
        Ok(self.solve_block(self.build_candidate_block(node)?))
    }

    fn reward_target_for_height(
        network: atho_core::network::Network,
        height: u64,
    ) -> (String, Vec<u8>) {
        let mut seed = Vec::with_capacity(network.id().len() + 8);
        seed.extend_from_slice(network.domain_tag().as_bytes());
        seed.extend_from_slice(&height.to_le_bytes());
        let keypair = falcon::generate_from_seed(&sha3_384(&seed)).expect("falcon reward keypair");
        let public_key = keypair.public_key.as_bytes().to_vec();
        let parts = address_parts_from_public_key(network, &public_key);
        (parts.base56_address, parts.payment_digest.to_vec())
    }
}
