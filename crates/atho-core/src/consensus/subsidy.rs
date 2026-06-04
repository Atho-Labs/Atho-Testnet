// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Block reward and long-horizon issuance helpers.
//!
//! This module centralizes Atho's emission schedule so mining, validation, and
//! wallet display code all reason about the same subsidy curve.

use crate::constants::{
    BLOCKS_PER_YEAR, HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_ATOMS, TAIL_REWARD_ATOMS,
};
use crate::network::Network;

/// Frozen description of the chain-wide emission schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmissionSchedule {
    /// Reward at height zero before any halving has occurred.
    pub initial_block_reward_atoms: u64,
    /// Minimum perpetual reward once halving reaches the tail-emission era.
    pub tail_reward_atoms: u64,
    /// Number of blocks between halvings.
    pub halving_interval_blocks: u64,
    /// Expected block production for a nominal year at the target block time.
    pub blocks_per_year: u64,
}

/// Canonical emission parameters shared by all currently supported networks.
pub const EMISSION_SCHEDULE: EmissionSchedule = EmissionSchedule {
    initial_block_reward_atoms: INITIAL_BLOCK_REWARD_ATOMS,
    tail_reward_atoms: TAIL_REWARD_ATOMS,
    halving_interval_blocks: HALVING_INTERVAL_BLOCKS,
    blocks_per_year: BLOCKS_PER_YEAR,
};

pub const TAIL_EMISSION_START_HEIGHT: u64 = EMISSION_SCHEDULE.halving_interval_blocks * 7;
pub const YEAR_20_HEIGHT: u64 = EMISSION_SCHEDULE.blocks_per_year * 20;

/// Returns the mined reward at `height`, including the permanent tail emission.
pub fn get_block_reward_atoms(height: u64) -> u64 {
    let halvings = height / HALVING_INTERVAL_BLOCKS;
    let reward = if halvings >= 64 {
        0
    } else {
        INITIAL_BLOCK_REWARD_ATOMS >> halvings
    };

    if reward < TAIL_REWARD_ATOMS {
        TAIL_REWARD_ATOMS
    } else {
        reward
    }
}

/// Alias retained for consensus-call-site clarity.
pub fn block_subsidy_atoms(height: u64) -> u64 {
    get_block_reward_atoms(height)
}

/// Network-specific reward accessor.
///
/// Atho currently shares one subsidy schedule across all networks, but this
/// wrapper preserves a stable call shape if a future network diverges.
pub fn block_subsidy_atoms_for_network(_network: Network, height: u64) -> u64 {
    get_block_reward_atoms(height)
}

/// Returns the genesis coinbase amount for the selected network.
pub fn genesis_coinbase_atoms_for_network(_network: Network) -> u64 {
    get_block_reward_atoms(0)
}

/// Computes total issuance strictly before `height`.
pub fn cumulative_issued_before_height(height: u64) -> u128 {
    if height == 0 {
        return 0;
    }

    let mut remaining_blocks = height;
    let mut issued = 0u128;
    let mut reward = INITIAL_BLOCK_REWARD_ATOMS;

    while remaining_blocks > 0 {
        let era_blocks = remaining_blocks.min(HALVING_INTERVAL_BLOCKS);
        let effective_reward = reward.max(TAIL_REWARD_ATOMS);
        issued = issued.saturating_add((era_blocks as u128) * (effective_reward as u128));
        remaining_blocks -= era_blocks;

        if reward > TAIL_REWARD_ATOMS {
            reward = (reward / 2).max(TAIL_REWARD_ATOMS);
        } else {
            reward = TAIL_REWARD_ATOMS;
        }
    }

    issued
}

/// Computes total issuance including the block at `height`.
pub fn cumulative_issued_through_height(height: u64) -> u128 {
    cumulative_issued_before_height(height.saturating_add(1))
}

/// Network-scoped issuance accessor.
pub fn cumulative_issued_before_height_for_network(_network: Network, height: u64) -> u128 {
    cumulative_issued_before_height(height)
}

/// Network-scoped issuance accessor including `height`.
pub fn cumulative_issued_through_height_for_network(_network: Network, height: u64) -> u128 {
    cumulative_issued_through_height(height)
}

/// Returns the configured maximum supply, if the network has a hard cap.
///
/// Atho currently uses indefinite tail emission, so there is no finite cap.
pub fn max_supply_atoms_for_network(_network: Network) -> Option<u128> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reward_schedule_matches_requested_boundaries() {
        assert_eq!(get_block_reward_atoms(0), 5_000_000_000);
        assert_eq!(get_block_reward_atoms(1_259_999), 5_000_000_000);
        assert_eq!(get_block_reward_atoms(1_260_000), 2_500_000_000);
        assert_eq!(get_block_reward_atoms(2_519_999), 2_500_000_000);
        assert_eq!(get_block_reward_atoms(2_520_000), 1_250_000_000);
        assert_eq!(get_block_reward_atoms(3_779_999), 1_250_000_000);
        assert_eq!(get_block_reward_atoms(3_780_000), 625_000_000);
        assert_eq!(get_block_reward_atoms(5_040_000), 312_500_000);
        assert_eq!(get_block_reward_atoms(6_300_000), 156_250_000);
        assert_eq!(get_block_reward_atoms(7_560_000), 78_125_000);
        assert_eq!(get_block_reward_atoms(8_819_999), 78_125_000);
        assert_eq!(get_block_reward_atoms(8_820_000), 39_062_500);
        assert_eq!(get_block_reward_atoms(10_000_000), 39_062_500);
    }

    #[test]
    fn cumulative_supply_matches_requested_checkpoints() {
        assert_eq!(cumulative_issued_before_height(0), 0);
        assert_eq!(
            cumulative_issued_before_height(1_260_000),
            63_000_000u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
        assert_eq!(
            cumulative_issued_before_height(2_520_000),
            94_500_000u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
        assert_eq!(
            cumulative_issued_before_height(3_780_000),
            110_250_000u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
        assert_eq!(
            cumulative_issued_before_height(8_820_000),
            125_015_625u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
    }

    #[test]
    fn tail_emission_identity_matches_requested_targets() {
        assert_eq!(EMISSION_SCHEDULE.blocks_per_year, 315_360);
        assert_eq!(TAIL_EMISSION_START_HEIGHT, 8_820_000);
        assert_eq!(YEAR_20_HEIGHT, 6_307_200);
        assert_eq!(
            cumulative_issued_before_height(TAIL_EMISSION_START_HEIGHT),
            125_015_625u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
        assert_eq!(
            EMISSION_SCHEDULE.blocks_per_year as u128 * EMISSION_SCHEDULE.tail_reward_atoms as u128,
            12_318_750_000_000u128
        );
    }

    #[test]
    fn no_finite_max_supply_cap_remains() {
        assert_eq!(max_supply_atoms_for_network(Network::Mainnet), None);
        assert_eq!(max_supply_atoms_for_network(Network::Testnet), None);
        assert_eq!(max_supply_atoms_for_network(Network::Regnet), None);
        assert_eq!(max_supply_atoms_for_network(Network::Prunetest), None);
    }

    #[test]
    fn long_horizon_emission_is_monotonic_and_additive_for_every_network() {
        let heights = [
            0,
            1,
            HALVING_INTERVAL_BLOCKS - 1,
            HALVING_INTERVAL_BLOCKS,
            (HALVING_INTERVAL_BLOCKS * 2) - 1,
            HALVING_INTERVAL_BLOCKS * 2,
            YEAR_20_HEIGHT,
            TAIL_EMISSION_START_HEIGHT - 1,
            TAIL_EMISSION_START_HEIGHT,
            BLOCKS_PER_YEAR * 100,
            100_000_000,
            1_000_000_000,
        ];

        for network in [
            Network::Mainnet,
            Network::Testnet,
            Network::Regnet,
            Network::Prunetest,
        ] {
            let mut previous_before = 0u128;
            for height in heights {
                let before = cumulative_issued_before_height_for_network(network, height);
                let through = cumulative_issued_through_height_for_network(network, height);
                let reward = block_subsidy_atoms_for_network(network, height) as u128;

                assert!(before >= previous_before);
                assert_eq!(through, before + reward);
                assert_eq!(
                    cumulative_issued_before_height_for_network(network, height + 1),
                    through
                );

                previous_before = before;
            }
        }
    }
}
