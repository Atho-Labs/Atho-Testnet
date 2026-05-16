// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::consensus::subsidy;
use atho_core::consensus::tx_policy::{
    minimum_required_fee_atoms, solve_transaction_pow, solve_transaction_pow_with_config,
    transaction_pow_is_valid, transaction_pow_preimage, TxPowSolveConfig,
};
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use num_bigint::BigUint;

fn legacy_target_for_next_block_with_timestamp(
    network: Network,
    previous_blocks: &[Block],
    next_timestamp: u64,
) -> [u8; 48] {
    if network == Network::Testnet {
        if let Some(previous_block) = previous_blocks.last() {
            if next_timestamp.saturating_sub(previous_block.header.timestamp)
                > pow::TESTNET_STALL_RESET_SECONDS
            {
                return pow::DIFFICULTY_PROFILE.min_difficulty_target;
            }
        }
    }

    let headers: Vec<BlockHeader> = previous_blocks
        .iter()
        .map(|block| block.header.clone())
        .collect();
    legacy_next_target_from_headers(network, &headers)
}

fn legacy_next_target_from_headers(network: Network, headers: &[BlockHeader]) -> [u8; 48] {
    let headers = match headers {
        [first, rest @ ..] if first.height == 0 && !rest.is_empty() => rest,
        _ => headers,
    };
    let averaging_window = pow::POW_PROFILE.averaging_window_blocks as usize;
    let window_len = headers.len().min(averaging_window);
    if window_len < 2 {
        return pow::initial_target_for_network(network);
    }

    let window = &headers[headers.len() - window_len..];
    let tip_index = headers.len() - 1;
    let old_index = headers.len() - window_len;
    let actual_timespan = if window_len < averaging_window {
        headers[tip_index]
            .timestamp
            .saturating_sub(headers[old_index].timestamp)
    } else {
        let tip_mtp = legacy_median_time_past(headers, tip_index);
        let old_mtp = legacy_median_time_past(headers, old_index);
        tip_mtp.saturating_sub(old_mtp)
    };
    let expected_timespan = pow::POW_PROFILE
        .target_block_time_seconds
        .saturating_mul(window_len.saturating_sub(1) as u64);
    let min_actual = expected_timespan
        .saturating_mul(100u64.saturating_sub(pow::POW_PROFILE.max_adjust_down_percent))
        / 100;
    let max_actual = expected_timespan
        .saturating_mul(100u64.saturating_add(pow::POW_PROFILE.max_adjust_up_percent))
        / 100;
    let bounded_timespan = {
        let damped = if actual_timespan >= expected_timespan {
            expected_timespan.saturating_add(
                (actual_timespan - expected_timespan) / pow::POW_PROFILE.damping_factor,
            )
        } else {
            expected_timespan.saturating_sub(
                (expected_timespan - actual_timespan) / pow::POW_PROFILE.damping_factor,
            )
        };
        damped.clamp(min_actual, max_actual)
    };

    let average_target = {
        let mut total = BigUint::default();
        for header in window {
            total += BigUint::from_bytes_be(&header.difficulty_target_or_bits);
        }
        total / BigUint::from(window.len() as u64)
    };
    let threshold =
        (average_target * BigUint::from(bounded_timespan)) / BigUint::from(expected_timespan);
    let threshold_bytes = threshold.to_bytes_be();
    let mut out = [0u8; 48];
    let out_len = out.len();
    if threshold_bytes.len() >= out.len() {
        out.copy_from_slice(&threshold_bytes[threshold_bytes.len() - out_len..]);
    } else {
        out[out_len - threshold_bytes.len()..].copy_from_slice(&threshold_bytes);
    }
    pow::clamp_target(out)
}

fn legacy_median_time_past(headers: &[BlockHeader], index: usize) -> u64 {
    let span = pow::POW_PROFILE.median_window_blocks as usize;
    let start = index.saturating_add(1).saturating_sub(span);
    let mut timestamps: Vec<u64> = headers[start..=index]
        .iter()
        .map(|header| header.timestamp)
        .collect();
    timestamps.sort_unstable();
    timestamps[timestamps.len() / 2]
}

fn sample_transaction() -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_txid: [1; 48],
            output_index: 0,
            unlocking_script: vec![1, 2, 3, 4],
        }],
        outputs: vec![TxOutput {
            value_atoms: 500,
            locking_script: vec![5, 6, 7, 8],
        }],
        lock_time: 0,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    }
}

fn sample_block() -> Block {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::block_subsidy_atoms(0),
            locking_script: vec![0],
        }],
        lock_time: 0,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let tx = sample_transaction();
    let transactions = vec![coinbase, tx.clone(), tx];
    let merkle = merkle_root(&transactions);
    Block::new(
        BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [2; 48],
            merkle_root: merkle,
            witness_root: witness_root(&transactions),
            timestamp: 75,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 42,
        },
        transactions,
    )
}

fn sample_pow_history(len: usize) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(len);
    for height in 0..len as u64 {
        blocks.push(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height,
                previous_block_hash: [height as u8; 48],
                merkle_root: [0; 48],
                witness_root: [0; 48],
                timestamp: 1_000 + height.saturating_mul(77),
                difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.max_difficulty_target,
                nonce: height,
            },
            Vec::new(),
        ));
    }
    blocks
}

fn bench_core_hot_paths(c: &mut Criterion) {
    let tx = sample_transaction();
    let block = sample_block();
    let pow_preimage = transaction_pow_preimage(Network::Regnet, &tx);
    let mut pow_tx = tx.clone();
    let pow_fee = minimum_required_fee_atoms(Network::Regnet, &pow_tx);
    solve_transaction_pow(Network::Regnet, &mut pow_tx, pow_fee);

    c.bench_function("transaction_canonical_bytes", |b| {
        b.iter(|| black_box(tx.canonical_bytes()))
    });

    c.bench_function("transaction_txid", |b| b.iter(|| black_box(tx.txid())));

    c.bench_function("transaction_pow_preimage", |b| {
        b.iter(|| black_box(transaction_pow_preimage(Network::Regnet, &tx)))
    });

    c.bench_function("transaction_pow_verify_2_output_low_fee", |b| {
        b.iter(|| black_box(transaction_pow_is_valid(Network::Regnet, &pow_tx, pow_fee)))
    });

    c.bench_function("transaction_pow_solve_2_output_min_fee", |b| {
        b.iter(|| {
            let mut candidate = tx.clone();
            black_box(solve_transaction_pow(
                Network::Regnet,
                &mut candidate,
                pow_fee,
            ))
        })
    });

    c.bench_function(
        "transaction_pow_solve_2_output_min_fee_single_thread",
        |b| {
            b.iter(|| {
                let mut candidate = tx.clone();
                black_box(solve_transaction_pow_with_config(
                    Network::Regnet,
                    &mut candidate,
                    pow_fee,
                    TxPowSolveConfig {
                        auto_threads: false,
                        thread_percent: 75,
                        max_threads: Some(1),
                        min_threads: 1,
                    },
                ))
            })
        },
    );

    c.bench_function("transaction_pow_solve_2_output_min_fee_auto_threads", |b| {
        b.iter(|| {
            let mut candidate = tx.clone();
            black_box(solve_transaction_pow_with_config(
                Network::Regnet,
                &mut candidate,
                pow_fee,
                TxPowSolveConfig {
                    auto_threads: true,
                    thread_percent: 75,
                    max_threads: Some(12),
                    min_threads: 1,
                },
            ))
        })
    });

    c.bench_function("block_canonical_bytes", |b| {
        b.iter(|| black_box(block.canonical_bytes()))
    });

    c.bench_function("block_hash", |b| {
        b.iter(|| black_box(block.header.block_hash()))
    });

    let pow_history = sample_pow_history(256);
    let next_timestamp = pow_history
        .last()
        .map(|block| block.header.timestamp + 77)
        .unwrap_or(0);
    c.bench_function("pow_next_target_clone_heavy_reference", |b| {
        b.iter(|| {
            black_box(legacy_target_for_next_block_with_timestamp(
                Network::Mainnet,
                &pow_history,
                next_timestamp,
            ))
        })
    });

    c.bench_function("pow_next_target_current", |b| {
        b.iter(|| {
            black_box(pow::target_for_next_block_with_timestamp(
                Network::Mainnet,
                &pow_history,
                next_timestamp,
            ))
        })
    });

    c.bench_function("transaction_pow_hash_ready_nonce", |b| {
        b.iter(|| {
            black_box(atho_core::consensus::tx_policy::transaction_pow_hash(
                &pow_preimage,
                pow_tx.tx_pow_nonce,
            ))
        })
    });
}

criterion_group!(benches, bench_core_hot_paths);
criterion_main!(benches);
