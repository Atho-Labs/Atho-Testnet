use atho_core::network::Network;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageConfig {
    pub network: Network,
}
