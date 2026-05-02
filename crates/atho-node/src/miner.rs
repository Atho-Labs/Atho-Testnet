//! Reference CPU nonce-search implementation.
use crate::dev;
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::crypto::hash::sha3_384;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

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
}
