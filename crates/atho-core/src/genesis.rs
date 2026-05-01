use crate::block::{merkle_root, witness_root, Block, BlockHeader};
use crate::consensus::pow;
use crate::consensus::rules::{BLOCK_VERSION_V1, TRANSACTION_VERSION_V1};
use crate::constants::GENESIS_COINBASE_ATOMS;
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
const MAINNET_GENESIS_NONCE: u64 = 135_392;
const MAINNET_GENESIS_TARGET: [u8; 48] = pow::DIFFICULTY_PROFILE.genesis_target;
const MAINNET_GENESIS_COINBASE_TXID: [u8; 48] =
    hex!("641758f47d003c211adab6540ce624baf2223dc0892149ea24f8b9015873a0976b3b468200553e672888f4ea3d6e8134");
const MAINNET_GENESIS_MERKLE_ROOT: [u8; 48] = MAINNET_GENESIS_COINBASE_TXID;
const MAINNET_GENESIS_WITNESS_ROOT: [u8; 48] = MAINNET_GENESIS_COINBASE_TXID;
const MAINNET_GENESIS_BLOCK_HASH: [u8; 48] =
    hex!("00004eab876c38017f5f3f512e38ff9192106253912e0eefbe2eee4af732f7798e951f8ee2a3e2afe876927c2f21688f");

const TESTNET_GENESIS_REWARD_ADDRESS: &str =
    "ATHT22b5382e49b9a2dafb0d2c7b1c2afe643a3c14a23f7a90e4e5dce0162b754623eb5566c3ca1348187e5f3e92c65c76ee";
const TESTNET_GENESIS_REWARD_SCRIPT: [u8; 48] =
    hex!("22b5382e49b9a2dafb0d2c7b1c2afe643a3c14a23f7a90e4e5dce0162b754623eb5566c3ca1348187e5f3e92c65c76ee");
const TESTNET_GENESIS_BLOCK_VERSION: u16 = BLOCK_VERSION_V1;
const TESTNET_GENESIS_TX_VERSION: u16 = TRANSACTION_VERSION_V1;
const TESTNET_GENESIS_LOCK_TIME: u32 = 0;
const TESTNET_GENESIS_TIMESTAMP: u64 = 1_773_360_489;
const TESTNET_GENESIS_NONCE: u64 = 31_066;
const TESTNET_GENESIS_TARGET: [u8; 48] = pow::DIFFICULTY_PROFILE.genesis_target;
const TESTNET_GENESIS_COINBASE_TXID: [u8; 48] =
    hex!("4f1bf33eb11b3c4d3369b23a7af3cc17b714787a207f78da76985f8808e5f1b42fb5a0c3810cd67f5f1a77f84c8fb826");
const TESTNET_GENESIS_MERKLE_ROOT: [u8; 48] = TESTNET_GENESIS_COINBASE_TXID;
const TESTNET_GENESIS_WITNESS_ROOT: [u8; 48] = TESTNET_GENESIS_COINBASE_TXID;
const TESTNET_GENESIS_BLOCK_HASH: [u8; 48] =
    hex!("000083b1a17dc251043f4a7dd9d5981c35382e6d17bb6fb05eab2bb83dde5fe8a08dc766c9fb3ce9e1342f6f2238ac8a");

const REGNET_GENESIS_REWARD_ADDRESS: &str = TESTNET_GENESIS_REWARD_ADDRESS;
const REGNET_GENESIS_REWARD_SCRIPT: [u8; 48] = TESTNET_GENESIS_REWARD_SCRIPT;
const REGNET_GENESIS_BLOCK_VERSION: u16 = BLOCK_VERSION_V1;
const REGNET_GENESIS_TX_VERSION: u16 = TRANSACTION_VERSION_V1;
const REGNET_GENESIS_LOCK_TIME: u32 = 0;
const REGNET_GENESIS_TIMESTAMP: u64 = TESTNET_GENESIS_TIMESTAMP;
const REGNET_GENESIS_NONCE: u64 = 202_541;
const REGNET_GENESIS_TARGET: [u8; 48] = pow::DIFFICULTY_PROFILE.genesis_target;
const REGNET_GENESIS_COINBASE_TXID: [u8; 48] = TESTNET_GENESIS_COINBASE_TXID;
const REGNET_GENESIS_MERKLE_ROOT: [u8; 48] = REGNET_GENESIS_COINBASE_TXID;
const REGNET_GENESIS_WITNESS_ROOT: [u8; 48] = REGNET_GENESIS_COINBASE_TXID;
const REGNET_GENESIS_BLOCK_HASH: [u8; 48] =
    hex!("0000747cfb613e8e66e9cf9af1c6eb1c666f4879aa3a99fb90b5dc948c129587ed20112e5fd43d131c8eaedeca7d465a");

const PRUNETEST_GENESIS_REWARD_ADDRESS: &str =
    "ATHP22b5382e49b9a2dafb0d2c7b1c2afe643a3c14a23f7a90e4e5dce0162b754623eb5566c3ca1348187e5f3e92c65c76ee";
const PRUNETEST_GENESIS_REWARD_SCRIPT: [u8; 48] = TESTNET_GENESIS_REWARD_SCRIPT;
const PRUNETEST_GENESIS_BLOCK_VERSION: u16 = BLOCK_VERSION_V1;
const PRUNETEST_GENESIS_TX_VERSION: u16 = TRANSACTION_VERSION_V1;
const PRUNETEST_GENESIS_LOCK_TIME: u32 = 0;
const PRUNETEST_GENESIS_TIMESTAMP: u64 = 1_773_360_490;
const PRUNETEST_GENESIS_TARGET: [u8; 48] = pow::PRUNETEST_INITIAL_TARGET;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisState {
    pub network: Network,
    pub block: Block,
    pub block_hash: [u8; 48],
    pub coinbase_txid: [u8; 48],
    pub reward_address: String,
    pub utxo_flag: &'static str,
}

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

pub fn genesis_state(network: Network) -> GenesisState {
    match network {
        Network::Mainnet => mainnet(),
        Network::Testnet => testnet(),
        Network::Regnet => regnet(),
        Network::Prunetest => prunetest(),
    }
}

pub fn genesis_block(network: Network) -> Block {
    genesis_state(network).block
}

pub fn genesis_hash(network: Network) -> [u8; 48] {
    genesis_state(network).block_hash
}

pub fn genesis_coinbase_txid(network: Network) -> [u8; 48] {
    genesis_state(network).coinbase_txid
}

pub fn genesis_reward_address(network: Network) -> String {
    genesis_state(network).reward_address
}

pub fn genesis_utxo_flag(network: Network) -> &'static str {
    genesis_state(network).utxo_flag
}

pub fn genesis_utxo_value(network: Network) -> u64 {
    let _ = network;
    GENESIS_COINBASE_ATOMS
}

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
            value_atoms: GENESIS_COINBASE_ATOMS,
            locking_script: reward_script.to_vec(),
        }],
        lock_time,
        witness: vec![],
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
    genesis_state_from_parts(
        Network::Mainnet,
        MAINNET_GENESIS_REWARD_ADDRESS,
        MAINNET_GENESIS_REWARD_SCRIPT,
        MAINNET_GENESIS_BLOCK_VERSION,
        MAINNET_GENESIS_TX_VERSION,
        MAINNET_GENESIS_LOCK_TIME,
        MAINNET_GENESIS_TIMESTAMP,
        MAINNET_GENESIS_NONCE,
        MAINNET_GENESIS_TARGET,
        MAINNET_GENESIS_MERKLE_ROOT,
        MAINNET_GENESIS_WITNESS_ROOT,
        "",
        MAINNET_GENESIS_COINBASE_TXID,
        MAINNET_GENESIS_BLOCK_HASH,
    )
}

fn testnet() -> GenesisState {
    genesis_state_from_parts(
        Network::Testnet,
        TESTNET_GENESIS_REWARD_ADDRESS,
        TESTNET_GENESIS_REWARD_SCRIPT,
        TESTNET_GENESIS_BLOCK_VERSION,
        TESTNET_GENESIS_TX_VERSION,
        TESTNET_GENESIS_LOCK_TIME,
        TESTNET_GENESIS_TIMESTAMP,
        TESTNET_GENESIS_NONCE,
        TESTNET_GENESIS_TARGET,
        TESTNET_GENESIS_MERKLE_ROOT,
        TESTNET_GENESIS_WITNESS_ROOT,
        "TEST-UTXO",
        TESTNET_GENESIS_COINBASE_TXID,
        TESTNET_GENESIS_BLOCK_HASH,
    )
}

fn regnet() -> GenesisState {
    genesis_state_from_parts(
        Network::Regnet,
        REGNET_GENESIS_REWARD_ADDRESS,
        REGNET_GENESIS_REWARD_SCRIPT,
        REGNET_GENESIS_BLOCK_VERSION,
        REGNET_GENESIS_TX_VERSION,
        REGNET_GENESIS_LOCK_TIME,
        REGNET_GENESIS_TIMESTAMP,
        REGNET_GENESIS_NONCE,
        REGNET_GENESIS_TARGET,
        REGNET_GENESIS_MERKLE_ROOT,
        REGNET_GENESIS_WITNESS_ROOT,
        "REG-UTXO",
        REGNET_GENESIS_COINBASE_TXID,
        REGNET_GENESIS_BLOCK_HASH,
    )
}

fn prunetest() -> GenesisState {
    static STATE: OnceLock<GenesisState> = OnceLock::new();
    STATE
        .get_or_init(|| {
            let profile = regenerate_genesis_profile(Network::Prunetest);
            genesis_state_from_parts(
                profile.network,
                PRUNETEST_GENESIS_REWARD_ADDRESS,
                profile.reward_script,
                profile.block_version,
                profile.tx_version,
                profile.lock_time,
                profile.timestamp,
                profile.nonce,
                profile.target,
                profile.merkle_root,
                profile.witness_root,
                Network::Prunetest.utxo_flag(),
                profile.coinbase_txid,
                profile.block_hash,
            )
        })
        .clone()
}

fn genesis_state_from_parts(
    network: Network,
    reward_address: &str,
    reward_script: [u8; 48],
    block_version: u16,
    tx_version: u16,
    lock_time: u32,
    timestamp: u64,
    nonce: u64,
    target: [u8; 48],
    merkle_root: [u8; 48],
    witness_root: [u8; 48],
    utxo_flag: &'static str,
    expected_coinbase_txid: [u8; 48],
    expected_block_hash: [u8; 48],
) -> GenesisState {
    let coinbase = Transaction {
        version: tx_version,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: GENESIS_COINBASE_ATOMS,
            locking_script: reward_script.to_vec(),
        }],
        lock_time,
        witness: vec![],
    };
    let coinbase_txid = coinbase.txid();
    assert_eq!(coinbase_txid, expected_coinbase_txid);
    assert_eq!(coinbase.outputs[0].locking_script, reward_script);

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
        transactions: vec![coinbase],
        witnesses: Default::default(),
        fees_total_atoms: 0,
        fees_miner_atoms: 0,
        fees_burned_atoms: 0,
        fees_pool_atoms: 0,
        cumulative_burned_atoms: 0,
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
            GENESIS_COINBASE_ATOMS
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
        assert_eq!(main.block.header.nonce, MAINNET_GENESIS_NONCE);
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
        assert_eq!(main.block.header.witness_root, MAINNET_GENESIS_WITNESS_ROOT);
        assert_eq!(main.block.fees_total_atoms, 0);
        assert_eq!(main.block.fees_miner_atoms, 0);
        assert_eq!(main.block.fees_burned_atoms, 0);
        assert_eq!(main.block.fees_pool_atoms, 0);
        assert_eq!(main.block.cumulative_burned_atoms, 0);
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
}
