use crate::consensus::params::CONSENSUS_PARAMS;
use crate::constants::{ATOMS_PER_ATHO, MAX_SUPPLY_ATOMS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubsidySchedule {
    pub initial_block_reward_atho: u64,
    pub halving_interval_blocks: u64,
}

pub const SUBSIDY_SCHEDULE: SubsidySchedule = SubsidySchedule {
    initial_block_reward_atho: CONSENSUS_PARAMS.initial_block_reward_atho,
    halving_interval_blocks: CONSENSUS_PARAMS.halving_interval_blocks,
};

pub fn block_subsidy_atho(height: u64) -> u64 {
    let halvings = height / SUBSIDY_SCHEDULE.halving_interval_blocks;
    if halvings >= 64 {
        return 0;
    }
    SUBSIDY_SCHEDULE.initial_block_reward_atho >> halvings
}

pub fn block_subsidy_atoms(height: u64) -> u64 {
    block_subsidy_atho(height).saturating_mul(ATOMS_PER_ATHO)
}

pub fn cumulative_subsidy_atho(height: u64) -> u64 {
    let mut remaining_blocks = height.saturating_add(1);
    let mut reward = SUBSIDY_SCHEDULE.initial_block_reward_atho;
    let interval = SUBSIDY_SCHEDULE.halving_interval_blocks;
    let mut total = 0u64;

    for _ in 0..64 {
        if reward == 0 || remaining_blocks == 0 {
            break;
        }
        let blocks = remaining_blocks.min(interval);
        total = total.saturating_add(blocks.saturating_mul(reward));
        remaining_blocks = remaining_blocks.saturating_sub(blocks);
        reward >>= 1;
    }

    total
}

pub fn cumulative_subsidy_atoms(height: u64) -> u64 {
    cumulative_subsidy_atho(height).saturating_mul(ATOMS_PER_ATHO)
}

pub fn max_supply_atoms() -> u64 {
    MAX_SUPPLY_ATOMS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsidy_starts_at_fifty_atho() {
        assert_eq!(block_subsidy_atho(0), 50);
        assert_eq!(block_subsidy_atoms(0), 50 * ATOMS_PER_ATHO);
    }

    #[test]
    fn subsidy_halves_on_schedule() {
        assert_eq!(block_subsidy_atho(1_679_999), 50);
        assert_eq!(block_subsidy_atho(1_680_000), 25);
    }

    #[test]
    fn cumulative_subsidy_is_bounded_by_max_supply() {
        assert_eq!(cumulative_subsidy_atho(0), 50);
        assert!(cumulative_subsidy_atoms(1_680_000) < max_supply_atoms());
        assert!(cumulative_subsidy_atoms(u64::MAX / 2) <= max_supply_atoms());
    }
}
