use atho_core::network::Network;

pub fn data_dir(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Regnet => "regnet",
    }
}
