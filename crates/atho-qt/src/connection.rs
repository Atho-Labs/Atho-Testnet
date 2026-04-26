use atho_core::network::Network;
use atho_node::system::AthoSystem;
use atho_rpc::error::RpcError;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::{MempoolInfo, RpcResponse};
use atho_rpc::transport::RpcClient;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub network: Network,
    pub rpc_address: String,
    pub block_count: u64,
    pub mempool_count: usize,
    pub connected: bool,
}

#[derive(Debug)]
enum ConnectionBackend {
    Local(AthoSystem),
    Rpc {
        client: RpcClient,
        _node: Option<NodeProcess>,
    },
}

#[derive(Debug)]
struct NodeProcess {
    child: Child,
}

impl Drop for NodeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug)]
pub struct ReadOnlyNodeConnection {
    backend: ConnectionBackend,
    network: Network,
    rpc_address: String,
}

impl ReadOnlyNodeConnection {
    pub fn new(network: Network) -> Self {
        Self::with_rpc_address(network, default_rpc_address(network))
    }

    pub fn with_rpc_address(network: Network, rpc_address: String) -> Self {
        let backend = if cfg!(test) || std::env::var("ATHO_QT_LOCAL").ok().as_deref() == Some("1") {
            let mut system = AthoSystem::new(atho_node::config::NodeConfig::new(network));
            system.start();
            ConnectionBackend::Local(system)
        } else {
            let node = start_local_node_if_needed(network, &rpc_address);
            ConnectionBackend::Rpc {
                client: RpcClient::new(rpc_address.clone()),
                _node: node,
            }
        };

        Self {
            backend,
            network,
            rpc_address,
        }
    }

    pub fn rpc_address(&self) -> &str {
        &self.rpc_address
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn has_local_node(&self) -> bool {
        matches!(&self.backend, ConnectionBackend::Rpc { _node: Some(_), .. })
    }

    pub fn request(&self, request: RpcRequest) -> RpcResponse {
        match &self.backend {
            ConnectionBackend::Local(system) => system.handle(request),
            ConnectionBackend::Rpc { client, .. } => match client.call(&request) {
                Ok(response) => response,
                Err(err) => {
                    let _ = atho_node::dev::append_log("atho-qt", &format!("rpc error: {err}"));
                    RpcResponse::Error(RpcError::Internal)
                }
            },
        }
    }

    pub fn status(&self) -> ConnectionStatus {
        match &self.backend {
            ConnectionBackend::Local(system) => {
                let status = system.status();
                ConnectionStatus {
                    network: status.network,
                    rpc_address: self.rpc_address.clone(),
                    block_count: status.block_count,
                    mempool_count: status.mempool_count,
                    connected: status.running,
                }
            }
            ConnectionBackend::Rpc { client, .. } => {
                let network_ok = matches!(
                    client.call(&RpcRequest::GetNetwork),
                    Ok(RpcResponse::Network(_))
                );
                let block_count = match client.call(&RpcRequest::GetBlockCount) {
                    Ok(RpcResponse::BlockCount(count)) => count,
                    _ => 0,
                };
                let mempool_count = match client.call(&RpcRequest::GetMempoolInfo) {
                    Ok(RpcResponse::MempoolInfo(MempoolInfo {
                        transaction_count, ..
                    })) => transaction_count,
                    _ => 0,
                };
                let connected = network_ok || block_count > 0 || mempool_count > 0;
                ConnectionStatus {
                    network: self.network,
                    rpc_address: self.rpc_address.clone(),
                    block_count,
                    mempool_count,
                    connected,
                }
            }
        }
    }
}

fn default_rpc_address(network: Network) -> String {
    atho_node::runtime::rpc_bind_address(network)
}

fn start_local_node_if_needed(network: Network, rpc_address: &str) -> Option<NodeProcess> {
    if probe_rpc(rpc_address) {
        return None;
    }

    let mut command = if let Some(binary) = node_binary_path() {
        let mut command = Command::new(binary);
        command.arg("run").arg(network.id());
        command
    } else {
        let _ = atho_node::dev::append_log(
            "atho-qt",
            "node binary not found; falling back to cargo run",
        );
        let manifest_path = workspace_manifest_path();
        let mut command = Command::new("cargo");
        command
            .arg("run")
            .arg("--manifest-path")
            .arg(manifest_path)
            .arg("-p")
            .arg("atho-node")
            .arg("--bin")
            .arg("athod")
            .arg("--")
            .arg("run")
            .arg(network.id());
        command
    };
    command
        .env("ATHO_RPC_ADDR", rpc_address)
        .env("ATHO_NETWORK", network.id())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match command.spawn() {
        Ok(child) => {
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!("spawned local node bootstrap for {}", network.id()),
            );
            Some(NodeProcess { child })
        }
        Err(err) => {
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!("failed to spawn local node: {err}"),
            );
            None
        }
    }
}

fn probe_rpc(rpc_address: &str) -> bool {
    let client = RpcClient::new(rpc_address.to_string());
    matches!(
        client.call(&RpcRequest::GetNetwork),
        Ok(RpcResponse::Network(_))
    )
}

fn node_binary_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ATHO_NODE_BIN") {
        return Some(PathBuf::from(path));
    }

    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    let candidate_dir = if parent.ends_with("deps") {
        parent.parent()?
    } else {
        parent
    };
    let name = if cfg!(windows) { "athod.exe" } else { "athod" };
    let candidate = candidate_dir.join(name);
    candidate.exists().then_some(candidate)
}

fn workspace_manifest_path() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_dir
        .parent()
        .and_then(|path| path.parent())
        .unwrap_or(crate_dir.as_path());
    workspace_root.join("Cargo.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_rpc::request::RpcRequest;
    use atho_rpc::response::RpcResponse;

    #[test]
    fn read_only_connection_forwards_rpc_requests() {
        let conn = ReadOnlyNodeConnection::new(Network::Mainnet);
        assert_eq!(
            conn.request(RpcRequest::GetNetwork),
            RpcResponse::Network("atho-mainnet".into())
        );
        assert_eq!(conn.status().network, Network::Mainnet);
    }
}
