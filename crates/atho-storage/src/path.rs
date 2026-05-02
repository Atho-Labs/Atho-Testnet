//! Network-specific data-directory and database-path resolution.
use atho_core::network::Network;
use std::path::PathBuf;

pub const ATHO_DATA_DIR_ENV: &str = "ATHO_DATA_DIR";

pub fn data_dir(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Regnet => "regnet",
        Network::Prunetest => "prunetest",
    }
}

pub fn sandbox_root() -> PathBuf {
    std::env::var_os(ATHO_DATA_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(default_operator_root)
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

fn default_operator_root() -> PathBuf {
    #[cfg(test)]
    {
        return std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("dev");
    }

    #[cfg(not(test))]
    {
        platform_data_home().join("Atho")
    }
}

#[cfg(not(test))]
fn platform_data_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .map(|path| path.join("AppData").join("Roaming"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
    }

    #[cfg(target_os = "macos")]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Application Support")
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|path| path.join(".local").join("share")))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[cfg(not(test))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_database_dirs_are_unique_and_include_prunetest() {
        assert_eq!(data_dir(Network::Mainnet), "mainnet");
        assert_eq!(data_dir(Network::Testnet), "testnet");
        assert_eq!(data_dir(Network::Regnet), "regnet");
        assert_eq!(data_dir(Network::Prunetest), "prunetest");
        assert_ne!(data_dir(Network::Regnet), data_dir(Network::Prunetest));
        assert!(database_dir(Network::Prunetest)
            .to_string_lossy()
            .ends_with("prunetest"));
    }
}
