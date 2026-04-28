use atho_core::network::Network;
use atho_node::system::AthoSystem;
use atho_rpc::error::RpcError;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::{MempoolInfo, NetworkPeerDiagnostics, NodeStatus, RpcResponse};
use atho_rpc::transport::RpcClient;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

const ATHO_QT_LOCAL_ENV: &str = "ATHO_QT_LOCAL";
const ATHO_QT_FORCE_RPC_ENV: &str = "ATHO_QT_FORCE_RPC";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub network: Network,
    pub rpc_address: String,
    pub block_count: u64,
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub peer_count: usize,
    pub inbound_peer_count: usize,
    pub outbound_peer_count: usize,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub peers: Vec<NetworkPeerDiagnostics>,
    pub running: bool,
    pub headers_synced: bool,
    pub sync_best_height: u64,
    pub connected: bool,
    pub startup_error: Option<String>,
}

#[derive(Debug)]
enum ConnectionBackend {
    Local(Arc<Mutex<AthoSystem>>),
    Unavailable {
        startup_error: String,
    },
    Rpc {
        client: RpcClient,
        node: Option<Arc<ManagedNodeState>>,
    },
}

impl Clone for ConnectionBackend {
    fn clone(&self) -> Self {
        match self {
            ConnectionBackend::Local(system) => ConnectionBackend::Local(Arc::clone(system)),
            ConnectionBackend::Unavailable { startup_error } => ConnectionBackend::Unavailable {
                startup_error: startup_error.clone(),
            },
            ConnectionBackend::Rpc { client, node } => ConnectionBackend::Rpc {
                client: client.clone(),
                node: node.clone(),
            },
        }
    }
}

#[derive(Debug)]
pub struct StatusMonitor {
    receiver: mpsc::Receiver<ConnectionStatus>,
}

impl StatusMonitor {
    pub fn try_recv_latest(&self) -> Option<ConnectionStatus> {
        let mut latest = None;
        while let Ok(status) = self.receiver.try_recv() {
            latest = Some(status);
        }
        latest
    }
}

#[derive(Debug)]
struct ManagedNodeState {
    child: Mutex<Option<Child>>,
    startup_error: Mutex<Option<String>>,
}

impl ManagedNodeState {
    fn new(child: Child) -> Self {
        Self {
            child: Mutex::new(Some(child)),
            startup_error: Mutex::new(None),
        }
    }

    fn startup_error(&self) -> Option<String> {
        self.startup_error
            .lock()
            .expect("managed node startup error mutex poisoned")
            .clone()
    }

    fn set_startup_error(&self, error: String) -> String {
        let mut startup_error = self
            .startup_error
            .lock()
            .expect("managed node startup error mutex poisoned");
        if startup_error.is_none() {
            *startup_error = Some(error.clone());
        }
        startup_error
            .clone()
            .expect("managed node startup error must be set")
    }

    fn observe_exit(&self, network: Network) -> Option<String> {
        if let Some(error) = self.startup_error() {
            return Some(error);
        }

        let mut child = self
            .child
            .lock()
            .expect("managed node child mutex poisoned");
        let Some(process) = child.as_mut() else {
            return self.startup_error();
        };

        match process.try_wait() {
            Ok(Some(status)) => {
                let error = if status.success() {
                    format!("local node exited unexpectedly for {}", network.id())
                } else {
                    format!(
                        "local node exited with status {status} for {}",
                        network.id()
                    )
                };
                let _ = child.take();
                Some(self.set_startup_error(error))
            }
            Ok(None) => None,
            Err(err) => {
                let _ = child.take();
                Some(self.set_startup_error(format!("failed to poll local node state: {err}")))
            }
        }
    }
}

impl Drop for ManagedNodeState {
    fn drop(&mut self) {
        if let Some(mut child) = self
            .child
            .lock()
            .expect("managed node child mutex poisoned")
            .take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug)]
pub struct ReadOnlyNodeConnection {
    backend: ConnectionBackend,
    network: Network,
    rpc_address: String,
}

impl Clone for ReadOnlyNodeConnection {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            network: self.network,
            rpc_address: self.rpc_address.clone(),
        }
    }
}

impl ReadOnlyNodeConnection {
    pub fn new(network: Network) -> Self {
        Self::with_rpc_address(network, default_rpc_address(network))
    }

    pub fn with_rpc_address(network: Network, rpc_address: String) -> Self {
        let backend = if use_inprocess_backend() {
            match AthoSystem::try_new(atho_node::config::NodeConfig::new(network)) {
                Ok(mut system) => {
                    system.start();
                    ConnectionBackend::Local(Arc::new(Mutex::new(system)))
                }
                Err(err) => {
                    let startup_error = format!("local node startup failed: {err}");
                    let _ = atho_node::dev::append_log("atho-qt", &startup_error);
                    ConnectionBackend::Unavailable { startup_error }
                }
            }
        } else {
            match start_local_node_if_needed(network, &rpc_address) {
                Ok(node) => ConnectionBackend::Rpc {
                    client: RpcClient::new(rpc_address.clone()),
                    node,
                },
                Err(startup_error) => ConnectionBackend::Unavailable { startup_error },
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
        matches!(
            &self.backend,
            ConnectionBackend::Local(_)
                | ConnectionBackend::Unavailable { .. }
                | ConnectionBackend::Rpc { node: Some(_), .. }
        )
    }

    pub fn request(&self, request: RpcRequest) -> RpcResponse {
        match &self.backend {
            ConnectionBackend::Local(system) => {
                let mut system = system.lock().expect("local node mutex poisoned");
                if matches!(
                    &request,
                    RpcRequest::SubmitBlock(_) | RpcRequest::SubmitTransaction { .. }
                ) {
                    system.handle_mut(request)
                } else {
                    system.handle(request)
                }
            }
            ConnectionBackend::Rpc { client, node } => {
                if let Some(node) = node {
                    if let Some(error) = node.observe_exit(self.network) {
                        return RpcResponse::Error(RpcError::InvalidRequest(error));
                    }
                }
                match client.call(&request) {
                    Ok(response) => response,
                    Err(err) => {
                        if let Some(node) = node {
                            if let Some(error) = node.observe_exit(self.network) {
                                return RpcResponse::Error(RpcError::InvalidRequest(error));
                            }
                            return RpcResponse::Error(RpcError::InvalidRequest(format!(
                                "local node RPC is not ready yet: {err}"
                            )));
                        }
                        let _ = atho_node::dev::append_log("atho-qt", &format!("rpc error: {err}"));
                        RpcResponse::Error(RpcError::Internal)
                    }
                }
            }
            ConnectionBackend::Unavailable { startup_error } => {
                RpcResponse::Error(RpcError::InvalidRequest(startup_error.clone()))
            }
        }
    }

    pub fn status(&self) -> ConnectionStatus {
        match &self.backend {
            ConnectionBackend::Local(system) => {
                let system = system.lock().expect("local node mutex poisoned");
                connection_status_from_node_status(
                    self.network,
                    self.rpc_address.clone(),
                    system.node_status(),
                )
            }
            ConnectionBackend::Unavailable { startup_error } => ConnectionStatus {
                network: self.network,
                rpc_address: self.rpc_address.clone(),
                block_count: 0,
                mempool_count: 0,
                mempool_total_fee_atoms: 0,
                peer_count: 0,
                inbound_peer_count: 0,
                outbound_peer_count: 0,
                bytes_sent: 0,
                bytes_received: 0,
                peers: Vec::new(),
                running: false,
                headers_synced: false,
                sync_best_height: 0,
                connected: false,
                startup_error: Some(startup_error.clone()),
            },
            ConnectionBackend::Rpc { client, node } => {
                collect_rpc_status(self.network, &self.rpc_address, client, node.as_ref())
            }
        }
    }

    pub fn spawn_status_monitor(&self, interval: Duration) -> StatusMonitor {
        let rpc_address = self.rpc_address.clone();
        let (sender, receiver) = mpsc::channel();

        match &self.backend {
            ConnectionBackend::Local(system) => {
                let system = Arc::clone(system);
                thread::spawn(move || loop {
                    let status = {
                        let system = system.lock().expect("local node mutex poisoned");
                        connection_status_from_node_status(
                            system.network(),
                            rpc_address.clone(),
                            system.node_status(),
                        )
                    };
                    if sender.send(status).is_err() {
                        break;
                    }
                    thread::sleep(interval);
                });
            }
            ConnectionBackend::Rpc { node, .. } => {
                let network = self.network;
                let node = node.clone();
                thread::spawn(move || {
                    let client = RpcClient::new(rpc_address.clone());
                    loop {
                        let status =
                            collect_rpc_status(network, &rpc_address, &client, node.as_ref());
                        if sender.send(status).is_err() {
                            break;
                        }
                        thread::sleep(interval);
                    }
                });
            }
            ConnectionBackend::Unavailable { startup_error } => {
                let network = self.network;
                let rpc_address = self.rpc_address.clone();
                let startup_error = startup_error.clone();
                thread::spawn(move || loop {
                    let status = ConnectionStatus {
                        network,
                        rpc_address: rpc_address.clone(),
                        block_count: 0,
                        mempool_count: 0,
                        mempool_total_fee_atoms: 0,
                        peer_count: 0,
                        inbound_peer_count: 0,
                        outbound_peer_count: 0,
                        bytes_sent: 0,
                        bytes_received: 0,
                        peers: Vec::new(),
                        running: false,
                        headers_synced: false,
                        sync_best_height: 0,
                        connected: false,
                        startup_error: Some(startup_error.clone()),
                    };
                    if sender.send(status).is_err() {
                        break;
                    }
                    thread::sleep(interval);
                });
            }
        }

        StatusMonitor { receiver }
    }

    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn with_local_system_for_test<T>(
        &self,
        f: impl FnOnce(&mut AthoSystem) -> T,
    ) -> Option<T> {
        match &self.backend {
            ConnectionBackend::Local(system) => {
                let mut system = system.lock().expect("local node mutex poisoned");
                Some(f(&mut system))
            }
            _ => None,
        }
    }
}

fn collect_rpc_status(
    network: Network,
    rpc_address: &str,
    client: &RpcClient,
    managed_node: Option<&Arc<ManagedNodeState>>,
) -> ConnectionStatus {
    if let Some(node) = managed_node {
        if let Some(startup_error) = node.observe_exit(network) {
            return ConnectionStatus {
                network,
                rpc_address: rpc_address.to_string(),
                block_count: 0,
                mempool_count: 0,
                mempool_total_fee_atoms: 0,
                peer_count: 0,
                inbound_peer_count: 0,
                outbound_peer_count: 0,
                bytes_sent: 0,
                bytes_received: 0,
                peers: Vec::new(),
                running: false,
                headers_synced: false,
                sync_best_height: 0,
                connected: false,
                startup_error: Some(startup_error),
            };
        }
    }

    if let Ok(RpcResponse::NodeStatus(status)) = client.call(&RpcRequest::GetNodeStatus) {
        return connection_status_from_node_status(network, rpc_address.to_string(), status);
    }

    if let Some(node) = managed_node {
        return ConnectionStatus {
            network,
            rpc_address: rpc_address.to_string(),
            block_count: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            peer_count: 0,
            inbound_peer_count: 0,
            outbound_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            running: false,
            headers_synced: false,
            sync_best_height: 0,
            connected: false,
            startup_error: node.startup_error(),
        };
    }

    let network_ok = match client.call(&RpcRequest::GetNetwork) {
        Ok(RpcResponse::Network(label)) => label == network.id(),
        _ => false,
    };
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
    ConnectionStatus {
        network,
        rpc_address: rpc_address.to_string(),
        block_count,
        mempool_count,
        mempool_total_fee_atoms: 0,
        peer_count: 0,
        inbound_peer_count: 0,
        outbound_peer_count: 0,
        bytes_sent: 0,
        bytes_received: 0,
        peers: Vec::new(),
        running: network_ok,
        headers_synced: network_ok,
        sync_best_height: block_count,
        connected: network_ok,
        startup_error: None,
    }
}

fn connection_status_from_node_status(
    expected_network: Network,
    rpc_address: String,
    status: NodeStatus,
) -> ConnectionStatus {
    let connected = status.network == expected_network && status.running;
    ConnectionStatus {
        network: status.network,
        rpc_address,
        block_count: status.block_count,
        mempool_count: status.mempool_count,
        mempool_total_fee_atoms: status.mempool_total_fee_atoms,
        peer_count: status.network_diagnostics.peer_count,
        inbound_peer_count: status.network_diagnostics.inbound_peer_count,
        outbound_peer_count: status.network_diagnostics.outbound_peer_count,
        bytes_sent: status.network_diagnostics.bytes_sent,
        bytes_received: status.network_diagnostics.bytes_received,
        peers: status.network_diagnostics.peers,
        running: status.running,
        headers_synced: status.headers_synced,
        sync_best_height: status.sync_best_height,
        connected,
        startup_error: None,
    }
}

fn default_rpc_address(network: Network) -> String {
    atho_node::runtime::rpc_bind_address(network)
}

fn use_inprocess_backend() -> bool {
    cfg!(test) && std::env::var(ATHO_QT_FORCE_RPC_ENV).ok().as_deref() != Some("1")
}

fn manage_local_node_requested() -> bool {
    std::env::var(ATHO_QT_LOCAL_ENV).ok().as_deref() == Some("1")
}

fn start_local_node_if_needed(
    network: Network,
    rpc_address: &str,
) -> Result<Option<Arc<ManagedNodeState>>, String> {
    if !manage_local_node_requested() {
        return Ok(None);
    }
    if probe_rpc(rpc_address) {
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "rpc already available at {} for {}",
                rpc_address,
                network.id()
            ),
        );
        return Ok(None);
    }

    let mut command = if prefer_workspace_node_command() {
        let _ = atho_node::dev::append_log(
            "atho-qt",
            "using cargo-run managed node path for source-matched local testing",
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
            .arg("--network")
            .arg(network.cli_arg())
            .arg("--rpc-addr")
            .arg(rpc_address);
        command
    } else if let Some(binary) = node_binary_path() {
        let mut command = Command::new(binary);
        command
            .arg("--network")
            .arg(network.cli_arg())
            .arg("--rpc-addr")
            .arg(rpc_address);
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
            .arg("--network")
            .arg(network.cli_arg())
            .arg("--rpc-addr")
            .arg(rpc_address);
        command
    };
    let (stdout, stderr) = local_node_stdio(network)?;
    command
        .env("ATHO_RPC_ADDR", rpc_address)
        .env("ATHO_NETWORK", network.cli_arg())
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);

    match command.spawn() {
        Ok(child) => {
            let node = Arc::new(ManagedNodeState::new(child));
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!(
                    "spawned local node bootstrap for {} rpc={} stdio_log={}",
                    network.id(),
                    rpc_address,
                    local_node_stdio_log_path(network).display()
                ),
            );
            spawn_bootstrap_watcher(network, rpc_address.to_string(), Arc::clone(&node));
            Ok(Some(node))
        }
        Err(err) => {
            let startup_error = format!("failed to spawn local node: {err}");
            let _ = atho_node::dev::append_log("atho-qt", &startup_error);
            Err(startup_error)
        }
    }
}

fn prefer_workspace_node_command() -> bool {
    if std::env::var_os("ATHO_NODE_BIN").is_some() {
        return false;
    }
    cfg!(debug_assertions) && workspace_manifest_path().exists()
}

fn spawn_bootstrap_watcher(
    network: Network,
    rpc_address: String,
    managed_node: Arc<ManagedNodeState>,
) {
    thread::spawn(move || {
        let client = RpcClient::new(rpc_address.clone());
        for attempt in 0..90 {
            if let Some(error) = managed_node.observe_exit(network) {
                let _ = atho_node::dev::append_log("atho-qt", &error);
                return;
            }
            if matches!(
                client.call(&RpcRequest::GetNodeStatus),
                Ok(RpcResponse::NodeStatus(_))
            ) {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!(
                        "local node ready network={} rpc={} attempts={}",
                        network.id(),
                        rpc_address,
                        attempt + 1
                    ),
                );
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "local node bootstrap still starting network={} rpc={}",
                network.id(),
                rpc_address
            ),
        );
    });
}

fn probe_rpc(rpc_address: &str) -> bool {
    let client = RpcClient::new(rpc_address.to_string());
    matches!(
        client.call(&RpcRequest::GetNetwork),
        Ok(RpcResponse::Network(_))
    )
}

fn local_node_stdio_log_path(network: Network) -> PathBuf {
    atho_node::dev::logs_dir().join(format!("athod-{}-stdio.log", network.cli_arg()))
}

fn local_node_stdio(network: Network) -> Result<(Stdio, Stdio), String> {
    let path = local_node_stdio_log_path(network);
    let _ = atho_node::dev::ensure_layout();
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| {
            format!(
                "failed to open local node stdio log {}: {err}",
                path.display()
            )
        })?;
    let stderr = stdout.try_clone().map_err(|err| {
        format!(
            "failed to clone local node stdio log {}: {err}",
            path.display()
        )
    })?;
    Ok((Stdio::from(stdout), Stdio::from(stderr)))
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
    use atho_node::miner::Miner;
    use atho_rpc::request::RpcRequest;
    use atho_rpc::response::{
        MempoolInfo, NetworkDiagnostics, NetworkPeerDiagnostics, NetworkPeerDirection, NodeStatus,
        RpcResponse,
    };
    use atho_rpc::transport::{read_message, write_message};
    use atho_storage::db::{ChainstateSnapshot, Database};
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use atho_storage::utxo::UtxoEntry;
    use std::ffi::OsString;
    use std::fs;
    use std::io::BufReader;
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn set_value(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn temp_data_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "atho-qt-connection-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn spawn_mock_rpc_server(
        network: Network,
        block_count: u64,
        mempool_count: usize,
        total_fee_atoms: u64,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock rpc");
        let address = listener.local_addr().expect("local addr").to_string();
        let handle = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept");
                let clone = stream.try_clone().expect("clone");
                let mut reader = BufReader::new(clone);
                let request: RpcRequest = read_message(&mut reader).expect("request");
                let response = match request {
                    RpcRequest::GetNodeStatus => RpcResponse::NodeStatus(NodeStatus {
                        network,
                        block_count,
                        mempool_count,
                        mempool_total_fee_atoms: total_fee_atoms,
                        running: true,
                        headers_synced: true,
                        sync_best_height: block_count,
                        network_diagnostics: NetworkDiagnostics {
                            peer_count: 1,
                            inbound_peer_count: 0,
                            outbound_peer_count: 1,
                            bytes_sent: 2_048,
                            bytes_received: 4_096,
                            peers: vec![NetworkPeerDiagnostics {
                                remote_addr: String::from("74.208.219.116:56000"),
                                direction: NetworkPeerDirection::Outbound,
                                handshake_ready: true,
                                best_height: Some(block_count),
                                protocol_version: Some(1),
                                services: Some(9),
                                user_agent: Some(String::from("/Atho:0.1.0/")),
                                ruleset_version: Some(1),
                                bytes_sent: 2_048,
                                bytes_received: 4_096,
                                last_send_unix: Some(1_777_416_445),
                                last_receive_unix: Some(1_777_416_445),
                                quality_score: Some(100),
                                consecutive_failures: Some(0),
                            }],
                        },
                    }),
                    RpcRequest::GetNetwork => RpcResponse::Network(network.id().to_string()),
                    RpcRequest::GetBlockCount => RpcResponse::BlockCount(block_count),
                    RpcRequest::GetMempoolInfo => RpcResponse::MempoolInfo(MempoolInfo {
                        transaction_count: mempool_count,
                        total_fee_atoms,
                    }),
                    other => RpcResponse::Error(RpcError::InvalidRequest(format!(
                        "unexpected request in mock rpc server: {other:?}"
                    ))),
                };
                write_message(&mut stream, &response).expect("response");
            }
        });
        (address, handle)
    }

    fn wait_for_status(
        conn: &ReadOnlyNodeConnection,
        predicate: impl Fn(&ConnectionStatus) -> bool,
    ) -> ConnectionStatus {
        for _ in 0..200 {
            let status = conn.status();
            if predicate(&status) {
                return status;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("timed out waiting for connection status");
    }

    fn free_rpc_address() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
        let address = listener.local_addr().expect("local addr").to_string();
        drop(listener);
        address
    }

    #[test]
    fn read_only_connection_forwards_rpc_requests() {
        let conn = ReadOnlyNodeConnection::new(Network::Mainnet);
        assert_eq!(
            conn.request(RpcRequest::GetNetwork),
            RpcResponse::Network("atho-mainnet".into())
        );
        assert_eq!(conn.status().network, Network::Mainnet);
    }

    #[test]
    fn local_connection_recovers_incomplete_history_without_aborting() {
        let root = temp_data_dir("recover");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_path(ATHO_QT_LOCAL_ENV, std::path::Path::new("1"));

        let db = Database::open(Network::Mainnet).expect("database");
        db.save_chainstate_snapshot(
            &ChainstateSnapshot {
                height: 1,
                tip_hash: [9; 48],
                tip_header: None,
            },
            &[],
        )
        .expect("snapshot");

        let conn = ReadOnlyNodeConnection::new(Network::Mainnet);
        let status = conn.status();
        assert!(status.connected);
        assert!(status.running);
        assert_eq!(status.block_count, 0);
        assert_eq!(status.startup_error, None);
    }

    #[test]
    fn local_flag_uses_rpc_status_path_when_rpc_is_available() {
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let (rpc_address, handle) = spawn_mock_rpc_server(Network::Mainnet, 42, 3, 55);

        let conn = ReadOnlyNodeConnection::with_rpc_address(Network::Mainnet, rpc_address);
        let status = conn.status();
        assert!(status.connected);
        assert!(status.running);
        assert_eq!(status.block_count, 42);
        assert_eq!(status.mempool_count, 3);
        assert_eq!(status.mempool_total_fee_atoms, 55);
        assert_eq!(status.sync_best_height, 42);
        assert_eq!(status.peer_count, 1);
        assert_eq!(status.outbound_peer_count, 1);
        assert_eq!(status.bytes_sent, 2_048);
        assert_eq!(status.bytes_received, 4_096);
        assert_eq!(status.peers.len(), 1);
        assert_eq!(status.peers[0].remote_addr, "74.208.219.116:56000");
        assert_eq!(
            conn.request(RpcRequest::GetNetwork),
            RpcResponse::Network(String::from("atho-mainnet"))
        );

        handle.join().expect("mock rpc server");
    }

    #[test]
    fn managed_local_node_rpc_status_tracks_real_chain_tip() {
        let root = temp_data_dir("real-rpc");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let rpc_address = free_rpc_address();

        let conn = ReadOnlyNodeConnection::with_rpc_address(Network::Regnet, rpc_address);
        let status = wait_for_status(&conn, |status| status.connected && status.running);
        assert_eq!(status.block_count, 0);

        let template = match conn.request(RpcRequest::GetBlockTemplate) {
            RpcResponse::BlockTemplate(template) => template,
            other => panic!("expected block template, got {other:?}"),
        };
        let block = Miner::new(1).solve_block(template.block);
        match conn.request(RpcRequest::SubmitBlock(block)) {
            RpcResponse::BlockSubmitted { accepted: true, .. } => {}
            other => panic!("expected accepted block submission, got {other:?}"),
        }

        let status = wait_for_status(&conn, |status| status.block_count >= 1);
        assert!(status.connected);
        assert!(status.running);
        assert_eq!(status.block_count, 1);
        assert_eq!(status.sync_best_height, 1);
    }

    #[test]
    fn local_backend_seeded_utxo_keeps_block_height_progressing() {
        let root = temp_data_dir("local-seed-followup");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        std::env::remove_var(ATHO_QT_FORCE_RPC_ENV);

        let conn = ReadOnlyNodeConnection::new(Network::Regnet);
        let status = wait_for_status(&conn, |status| status.connected && status.running);
        assert_eq!(status.block_count, 0);

        let first_template = match conn.request(RpcRequest::GetBlockTemplate) {
            RpcResponse::BlockTemplate(template) => template,
            other => panic!("expected block template, got {other:?}"),
        };
        let first_block = Miner::new(1).solve_block(first_template.block);
        match conn.request(RpcRequest::SubmitBlock(first_block)) {
            RpcResponse::BlockSubmitted { accepted: true, .. } => {}
            other => panic!("expected accepted first block submission, got {other:?}"),
        }
        let status = wait_for_status(&conn, |status| status.block_count >= 1);
        assert_eq!(status.block_count, 1);

        let seeded = conn.with_local_system_for_test(|system| {
            system.sandbox_with_node_mut(|node| {
                node.dev_seed_chainstate(
                    node.height(),
                    node.tip_hash(),
                    [UtxoEntry::new(
                        Network::Regnet,
                        [0x5a; 48],
                        0,
                        20 * atho_core::constants::ATOMS_PER_ATHO,
                        vec![0x42; 32],
                        node.height(),
                        false,
                    )],
                )
                .expect("seed utxo");
            });
        });
        assert!(seeded.is_some(), "expected local backend");

        let second_template = match conn.request(RpcRequest::GetBlockTemplate) {
            RpcResponse::BlockTemplate(template) => template,
            other => panic!("expected second block template, got {other:?}"),
        };
        let second_block = Miner::new(1).solve_block(second_template.block);
        match conn.request(RpcRequest::SubmitBlock(second_block)) {
            RpcResponse::BlockSubmitted { accepted: true, .. } => {}
            other => panic!("expected accepted second block submission, got {other:?}"),
        }
        let status = wait_for_status(&conn, |status| status.block_count >= 2);
        assert_eq!(status.block_count, 2);
        assert_eq!(status.sync_best_height, 2);
    }
}
