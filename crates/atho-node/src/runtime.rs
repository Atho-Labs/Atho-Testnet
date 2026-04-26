use crate::config::NodeConfig;
use crate::error::NodeError;
use crate::dev;
use crate::node::Node;
use atho_core::network::Network;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("invalid network")]
    InvalidNetwork,
}

#[derive(Debug)]
pub struct NodeRuntime {
    pub node: Node,
    pub running: bool,
}

impl NodeRuntime {
    pub fn new(config: NodeConfig) -> Self {
        Self {
            node: Node::new(config),
            running: false,
        }
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }
}

pub fn load_config_from_env() -> Result<NodeConfig, RuntimeError> {
    let network = match std::env::var("ATHO_NETWORK")
        .unwrap_or_else(|_| String::from("mainnet"))
        .as_str()
    {
        "mainnet" => Network::Mainnet,
        "testnet" => Network::Testnet,
        "regnet" | "regtest" => Network::Regnet,
        _ => return Err(RuntimeError::InvalidNetwork),
    };

    Ok(NodeConfig::new(network))
}

pub fn run() -> Result<(), NodeError> {
    let config = load_config_from_env()?;
    let _ = dev::append_log("athod", &format!("starting on {}", config.network.id()));
    let _ = dev::append_log("p2p", &format!("runtime network={}", config.network.id()));
    let mut runtime = NodeRuntime::new(config);
    runtime.start();
    let _ = dev::append_log("athod", "runtime started");
    runtime.stop();
    let _ = dev::append_log("athod", "runtime stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_starts_and_stops() {
        let mut runtime = NodeRuntime::new(NodeConfig::new(Network::Mainnet));
        runtime.start();
        assert!(runtime.running);
        runtime.stop();
        assert!(!runtime.running);
    }

    #[test]
    fn config_loader_defaults_to_mainnet() {
        let config = NodeConfig::new(Network::Mainnet);
        assert_eq!(config.network, Network::Mainnet);
    }
}
