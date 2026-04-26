use crate::consensus::params::CONSENSUS_PARAMS;
use crate::constants::ATOMS_PER_ATHO;

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
}
