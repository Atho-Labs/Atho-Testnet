// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Transaction anti-spam policy helpers shared across wallets, nodes, and tests.
//!
//! These helpers define the active relay/consensus transaction policy: fee
//! floors, dust rules, output caps, and wallet transaction proof-of-work.

use crate::constants::{
    DUST_RELAY_VALUE_ATOMS, MAX_STANDARD_OUTPUTS, MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE,
    MIN_TX_FEE_ATOMS, TX_POW_DOMAIN, TX_POW_MAX_BITS, TX_POW_MIN_BITS,
};
use crate::crypto::hash::sha3_256;
use crate::genesis::genesis_hash;
use crate::network::Network;
use crate::transaction::Transaction;
use getrandom::getrandom;
use sha3::{Digest, Sha3_256};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

const TESTNET_TX_POW_BITS: u8 = 12;

#[cfg(test)]
static FORCE_TX_POW_SPAWN_FAILURE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxPowSolveConfig {
    pub auto_threads: bool,
    pub thread_percent: u8,
    pub max_threads: Option<usize>,
    pub min_threads: usize,
}

impl Default for TxPowSolveConfig {
    fn default() -> Self {
        Self {
            auto_threads: true,
            thread_percent: 75,
            max_threads: None,
            min_threads: 1,
        }
    }
}

impl TxPowSolveConfig {
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            auto_threads: env_bool("ATHO_TX_POW_AUTO_THREADS", default.auto_threads),
            thread_percent: env_u8("ATHO_TX_POW_THREAD_PERCENT", default.thread_percent),
            max_threads: env_usize_opt("ATHO_TX_POW_MAX_THREADS"),
            min_threads: env_usize("ATHO_TX_POW_MIN_THREADS", default.min_threads),
        }
    }

    pub fn resolved_thread_count(self) -> usize {
        let available = thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        self.resolved_thread_count_for_available(available)
    }

    pub fn resolved_thread_count_for_available(self, available_threads: usize) -> usize {
        let available_threads = available_threads.max(1);
        let min_threads = self.min_threads.max(1).min(available_threads);
        let max_threads = self
            .max_threads
            .unwrap_or(available_threads)
            .max(1)
            .min(available_threads);
        let mut threads = if self.auto_threads {
            let percent = self.thread_percent.clamp(1, 100) as usize;
            available_threads.saturating_mul(percent) / 100
        } else {
            max_threads
        };
        if threads == 0 {
            threads = 1;
        }
        threads.max(min_threads).min(max_threads).max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxPowCancelled;

fn update_tx_pow_message_hasher(hasher: &mut Sha3_256, tx: &Transaction) {
    tx.update_base_hasher(hasher);

    if tx.witness.is_empty() {
        hasher.update(0u32.to_le_bytes());
        return;
    }
    let Some(witness) = tx.witness_payload() else {
        hasher.update(0u32.to_le_bytes());
        return;
    };

    // Wallet tx PoW must survive block assembly. The per-input
    // witness_commit_ref is block-specific and is rewritten when a miner
    // binds the transaction to a block witness root, so exclude it here
    // while still binding the PoW to the signed witness material.
    hasher.update((witness.signer_group_count() as u32).to_le_bytes());
    witness.for_each_signer_group(|signature, pubkey, input_refs| {
        hasher.update((signature.len() as u32).to_le_bytes());
        hasher.update(signature);
        hasher.update((pubkey.len() as u32).to_le_bytes());
        hasher.update(pubkey);
        hasher.update((input_refs.len() as u32).to_le_bytes());
        for input_ref in input_refs {
            hasher.update(input_ref.input_index.to_le_bytes());
            hasher.update(input_ref.sig_ref_short);
        }
    });
}

pub fn minimum_required_fee_atoms(network: Network, tx: &Transaction) -> u64 {
    let _ = network;
    let vbytes = tx.vsize_bytes().max(1) as u64;
    MIN_TX_FEE_ATOMS.max(vbytes.saturating_mul(MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE))
}

pub fn minimum_output_amount_atoms(network: Network, tx: &Transaction) -> u64 {
    let _ = network;
    let _ = tx;
    DUST_RELAY_VALUE_ATOMS
}

pub fn maximum_standard_outputs(network: Network, tx: &Transaction) -> usize {
    let _ = network;
    let _ = tx;
    MAX_STANDARD_OUTPUTS
}

pub fn required_tx_pow_bits(network: Network, tx: &Transaction, fee_atoms: u64) -> u8 {
    if tx.is_coinbase() {
        return 0;
    }
    if network == Network::Testnet {
        return TESTNET_TX_POW_BITS;
    }

    let tx_vbytes = tx.vsize_bytes().max(1) as u64;
    let fee_rate = fee_atoms / tx_vbytes;
    let output_count = tx.outputs.len();

    let mut bits = TX_POW_MIN_BITS as i16;

    if tx_vbytes > 500 {
        bits += 1;
    }
    if tx_vbytes > 1_000 {
        bits += 1;
    }
    if tx_vbytes > 2_000 {
        bits += 1;
    }

    if output_count > 2 {
        bits += 1;
    }
    if output_count > 8 {
        bits += 1;
    }
    if output_count > 32 {
        bits += 1;
    }

    if fee_rate >= 100 {
        bits -= 1;
    } else if fee_rate >= 10 {
    } else if fee_rate >= 1 {
        bits += 2;
    } else {
        bits += 4;
    }

    if output_count > 32 && fee_rate <= 1 {
        bits += 2;
    }

    bits.clamp(TX_POW_MIN_BITS as i16, TX_POW_MAX_BITS as i16) as u8
}

pub fn transaction_pow_preimage(network: Network, tx: &Transaction) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(TX_POW_DOMAIN);
    hasher.update([network.consensus_id()]);
    hasher.update(genesis_hash(network));
    update_tx_pow_message_hasher(&mut hasher, tx);
    hasher.finalize().into()
}

pub fn leading_zero_bits(bytes: &[u8]) -> u8 {
    let mut count = 0u8;
    for byte in bytes {
        if *byte == 0 {
            count = count.saturating_add(8);
            continue;
        }
        for bit in (0..8).rev() {
            if byte & (1 << bit) != 0 {
                return count;
            }
            count = count.saturating_add(1);
        }
    }
    count
}

pub fn transaction_pow_hash(preimage: &[u8; 32], nonce: u64) -> [u8; 32] {
    let mut payload = [0u8; 40];
    payload[..32].copy_from_slice(preimage);
    payload[32..].copy_from_slice(&nonce.to_be_bytes());
    sha3_256(&payload)
}

fn transaction_pow_nonce_start(preimage: &[u8; 32]) -> u64 {
    let mut nonce = [0u8; 8];
    if getrandom(&mut nonce).is_ok() {
        return u64::from_be_bytes(nonce);
    }
    nonce.copy_from_slice(&preimage[..8]);
    u64::from_be_bytes(nonce)
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

fn env_u8(key: &str, default: u8) -> u8 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_usize_opt(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

pub fn transaction_pow_is_valid(network: Network, tx: &Transaction, fee_atoms: u64) -> bool {
    let required_bits = required_tx_pow_bits(network, tx, fee_atoms);
    transaction_pow_is_valid_for_bits(network, tx, required_bits)
}

pub fn transaction_pow_is_valid_for_bits(
    network: Network,
    tx: &Transaction,
    required_bits: u8,
) -> bool {
    if required_bits == 0 {
        return true;
    }
    if tx.tx_pow_bits != required_bits {
        return false;
    }
    let preimage = transaction_pow_preimage(network, tx);
    leading_zero_bits(&transaction_pow_hash(&preimage, tx.tx_pow_nonce)) >= required_bits
}

pub fn solve_transaction_pow(network: Network, tx: &mut Transaction, fee_atoms: u64) -> u64 {
    let config = TxPowSolveConfig::from_env();
    let stop_requested = AtomicBool::new(false);
    solve_transaction_pow_with_config_and_cancel(network, tx, fee_atoms, config, &stop_requested)
        .expect("default tx pow solver should not be cancelled")
}

pub fn solve_transaction_pow_with_config(
    network: Network,
    tx: &mut Transaction,
    fee_atoms: u64,
    config: TxPowSolveConfig,
) -> u64 {
    let stop_requested = AtomicBool::new(false);
    solve_transaction_pow_with_config_and_cancel(network, tx, fee_atoms, config, &stop_requested)
        .expect("configured tx pow solver should not be cancelled")
}

pub fn solve_transaction_pow_with_config_and_cancel(
    network: Network,
    tx: &mut Transaction,
    fee_atoms: u64,
    config: TxPowSolveConfig,
    stop_requested: &AtomicBool,
) -> Result<u64, TxPowCancelled> {
    let required_bits = required_tx_pow_bits(network, tx, fee_atoms);
    tx.tx_pow_bits = required_bits;
    tx.tx_pow_nonce = 0;
    if required_bits == 0 {
        return Ok(0);
    }
    let preimage = transaction_pow_preimage(network, tx);
    let start_nonce = transaction_pow_nonce_start(&preimage);
    let worker_count = config.resolved_thread_count().max(1);
    solve_transaction_pow_with_worker_count_and_cancel(
        tx,
        &preimage,
        required_bits,
        start_nonce,
        worker_count,
        stop_requested,
    )
}

fn solve_transaction_pow_single_thread(
    tx: &mut Transaction,
    preimage: &[u8; 32],
    required_bits: u8,
    start_nonce: u64,
    stop_requested: &AtomicBool,
) -> Result<u64, TxPowCancelled> {
    let mut nonce = start_nonce;
    loop {
        if stop_requested.load(Ordering::Acquire) {
            tx.tx_pow_nonce = 0;
            return Err(TxPowCancelled);
        }
        if leading_zero_bits(&transaction_pow_hash(preimage, nonce)) >= required_bits {
            tx.tx_pow_nonce = nonce;
            return Ok(nonce);
        }
        nonce = nonce.wrapping_add(1);
    }
}

fn solve_transaction_pow_with_worker_count_and_cancel(
    tx: &mut Transaction,
    preimage: &[u8; 32],
    required_bits: u8,
    start_nonce: u64,
    worker_count: usize,
    stop_requested: &AtomicBool,
) -> Result<u64, TxPowCancelled> {
    if worker_count <= 1 {
        return solve_transaction_pow_single_thread(
            tx,
            preimage,
            required_bits,
            start_nonce,
            stop_requested,
        );
    }

    let found = AtomicBool::new(false);
    let solved_nonce = AtomicU64::new(0);
    let parallel_abort = AtomicBool::new(false);
    let spawn_failed = AtomicBool::new(false);
    thread::scope(|scope| {
        for worker_index in 1..worker_count {
            let found = &found;
            let solved_nonce = &solved_nonce;
            let parallel_abort = &parallel_abort;
            #[cfg(test)]
            if FORCE_TX_POW_SPAWN_FAILURE.load(Ordering::Acquire) {
                spawn_failed.store(true, Ordering::Release);
                parallel_abort.store(true, Ordering::Release);
                break;
            }
            if thread::Builder::new()
                .name(format!("atho-txpow-{worker_index}"))
                .spawn_scoped(scope, move || {
                    let mut candidate = start_nonce.wrapping_add(worker_index as u64);
                    while !found.load(Ordering::Relaxed)
                        && !stop_requested.load(Ordering::Acquire)
                        && !parallel_abort.load(Ordering::Acquire)
                    {
                        if leading_zero_bits(&transaction_pow_hash(preimage, candidate))
                            >= required_bits
                        {
                            if found
                                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                                .is_ok()
                            {
                                solved_nonce.store(candidate, Ordering::Release);
                            }
                            break;
                        }
                        candidate = candidate.wrapping_add(worker_count as u64);
                    }
                })
                .is_err()
            {
                spawn_failed.store(true, Ordering::Release);
                parallel_abort.store(true, Ordering::Release);
                break;
            }
        }

        if !spawn_failed.load(Ordering::Acquire) {
            let mut candidate = start_nonce;
            while !found.load(Ordering::Relaxed)
                && !stop_requested.load(Ordering::Acquire)
                && !parallel_abort.load(Ordering::Acquire)
            {
                if leading_zero_bits(&transaction_pow_hash(preimage, candidate)) >= required_bits {
                    if found
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                        .is_ok()
                    {
                        solved_nonce.store(candidate, Ordering::Release);
                    }
                    break;
                }
                candidate = candidate.wrapping_add(worker_count as u64);
            }
        }
    });

    if found.load(Ordering::Acquire) {
        let nonce = solved_nonce.load(Ordering::Acquire);
        tx.tx_pow_nonce = nonce;
        return Ok(nonce);
    }

    if spawn_failed.load(Ordering::Acquire) {
        return solve_transaction_pow_single_thread(
            tx,
            preimage,
            required_bits,
            start_nonce,
            stop_requested,
        );
    }

    if stop_requested.load(Ordering::Acquire) {
        tx.tx_pow_nonce = 0;
        return Err(TxPowCancelled);
    }

    solve_transaction_pow_single_thread(tx, preimage, required_bits, start_nonce, stop_requested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::TX_POW_DOMAIN;
    use crate::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};

    fn sample_tx(output_count: usize, output_value: u64) -> Transaction {
        Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![2; 32],
            }],
            outputs: (0..output_count)
                .map(|_| TxOutput {
                    value_atoms: output_value,
                    locking_script: vec![3; 32],
                })
                .collect(),
            lock_time: 0,
            witness: vec![4; 0],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        }
    }

    fn inflate_tx_to_min_vbytes(tx: &mut Transaction, minimum_vbytes: usize) {
        while tx.vsize_bytes() <= minimum_vbytes {
            tx.witness.push(0);
        }
    }

    #[test]
    fn normal_low_fee_payment_requires_nineteen_bits() {
        let mut tx = sample_tx(2, 2_000);
        inflate_tx_to_min_vbytes(&mut tx, 500);
        let fee = tx.vsize_bytes() as u64;
        assert!(tx.vsize_bytes() > 500);
        assert!(tx.vsize_bytes() <= 1_000);
        assert_eq!(required_tx_pow_bits(Network::Regnet, &tx, fee), 19);
    }

    #[test]
    fn many_output_low_fee_shape_hits_harder_pow() {
        let mut tx = sample_tx(64, 1_000);
        tx.inputs = vec![TxInput {
            previous_txid: [9; 48],
            output_index: 0,
            unlocking_script: vec![8; 32],
        }];
        inflate_tx_to_min_vbytes(&mut tx, 2_000);
        let fee = tx.vsize_bytes() as u64;
        assert!(tx.vsize_bytes() > 2_000);
        assert_eq!(required_tx_pow_bits(Network::Regnet, &tx, fee), 26);
    }

    #[test]
    fn fragmentation_spam_faces_fee_and_pow_backpressure() {
        let compact = sample_tx(2, DUST_RELAY_VALUE_ATOMS + 1);
        let mut fragmented = sample_tx(MAX_STANDARD_OUTPUTS, DUST_RELAY_VALUE_ATOMS + 1);
        inflate_tx_to_min_vbytes(&mut fragmented, 2_000);

        let compact_fee = minimum_required_fee_atoms(Network::Mainnet, &compact);
        let fragmented_fee = minimum_required_fee_atoms(Network::Mainnet, &fragmented);

        assert_eq!(
            minimum_output_amount_atoms(Network::Mainnet, &compact),
            DUST_RELAY_VALUE_ATOMS
        );
        assert_eq!(
            maximum_standard_outputs(Network::Mainnet, &fragmented),
            MAX_STANDARD_OUTPUTS
        );
        assert!(fragmented_fee > compact_fee);
        assert!(
            required_tx_pow_bits(Network::Mainnet, &fragmented, fragmented_fee)
                > required_tx_pow_bits(Network::Mainnet, &compact, compact_fee)
        );

        let split = sample_tx(MAX_STANDARD_OUTPUTS / 4, DUST_RELAY_VALUE_ATOMS + 1);
        let split_fee_total =
            minimum_required_fee_atoms(Network::Mainnet, &split).saturating_mul(4);
        assert!(split_fee_total >= compact_fee.saturating_mul(4));
    }

    #[test]
    fn required_fee_examples_match_policy_floor() {
        for (vbytes, expected) in [(250usize, 250u64), (500, 500), (650, 650), (2_500, 2_500)] {
            let mut tx = sample_tx(2, 2_000);
            while tx.vsize_bytes() < vbytes {
                tx.witness.push(0);
            }
            assert_eq!(minimum_required_fee_atoms(Network::Regnet, &tx), expected);
        }
    }

    #[test]
    fn pow_bit_examples_match_final_table() {
        let mut low_fee = sample_tx(2, 2_000);
        inflate_tx_to_min_vbytes(&mut low_fee, 500);
        assert_eq!(
            required_tx_pow_bits(Network::Regnet, &low_fee, low_fee.vsize_bytes() as u64),
            19
        );

        let small = sample_tx(2, 2_000);
        assert_eq!(required_tx_pow_bits(Network::Regnet, &small, 500), 18);

        let mut normal_fee = sample_tx(2, 2_000);
        inflate_tx_to_min_vbytes(&mut normal_fee, 500);
        assert_eq!(
            required_tx_pow_bits(Network::Regnet, &normal_fee, 6_500),
            17
        );
        assert_eq!(
            required_tx_pow_bits(Network::Regnet, &normal_fee, 65_000),
            16
        );
    }

    #[test]
    fn solver_finds_valid_nonce_for_regnet_v1() {
        let mut tx = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Regnet, &tx);
        let nonce = solve_transaction_pow(Network::Regnet, &mut tx, fee);
        assert_eq!(nonce, tx.tx_pow_nonce);
        assert!(transaction_pow_is_valid(Network::Regnet, &tx, fee));
    }

    #[test]
    fn tx_pow_thread_budget_defaults_to_seventy_five_percent() {
        let config = TxPowSolveConfig::default();
        assert_eq!(config.resolved_thread_count_for_available(1), 1);
        assert_eq!(config.resolved_thread_count_for_available(2), 1);
        assert_eq!(config.resolved_thread_count_for_available(4), 3);
        assert_eq!(config.resolved_thread_count_for_available(8), 6);
        assert_eq!(config.resolved_thread_count_for_available(16), 12);
    }

    #[test]
    fn tx_pow_thread_budget_honors_caps_and_minimums() {
        let config = TxPowSolveConfig {
            auto_threads: true,
            thread_percent: 75,
            max_threads: Some(5),
            min_threads: 2,
        };
        assert_eq!(config.resolved_thread_count_for_available(16), 5);

        let manual = TxPowSolveConfig {
            auto_threads: false,
            thread_percent: 75,
            max_threads: Some(2),
            min_threads: 1,
        };
        assert_eq!(manual.resolved_thread_count_for_available(8), 2);
    }

    #[test]
    fn tx_pow_config_reads_environment_overrides() {
        std::env::set_var("ATHO_TX_POW_AUTO_THREADS", "false");
        std::env::set_var("ATHO_TX_POW_THREAD_PERCENT", "60");
        std::env::set_var("ATHO_TX_POW_MAX_THREADS", "3");
        std::env::set_var("ATHO_TX_POW_MIN_THREADS", "2");
        let config = TxPowSolveConfig::from_env();
        std::env::remove_var("ATHO_TX_POW_AUTO_THREADS");
        std::env::remove_var("ATHO_TX_POW_THREAD_PERCENT");
        std::env::remove_var("ATHO_TX_POW_MAX_THREADS");
        std::env::remove_var("ATHO_TX_POW_MIN_THREADS");

        assert!(!config.auto_threads);
        assert_eq!(config.thread_percent, 60);
        assert_eq!(config.max_threads, Some(3));
        assert_eq!(config.min_threads, 2);
    }

    #[test]
    fn configured_solver_matches_single_threaded_validity() {
        let mut single = sample_tx(2, 2_000);
        let mut parallel = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Regnet, &single);
        let single_nonce = solve_transaction_pow_with_config(
            Network::Regnet,
            &mut single,
            fee,
            TxPowSolveConfig {
                auto_threads: false,
                thread_percent: 75,
                max_threads: Some(1),
                min_threads: 1,
            },
        );
        let parallel_nonce = solve_transaction_pow_with_config(
            Network::Regnet,
            &mut parallel,
            fee,
            TxPowSolveConfig {
                auto_threads: true,
                thread_percent: 75,
                max_threads: Some(4),
                min_threads: 1,
            },
        );
        assert_eq!(single_nonce, single.tx_pow_nonce);
        assert_eq!(parallel_nonce, parallel.tx_pow_nonce);
        assert!(transaction_pow_is_valid(Network::Regnet, &single, fee));
        assert!(transaction_pow_is_valid(Network::Regnet, &parallel, fee));
        assert_eq!(single.tx_pow_bits, parallel.tx_pow_bits);
    }

    #[test]
    fn configured_solver_falls_back_to_single_core_when_parallel_spawn_fails() {
        let mut tx = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Regnet, &tx);
        let required_bits = required_tx_pow_bits(Network::Regnet, &tx, fee);
        tx.tx_pow_bits = required_bits;
        let preimage = transaction_pow_preimage(Network::Regnet, &tx);
        let stop_requested = AtomicBool::new(false);

        FORCE_TX_POW_SPAWN_FAILURE.store(true, Ordering::Release);
        let result = solve_transaction_pow_with_worker_count_and_cancel(
            &mut tx,
            &preimage,
            required_bits,
            transaction_pow_nonce_start(&preimage),
            4,
            &stop_requested,
        );
        FORCE_TX_POW_SPAWN_FAILURE.store(false, Ordering::Release);

        let nonce = result.expect("fallback solve succeeds");
        assert_eq!(nonce, tx.tx_pow_nonce);
        assert!(transaction_pow_is_valid(Network::Regnet, &tx, fee));
    }

    #[test]
    fn configured_solver_respects_cancellation() {
        let mut tx = sample_tx(64, 1_000);
        inflate_tx_to_min_vbytes(&mut tx, 2_000);
        let fee = tx.vsize_bytes() as u64;
        let stop = AtomicBool::new(true);
        let result = solve_transaction_pow_with_config_and_cancel(
            Network::Regnet,
            &mut tx,
            fee,
            TxPowSolveConfig {
                auto_threads: true,
                thread_percent: 75,
                max_threads: Some(4),
                min_threads: 1,
            },
            &stop,
        );
        assert_eq!(result, Err(TxPowCancelled));
    }

    #[test]
    fn all_networks_require_pow_for_normal_transactions() {
        let tx = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Mainnet, &tx);
        assert_eq!(required_tx_pow_bits(Network::Mainnet, &tx, fee), 18);
        assert_eq!(required_tx_pow_bits(Network::Testnet, &tx, fee), 12);
    }

    #[test]
    fn testnet_transaction_pow_is_fast_but_network_scoped() {
        let mut tx = sample_tx(2, 2_000);
        inflate_tx_to_min_vbytes(&mut tx, 500);
        let fee = tx.vsize_bytes() as u64;

        assert_eq!(required_tx_pow_bits(Network::Testnet, &tx, fee), 12);
        assert_eq!(required_tx_pow_bits(Network::Mainnet, &tx, fee), 19);
        assert_eq!(required_tx_pow_bits(Network::Regnet, &tx, fee), 19);
    }

    #[test]
    fn pow_domain_constant_is_frozen() {
        assert_eq!(TX_POW_DOMAIN, b"ATHO_TX_POW_V1");
    }

    #[test]
    fn changing_nonce_keeps_same_preimage_and_can_change_validity() {
        let mut tx = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Regnet, &tx);
        solve_transaction_pow(Network::Regnet, &mut tx, fee);
        let original_preimage = transaction_pow_preimage(Network::Regnet, &tx);
        let original_nonce = tx.tx_pow_nonce;
        tx.tx_pow_nonce = tx.tx_pow_nonce.wrapping_add(1);

        assert_eq!(
            transaction_pow_preimage(Network::Regnet, &tx),
            original_preimage
        );
        if tx.tx_pow_nonce != original_nonce {
            assert!(!transaction_pow_is_valid(Network::Regnet, &tx, fee));
        }
    }

    #[test]
    fn transaction_pow_preimage_is_network_scoped() {
        let tx = sample_tx(2, 2_000);

        assert_ne!(
            transaction_pow_preimage(Network::Mainnet, &tx),
            transaction_pow_preimage(Network::Testnet, &tx)
        );
        assert_ne!(
            transaction_pow_preimage(Network::Mainnet, &tx),
            transaction_pow_preimage(Network::Regnet, &tx)
        );
    }

    #[test]
    fn block_specific_witness_refs_do_not_invalidate_transaction_pow() {
        let mut tx = sample_tx(2, 2_000);
        tx.witness = TxWitness {
            signature: vec![7; crate::constants::FALCON_512_SIGNATURE_BYTES],
            pubkey: vec![8; crate::constants::FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: [9, 10],
                witness_commit_ref: [0; 16],
            }],
            additional_signers: vec![],
        }
        .canonical_bytes();
        inflate_tx_to_min_vbytes(&mut tx, 500);
        let fee = tx.vsize_bytes() as u64;
        solve_transaction_pow(Network::Regnet, &mut tx, fee);
        assert!(transaction_pow_is_valid(Network::Regnet, &tx, fee));

        let mut witness = tx.witness_payload().expect("witness");
        witness.input_refs[0].witness_commit_ref = [0xaa; 16];
        tx.witness = witness.canonical_bytes();

        assert!(transaction_pow_is_valid(Network::Regnet, &tx, fee));
    }
}
