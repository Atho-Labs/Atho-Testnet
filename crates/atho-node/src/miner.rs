use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use crate::validation::finalize_witness_commit_refs;
use atho_core::address::address_parts_from_public_key;
use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::rules;
use atho_core::consensus::{pow, subsidy};
use atho_core::crypto::hash::sha3_384;
use atho_core::transaction::{Transaction, TxOutput};
use atho_crypto::falcon;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiningInterrupted;

#[derive(Debug, Default)]
pub struct Miner {
    pub cores: u32,
}

impl Miner {
    pub fn new(cores: u32) -> Self {
        Self { cores }
    }

    fn current_unix_timestamp_seconds() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    }

    fn candidate_block_timestamp(previous_blocks: &[Block]) -> u64 {
        let now = Self::current_unix_timestamp_seconds();
        pow::minimum_next_block_timestamp(previous_blocks).map_or(now, |minimum| now.max(minimum))
    }

    fn mine_nonce(
        mut header: BlockHeader,
        cores: usize,
        stop_requested: Arc<AtomicBool>,
    ) -> Result<BlockHeader, MiningInterrupted> {
        let prefix = header.canonical_bytes_without_nonce();
        let target = header.difficulty_target_or_bits;
        let worker_count = cores.max(1);
        let found = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let nonce = Arc::new(AtomicU64::new(0));
        let attempts = Arc::new(AtomicU64::new(0));
        let best_nonce = Arc::new(AtomicU64::new(0));
        let monitor_finished = Arc::clone(&finished);
        let monitor_attempts = Arc::clone(&attempts);
        let monitor_best_nonce = Arc::clone(&best_nonce);
        let monitor_stop = Arc::clone(&stop_requested);
        let monitor_height = header.height;
        let monitor_cores = worker_count;
        let monitor_target = target;
        let progress_monitor = thread::spawn(move || {
            let start = Instant::now();
            let mut last_attempts = 0u64;
            while !monitor_finished.load(Ordering::Acquire) && !monitor_stop.load(Ordering::Acquire)
            {
                thread::sleep(Duration::from_secs(1));
                let checked = monitor_attempts.load(Ordering::Relaxed);
                let delta = checked.saturating_sub(last_attempts);
                let rate = delta;
                last_attempts = checked;
                let snapshot_nonce = monitor_best_nonce.load(Ordering::Relaxed);
                let _ = dev::append_log(
                    "miner",
                    &format!(
                        "progress height={} cores={} checked={} nonce={} rate={} hps target={}",
                        monitor_height,
                        monitor_cores,
                        checked,
                        snapshot_nonce,
                        rate,
                        hex::encode(monitor_target)
                    ),
                );
                if checked == 0 && start.elapsed() > Duration::from_secs(5) {
                    let _ = dev::append_log(
                        "miner",
                        &format!(
                            "progress height={} cores={} still searching nonce=0 target={}",
                            monitor_height,
                            monitor_cores,
                            hex::encode(monitor_target)
                        ),
                    );
                }
            }
        });

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .build()
            .expect("rayon pool");
        let worker_stop = Arc::clone(&stop_requested);
        pool.install(|| {
            (0..worker_count).into_par_iter().for_each(|worker_index| {
                let mut candidate = worker_index as u64;
                let mut local_attempts = 0u64;
                let mut bytes = Vec::with_capacity(prefix.len().saturating_add(8));
                bytes.extend_from_slice(&prefix);
                bytes.resize(prefix.len().saturating_add(8), 0);
                while !found.load(Ordering::Relaxed) && !worker_stop.load(Ordering::Relaxed) {
                    bytes[prefix.len()..].copy_from_slice(&candidate.to_le_bytes());
                    if pow::meets_target(&sha3_384(&bytes), &target) {
                        if found
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
                            .is_ok()
                        {
                            nonce.store(candidate, Ordering::Release);
                            best_nonce.store(candidate, Ordering::Relaxed);
                        }
                        attempts.fetch_add(local_attempts.saturating_add(1), Ordering::Relaxed);
                        break;
                    }
                    local_attempts = local_attempts.saturating_add(1);
                    if local_attempts >= 8_192 {
                        attempts.fetch_add(local_attempts, Ordering::Relaxed);
                        best_nonce.store(candidate, Ordering::Relaxed);
                        local_attempts = 0;
                    }
                    candidate = candidate.wrapping_add(worker_count as u64);
                }
                if local_attempts > 0 {
                    attempts.fetch_add(local_attempts, Ordering::Relaxed);
                    best_nonce.store(candidate, Ordering::Relaxed);
                }
            });
        });
        finished.store(true, Ordering::Release);
        let _ = progress_monitor.join();
        if stop_requested.load(Ordering::Acquire) && !found.load(Ordering::Acquire) {
            return Err(MiningInterrupted);
        }
        header.nonce = nonce.load(Ordering::Acquire);
        Ok(header)
    }

    pub fn solve_block(&self, block: Block) -> Block {
        let stop_requested = Arc::new(AtomicBool::new(false));
        self.solve_block_with_cancel(block, stop_requested)
            .expect("mining should not be cancelled in non-cancellable path")
    }

    pub fn solve_block_with_cancel(
        &self,
        mut block: Block,
        stop_requested: Arc<AtomicBool>,
    ) -> Result<Block, MiningInterrupted> {
        let _ = dev::append_log(
            "miner",
            &format!(
                "mining block height={} cores={} txs={}",
                block.header.height,
                self.cores,
                block.transactions.len()
            ),
        );
        let header = Self::mine_nonce(block.header.clone(), self.cores as usize, stop_requested);
        block.header = header?;
        let _ = dev::append_log(
            "miner",
            &format!(
                "mined block hash={} height={} nonce={}",
                hex::encode(block.header.block_hash()),
                block.header.height,
                block.header.nonce
            ),
        );
        Ok(block)
    }

    pub fn build_candidate_block(&self, node: &Node) -> Result<Block, NodeError> {
        let utxos = node.utxo_snapshot();
        let (pending_transactions, fees_atoms) = node
            .mempool
            .valid_transactions(node.height().saturating_add(1), |txid, output_index| {
                utxos.get(*txid, output_index).cloned()
            })?;
        let height = node.height().saturating_add(1);
        let active_rules = rules::rules_at_height(height);
        let subsidy_atoms = subsidy::block_subsidy_atoms(node.height().saturating_add(1));
        let (reward_address, reward_script) =
            Self::reward_target_for_height(node.network(), height);
        let coinbase = Transaction {
            version: active_rules.transaction_version,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy_atoms.saturating_add(fees_atoms),
                locking_script: reward_script,
            }],
            lock_time: u32::try_from(height).unwrap_or(u32::MAX),
            witness: vec![],
        };
        let mut transactions = Vec::with_capacity(pending_transactions.len().saturating_add(1));
        transactions.push(coinbase);
        transactions.extend(pending_transactions);
        let previous_block_hash = node.tip_hash();
        let witness_root = witness_root(&transactions);
        transactions = transactions
            .into_iter()
            .map(|tx| finalize_witness_commit_refs(&tx, witness_root))
            .collect();
        let difficulty_target_or_bits = node.difficulty_target_for_next_block();
        let timestamp = Self::candidate_block_timestamp(node.blocks());
        let header = BlockHeader {
            version: active_rules.block_version,
            network_id: node.network(),
            height,
            previous_block_hash,
            merkle_root: merkle_root(&transactions),
            witness_root,
            timestamp,
            difficulty_target_or_bits,
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
        let _ = dev::append_log(
            "miner",
            &format!(
                "candidate request network={} height={} mempool={}",
                node.config.network.id(),
                node.height().saturating_add(1),
                node.mempool_len()
            ),
        );
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
