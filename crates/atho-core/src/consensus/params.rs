use crate::constants::{
    ATOMS_PER_ATHO, BLOCK_TIME_SECONDS, COINBASE_MATURITY_BLOCKS, DECIMALS,
    HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_ATOMS, MIN_TX_FEE_ATOMS,
    STANDARD_TX_CONFIRMATIONS,
};
use crate::network::Network;

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
            coinbase_maturity_blocks: 2,
            standard_tx_confirmations: 1,
            ..CONSENSUS_PARAMS
        },
        Network::Regnet | Network::Prunetest => CONSENSUS_PARAMS,
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
        assert_eq!(params.halving_interval_blocks, 1_680_000);
        assert_eq!(params.coinbase_maturity_blocks, 150);
        assert_eq!(params.standard_tx_confirmations, 7);
        assert_eq!(params.min_tx_fee_atoms, 500);
        assert_eq!(params.block_time_seconds, 75);
    }

    #[test]
    fn testnet_uses_fast_confirmation_params_without_touching_mainnet() {
        let mainnet = consensus_params_for_network(Network::Mainnet);
        let testnet = consensus_params_for_network(Network::Testnet);

        assert_eq!(mainnet.coinbase_maturity_blocks, COINBASE_MATURITY_BLOCKS);
        assert_eq!(mainnet.standard_tx_confirmations, STANDARD_TX_CONFIRMATIONS);
        assert_eq!(testnet.coinbase_maturity_blocks, 2);
        assert_eq!(testnet.standard_tx_confirmations, 1);
        assert_eq!(
            testnet.initial_block_reward_atoms,
            mainnet.initial_block_reward_atoms
        );
        assert_eq!(
            testnet.halving_interval_blocks,
            mainnet.halving_interval_blocks
        );
    }
}
