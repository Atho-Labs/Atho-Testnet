// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use crate::constants::{
    BLOCKS_PER_YEAR, HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_ATOMS, TAIL_REWARD_ATOMS,
};
use crate::network::Network;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmissionSchedule {
    pub initial_block_reward_atoms: u64,
    pub tail_reward_atoms: u64,
    pub halving_interval_blocks: u64,
    pub blocks_per_year: u64,
}

pub const EMISSION_SCHEDULE: EmissionSchedule = EmissionSchedule {
    initial_block_reward_atoms: INITIAL_BLOCK_REWARD_ATOMS,
    tail_reward_atoms: TAIL_REWARD_ATOMS,
    halving_interval_blocks: HALVING_INTERVAL_BLOCKS,
    blocks_per_year: BLOCKS_PER_YEAR,
};

pub const TAIL_EMISSION_START_HEIGHT: u64 = EMISSION_SCHEDULE.halving_interval_blocks * 3;
pub const YEAR_20_HEIGHT: u64 = EMISSION_SCHEDULE.blocks_per_year * 20;

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

pub fn block_subsidy_atoms(height: u64) -> u64 {
    get_block_reward_atoms(height)
}

pub fn block_subsidy_atoms_for_network(_network: Network, height: u64) -> u64 {
    get_block_reward_atoms(height)
}

pub fn genesis_coinbase_atoms_for_network(_network: Network) -> u64 {
    get_block_reward_atoms(0)
}

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

pub fn cumulative_issued_through_height(height: u64) -> u128 {
    cumulative_issued_before_height(height.saturating_add(1))
}

pub fn cumulative_issued_before_height_for_network(_network: Network, height: u64) -> u128 {
    cumulative_issued_before_height(height)
}

pub fn cumulative_issued_through_height_for_network(_network: Network, height: u64) -> u128 {
    cumulative_issued_through_height(height)
}

pub fn max_supply_atoms_for_network(_network: Network) -> Option<u128> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reward_schedule_matches_requested_boundaries() {
        assert_eq!(get_block_reward_atoms(0), 6_250_000_000_000);
        assert_eq!(get_block_reward_atoms(1_679_999), 6_250_000_000_000);
        assert_eq!(get_block_reward_atoms(1_680_000), 3_125_000_000_000);
        assert_eq!(get_block_reward_atoms(3_359_999), 3_125_000_000_000);
        assert_eq!(get_block_reward_atoms(3_360_000), 1_562_500_000_000);
        assert_eq!(get_block_reward_atoms(5_039_999), 1_562_500_000_000);
        assert_eq!(get_block_reward_atoms(5_040_000), 781_250_000_000);
        assert_eq!(get_block_reward_atoms(10_000_000), 781_250_000_000);
    }

    #[test]
    fn cumulative_supply_matches_requested_checkpoints() {
        assert_eq!(cumulative_issued_before_height(0), 0);
        assert_eq!(
            cumulative_issued_before_height(1_680_000),
            10_500_000u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
        assert_eq!(
            cumulative_issued_before_height(3_360_000),
            15_750_000u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
        assert_eq!(
            cumulative_issued_before_height(5_040_000),
            18_375_000u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
        assert_eq!(
            cumulative_issued_before_height(8_409_600),
            21_007_500u128 * crate::constants::ATOMS_PER_ATHO as u128
        );
    }

    #[test]
    fn tail_emission_identity_matches_requested_targets() {
        assert_eq!(EMISSION_SCHEDULE.blocks_per_year, 420_480);
        assert_eq!(TAIL_EMISSION_START_HEIGHT, 5_040_000);
        assert_eq!(YEAR_20_HEIGHT, 8_409_600);
        assert_eq!(
            cumulative_issued_before_height(YEAR_20_HEIGHT),
            21_007_500u128 * crate::constants::ATOMS_PER_ATHO as u128
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
            TAIL_EMISSION_START_HEIGHT - 1,
            TAIL_EMISSION_START_HEIGHT,
            YEAR_20_HEIGHT,
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
