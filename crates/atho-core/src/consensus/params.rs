// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Frozen consensus constants exposed as a structured parameter set.

use crate::constants::{
    ATOMS_PER_ATHO, BLOCK_TIME_SECONDS, COINBASE_MATURITY_BLOCKS, DECIMALS,
    DEFAULT_SAFE_CONFIRMATIONS, DEFAULT_WALLET_MIN_CONFIRMATIONS, HALVING_INTERVAL_BLOCKS,
    HIGH_VALUE_CONFIRMATIONS, INITIAL_BLOCK_REWARD_ATOMS, MIN_TX_FEE_ATOMS,
    NORMAL_TX_VALID_AFTER_CONFIRMATIONS,
};
use crate::network::Network;

/// User-facing and validation-facing constants for a network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusParams {
    /// Number of decimal places supported by Atho amounts.
    pub decimals: usize,
    /// Smallest indivisible units in one ATHO.
    pub atoms_per_atho: u64,
    /// Finite max supply expressed in ATHO, if the network has one.
    pub max_supply_atho: Option<u64>,
    /// Initial block reward before halvings.
    pub initial_block_reward_atoms: u64,
    /// Number of blocks between halving events.
    pub halving_interval_blocks: u64,
    /// Required confirmations before coinbase outputs are spendable.
    pub coinbase_maturity_blocks: u64,
    /// Consensus confirmation count where normal transactions are valid to spend.
    pub normal_tx_valid_after_confirmations: u64,
    /// Default wallet-facing minimum confirmation filter for normal payments.
    pub default_wallet_min_confirmations: u64,
    /// Recommended wallet label threshold for stronger settlement display.
    pub default_safe_confirmations: u64,
    /// Recommended wallet/exchange threshold for high-value payments.
    pub high_value_confirmations: u64,
    /// Minimum relay/mining fee floor in atoms.
    pub min_tx_fee_atoms: u64,
    /// Target inter-block time used by scheduling and UX.
    pub block_time_seconds: u64,
}

/// Shared consensus parameters for all current Atho networks.
pub const CONSENSUS_PARAMS: ConsensusParams = ConsensusParams {
    decimals: DECIMALS,
    atoms_per_atho: ATOMS_PER_ATHO,
    max_supply_atho: None,
    initial_block_reward_atoms: INITIAL_BLOCK_REWARD_ATOMS,
    halving_interval_blocks: HALVING_INTERVAL_BLOCKS,
    coinbase_maturity_blocks: COINBASE_MATURITY_BLOCKS,
    normal_tx_valid_after_confirmations: NORMAL_TX_VALID_AFTER_CONFIRMATIONS,
    default_wallet_min_confirmations: DEFAULT_WALLET_MIN_CONFIRMATIONS,
    default_safe_confirmations: DEFAULT_SAFE_CONFIRMATIONS,
    high_value_confirmations: HIGH_VALUE_CONFIRMATIONS,
    min_tx_fee_atoms: MIN_TX_FEE_ATOMS,
    block_time_seconds: BLOCK_TIME_SECONDS,
};

/// Returns the active parameter set for `network`.
///
/// All networks currently share the same values, but this API keeps callers
/// network-aware and leaves room for future divergence.
pub const fn consensus_params_for_network(network: Network) -> ConsensusParams {
    match network {
        Network::Mainnet => CONSENSUS_PARAMS,
        Network::Testnet => CONSENSUS_PARAMS,
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
        assert_eq!(params.halving_interval_blocks, 1_260_000);
        assert_eq!(params.coinbase_maturity_blocks, 100);
        assert_eq!(params.normal_tx_valid_after_confirmations, 1);
        assert_eq!(params.default_wallet_min_confirmations, 3);
        assert_eq!(params.default_safe_confirmations, 6);
        assert_eq!(params.high_value_confirmations, 20);
        assert_eq!(params.min_tx_fee_atoms, 1);
        assert_eq!(params.block_time_seconds, 100);
    }

    #[test]
    fn all_networks_share_current_wallet_and_maturity_policy() {
        let mainnet = consensus_params_for_network(Network::Mainnet);
        let testnet = consensus_params_for_network(Network::Testnet);

        assert_eq!(mainnet.coinbase_maturity_blocks, COINBASE_MATURITY_BLOCKS);
        assert_eq!(
            mainnet.normal_tx_valid_after_confirmations,
            NORMAL_TX_VALID_AFTER_CONFIRMATIONS
        );
        assert_eq!(
            mainnet.default_wallet_min_confirmations,
            DEFAULT_WALLET_MIN_CONFIRMATIONS
        );
        assert_eq!(testnet.coinbase_maturity_blocks, COINBASE_MATURITY_BLOCKS);
        assert_eq!(
            testnet.normal_tx_valid_after_confirmations,
            NORMAL_TX_VALID_AFTER_CONFIRMATIONS
        );
        assert_eq!(
            testnet.default_wallet_min_confirmations,
            DEFAULT_WALLET_MIN_CONFIRMATIONS
        );
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
