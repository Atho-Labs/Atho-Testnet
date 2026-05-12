use crate::constants::{
    ATOMS_PER_ATHO, BLOCK_TIME_SECONDS, COINBASE_MATURITY_BLOCKS, DECIMALS,
    HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_ATOMS, MIN_TX_FEE_ATOMS,
    STANDARD_TX_CONFIRMATIONS,
};
use crate::network::Network;

const LEGACY_TESTNET_BLOCK_TIME_SECONDS: u64 = 75;
const LEGACY_TESTNET_HALVING_INTERVAL_BLOCKS: u64 = 1_680_000;
const LEGACY_TESTNET_INITIAL_BLOCK_REWARD_ATOMS: u64 = 6_250_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusParams {
    pub decimals: usize,
    pub atoms_per_atho: u64,
    pub max_supply_atho: Option<u64>,
    pub initial_block_reward_atoms: u64,
    pub halving_interval_blocks: u64,
    pub coinbase_maturity_blocks: u64,
    pub standard_tx_confirmations: u64,
    pub min_tx_fee_atoms: u64,
    pub block_time_seconds: u64,
}

pub const CONSENSUS_PARAMS: ConsensusParams = ConsensusParams {
    decimals: DECIMALS,
    atoms_per_atho: ATOMS_PER_ATHO,
    max_supply_atho: None,
    initial_block_reward_atoms: INITIAL_BLOCK_REWARD_ATOMS,
    halving_interval_blocks: HALVING_INTERVAL_BLOCKS,
    coinbase_maturity_blocks: COINBASE_MATURITY_BLOCKS,
    standard_tx_confirmations: STANDARD_TX_CONFIRMATIONS,
    min_tx_fee_atoms: MIN_TX_FEE_ATOMS,
    block_time_seconds: BLOCK_TIME_SECONDS,
};

pub const fn consensus_params_for_network(network: Network) -> ConsensusParams {
    match network {
        Network::Mainnet => CONSENSUS_PARAMS,
        Network::Testnet => ConsensusParams {
            initial_block_reward_atoms: LEGACY_TESTNET_INITIAL_BLOCK_REWARD_ATOMS,
            halving_interval_blocks: LEGACY_TESTNET_HALVING_INTERVAL_BLOCKS,
            coinbase_maturity_blocks: COINBASE_MATURITY_BLOCKS,
            standard_tx_confirmations: 1,
            block_time_seconds: LEGACY_TESTNET_BLOCK_TIME_SECONDS,
            ..CONSENSUS_PARAMS
        },
        Network::Regnet => CONSENSUS_PARAMS,
        Network::Prunetest => ConsensusParams {
            initial_block_reward_atoms: LEGACY_TESTNET_INITIAL_BLOCK_REWARD_ATOMS,
            halving_interval_blocks: LEGACY_TESTNET_HALVING_INTERVAL_BLOCKS,
            block_time_seconds: LEGACY_TESTNET_BLOCK_TIME_SECONDS,
            ..CONSENSUS_PARAMS
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consensus_params_are_frozen() {
        let params = consensus_params_for_network(Network::Mainnet);
        assert_eq!(params.decimals, DECIMALS);
        assert_eq!(params.atoms_per_atho, ATOMS_PER_ATHO);
        assert_eq!(params.max_supply_atho, None);
        assert_eq!(
            params.initial_block_reward_atoms,
            INITIAL_BLOCK_REWARD_ATOMS
        );
        assert_eq!(params.halving_interval_blocks, 1_260_000);
        assert_eq!(params.coinbase_maturity_blocks, 150);
        assert_eq!(params.standard_tx_confirmations, 6);
        assert_eq!(params.min_tx_fee_atoms, 500);
        assert_eq!(params.block_time_seconds, 100);
    }

    #[test]
    fn testnet_preserves_legacy_monetary_policy_without_touching_fast_confirmations() {
        let mainnet = consensus_params_for_network(Network::Mainnet);
        let testnet = consensus_params_for_network(Network::Testnet);

        assert_eq!(mainnet.coinbase_maturity_blocks, COINBASE_MATURITY_BLOCKS);
        assert_eq!(mainnet.standard_tx_confirmations, STANDARD_TX_CONFIRMATIONS);
        assert_eq!(testnet.coinbase_maturity_blocks, COINBASE_MATURITY_BLOCKS);
        assert_eq!(testnet.standard_tx_confirmations, 1);
        assert_eq!(
            testnet.initial_block_reward_atoms,
            LEGACY_TESTNET_INITIAL_BLOCK_REWARD_ATOMS
        );
        assert_eq!(
            testnet.halving_interval_blocks,
            LEGACY_TESTNET_HALVING_INTERVAL_BLOCKS
        );
        assert_eq!(
            testnet.block_time_seconds,
            LEGACY_TESTNET_BLOCK_TIME_SECONDS
        );
        assert_ne!(
            testnet.initial_block_reward_atoms,
            mainnet.initial_block_reward_atoms
        );
        assert_ne!(
            testnet.halving_interval_blocks,
            mainnet.halving_interval_blocks
        );
        assert_ne!(testnet.block_time_seconds, mainnet.block_time_seconds);
    }
}
