use atho_core::network::Network;
use std::path::PathBuf;

pub const ATHO_DATA_DIR_ENV: &str = "ATHO_DATA_DIR";

pub fn data_dir(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Regnet => "regnet",
    }
}

pub fn sandbox_root() -> PathBuf {
    std::env::var_os(ATHO_DATA_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("dev")
        })
}

pub fn data_root() -> PathBuf {
    if std::env::var_os(ATHO_DATA_DIR_ENV).is_some() {
        sandbox_root()
    } else {
        sandbox_root().join("db")
    }
}

pub fn database_dir(network: Network) -> PathBuf {
    data_root().join(data_dir(network))
}

pub fn chain_dir() -> PathBuf {
    sandbox_root().join("chain")
}

pub fn quarantine_dir() -> PathBuf {
    sandbox_root().join("quarantine")
}
