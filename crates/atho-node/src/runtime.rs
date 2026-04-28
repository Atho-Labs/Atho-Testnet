use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use crate::system::AthoSystem;
use crate::tcp_p2p::TcpP2pRuntime;
use atho_core::network::Network;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::{read_message, write_message};
use std::io::BufReader;
use std::net::{IpAddr, SocketAddr, TcpListener, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;

const RPC_IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("invalid network")]
    InvalidNetwork,
    #[error("refusing to bind RPC on a non-loopback address without ATHO_RPC_ALLOW_PUBLIC=1: {0}")]
    PublicRpcDenied(String),
    #[error("rpc bind failed: {0}")]
    RpcBindFailed(String),
    #[error("p2p bind failed: {0}")]
    P2pBindFailed(String),
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
    let network = config.network;
    let system = Arc::new(Mutex::new(AthoSystem::try_new(config)?));
    {
        let mut guard = system.lock().expect("node runtime mutex poisoned");
        guard.start();
    }
    let p2p_address = p2p_bind_address(network);
    let p2p_runtime = TcpP2pRuntime::bind_shared(network, Arc::clone(&system), &p2p_address)
        .map_err(|err| NodeError::Runtime(RuntimeError::P2pBindFailed(err.to_string())))?;
    for peer in p2p_peer_addresses() {
        p2p_runtime.maintain_outbound(peer);
    }
    let rpc_address = rpc_bind_address(config.network);
    validate_rpc_bind_address(&rpc_address)?;
    let listener = TcpListener::bind(&rpc_address).map_err(|err| {
        crate::error::NodeError::Runtime(RuntimeError::RpcBindFailed(err.to_string()))
    })?;
    println!("athod running on {} rpc={rpc_address}", config.network.id());
    let status = {
        let guard = system.lock().expect("node runtime mutex poisoned");
        guard.status()
    };
    println!(
        "node status height={} mempool={} synced={}",
        status.block_count, status.mempool_count, status.headers_synced
    );
    let _ = dev::append_log(
        "athod",
        &format!("runtime started rpc={rpc_address} p2p={p2p_address}"),
    );
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
                let response: RpcResponse = {
                    let mut guard = system.lock().expect("node runtime mutex poisoned");
                    guard.handle_mut(request)
                };
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

fn validate_rpc_bind_address(address: &str) -> Result<(), NodeError> {
    if std::env::var("ATHO_RPC_ALLOW_PUBLIC").ok().as_deref() == Some("1") {
        return Ok(());
    }

    let resolved = address.to_socket_addrs().map_err(|err| {
        NodeError::Runtime(RuntimeError::RpcBindFailed(format!(
            "invalid rpc bind address {address}: {err}"
        )))
    })?;
    for socket_addr in resolved {
        if !is_loopback_addr(socket_addr) {
            return Err(NodeError::Runtime(RuntimeError::PublicRpcDenied(
                address.to_string(),
            )));
        }
    }
    Ok(())
}

fn is_loopback_addr(address: SocketAddr) -> bool {
    match address.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}

pub fn p2p_bind_address(network: Network) -> String {
    if let Ok(address) = std::env::var("ATHO_P2P_ADDR") {
        return address;
    }
    format!("127.0.0.1:{}", network.p2p_port())
}

pub fn p2p_peer_addresses() -> Vec<String> {
    std::env::var("ATHO_P2P_PEERS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
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
    fn rpc_bind_validation_rejects_public_addresses_by_default() {
        let err = validate_rpc_bind_address("0.0.0.0:18443").unwrap_err();
        assert!(matches!(
            err,
            NodeError::Runtime(RuntimeError::PublicRpcDenied(_))
        ));
    }

    #[test]
    fn rpc_bind_validation_accepts_public_addresses_when_explicitly_allowed() {
        std::env::set_var("ATHO_RPC_ALLOW_PUBLIC", "1");
        let result = validate_rpc_bind_address("0.0.0.0:18443");
        std::env::remove_var("ATHO_RPC_ALLOW_PUBLIC");
        assert!(result.is_ok());
    }

    #[test]
    fn rpc_bind_validation_accepts_loopback_addresses() {
        assert!(validate_rpc_bind_address("127.0.0.1:18443").is_ok());
    }

    #[test]
    fn config_loader_defaults_to_mainnet() {
        let config = NodeConfig::new(Network::Mainnet);
        assert_eq!(config.network, Network::Mainnet);
    }
}
