// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Network-specific genesis block definitions.
//!
//! This module hard-codes the genesis profile for each supported Atho network
//! and exposes helpers used by startup, storage, and P2P identity checks.
//!
//! CONSENSUS: Genesis constants anchor the network identity. Changing them
//! creates a new network and invalidates all existing chain data.
use crate::block::{merkle_root, witness_root, Block, BlockHeader};
use crate::consensus::pow;
use crate::consensus::rules::{BLOCK_VERSION_V1, TRANSACTION_VERSION_V1};
use crate::consensus::subsidy;
use crate::network::Network;
use crate::transaction::{Transaction, TxOutput};
use hex_literal::hex;
use std::sync::OnceLock;

const MAINNET_GENESIS_REWARD_ADDRESS: &str =
    "ATHO9529a6358612b193cc100b4150f46235505a948caacf331b15a171993ad3124c008f45d692886ecc6417aa6ab964488c";
const MAINNET_GENESIS_REWARD_SCRIPT: [u8; 48] =
    hex!("9529a6358612b193cc100b4150f46235505a948caacf331b15a171993ad3124c008f45d692886ecc6417aa6ab964488c");
const MAINNET_GENESIS_BLOCK_VERSION: u16 = BLOCK_VERSION_V1;
const MAINNET_GENESIS_TX_VERSION: u16 = TRANSACTION_VERSION_V1;
const MAINNET_GENESIS_LOCK_TIME: u32 = 0;
const MAINNET_GENESIS_TIMESTAMP: u64 = 1_773_360_488;
const MAINNET_GENESIS_TARGET: [u8; 48] = pow::DIFFICULTY_PROFILE.genesis_target;
const MAINNET_GENESIS_NONCE: u64 = 20_164;
const MAINNET_GENESIS_COINBASE_TXID: [u8; 48] =
    hex!("d5433104080b7866ca0a0e0cf4500b5312f1a97db394765eacb1f267caa1552b9ee2191704bfbdcf777cc55f2c861470");
const MAINNET_GENESIS_BLOCK_HASH: [u8; 48] =
    hex!("000049993ab4e8874c71e35c659756c3f13d17f5e688a1271800704009017a8d0f69d9b5d8da7d7e398f720b037fd2c8");

const TESTNET_GENESIS_REWARD_ADDRESS: &str =
    "ATHT22b5382e49b9a2dafb0d2c7b1c2afe643a3c14a23f7a90e4e5dce0162b754623eb5566c3ca1348187e5f3e92c65c76ee";
const TESTNET_GENESIS_REWARD_SCRIPT: [u8; 48] =
    hex!("22b5382e49b9a2dafb0d2c7b1c2afe643a3c14a23f7a90e4e5dce0162b754623eb5566c3ca1348187e5f3e92c65c76ee");
const TESTNET_GENESIS_BLOCK_VERSION: u16 = BLOCK_VERSION_V1;
const TESTNET_GENESIS_TX_VERSION: u16 = TRANSACTION_VERSION_V1;
const TESTNET_GENESIS_LOCK_TIME: u32 = 0;
const TESTNET_GENESIS_TIMESTAMP: u64 = 1_773_360_489;
const TESTNET_GENESIS_TARGET: [u8; 48] = pow::DIFFICULTY_PROFILE.genesis_target;
const TESTNET_GENESIS_NONCE: u64 = 6_632;
const TESTNET_GENESIS_COINBASE_TXID: [u8; 48] =
    hex!("0da7f2bc20dff426397990d053f60534d27630e0b2e51ebdf9317a58c0507afb22c315e9ac440085944d451075caecd3");
const TESTNET_GENESIS_BLOCK_HASH: [u8; 48] =
    hex!("0000860604effa76502794b0d0f4b4d0c3a08dec8ac0fd2fe1c05e235b31efe0144d8c9003478c5495e7566edecb40be");

const REGNET_GENESIS_REWARD_ADDRESS: &str = TESTNET_GENESIS_REWARD_ADDRESS;
const REGNET_GENESIS_REWARD_SCRIPT: [u8; 48] = TESTNET_GENESIS_REWARD_SCRIPT;
const REGNET_GENESIS_BLOCK_VERSION: u16 = BLOCK_VERSION_V1;
const REGNET_GENESIS_TX_VERSION: u16 = TRANSACTION_VERSION_V1;
const REGNET_GENESIS_LOCK_TIME: u32 = 0;
const REGNET_GENESIS_TIMESTAMP: u64 = TESTNET_GENESIS_TIMESTAMP;
const REGNET_GENESIS_TARGET: [u8; 48] = pow::DIFFICULTY_PROFILE.genesis_target;
const REGNET_GENESIS_NONCE: u64 = 87_837;
const REGNET_GENESIS_COINBASE_TXID: [u8; 48] = TESTNET_GENESIS_COINBASE_TXID;
const REGNET_GENESIS_BLOCK_HASH: [u8; 48] =
    hex!("00005f08e72e60f4f87c4e11e24fe9b01b32b591df06e7dab2a21239be7efd75d43c0aee5573f69a4067a6117cc3ffdc");

const PRUNETEST_GENESIS_REWARD_ADDRESS: &str =
    "ATHP22b5382e49b9a2dafb0d2c7b1c2afe643a3c14a23f7a90e4e5dce0162b754623eb5566c3ca1348187e5f3e92c65c76ee";
const PRUNETEST_GENESIS_REWARD_SCRIPT: [u8; 48] = TESTNET_GENESIS_REWARD_SCRIPT;
const PRUNETEST_GENESIS_BLOCK_VERSION: u16 = BLOCK_VERSION_V1;
const PRUNETEST_GENESIS_TX_VERSION: u16 = TRANSACTION_VERSION_V1;
const PRUNETEST_GENESIS_LOCK_TIME: u32 = 0;
const PRUNETEST_GENESIS_TIMESTAMP: u64 = 1_773_360_490;
const PRUNETEST_GENESIS_TARGET: [u8; 48] = pow::PRUNETEST_INITIAL_TARGET;
const PRUNETEST_GENESIS_NONCE: u64 = 48_371;
const PRUNETEST_GENESIS_COINBASE_TXID: [u8; 48] = TESTNET_GENESIS_COINBASE_TXID;
const PRUNETEST_GENESIS_BLOCK_HASH: [u8; 48] =
    hex!("0000b4f337af0e8119b30529a5e6bde81251a7ec579cc24e397f8ee117e63279765c000bb9d881b9e571fb573b56d72d");

/// Fully materialized genesis state for one Atho network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisState {
    pub network: Network,
    pub block: Block,
    pub block_hash: [u8; 48],
    pub coinbase_txid: [u8; 48],
    pub reward_address: String,
    pub utxo_flag: &'static str,
}

/// Regenerated genesis profile including the solved nonce and block hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisProfile {
    pub network: Network,
    pub reward_address: String,
    pub reward_script: [u8; 48],
    pub block_version: u16,
    pub tx_version: u16,
    pub lock_time: u32,
    pub timestamp: u64,
    pub target: [u8; 48],
    pub coinbase_txid: [u8; 48],
    pub merkle_root: [u8; 48],
    pub witness_root: [u8; 48],
    pub nonce: u64,
    pub block_hash: [u8; 48],
}

/// Returns the hard-coded genesis state for the selected network.
pub fn genesis_state(network: Network) -> GenesisState {
    match network {
        Network::Mainnet => mainnet(),
        Network::Testnet => testnet(),
        Network::Regnet => regnet(),
        Network::Prunetest => prunetest(),
    }
}

/// Returns the canonical genesis block for the selected network.
pub fn genesis_block(network: Network) -> Block {
    genesis_state(network).block
}

/// Returns the canonical genesis block hash for the selected network.
pub fn genesis_hash(network: Network) -> [u8; 48] {
    genesis_state(network).block_hash
}

/// Returns the genesis coinbase txid for the selected network.
pub fn genesis_coinbase_txid(network: Network) -> [u8; 48] {
    genesis_state(network).coinbase_txid
}

/// Returns the genesis reward address encoded for the selected network.
pub fn genesis_reward_address(network: Network) -> String {
    genesis_state(network).reward_address
}

/// Returns the network-specific UTXO tag seeded by genesis.
pub fn genesis_utxo_flag(network: Network) -> &'static str {
    genesis_state(network).utxo_flag
}

/// Returns the genesis coinbase value in atoms.
pub fn genesis_utxo_value(network: Network) -> u64 {
    subsidy::genesis_coinbase_atoms_for_network(network)
}

/// Re-solves the genesis profile from the static parameters.
///
/// This helper is for development tooling and verification. Production nodes
/// use the fixed constants above and do not mine genesis at startup.
pub fn regenerate_genesis_profile(network: Network) -> GenesisProfile {
    let (reward_address, reward_script, block_version, tx_version, lock_time, timestamp, target) =
        match network {
            Network::Mainnet => (
                MAINNET_GENESIS_REWARD_ADDRESS,
                MAINNET_GENESIS_REWARD_SCRIPT,
                MAINNET_GENESIS_BLOCK_VERSION,
                MAINNET_GENESIS_TX_VERSION,
                MAINNET_GENESIS_LOCK_TIME,
                MAINNET_GENESIS_TIMESTAMP,
                MAINNET_GENESIS_TARGET,
            ),
            Network::Testnet => (
                TESTNET_GENESIS_REWARD_ADDRESS,
                TESTNET_GENESIS_REWARD_SCRIPT,
                TESTNET_GENESIS_BLOCK_VERSION,
                TESTNET_GENESIS_TX_VERSION,
                TESTNET_GENESIS_LOCK_TIME,
                TESTNET_GENESIS_TIMESTAMP,
                TESTNET_GENESIS_TARGET,
            ),
            Network::Regnet => (
                REGNET_GENESIS_REWARD_ADDRESS,
                REGNET_GENESIS_REWARD_SCRIPT,
                REGNET_GENESIS_BLOCK_VERSION,
                REGNET_GENESIS_TX_VERSION,
                REGNET_GENESIS_LOCK_TIME,
                REGNET_GENESIS_TIMESTAMP,
                REGNET_GENESIS_TARGET,
            ),
            Network::Prunetest => (
                PRUNETEST_GENESIS_REWARD_ADDRESS,
                PRUNETEST_GENESIS_REWARD_SCRIPT,
                PRUNETEST_GENESIS_BLOCK_VERSION,
                PRUNETEST_GENESIS_TX_VERSION,
                PRUNETEST_GENESIS_LOCK_TIME,
                PRUNETEST_GENESIS_TIMESTAMP,
                PRUNETEST_GENESIS_TARGET,
            ),
        };

    let coinbase = Transaction {
        version: tx_version,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::genesis_coinbase_atoms_for_network(network),
            locking_script: reward_script.to_vec(),
        }],
        lock_time,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let coinbase_txid = coinbase.txid();
    let transactions = vec![coinbase];
    let merkle_root = merkle_root(&transactions);
    let witness_root = witness_root(&transactions);

    let mut header = BlockHeader {
        version: block_version,
        network_id: network,
        height: 0,
        previous_block_hash: [0; 48],
        merkle_root,
        witness_root,
        timestamp,
        difficulty_target_or_bits: target,
        nonce: 0,
    };

    loop {
        let block_hash = header.block_hash();
        if pow::meets_target(&block_hash, &target) {
            return GenesisProfile {
                network,
                reward_address: reward_address.to_string(),
                reward_script,
                block_version,
                tx_version,
                lock_time,
                timestamp,
                target,
                coinbase_txid,
                merkle_root,
                witness_root,
                nonce: header.nonce,
                block_hash,
            };
        }
        header.nonce = header.nonce.wrapping_add(1);
    }
}

fn mainnet() -> GenesisState {
    static STATE: OnceLock<GenesisState> = OnceLock::new();
    STATE
        .get_or_init(|| {
            genesis_state_from_parts(GenesisParts {
                network: Network::Mainnet,
                reward_address: MAINNET_GENESIS_REWARD_ADDRESS,
                reward_script: MAINNET_GENESIS_REWARD_SCRIPT,
                block_version: MAINNET_GENESIS_BLOCK_VERSION,
                tx_version: MAINNET_GENESIS_TX_VERSION,
                lock_time: MAINNET_GENESIS_LOCK_TIME,
                timestamp: MAINNET_GENESIS_TIMESTAMP,
                nonce: MAINNET_GENESIS_NONCE,
                target: MAINNET_GENESIS_TARGET,
                expected_coinbase_txid: MAINNET_GENESIS_COINBASE_TXID,
                expected_block_hash: MAINNET_GENESIS_BLOCK_HASH,
                utxo_flag: Network::Mainnet.utxo_flag(),
            })
        })
        .clone()
}

fn testnet() -> GenesisState {
    static STATE: OnceLock<GenesisState> = OnceLock::new();
    STATE
        .get_or_init(|| {
            genesis_state_from_parts(GenesisParts {
                network: Network::Testnet,
                reward_address: TESTNET_GENESIS_REWARD_ADDRESS,
                reward_script: TESTNET_GENESIS_REWARD_SCRIPT,
                block_version: TESTNET_GENESIS_BLOCK_VERSION,
                tx_version: TESTNET_GENESIS_TX_VERSION,
                lock_time: TESTNET_GENESIS_LOCK_TIME,
                timestamp: TESTNET_GENESIS_TIMESTAMP,
                nonce: TESTNET_GENESIS_NONCE,
                target: TESTNET_GENESIS_TARGET,
                expected_coinbase_txid: TESTNET_GENESIS_COINBASE_TXID,
                expected_block_hash: TESTNET_GENESIS_BLOCK_HASH,
                utxo_flag: Network::Testnet.utxo_flag(),
            })
        })
        .clone()
}

fn regnet() -> GenesisState {
    static STATE: OnceLock<GenesisState> = OnceLock::new();
    STATE
        .get_or_init(|| {
            genesis_state_from_parts(GenesisParts {
                network: Network::Regnet,
                reward_address: REGNET_GENESIS_REWARD_ADDRESS,
                reward_script: REGNET_GENESIS_REWARD_SCRIPT,
                block_version: REGNET_GENESIS_BLOCK_VERSION,
                tx_version: REGNET_GENESIS_TX_VERSION,
                lock_time: REGNET_GENESIS_LOCK_TIME,
                timestamp: REGNET_GENESIS_TIMESTAMP,
                nonce: REGNET_GENESIS_NONCE,
                target: REGNET_GENESIS_TARGET,
                expected_coinbase_txid: REGNET_GENESIS_COINBASE_TXID,
                expected_block_hash: REGNET_GENESIS_BLOCK_HASH,
                utxo_flag: Network::Regnet.utxo_flag(),
            })
        })
        .clone()
}

fn prunetest() -> GenesisState {
    static STATE: OnceLock<GenesisState> = OnceLock::new();
    STATE
        .get_or_init(|| {
            genesis_state_from_parts(GenesisParts {
                network: Network::Prunetest,
                reward_address: PRUNETEST_GENESIS_REWARD_ADDRESS,
                reward_script: PRUNETEST_GENESIS_REWARD_SCRIPT,
                block_version: PRUNETEST_GENESIS_BLOCK_VERSION,
                tx_version: PRUNETEST_GENESIS_TX_VERSION,
                lock_time: PRUNETEST_GENESIS_LOCK_TIME,
                timestamp: PRUNETEST_GENESIS_TIMESTAMP,
                nonce: PRUNETEST_GENESIS_NONCE,
                target: PRUNETEST_GENESIS_TARGET,
                expected_coinbase_txid: PRUNETEST_GENESIS_COINBASE_TXID,
                expected_block_hash: PRUNETEST_GENESIS_BLOCK_HASH,
                utxo_flag: Network::Prunetest.utxo_flag(),
            })
        })
        .clone()
}

struct GenesisParts {
    network: Network,
    reward_address: &'static str,
    reward_script: [u8; 48],
    block_version: u16,
    tx_version: u16,
    lock_time: u32,
    timestamp: u64,
    nonce: u64,
    target: [u8; 48],
    expected_coinbase_txid: [u8; 48],
    expected_block_hash: [u8; 48],
    utxo_flag: &'static str,
}

fn genesis_state_from_parts(parts: GenesisParts) -> GenesisState {
    let GenesisParts {
        network,
        reward_address,
        reward_script,
        block_version,
        tx_version,
        lock_time,
        timestamp,
        nonce,
        target,
        expected_coinbase_txid,
        expected_block_hash,
        utxo_flag,
    } = parts;

    let coinbase = Transaction {
        version: tx_version,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::genesis_coinbase_atoms_for_network(network),
            locking_script: reward_script.to_vec(),
        }],
        lock_time,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let coinbase_txid = coinbase.txid();
    assert_eq!(coinbase_txid, expected_coinbase_txid);
    assert_eq!(coinbase.outputs[0].locking_script, reward_script);
    let transactions = vec![coinbase];
    let merkle_root = merkle_root(&transactions);
    let witness_root = witness_root(&transactions);

    let header = BlockHeader {
        version: block_version,
        network_id: network,
        height: 0,
        previous_block_hash: [0; 48],
        merkle_root,
        witness_root,
        timestamp,
        difficulty_target_or_bits: target,
        nonce,
    };
    let block = Block {
        header,
        transactions,
        witnesses: Default::default(),
        fees_total_atoms: 0,
        fees_miner_atoms: 0,
    };
    assert_eq!(block.transactions[0].version, tx_version);
    assert_eq!(block.transactions[0].lock_time, lock_time);
    assert_eq!(block.header.version, block_version);
    assert_eq!(block.header.network_id, network);
    assert_eq!(block.header.height, 0);
    assert_eq!(block.header.merkle_root, merkle_root);
    assert_eq!(block.header.witness_root, witness_root);
    assert_eq!(block.header.merkle_root, block.merkle_root());
    assert_eq!(block.header.witness_root, block.compute_witness_root());
    let block_hash = block.header.block_hash();
    assert_eq!(block_hash, expected_block_hash);
    assert!(pow::meets_target(
        &block_hash,
        &block.header.difficulty_target_or_bits
    ));

    GenesisState {
        network,
        block,
        block_hash,
        coinbase_txid,
        reward_address: reward_address.to_string(),
        utxo_flag,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_state_is_network_scoped() {
        let main = genesis_state(Network::Mainnet);
        let test = genesis_state(Network::Testnet);
        let reg = genesis_state(Network::Regnet);
        let prune = genesis_state(Network::Prunetest);
        assert_eq!(main.network, Network::Mainnet);
        assert_eq!(test.network, Network::Testnet);
        assert_eq!(reg.network, Network::Regnet);
        assert_eq!(prune.network, Network::Prunetest);
        assert_ne!(main.block_hash, test.block_hash);
        assert_ne!(test.block_hash, reg.block_hash);
        assert_ne!(reg.block_hash, prune.block_hash);
        assert_eq!(
            main.block.transactions[0].outputs[0].value_atoms,
            subsidy::genesis_coinbase_atoms_for_network(Network::Mainnet)
        );
        assert_eq!(main.block.header.version, 1);
        assert_eq!(main.block.header.previous_block_hash, [0; 48]);
        assert_eq!(main.block.header.merkle_root, main.coinbase_txid);
        assert_eq!(
            main.block.header.difficulty_target_or_bits,
            pow::DIFFICULTY_PROFILE.genesis_target
        );
        assert_eq!(main.block.header.network_id, Network::Mainnet);
        assert_eq!(main.block.header.height, 0);
        assert!(pow::meets_target(
            &main.block_hash,
            &main.block.header.difficulty_target_or_bits
        ));
        assert_eq!(main.block.transactions[0].version, 1);
        assert_eq!(main.block.transactions[0].inputs.len(), 0);
        assert_eq!(main.block.transactions[0].outputs.len(), 1);
        assert_eq!(main.block.transactions[0].lock_time, 0);
        assert_eq!(main.block.transactions[0].witness.len(), 0);
        assert_eq!(
            main.block.transactions[0].outputs[0]
                .locking_script
                .as_slice(),
            MAINNET_GENESIS_REWARD_SCRIPT
        );
        assert_eq!(main.block.witnesses.len(), 0);
        assert_eq!(
            main.block.header.witness_root,
            main.block.compute_witness_root()
        );
        assert_eq!(main.block.fees_total_atoms, 0);
        assert_eq!(main.block.fees_miner_atoms, 0);
        assert_eq!(test.utxo_flag, "TEST-UTXO");
        assert_eq!(reg.utxo_flag, "REG-UTXO");
        assert_eq!(prune.utxo_flag, "PRUNE-UTXO");
        assert_eq!(prune.reward_address, PRUNETEST_GENESIS_REWARD_ADDRESS);
        assert_eq!(prune.block.header.network_id, Network::Prunetest);
        assert_eq!(
            prune.block.header.difficulty_target_or_bits,
            PRUNETEST_GENESIS_TARGET
        );
    }

    #[test]
    fn regenerated_profiles_match_frozen_genesis_state() {
        for network in [
            Network::Mainnet,
            Network::Testnet,
            Network::Regnet,
            Network::Prunetest,
        ] {
            let profile = regenerate_genesis_profile(network);
            let state = genesis_state(network);
            assert_eq!(profile.coinbase_txid, state.coinbase_txid);
            assert_eq!(profile.block_hash, state.block_hash);
            assert_eq!(profile.merkle_root, state.block.header.merkle_root);
            assert_eq!(profile.witness_root, state.block.header.witness_root);
            assert_eq!(profile.nonce, state.block.header.nonce);
        }
    }
}
