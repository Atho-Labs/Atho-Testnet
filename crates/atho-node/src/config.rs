use atho_core::network::Network;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeConfig {
    pub network: Network,
}

impl NodeConfig {
    pub fn new(network: Network) -> Self {
        Self { network }
    }
}
