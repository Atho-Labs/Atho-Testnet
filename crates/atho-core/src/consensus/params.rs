use crate::constants::{
    BLOCK_TIME_SECONDS, HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_ATHO, MAX_SUPPLY_ATHO,
    MIN_TX_FEE_ATOMS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusParams {
    pub max_supply_atho: u64,
    pub initial_block_reward_atho: u64,
    pub halving_interval_blocks: u64,
    pub min_tx_fee_atoms: u64,
    pub block_time_seconds: u64,
}

pub const CONSENSUS_PARAMS: ConsensusParams = ConsensusParams {
    max_supply_atho: MAX_SUPPLY_ATHO,
    initial_block_reward_atho: INITIAL_BLOCK_REWARD_ATHO,
    halving_interval_blocks: HALVING_INTERVAL_BLOCKS,
    min_tx_fee_atoms: MIN_TX_FEE_ATOMS,
    block_time_seconds: BLOCK_TIME_SECONDS,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consensus_params_are_frozen() {
        assert_eq!(CONSENSUS_PARAMS.max_supply_atho, 168_000_000);
        assert_eq!(CONSENSUS_PARAMS.initial_block_reward_atho, 50);
        assert_eq!(CONSENSUS_PARAMS.halving_interval_blocks, 1_680_000);
        assert_eq!(CONSENSUS_PARAMS.min_tx_fee_atoms, 500);
        assert_eq!(CONSENSUS_PARAMS.block_time_seconds, 75);
    }
}
