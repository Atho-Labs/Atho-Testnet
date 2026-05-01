use crate::block::{Block, BlockHeader};
use crate::constants::{
    POW_AVERAGING_WINDOW_BLOCKS, POW_DAMPING_FACTOR, POW_MAX_ADJUST_DOWN_PERCENT,
    POW_MAX_ADJUST_UP_PERCENT, POW_MEDIAN_WINDOW_BLOCKS, POW_RETARGET_INTERVAL_BLOCKS,
    TARGET_BLOCK_TIME_SECONDS,
};
use crate::network::Network;
use hex_literal::hex;
use num_bigint::BigUint;
use num_traits::Zero;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofOfWork {
    pub target_block_time_seconds: u64,
    pub retarget_interval_blocks: u64,
    pub averaging_window_blocks: u64,
    pub median_window_blocks: u64,
    pub damping_factor: u64,
    pub max_adjust_up_percent: u64,
    pub max_adjust_down_percent: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DifficultyTargetProfile {
    pub genesis_target: [u8; 48],
    pub min_difficulty_target: [u8; 48],
    pub max_difficulty_target: [u8; 48],
    pub standard_transaction_allocation_bps: u16,
}

pub const POW_PROFILE: ProofOfWork = ProofOfWork {
    target_block_time_seconds: TARGET_BLOCK_TIME_SECONDS,
    retarget_interval_blocks: POW_RETARGET_INTERVAL_BLOCKS,
    averaging_window_blocks: POW_AVERAGING_WINDOW_BLOCKS,
    median_window_blocks: POW_MEDIAN_WINDOW_BLOCKS,
    damping_factor: POW_DAMPING_FACTOR,
    max_adjust_up_percent: POW_MAX_ADJUST_UP_PERCENT,
    max_adjust_down_percent: POW_MAX_ADJUST_DOWN_PERCENT,
};

pub const SHA3_384_HASH_BITS: usize = 384;
pub const SHA3_384_HASH_HEX_CHARS: usize = 96;

pub const DIFFICULTY_PROFILE: DifficultyTargetProfile = DifficultyTargetProfile {
    genesis_target: hex!("0000FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
    min_difficulty_target: hex!("0000FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
    max_difficulty_target: hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000FFF"),
    standard_transaction_allocation_bps: 9_500,
};

pub const MAINNET_INITIAL_TARGET: [u8; 48] = DIFFICULTY_PROFILE.genesis_target;
pub const TESTNET_INITIAL_TARGET: [u8; 48] = DIFFICULTY_PROFILE.genesis_target;
pub const REGNET_INITIAL_TARGET: [u8; 48] = DIFFICULTY_PROFILE.genesis_target;
pub const PRUNETEST_INITIAL_TARGET: [u8; 48] = DIFFICULTY_PROFILE.min_difficulty_target;

pub fn expected_timespan_seconds() -> u64 {
    expected_timespan_seconds_for_window(POW_PROFILE.averaging_window_blocks as usize)
}

fn expected_timespan_seconds_for_window(window_blocks: usize) -> u64 {
    POW_PROFILE
        .target_block_time_seconds
        .saturating_mul(window_blocks.saturating_sub(1) as u64)
}

pub fn clamp_difficulty_delta(current: i64, change: i64, limit: i64) -> i64 {
    let next = current.saturating_add(change);
    if next > limit {
        limit
    } else if next < -limit {
        -limit
    } else {
        next
    }
}

pub fn initial_target_for_network(network: Network) -> [u8; 48] {
    match network {
        Network::Mainnet => MAINNET_INITIAL_TARGET,
        Network::Testnet => TESTNET_INITIAL_TARGET,
        Network::Regnet => REGNET_INITIAL_TARGET,
        Network::Prunetest => PRUNETEST_INITIAL_TARGET,
    }
}

pub fn target_for_height(network: Network, height: u64) -> [u8; 48] {
    if height == 0 {
        DIFFICULTY_PROFILE.genesis_target
    } else {
        initial_target_for_network(network)
    }
}

fn target_to_biguint(target: &[u8; 48]) -> BigUint {
    BigUint::from_bytes_be(target)
}

fn biguint_to_target(value: &BigUint) -> [u8; 48] {
    let bytes = value.to_bytes_be();
    let mut out = [0u8; 48];
    let out_len = out.len();
    if bytes.len() >= out_len {
        out.copy_from_slice(&bytes[bytes.len() - out_len..]);
    } else {
        out[out_len - bytes.len()..].copy_from_slice(&bytes);
    }
    out
}

fn block_proof(target: &[u8; 48]) -> BigUint {
    let target = target_to_biguint(target);
    if target.is_zero() {
        return BigUint::zero();
    }
    let numerator = (BigUint::from(1u8) << SHA3_384_HASH_BITS) - BigUint::from(1u8) - &target;
    (numerator / (&target + BigUint::from(1u8))) + BigUint::from(1u8)
}

fn median_u64(values: &[u64]) -> u64 {
    debug_assert!(!values.is_empty());
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

fn median_time_past(headers: &[BlockHeader], index: usize) -> u64 {
    let span = POW_PROFILE.median_window_blocks as usize;
    let start = index.saturating_add(1).saturating_sub(span);
    let timestamps: Vec<u64> = headers[start..=index]
        .iter()
        .map(|header| header.timestamp)
        .collect();
    median_u64(&timestamps)
}

fn bounded_actual_timespan(actual_timespan: u64, target_timespan: u64) -> u64 {
    let target = target_timespan;
    let min_actual =
        target.saturating_mul(100u64.saturating_sub(POW_PROFILE.max_adjust_down_percent)) / 100;
    let max_actual =
        target.saturating_mul(100u64.saturating_add(POW_PROFILE.max_adjust_up_percent)) / 100;
    let damped = if actual_timespan >= target {
        target.saturating_add((actual_timespan - target) / POW_PROFILE.damping_factor)
    } else {
        target.saturating_sub((target - actual_timespan) / POW_PROFILE.damping_factor)
    };
    damped.clamp(min_actual, max_actual)
}

fn mean_target(headers: &[BlockHeader]) -> BigUint {
    let mut total = BigUint::zero();
    for header in headers {
        total += target_to_biguint(&header.difficulty_target_or_bits);
    }
    total / BigUint::from(headers.len() as u64)
}

fn next_target_from_headers(network: Network, headers: &[BlockHeader]) -> [u8; 48] {
    let headers = match headers {
        [first, rest @ ..] if first.height == 0 && !rest.is_empty() => rest,
        _ => headers,
    };
    let averaging_window = POW_PROFILE.averaging_window_blocks as usize;
    let window_len = headers.len().min(averaging_window);
    if window_len < 2 {
        return initial_target_for_network(network);
    }

    let window = &headers[headers.len() - window_len..];
    let tip_index = headers.len() - 1;
    let old_index = headers.len() - window_len;
    let actual_timespan = if window_len < averaging_window {
        // During bootstrap there is not enough history for Zcash-style MTP to be
        // representative, so use the observed first/last timestamps. Once the
        // full window exists we switch to the MTP path below.
        headers[tip_index]
            .timestamp
            .saturating_sub(headers[old_index].timestamp)
    } else {
        let tip_mtp = median_time_past(headers, tip_index);
        let old_mtp = median_time_past(headers, old_index);
        tip_mtp.saturating_sub(old_mtp)
    };
    let expected_timespan = expected_timespan_seconds_for_window(window_len);
    let bounded_timespan = bounded_actual_timespan(actual_timespan, expected_timespan);
    let average_target = mean_target(window);
    let threshold =
        (average_target * BigUint::from(bounded_timespan)) / BigUint::from(expected_timespan);
    clamp_target(biguint_to_target(&threshold))
}

pub fn target_for_next_block(network: Network, previous_blocks: &[Block]) -> [u8; 48] {
    let headers: Vec<BlockHeader> = previous_blocks
        .iter()
        .map(|block| block.header.clone())
        .collect();
    next_target_from_headers(network, &headers)
}

pub fn accumulated_chain_work(blocks: &[Block]) -> BigUint {
    let mut total = BigUint::zero();
    for block in blocks {
        total += block_proof(&block.header.difficulty_target_or_bits);
    }
    total
}

pub fn compare_branch_work(candidate: &[Block], current: &[Block]) -> Ordering {
    let candidate_work = accumulated_chain_work(candidate);
    let current_work = accumulated_chain_work(current);
    candidate_work
        .cmp(&current_work)
        .then_with(|| {
            candidate
                .last()
                .map(|block| block.header.height)
                .cmp(&current.last().map(|block| block.header.height))
        })
        .then_with(|| {
            current
                .last()
                .map(|block| block.header.block_hash())
                .cmp(&candidate.last().map(|block| block.header.block_hash()))
        })
}

pub fn branch_is_preferred(candidate: &[Block], current: &[Block]) -> bool {
    compare_branch_work(candidate, current).is_gt()
}

pub fn median_time_past_from_blocks(previous_blocks: &[Block]) -> Option<u64> {
    if previous_blocks.is_empty() {
        return None;
    }
    let headers: Vec<BlockHeader> = previous_blocks
        .iter()
        .map(|block| block.header.clone())
        .collect();
    Some(median_time_past(&headers, headers.len() - 1))
}

pub fn minimum_next_block_timestamp(previous_blocks: &[Block]) -> Option<u64> {
    median_time_past_from_blocks(previous_blocks).map(|timestamp| timestamp.saturating_add(1))
}

pub fn target_within_bounds(target: &[u8; 48]) -> bool {
    target >= &DIFFICULTY_PROFILE.max_difficulty_target
        && target <= &DIFFICULTY_PROFILE.min_difficulty_target
}

pub fn meets_target(hash: &[u8; 48], target: &[u8; 48]) -> bool {
    hash <= target
}

pub fn clamp_target(target: [u8; 48]) -> [u8; 48] {
    if target < DIFFICULTY_PROFILE.max_difficulty_target {
        DIFFICULTY_PROFILE.max_difficulty_target
    } else if target > DIFFICULTY_PROFILE.min_difficulty_target {
        DIFFICULTY_PROFILE.min_difficulty_target
    } else {
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pow_profile_matches_reference_timing() {
        assert_eq!(POW_PROFILE.target_block_time_seconds, 75);
        assert_eq!(POW_PROFILE.retarget_interval_blocks, 1);
        assert_eq!(POW_PROFILE.averaging_window_blocks, 17);
        assert_eq!(POW_PROFILE.median_window_blocks, 11);
        assert_eq!(POW_PROFILE.damping_factor, 4);
        assert_eq!(POW_PROFILE.max_adjust_up_percent, 16);
        assert_eq!(POW_PROFILE.max_adjust_down_percent, 32);
        assert_eq!(SHA3_384_HASH_BITS, 384);
        assert_eq!(SHA3_384_HASH_HEX_CHARS, 96);
        assert_eq!(
            DIFFICULTY_PROFILE.standard_transaction_allocation_bps,
            9_500
        );
    }

    #[test]
    fn expected_timespan_is_derived_from_target_block_time() {
        assert_eq!(expected_timespan_seconds(), 1_200);
    }

    #[test]
    fn clamp_difficulty_delta_limits_changes() {
        assert_eq!(clamp_difficulty_delta(10, 3, 12), 12);
        assert_eq!(clamp_difficulty_delta(10, -20, 12), -10);
        assert_eq!(clamp_difficulty_delta(0, 5, 12), 5);
    }

    #[test]
    fn difficulty_targets_clamp_and_validate() {
        assert!(target_within_bounds(&initial_target_for_network(
            Network::Mainnet
        )));
        assert!(target_within_bounds(&target_for_height(
            Network::Mainnet,
            0
        )));
        assert!(target_within_bounds(&initial_target_for_network(
            Network::Testnet
        )));
        assert!(target_within_bounds(&initial_target_for_network(
            Network::Regnet
        )));
        assert!(target_within_bounds(&initial_target_for_network(
            Network::Prunetest
        )));
        assert_eq!(
            clamp_target([0xFF; 48]),
            DIFFICULTY_PROFILE.min_difficulty_target
        );
        assert_eq!(
            clamp_target([0x00; 48]),
            DIFFICULTY_PROFILE.max_difficulty_target
        );
    }

    #[test]
    fn next_target_stays_within_bounds() {
        let mut blocks = Vec::new();
        for height in 0..POW_PROFILE.averaging_window_blocks {
            blocks.push(Block {
                header: BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash: [0; 48],
                    merkle_root: [0; 48],
                    witness_root: [0; 48],
                    timestamp: height.saturating_mul(POW_PROFILE.target_block_time_seconds),
                    difficulty_target_or_bits: initial_target_for_network(Network::Mainnet),
                    nonce: 0,
                },
                ..Block::default()
            });
        }

        let target = target_for_next_block(Network::Mainnet, &blocks);
        assert!(target_within_bounds(&target));
    }

    #[test]
    fn next_target_changes_with_block_spacing() {
        let mut on_time = Vec::new();
        let mut slow = Vec::new();
        let starting_target = DIFFICULTY_PROFILE.max_difficulty_target;
        for height in 0..(POW_PROFILE.averaging_window_blocks + POW_PROFILE.median_window_blocks) {
            on_time.push(Block {
                header: BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash: [0; 48],
                    merkle_root: [0; 48],
                    witness_root: [0; 48],
                    timestamp: height.saturating_mul(POW_PROFILE.target_block_time_seconds),
                    difficulty_target_or_bits: starting_target,
                    nonce: 0,
                },
                ..Block::default()
            });
            slow.push(Block {
                header: BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash: [0; 48],
                    merkle_root: [0; 48],
                    witness_root: [0; 48],
                    timestamp: height.saturating_mul(POW_PROFILE.target_block_time_seconds * 2),
                    difficulty_target_or_bits: starting_target,
                    nonce: 0,
                },
                ..Block::default()
            });
        }

        let on_time_target = target_for_next_block(Network::Mainnet, &on_time);
        let slow_target = target_for_next_block(Network::Mainnet, &slow);
        assert!(target_within_bounds(&on_time_target));
        assert!(target_within_bounds(&slow_target));
        assert!(slow_target > on_time_target);
    }

    #[test]
    fn bootstrap_target_changes_before_full_window() {
        let mut fast = Vec::new();
        let mut on_time = Vec::new();
        let starting_target = DIFFICULTY_PROFILE.min_difficulty_target;
        fast.push(Block {
            header: BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 0,
                previous_block_hash: [0; 48],
                merkle_root: [0; 48],
                witness_root: [0; 48],
                timestamp: 1_000,
                difficulty_target_or_bits: starting_target,
                nonce: 0,
            },
            ..Block::default()
        });
        on_time.push(Block {
            header: BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 0,
                previous_block_hash: [0; 48],
                merkle_root: [0; 48],
                witness_root: [0; 48],
                timestamp: 1_000,
                difficulty_target_or_bits: starting_target,
                nonce: 0,
            },
            ..Block::default()
        });

        for height in 1..=8 {
            fast.push(Block {
                header: BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash: [0; 48],
                    merkle_root: [0; 48],
                    witness_root: [0; 48],
                    timestamp: 1_000_000 + height,
                    difficulty_target_or_bits: starting_target,
                    nonce: 0,
                },
                ..Block::default()
            });
            on_time.push(Block {
                header: BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash: [0; 48],
                    merkle_root: [0; 48],
                    witness_root: [0; 48],
                    timestamp: 1_000 + height.saturating_mul(POW_PROFILE.target_block_time_seconds),
                    difficulty_target_or_bits: starting_target,
                    nonce: 0,
                },
                ..Block::default()
            });
        }

        let fast_target = target_for_next_block(Network::Mainnet, &fast);
        let on_time_target = target_for_next_block(Network::Mainnet, &on_time);
        assert!(fast_target < starting_target);
        assert_eq!(on_time_target, starting_target);
        assert!(fast_target < on_time_target);
    }

    #[test]
    fn minimum_next_block_timestamp_tracks_median_time_past() {
        let mut blocks = Vec::new();
        for height in 0..12 {
            blocks.push(Block {
                header: BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash: [0; 48],
                    merkle_root: [0; 48],
                    witness_root: [0; 48],
                    timestamp: 1_000 + height,
                    difficulty_target_or_bits: initial_target_for_network(Network::Mainnet),
                    nonce: 0,
                },
                ..Block::default()
            });
        }

        assert_eq!(median_time_past_from_blocks(&blocks), Some(1_006));
        assert_eq!(minimum_next_block_timestamp(&blocks), Some(1_007));
    }

    #[test]
    fn higher_work_branch_is_preferred_deterministically() {
        let candidate = vec![Block {
            header: BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: [0; 48],
                witness_root: [0; 48],
                timestamp: 1,
                difficulty_target_or_bits: DIFFICULTY_PROFILE.max_difficulty_target,
                nonce: 0,
            },
            ..Block::default()
        }];
        let current = vec![Block {
            header: BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: [0; 48],
                witness_root: [0; 48],
                timestamp: 1,
                difficulty_target_or_bits: DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            },
            ..Block::default()
        }];
        assert!(branch_is_preferred(&candidate, &current));
    }
}
