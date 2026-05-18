// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Frozen consensus constants exposed as a structured parameter set.

use crate::constants::{
    ATOMS_PER_ATHO, BLOCK_TIME_SECONDS, COINBASE_MATURITY_BLOCKS, DECIMALS,
    HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_ATOMS, MIN_TX_FEE_ATOMS,
    STANDARD_TX_CONFIRMATIONS,
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
    /// Default wallet-facing confirmation threshold for standard payments.
    pub standard_tx_confirmations: u64,
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
    standard_tx_confirmations: STANDARD_TX_CONFIRMATIONS,
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
        assert_eq!(params.standard_tx_confirmations, 6);
        assert_eq!(params.min_tx_fee_atoms, 500);
        assert_eq!(params.block_time_seconds, 100);
    }

    #[test]
    fn all_networks_share_current_confirmation_and_maturity_policy() {
        let mainnet = consensus_params_for_network(Network::Mainnet);
        let testnet = consensus_params_for_network(Network::Testnet);

        assert_eq!(mainnet.coinbase_maturity_blocks, COINBASE_MATURITY_BLOCKS);
        assert_eq!(mainnet.standard_tx_confirmations, STANDARD_TX_CONFIRMATIONS);
        assert_eq!(testnet.coinbase_maturity_blocks, COINBASE_MATURITY_BLOCKS);
        assert_eq!(testnet.standard_tx_confirmations, STANDARD_TX_CONFIRMATIONS);
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
