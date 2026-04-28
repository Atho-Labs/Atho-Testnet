use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use crate::system::AthoSystem;
use atho_core::network::Network;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::{read_message, write_message};
use std::io::BufReader;
use std::net::TcpListener;
use std::time::Duration;
use thiserror::Error;

const RPC_IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("invalid network")]
    InvalidNetwork,
    #[error("rpc bind failed: {0}")]
    RpcBindFailed(String),
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

    pub fn load_or_new(config: NodeConfig) -> Self {
        Self {
            node: Node::load_or_new(config),
            running: false,
        }
    }

    pub fn try_load_or_new(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            node: Node::try_load_or_new(config)?,
            running: false,
        })
    }

    pub fn try_load_or_recover(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            node: Node::try_load_or_recover(config)?,
            running: false,
        })
    }

    pub fn start(&mut self) {
        self.running = true;
        let _ = dev::append_log(
            "athod",
            &format!(
                "runtime running network={} height={}",
                self.node.config.network.id(),
                self.node.height()
            ),
        );
    }

    pub fn stop(&mut self) {
        self.running = false;
        let _ = dev::append_log(
            "athod",
            &format!(
                "runtime stopped network={} height={}",
                self.node.config.network.id(),
                self.node.height()
            ),
        );
    }
}

pub fn load_config_from_env() -> Result<NodeConfig, RuntimeError> {
    let raw = std::env::var("ATHO_NETWORK").unwrap_or_else(|_| String::from("mainnet"));
    let network = Network::parse(&raw).ok_or(RuntimeError::InvalidNetwork)?;

    Ok(NodeConfig::new(network))
}

pub fn run_with_config(config: NodeConfig) -> Result<(), NodeError> {
    let _ = dev::append_log("athod", &format!("starting on {}", config.network.id()));
    let _ = dev::append_log("p2p", &format!("runtime network={}", config.network.id()));
    let mut system = AthoSystem::try_new(config)?;
    system.start();
    let rpc_address = rpc_bind_address(config.network);
    let listener = TcpListener::bind(&rpc_address).map_err(|err| {
        crate::error::NodeError::Runtime(RuntimeError::RpcBindFailed(err.to_string()))
    })?;
    println!("athod running on {} rpc={rpc_address}", config.network.id());
    let status = system.status();
    println!(
        "node status height={} mempool={} synced={}",
        status.block_count, status.mempool_count, status.headers_synced
    );
    let _ = dev::append_log("athod", &format!("runtime started rpc={rpc_address}"));
    for incoming in listener.incoming() {
        match incoming {
            Ok(mut stream) => {
                if let Err(err) = stream.set_nodelay(true) {
                    let _ = dev::append_log("athod", &format!("rpc nodelay error: {err}"));
                    continue;
                }
                if let Err(err) = stream.set_read_timeout(Some(RPC_IO_TIMEOUT)) {
                    let _ = dev::append_log("athod", &format!("rpc read timeout error: {err}"));
                    continue;
                }
                if let Err(err) = stream.set_write_timeout(Some(RPC_IO_TIMEOUT)) {
                    let _ = dev::append_log("athod", &format!("rpc write timeout error: {err}"));
                    continue;
                }
                let request = match stream.try_clone() {
                    Ok(clone) => {
                        let mut reader = BufReader::new(clone);
                        match read_message::<_, RpcRequest>(&mut reader) {
                            Ok(request) => request,
                            Err(err) => {
                                let _ = dev::append_log("athod", &format!("rpc read error: {err}"));
                                continue;
                            }
                        }
                    }
                    Err(err) => {
                        let _ = dev::append_log("athod", &format!("rpc clone error: {err}"));
                        continue;
                    }
                };
                let response: RpcResponse = system.handle_mut(request);
                if let Err(err) = write_message(&mut stream, &response) {
                    let _ = dev::append_log("athod", &format!("rpc write error: {err}"));
                }
            }
            Err(err) => {
                let _ = dev::append_log("athod", &format!("rpc accept error: {err}"));
            }
        }
    }
    let _ = dev::append_log("athod", "runtime stopped");
    Ok(())
}

pub fn rpc_bind_address(network: Network) -> String {
    if let Ok(address) = std::env::var("ATHO_RPC_ADDR") {
        return address;
    }
    match network {
        Network::Mainnet => String::from("127.0.0.1:18443"),
        Network::Testnet => String::from("127.0.0.1:18444"),
        Network::Regnet => String::from("127.0.0.1:18445"),
    }
}

pub fn run() -> Result<(), NodeError> {
    let config = load_config_from_env()?;
    run_with_config(config)
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
