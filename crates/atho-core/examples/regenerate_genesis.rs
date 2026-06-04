use atho_core::genesis::regenerate_genesis_profile;
use atho_core::network::Network;

fn main() {
    for network in [
        Network::Mainnet,
        Network::Testnet,
        Network::Regnet,
        Network::Prunetest,
    ] {
        let profile = regenerate_genesis_profile(network);
        println!("network={}", network.id());
        println!("coinbase_txid={}", hex::encode(profile.coinbase_txid));
        println!("merkle_root={}", hex::encode(profile.merkle_root));
        println!("witness_root={}", hex::encode(profile.witness_root));
        println!("nonce={}", profile.nonce);
        println!("block_hash={}", hex::encode(profile.block_hash));
        println!();
    }
}
