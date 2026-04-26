use crate::block::{Block, BlockHeader};
use crate::address::internal_hpk_bytes;
use crate::consensus::pow;
use crate::constants::GENESIS_COINBASE_ATOMS;
use crate::network::Network;
use crate::transaction::{Transaction, TxOutput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisState {
    pub network: Network,
    pub block: Block,
    pub block_hash: [u8; 48],
    pub coinbase_txid: [u8; 48],
    pub reward_address: String,
    pub utxo_flag: &'static str,
}

pub fn genesis_state(network: Network) -> GenesisState {
    match network {
        Network::Mainnet => mainnet(),
        Network::Testnet => testnet(),
        Network::Regnet => regnet(),
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

fn mainnet() -> GenesisState {
    genesis_state_from_parts(
        Network::Mainnet,
        "ATHO9529a6358612b193cc100b4150f46235505a948caacf331b15a171993ad3124c008f45d692886ecc6417aa6ab964488c",
        1_773_360_488,
        138_553,
        "",
    )
}

fn testnet() -> GenesisState {
    genesis_state_from_parts(
        Network::Testnet,
        "ATHT22b5382e49b9a2dafb0d2c7b1c2afe643a3c14a23f7a90e4e5dce0162b754623eb5566c3ca1348187e5f3e92c65c76ee",
        1_773_360_489,
        82_673,
        "TEST-UTXO",
    )
}

fn regnet() -> GenesisState {
    let mut state = testnet();
    state.network = Network::Regnet;
    state.utxo_flag = "REG-UTXO";
    state
}

fn genesis_state_from_parts(
    network: Network,
    reward_address: &str,
    timestamp: u64,
    nonce: u64,
    utxo_flag: &'static str,
) -> GenesisState {
    let reward_script = internal_hpk_bytes(network, reward_address)
        .unwrap_or_else(|| reward_address.as_bytes().to_vec());
    let coinbase = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: GENESIS_COINBASE_ATOMS,
            locking_script: reward_script,
        }],
        lock_time: 0,
        witness: vec![],
    };
    let coinbase_txid = coinbase.txid();

    let header = BlockHeader {
        version: 1,
        previous_block_hash: [0; 48],
        merkle_root: coinbase_txid,
        timestamp,
        target: pow::initial_target_for_network(network),
        nonce,
    };
    let mut block = Block::new(header, vec![coinbase]);
    block.witness_commitment = block.compute_witness_commitment();
    block.state_root = [0; 48];
    block.fees_total_atoms = 0;
    block.fees_miner_atoms = 0;
    block.fees_burned_atoms = 0;
    block.fees_pool_atoms = 0;
    block.cumulative_burned_atoms = 0;
    let block_hash = block.header.block_hash();

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
        assert_eq!(main.network, Network::Mainnet);
        assert_eq!(test.network, Network::Testnet);
        assert_ne!(main.block_hash, test.block_hash);
        assert_eq!(main.block.transactions[0].outputs[0].value_atoms, GENESIS_COINBASE_ATOMS);
        assert_eq!(test.utxo_flag, "TEST-UTXO");
    }
}
