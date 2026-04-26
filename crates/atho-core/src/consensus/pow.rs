use crate::constants::{
    POW_AVERAGING_WINDOW_BLOCKS, POW_DAMPING_FACTOR, POW_MAX_ADJUST_DOWN_PERCENT,
    POW_MAX_ADJUST_UP_PERCENT, POW_MEDIAN_WINDOW_BLOCKS, POW_RETARGET_INTERVAL_BLOCKS,
    TARGET_BLOCK_TIME_SECONDS,
};
use crate::network::Network;
use hex_literal::hex;

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
    genesis_target: hex!("000000FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
    min_difficulty_target: hex!("000000FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
    max_difficulty_target: hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000FFF"),
    standard_transaction_allocation_bps: 9_500,
};

pub const MAINNET_INITIAL_TARGET: [u8; 48] = hex!("0000003FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
pub const TESTNET_INITIAL_TARGET: [u8; 48] = hex!("000000466666666666680000000000000000000000000000000000000000000000000000000000000000000000000000");
pub const REGNET_INITIAL_TARGET: [u8; 48] = hex!("0000004CCCCCCCCCCCD00000000000000000000000000000000000000000000000000000000000000000000000000000");

pub fn expected_timespan_seconds() -> u64 {
    POW_PROFILE.target_block_time_seconds * POW_PROFILE.averaging_window_blocks
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
    }
}

pub fn target_for_height(network: Network, height: u64) -> [u8; 48] {
    if height == 0 {
        DIFFICULTY_PROFILE.genesis_target
    } else {
        initial_target_for_network(network)
    }
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
        assert_eq!(expected_timespan_seconds(), 1_275);
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
        assert_eq!(
            clamp_target([0xFF; 48]),
            DIFFICULTY_PROFILE.min_difficulty_target
        );
        assert_eq!(
            clamp_target([0x00; 48]),
            DIFFICULTY_PROFILE.max_difficulty_target
        );
    }
}
