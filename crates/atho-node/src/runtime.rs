//! Process-level node runtime and RPC/P2P bind orchestration.
//!
//! The runtime owns the running `Node`, binds loopback RPC and public P2P
//! listeners, and forwards incoming RPC requests into the validated node path.
//!
//! SECURITY: RPC remains loopback-only by default. Public binding requires an
//! explicit override because wallet and admin commands assume local trust.
use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use crate::system::AthoSystem;
use crate::tcp_p2p::{outbound_target_dedup_key, TcpP2pRuntime};
use atho_core::network::Network;
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, LAUNCH_P2P_BIND_FAILED, LAUNCH_PUBLIC_RPC_DENIED,
    LAUNCH_RPC_BIND_FAILED, NET_INVALID_NETWORK_SELECTION,
};
use atho_p2p::config::{configured_bootstrap_peers, network_params};
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::{read_message, write_message};
use std::collections::BTreeSet;
use std::io::BufReader;
use std::net::{IpAddr, SocketAddr, TcpListener, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const RPC_IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Runtime-level launch failures before the node can serve traffic.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("invalid network")]
    InvalidNetwork,
    #[error("refusing to bind RPC on a non-loopback address without ATHO_RPC_ALLOW_PUBLIC=1: {0}")]
    PublicRpcDenied(String),
    #[error("rpc bind failed: {0}")]
    RpcBindFailed(String),
    #[error("api bind failed: {0}")]
    ApiBindFailed(String),
    #[error("p2p bind failed: {0}")]
    P2pBindFailed(String),
}

impl AthoErrorMeta for RuntimeError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::InvalidNetwork => &NET_INVALID_NETWORK_SELECTION,
            Self::PublicRpcDenied(_) => &LAUNCH_PUBLIC_RPC_DENIED,
            Self::RpcBindFailed(_) => &LAUNCH_RPC_BIND_FAILED,
            Self::ApiBindFailed(_) => &LAUNCH_RPC_BIND_FAILED,
            Self::P2pBindFailed(_) => &LAUNCH_P2P_BIND_FAILED,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-node::runtime"
    }

    fn safe_details(&self) -> Option<String> {
        match self {
            Self::PublicRpcDenied(address)
            | Self::RpcBindFailed(address)
            | Self::ApiBindFailed(address)
            | Self::P2pBindFailed(address) => Some(address.clone()),
            Self::InvalidNetwork => None,
        }
    }
}

/// Running node process state.
#[derive(Debug)]
pub struct NodeRuntime {
    pub node: Node,
    pub running: bool,
    pub started_at_unix: Option<u64>,
}

impl NodeRuntime {
    /// Creates a new runtime around a fresh node instance.
    pub fn new(config: NodeConfig) -> Self {
        Self {
            node: Node::new(config),
            running: false,
            started_at_unix: None,
        }
    }

    /// Loads persisted state when available and otherwise creates a fresh node.
    pub fn load_or_new(config: NodeConfig) -> Self {
        Self {
            node: Node::load_or_new(config),
            running: false,
            started_at_unix: None,
        }
    }

    pub fn try_load_or_new(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            node: Node::try_load_or_new(config)?,
            running: false,
            started_at_unix: None,
        })
    }

    pub fn try_load_or_recover(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            node: Node::try_load_or_recover(config)?,
            running: false,
            started_at_unix: None,
        })
    }

    /// Starts the runtime and records the process start time.
    pub fn start(&mut self) {
        self.running = true;
        self.started_at_unix = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        let _ = dev::append_log(
            "athod",
            &format!(
                "runtime running network={} height={}",
                self.node.config.network.id(),
                self.node.height()
            ),
        );
    }

    /// Stops the runtime without mutating persisted state.
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

/// Loads the node configuration from environment variables.
pub fn load_config_from_env() -> Result<NodeConfig, RuntimeError> {
    let network = match std::env::var("ATHO_NETWORK") {
        Ok(raw) => Network::parse(&raw).ok_or(RuntimeError::InvalidNetwork)?,
        Err(_) => Network::operator_default(),
    };

    Ok(NodeConfig::from_env(network))
}

/// Runs the full Atho node with live RPC and P2P listeners.
pub fn run_with_config(config: NodeConfig) -> Result<(), NodeError> {
    let _ = dev::append_log("athod", &format!("starting on {}", config.network.id()));
    let _ = dev::append_log("p2p", &format!("runtime network={}", config.network.id()));
    let network = config.network;
    let api_config = config.api.clone();
    let system = Arc::new(Mutex::new(AthoSystem::try_new(config)?));
    {
        let mut guard = system.lock().expect("node runtime mutex poisoned");
        guard.start();
    }
    let p2p_address = p2p_bind_address(network);
    let p2p_runtime = TcpP2pRuntime::bind_shared(network, Arc::clone(&system), &p2p_address)
        .map_err(|err| NodeError::Runtime(RuntimeError::P2pBindFailed(err.to_string())))?;
    let bootstrap_limit = network_params(network)
        .limits
        .max_outbound_peers
        .saturating_mul(4)
        .max(8);
    let bootstrap_peers = {
        let mut guard = system.lock().expect("node runtime mutex poisoned");
        guard.p2p_bootstrap_peers(bootstrap_limit)
    };
    for peer in initial_outbound_peers(network, bootstrap_peers) {
        p2p_runtime.maintain_outbound(peer);
    }
    let rpc_address = rpc_bind_address(network);
    validate_rpc_bind_address(&rpc_address)?;
    let listener = TcpListener::bind(&rpc_address).map_err(|err| {
        crate::error::NodeError::Runtime(RuntimeError::RpcBindFailed(err.to_string()))
    })?;
    println!("athod running on {} rpc={rpc_address}", network.id());
    let status = {
        let guard = system.lock().expect("node runtime mutex poisoned");
        guard.status()
    };
    let chain_synced = status.headers_synced && status.block_count >= status.sync_best_height;
    println!(
        "node status height={} target={} mempool={} headers_synced={} chain_synced={}",
        status.block_count,
        status.sync_best_height,
        status.mempool_count,
        status.headers_synced,
        chain_synced
    );
    let _ = dev::append_log(
        "athod",
        &format!("runtime started rpc={rpc_address} p2p={p2p_address}"),
    );
    if api_config.enabled {
        let shared = Arc::clone(&system);
        let bind = api_config.bind_address();
        let server = crate::api::bind_http_server(&api_config)
            .map_err(|err| NodeError::Runtime(RuntimeError::ApiBindFailed(err)))?;
        std::thread::Builder::new()
            .name(format!("atho-api-{}", network.domain_tag()))
            .spawn(move || {
                if let Err(err) = crate::api::run_http_server(server, shared, api_config) {
                    let _ = dev::append_log(
                        "api",
                        &format!("http api stopped bind={} error={err}", bind),
                    );
                }
            })
            .map_err(|err| NodeError::Runtime(RuntimeError::ApiBindFailed(err.to_string())))?;
    }
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
                // RPC requests are always executed through the validated node
                // service path; there is no direct mutation of chainstate here.
                let response: RpcResponse = {
                    let mut guard = system.lock().expect("node runtime mutex poisoned");
                    guard.handle_mut(request)
                };
                let shutdown_requested = response_requests_runtime_shutdown(&response);
                if let Err(err) = write_message(&mut stream, &response) {
                    let _ = dev::append_log("athod", &format!("rpc write error: {err}"));
                }
                if shutdown_requested {
                    let _ = dev::append_log(
                        "athod",
                        &format!(
                            "runtime shutdown requested over rpc network={}",
                            network.id()
                        ),
                    );
                    break;
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

fn response_requests_runtime_shutdown(response: &RpcResponse) -> bool {
    match response {
        RpcResponse::Command(command) if command.command == "stop" => command
            .data
            .get("stopping")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        _ => false,
    }
}

/// Returns the effective RPC bind address for the selected network.
pub fn rpc_bind_address(network: Network) -> String {
    if let Ok(address) = std::env::var("ATHO_RPC_ADDR") {
        return address;
    }
    default_rpc_bind_address(network)
}

pub fn default_rpc_bind_address(network: Network) -> String {
    format!("127.0.0.1:{}", network.rpc_port())
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
    default_p2p_bind_address(network)
}

pub fn default_p2p_bind_address(network: Network) -> String {
    // P2P is the public node interface. Defaulting it to a routable bind makes a freshly
    // deployed VPS node actually reachable without extra operator discovery.
    format!("0.0.0.0:{}", network.p2p_port())
}

fn initial_outbound_peers(
    network: Network,
    discovered_bootstrap_peers: Vec<String>,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    discovered_bootstrap_peers
        .into_iter()
        .chain(configured_bootstrap_peers(network))
        .filter(|peer| seen.insert(outbound_target_dedup_key(peer)))
        .collect()
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
        let err = validate_rpc_bind_address("0.0.0.0:9010").unwrap_err();
        assert!(matches!(
            err,
            NodeError::Runtime(RuntimeError::PublicRpcDenied(_))
        ));
    }

    #[test]
    fn rpc_bind_validation_accepts_public_addresses_when_explicitly_allowed() {
        std::env::set_var("ATHO_RPC_ALLOW_PUBLIC", "1");
        let result = validate_rpc_bind_address("0.0.0.0:9010");
        std::env::remove_var("ATHO_RPC_ALLOW_PUBLIC");
        assert!(result.is_ok());
    }

    #[test]
    fn rpc_bind_validation_accepts_loopback_addresses() {
        assert!(validate_rpc_bind_address("127.0.0.1:9010").is_ok());
    }

    #[test]
    fn default_p2p_bind_is_public_for_vps_nodes() {
        std::env::remove_var("ATHO_P2P_ADDR");
        assert_eq!(p2p_bind_address(Network::Mainnet), "0.0.0.0:56000");
    }

    #[test]
    fn config_loader_defaults_to_testnet() {
        std::env::remove_var("ATHO_NETWORK");
        let config = load_config_from_env().expect("config");
        assert_eq!(config.network, Network::Testnet);
    }

    #[test]
    fn config_loader_accepts_prunetest_network_from_env() {
        std::env::set_var("ATHO_NETWORK", "prune-test");
        let config = load_config_from_env().expect("config");
        std::env::remove_var("ATHO_NETWORK");
        assert_eq!(config.network, Network::Prunetest);
        assert_eq!(
            default_rpc_bind_address(Network::Prunetest),
            "127.0.0.1:9310"
        );
        assert_eq!(default_p2p_bind_address(Network::Prunetest), "0.0.0.0:9300");
    }

    #[test]
    fn initial_outbound_peers_deduplicate_targets_that_resolve_to_the_same_socket() {
        std::env::remove_var("ATHO_P2P_PEERS");

        let peers = initial_outbound_peers(
            Network::Regnet,
            vec![
                String::from("localhost:9200"),
                String::from("127.0.0.1:9200"),
                String::from("127.0.0.1:9201"),
            ],
        );

        assert_eq!(peers.len(), 2);
        assert!(peers.iter().any(|peer| peer == "localhost:9200"));
        assert!(peers.iter().any(|peer| peer == "127.0.0.1:9201"));
    }

    #[test]
    fn initial_outbound_peers_try_discovered_anchors_before_static_bootstrap() {
        std::env::remove_var("ATHO_P2P_PEERS");

        let peers = initial_outbound_peers(Network::Testnet, vec![String::from("8.8.8.8:9100")]);

        assert_eq!(peers.first().map(String::as_str), Some("8.8.8.8:9100"));
        assert!(peers
            .iter()
            .any(|peer| peer == "testnet-node1.atho.io:9100"));
    }

    #[test]
    fn stop_command_response_requests_runtime_shutdown() {
        let response = RpcResponse::Command(atho_rpc::command::CommandResponse {
            command: String::from("stop"),
            group: atho_rpc::command::CommandGroup::Control,
            permission: atho_rpc::command::CommandPermission::NodeAdmin,
            dangerous: true,
            network: String::from("atho-testnet"),
            data: serde_json::json!({
                "stopping": true,
                "network": "atho-testnet",
                "height": 0,
            }),
        });

        assert!(response_requests_runtime_shutdown(&response));
        assert!(!response_requests_runtime_shutdown(&RpcResponse::Network(
            String::from("atho-testnet")
        )));
    }
}
