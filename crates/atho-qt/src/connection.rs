//! Client-side RPC and managed local-node connection handling.
use atho_core::network::Network;
use atho_node::system::AthoSystem;
use atho_rpc::command::{command_requires_mutable_access, CommandInvocation};
use atho_rpc::error::RpcError;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::{MempoolInfo, NetworkPeerDiagnostics, NodeStatus, RpcResponse};
use atho_rpc::transport::{RpcClient, RpcTransportError};
use atho_storage::db::Database;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

const ATHO_QT_LOCAL_ENV: &str = "ATHO_QT_LOCAL";
const ATHO_QT_FORCE_RPC_ENV: &str = "ATHO_QT_FORCE_RPC";
const LOCAL_RPC_READY_RETRY_ATTEMPTS: usize = 20;
const LOCAL_RPC_READY_RETRY_DELAY_MS: u64 = 100;
const LOCAL_RPC_STATUS_ERROR_RETRY_ATTEMPTS: usize = 2;
const LOCAL_RPC_STOP_RETRY_ATTEMPTS: usize = 50;
const LEGACY_RPC_COMPAT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const LEGACY_RPC_COMPAT_IO_TIMEOUT: Duration = Duration::from_secs(2);
const LOCAL_RPC_FORCE_STOP_RETRY_ATTEMPTS: usize = 30;
const MANAGED_LOCAL_NODE_SELF_HEAL_RESTARTS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PersistedChainTipStatus {
    height: u64,
    tip_hash: [u8; 48],
    tip_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub network: Network,
    pub rpc_address: String,
    pub block_count: u64,
    pub tip_hash: [u8; 48],
    pub tip_timestamp: u64,
    pub estimated_hashrate_hps: u64,
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub mempool_fingerprint: [u8; 32],
    pub peer_count: usize,
    pub inbound_peer_count: usize,
    pub outbound_peer_count: usize,
    pub connecting_peer_count: usize,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub peers: Vec<NetworkPeerDiagnostics>,
    pub connecting_peers: Vec<NetworkPeerDiagnostics>,
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
        local_node: bool,
    },
}

impl Clone for ConnectionBackend {
    fn clone(&self) -> Self {
        match self {
            ConnectionBackend::Local(system) => ConnectionBackend::Local(Arc::clone(system)),
            ConnectionBackend::Unavailable { startup_error } => ConnectionBackend::Unavailable {
                startup_error: startup_error.clone(),
            },
            ConnectionBackend::Rpc {
                client,
                node,
                local_node,
            } => ConnectionBackend::Rpc {
                client: client.clone(),
                node: node.clone(),
                local_node: *local_node,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct LocalNodeStartup {
    node: Option<Arc<ManagedNodeState>>,
    local_node: bool,
}

impl LocalNodeStartup {
    fn external_rpc() -> Self {
        Self {
            node: None,
            local_node: false,
        }
    }

    fn managed(node: Arc<ManagedNodeState>) -> Self {
        Self {
            node: Some(node),
            local_node: true,
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
    network: Network,
    rpc_address: String,
    child: Mutex<Option<Child>>,
    startup_error: Mutex<Option<String>>,
    self_heal_restarts: Mutex<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortOwnerProcess {
    pid: u32,
    command: String,
}

impl ManagedNodeState {
    fn new(network: Network, rpc_address: String, child: Child) -> Self {
        Self {
            network,
            rpc_address,
            child: Mutex::new(Some(child)),
            startup_error: Mutex::new(None),
            self_heal_restarts: Mutex::new(0),
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
        debug_assert_eq!(network, self.network);
        if let Some(error) = self.startup_error() {
            return Some(error);
        }

        let poll_result = {
            let mut child = self
                .child
                .lock()
                .expect("managed node child mutex poisoned");
            let Some(process) = child.as_mut() else {
                return self.startup_error();
            };

            match process.try_wait() {
                Ok(Some(status)) => {
                    let _ = child.take();
                    Ok(Some(status))
                }
                Ok(None) => Ok(None),
                Err(err) => Err(err),
            }
        };

        match poll_result {
            Ok(Some(status)) => {
                let error = managed_node_exit_error(network, &status);
                match self.self_heal_after_exit(&status) {
                    Ok(true) => None,
                    Ok(false) => Some(self.set_startup_error(error)),
                    Err(err) => Some(
                        self.set_startup_error(format!("{error}; self-heal restart failed: {err}")),
                    ),
                }
            }
            Ok(None) => None,
            Err(err) => {
                Some(self.set_startup_error(format!("failed to poll local node state: {err}")))
            }
        }
    }

    fn self_heal_after_exit(&self, status: &std::process::ExitStatus) -> Result<bool, String> {
        if !managed_node_exit_is_retryable(status) {
            return Ok(false);
        }

        let attempt = {
            let mut restarts = self
                .self_heal_restarts
                .lock()
                .expect("managed node self-heal mutex poisoned");
            if *restarts >= MANAGED_LOCAL_NODE_SELF_HEAL_RESTARTS {
                return Ok(false);
            }
            *restarts += 1;
            *restarts
        };

        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "local node exited with status {status} for {}; attempting lightweight self-heal restart {attempt}/{} rpc={}",
                self.network.id(),
                MANAGED_LOCAL_NODE_SELF_HEAL_RESTARTS,
                self.rpc_address
            ),
        );

        self_heal_managed_local_node_before_start(self.network, &self.rpc_address)?;
        let child = spawn_managed_local_node_child(self.network, &self.rpc_address)?;
        {
            let mut child_slot = self
                .child
                .lock()
                .expect("managed node child mutex poisoned");
            *child_slot = Some(child);
        }
        {
            let mut startup_error = self
                .startup_error
                .lock()
                .expect("managed node startup error mutex poisoned");
            *startup_error = None;
        }

        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "local node self-heal restart launched for {} rpc={} stdio_log={}",
                self.network.id(),
                self.rpc_address,
                local_node_stdio_log_path(self.network).display()
            ),
        );
        Ok(true)
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

fn managed_node_exit_error(network: Network, status: &std::process::ExitStatus) -> String {
    if status.success() {
        format!("local node exited unexpectedly for {}", network.id())
    } else {
        format!(
            "local node exited with status {status} for {}",
            network.id()
        )
    }
}

fn managed_node_exit_is_retryable(status: &std::process::ExitStatus) -> bool {
    managed_node_exit_status_is_retryable(status.success(), status.code())
}

fn managed_node_exit_status_is_retryable(success: bool, _code: Option<i32>) -> bool {
    !success
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
                Ok(startup) => ConnectionBackend::Rpc {
                    client: RpcClient::new(rpc_address.clone()),
                    node: startup.node,
                    local_node: startup.local_node,
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
                | ConnectionBackend::Rpc {
                    local_node: true,
                    ..
                }
        )
    }

    pub fn request(&self, request: RpcRequest) -> RpcResponse {
        match &self.backend {
            ConnectionBackend::Local(system) => {
                let mut system = system.lock().expect("local node mutex poisoned");
                let requires_mutable = match &request {
                    RpcRequest::SubmitBlock(_) | RpcRequest::SubmitTransaction { .. } => true,
                    RpcRequest::ExecuteCommand(invocation) => {
                        command_requires_mutable_access(&invocation.name)
                    }
                    _ => false,
                };
                if requires_mutable {
                    system.handle_mut(request)
                } else {
                    system.handle(request)
                }
            }
            ConnectionBackend::Rpc {
                client,
                node,
                local_node,
            } => {
                if let Some(node) = node {
                    if let Some(error) = node.observe_exit(self.network) {
                        return RpcResponse::Error(RpcError::invalid_request(error));
                    }
                }
                for attempt in 0..LOCAL_RPC_READY_RETRY_ATTEMPTS {
                    match client.call(&request) {
                        Ok(response) => return response,
                        Err(err) => {
                            if *local_node {
                                if let Some(node) = node {
                                    if let Some(error) = node.observe_exit(self.network) {
                                        return RpcResponse::Error(RpcError::invalid_request(
                                            error,
                                        ));
                                    }
                                }
                                if attempt + 1 < LOCAL_RPC_READY_RETRY_ATTEMPTS {
                                    thread::sleep(Duration::from_millis(
                                        LOCAL_RPC_READY_RETRY_DELAY_MS,
                                    ));
                                    continue;
                                }
                                return RpcResponse::Error(RpcError::invalid_request(format!(
                                    "local node RPC is not ready yet: {err}"
                                )));
                            }
                            if let Some(node) = node {
                                if let Some(error) = node.observe_exit(self.network) {
                                    return RpcResponse::Error(RpcError::invalid_request(error));
                                }
                            }
                            let _ =
                                atho_node::dev::append_log("atho-qt", &format!("rpc error: {err}"));
                            return RpcResponse::Error(RpcError::internal());
                        }
                    }
                }
                RpcResponse::Error(RpcError::internal())
            }
            ConnectionBackend::Unavailable { startup_error } => {
                RpcResponse::Error(RpcError::invalid_request(startup_error.clone()))
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
                tip_hash: [0; 48],
                tip_timestamp: 0,
                estimated_hashrate_hps: 0,
                mempool_count: 0,
                mempool_total_fee_atoms: 0,
                mempool_fingerprint: [0; 32],
                peer_count: 0,
                inbound_peer_count: 0,
                outbound_peer_count: 0,
                connecting_peer_count: 0,
                bytes_sent: 0,
                bytes_received: 0,
                peers: Vec::new(),
                connecting_peers: Vec::new(),
                running: false,
                headers_synced: false,
                sync_best_height: 0,
                connected: false,
                startup_error: Some(startup_error.clone()),
            },
            ConnectionBackend::Rpc {
                client,
                node,
                local_node,
            } => collect_rpc_status(
                self.network,
                &self.rpc_address,
                client,
                node.as_ref(),
                *local_node,
                None,
            ),
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
            ConnectionBackend::Rpc {
                node, local_node, ..
            } => {
                let network = self.network;
                let node = node.clone();
                let local_node = *local_node;
                thread::spawn(move || {
                    let client = RpcClient::new(rpc_address.clone());
                    let mut last_status: Option<ConnectionStatus> = None;
                    loop {
                        let status = collect_rpc_status(
                            network,
                            &rpc_address,
                            &client,
                            node.as_ref(),
                            local_node,
                            last_status.as_ref(),
                        );
                        last_status = Some(status.clone());
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
                        tip_hash: [0; 48],
                        tip_timestamp: 0,
                        estimated_hashrate_hps: 0,
                        mempool_count: 0,
                        mempool_total_fee_atoms: 0,
                        mempool_fingerprint: [0; 32],
                        peer_count: 0,
                        inbound_peer_count: 0,
                        outbound_peer_count: 0,
                        connecting_peer_count: 0,
                        bytes_sent: 0,
                        bytes_received: 0,
                        peers: Vec::new(),
                        connecting_peers: Vec::new(),
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
    local_node: bool,
    last_known: Option<&ConnectionStatus>,
) -> ConnectionStatus {
    if let Some(node) = managed_node {
        if let Some(startup_error) = node.observe_exit(network) {
            return ConnectionStatus {
                network,
                rpc_address: rpc_address.to_string(),
                block_count: 0,
                tip_hash: [0; 48],
                tip_timestamp: 0,
                estimated_hashrate_hps: 0,
                mempool_count: 0,
                mempool_total_fee_atoms: 0,
                mempool_fingerprint: [0; 32],
                peer_count: 0,
                inbound_peer_count: 0,
                outbound_peer_count: 0,
                connecting_peer_count: 0,
                bytes_sent: 0,
                bytes_received: 0,
                peers: Vec::new(),
                connecting_peers: Vec::new(),
                running: false,
                headers_synced: false,
                sync_best_height: 0,
                connected: false,
                startup_error: Some(startup_error),
            };
        }
    }

    let managed_local = local_node;

    if let Ok(RpcResponse::NodeStatus(status)) =
        call_status_rpc(client, &RpcRequest::GetNodeStatus, managed_local)
    {
        let mut connection_status =
            connection_status_from_node_status(network, rpc_address.to_string(), status);
        if managed_local {
            connection_status.running = true;
            connection_status.connected = connection_status.network == network;
        }
        return connection_status;
    }

    let (network_replied, network_ok) =
        match call_status_rpc(client, &RpcRequest::GetNetwork, managed_local) {
            Ok(RpcResponse::Network(label)) => (true, label == network.id()),
            _ => (false, false),
        };
    let block_count_reply = match call_status_rpc(client, &RpcRequest::GetBlockCount, managed_local)
    {
        Ok(RpcResponse::BlockCount(count)) => Some(count),
        _ => None,
    };
    let mempool_reply = match call_status_rpc(client, &RpcRequest::GetMempoolInfo, managed_local) {
        Ok(RpcResponse::MempoolInfo(MempoolInfo {
            transaction_count,
            total_fee_atoms,
        })) => Some((transaction_count, total_fee_atoms)),
        _ => None,
    };
    let rpc_reachable = network_replied || block_count_reply.is_some() || mempool_reply.is_some();

    if managed_node.is_some() || (rpc_reachable && last_known.is_some()) {
        return degrade_rpc_status(DegradedRpcStatusInput {
            network,
            rpc_address,
            block_count_reply,
            mempool_reply,
            network_ok,
            rpc_reachable,
            managed_local,
            last_known,
        });
    }

    ConnectionStatus {
        network,
        rpc_address: rpc_address.to_string(),
        block_count: block_count_reply.unwrap_or(0),
        tip_hash: [0; 48],
        tip_timestamp: 0,
        estimated_hashrate_hps: 0,
        mempool_count: mempool_reply.map(|reply| reply.0).unwrap_or(0),
        mempool_total_fee_atoms: mempool_reply.map(|reply| reply.1).unwrap_or(0),
        mempool_fingerprint: [0; 32],
        peer_count: 0,
        inbound_peer_count: 0,
        outbound_peer_count: 0,
        connecting_peer_count: 0,
        bytes_sent: 0,
        bytes_received: 0,
        peers: Vec::new(),
        connecting_peers: Vec::new(),
        running: false,
        headers_synced: false,
        sync_best_height: block_count_reply.unwrap_or(0),
        connected: false,
        startup_error: None,
    }
}

fn call_status_rpc(
    client: &RpcClient,
    request: &RpcRequest,
    managed_local: bool,
) -> Result<RpcResponse, RpcTransportError> {
    let transport_attempts = if managed_local {
        LOCAL_RPC_READY_RETRY_ATTEMPTS
    } else {
        1
    };
    let status_error_attempts = if managed_local {
        LOCAL_RPC_STATUS_ERROR_RETRY_ATTEMPTS
    } else {
        1
    };

    let mut status_error_retries = 0usize;
    for attempt in 0..transport_attempts {
        match client.call(request) {
            Ok(RpcResponse::Error(_))
                if managed_local && status_error_retries + 1 < status_error_attempts =>
            {
                status_error_retries += 1;
                thread::sleep(Duration::from_millis(LOCAL_RPC_READY_RETRY_DELAY_MS));
            }
            Ok(response) => return Ok(response),
            Err(_err) if managed_local && attempt + 1 < transport_attempts => {
                thread::sleep(Duration::from_millis(LOCAL_RPC_READY_RETRY_DELAY_MS));
            }
            Err(err) => return Err(err),
        }
    }

    unreachable!("status RPC retry loop must return or error")
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
        tip_hash: status.tip_hash,
        tip_timestamp: status.tip_timestamp,
        estimated_hashrate_hps: status.estimated_hashrate_hps,
        mempool_count: status.mempool_count,
        mempool_total_fee_atoms: status.mempool_total_fee_atoms,
        mempool_fingerprint: status.mempool_fingerprint,
        peer_count: status.network_diagnostics.peer_count,
        inbound_peer_count: status.network_diagnostics.inbound_peer_count,
        outbound_peer_count: status.network_diagnostics.outbound_peer_count,
        connecting_peer_count: status.network_diagnostics.connecting_peer_count,
        bytes_sent: status.network_diagnostics.bytes_sent,
        bytes_received: status.network_diagnostics.bytes_received,
        peers: status.network_diagnostics.peers,
        connecting_peers: status.network_diagnostics.connecting_peers,
        running: status.running,
        headers_synced: status.headers_synced,
        sync_best_height: status.sync_best_height,
        connected,
        startup_error: None,
    }
}

struct DegradedRpcStatusInput<'a> {
    network: Network,
    rpc_address: &'a str,
    block_count_reply: Option<u64>,
    mempool_reply: Option<(usize, u64)>,
    network_ok: bool,
    rpc_reachable: bool,
    managed_local: bool,
    last_known: Option<&'a ConnectionStatus>,
}

fn degrade_rpc_status(input: DegradedRpcStatusInput<'_>) -> ConnectionStatus {
    let DegradedRpcStatusInput {
        network,
        rpc_address,
        block_count_reply,
        mempool_reply,
        network_ok,
        rpc_reachable,
        managed_local,
        last_known,
    } = input;
    let mut status = last_known.cloned().unwrap_or(ConnectionStatus {
        network,
        rpc_address: rpc_address.to_string(),
        block_count: block_count_reply.unwrap_or(0),
        tip_hash: [0; 48],
        tip_timestamp: 0,
        estimated_hashrate_hps: 0,
        mempool_count: mempool_reply.map(|reply| reply.0).unwrap_or(0),
        mempool_total_fee_atoms: mempool_reply.map(|reply| reply.1).unwrap_or(0),
        mempool_fingerprint: [0; 32],
        peer_count: 0,
        inbound_peer_count: 0,
        outbound_peer_count: 0,
        connecting_peer_count: 0,
        bytes_sent: 0,
        bytes_received: 0,
        peers: Vec::new(),
        connecting_peers: Vec::new(),
        running: false,
        headers_synced: false,
        sync_best_height: block_count_reply.unwrap_or(0),
        connected: false,
        startup_error: None,
    });

    if managed_local && !rpc_reachable {
        let persisted = load_persisted_chain_tip_status(network);
        status.network = network;
        status.rpc_address = rpc_address.to_string();
        status.block_count = persisted.map(|snapshot| snapshot.height).unwrap_or(0);
        status.tip_hash = persisted
            .map(|snapshot| snapshot.tip_hash)
            .unwrap_or([0; 48]);
        status.tip_timestamp = persisted
            .map(|snapshot| snapshot.tip_timestamp)
            .unwrap_or(0);
        status.mempool_count = 0;
        status.mempool_total_fee_atoms = 0;
        status.mempool_fingerprint = [0; 32];
        status.peer_count = 0;
        status.inbound_peer_count = 0;
        status.outbound_peer_count = 0;
        status.connecting_peer_count = 0;
        status.bytes_sent = 0;
        status.bytes_received = 0;
        status.peers.clear();
        status.connecting_peers.clear();
        status.running = false;
        status.headers_synced = false;
        status.connected = false;
        status.startup_error = None;
        status.sync_best_height = status.sync_best_height.max(status.block_count);
        return status;
    }

    status.network = network;
    status.rpc_address = rpc_address.to_string();
    if let Some(block_count) = block_count_reply {
        status.block_count = block_count;
        status.sync_best_height = status.sync_best_height.max(block_count);
    }
    if let Some((mempool_count, mempool_total_fee_atoms)) = mempool_reply {
        status.mempool_count = mempool_count;
        status.mempool_total_fee_atoms = mempool_total_fee_atoms;
    }
    status.connected = if managed_local {
        rpc_reachable
    } else {
        network_ok
            || (rpc_reachable
                && last_known
                    .is_some_and(|previous| previous.connected && previous.network == network))
    };
    status.running = rpc_reachable || status.running;
    status.startup_error = None;
    status
}

fn default_rpc_address(network: Network) -> String {
    atho_node::runtime::rpc_bind_address(network)
}

fn use_inprocess_backend() -> bool {
    if std::env::var(ATHO_QT_FORCE_RPC_ENV).ok().as_deref() == Some("1") {
        return false;
    }

    // The embedded backend is reserved for tests and explicit local-node overrides.
    // The normal desktop `--local-node` path should exercise a managed child node over RPC so
    // the client uses the same runtime path as an external node attachment.
    cfg!(test) || manage_local_node_requested()
}

fn manage_local_node_requested() -> bool {
    std::env::var(ATHO_QT_LOCAL_ENV).ok().as_deref() == Some("1")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExistingRpcEndpoint {
    None,
    SameNetworkRunning,
    SameNetworkStopped,
    SameNetworkIncomplete,
    SameNetworkLegacy,
    WrongNetwork(String),
    OccupiedByNonAtho,
}

fn start_local_node_if_needed(
    network: Network,
    rpc_address: &str,
) -> Result<LocalNodeStartup, String> {
    if !manage_local_node_requested() {
        return Ok(LocalNodeStartup::external_rpc());
    }

    self_heal_managed_local_node_before_start(network, rpc_address)?;
    let child = spawn_managed_local_node_child(network, rpc_address)?;
    let node = Arc::new(ManagedNodeState::new(
        network,
        rpc_address.to_string(),
        child,
    ));
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
    Ok(LocalNodeStartup::managed(node))
}

fn self_heal_managed_local_node_before_start(
    network: Network,
    rpc_address: &str,
) -> Result<(), String> {
    match inspect_existing_rpc_endpoint(network, rpc_address) {
        ExistingRpcEndpoint::None => {}
        ExistingRpcEndpoint::SameNetworkRunning => {
            // Managed local-node mode owns the local athod lifecycle. Reusing an already-running
            // node here risks attaching to an older binary or stale runtime root after rebuilds.
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!(
                    "managed local node requested with existing healthy same-network rpc endpoint network={} rpc={}; stopping before restart so managed local-node mode owns the process lifecycle",
                    network.id(),
                    rpc_address
                ),
            );
            stop_existing_local_node(rpc_address, network)?;
        }
        ExistingRpcEndpoint::SameNetworkStopped | ExistingRpcEndpoint::SameNetworkIncomplete => {
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!(
                    "managed local node requested with stale same-network rpc endpoint network={} rpc={}; stopping before restart",
                    network.id(),
                    rpc_address,
                ),
            );
            stop_existing_local_node(rpc_address, network)?;
        }
        ExistingRpcEndpoint::SameNetworkLegacy => {
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!(
                    "managed local node requested with legacy same-network rpc endpoint network={} rpc={}; stopping before restart",
                    network.id(),
                    rpc_address,
                ),
            );
            stop_existing_local_node(rpc_address, network)?;
        }
        ExistingRpcEndpoint::WrongNetwork(actual) => {
            return Err(format!(
                "rpc address {rpc_address} is already serving {actual}; expected {}. Use the matching network, choose a different RPC port, or stop the conflicting athod process.",
                network.id()
            ));
        }
        ExistingRpcEndpoint::OccupiedByNonAtho => {
            if !force_stop_owned_local_athod(rpc_address, network)? {
                return Err(format!(
                    "rpc address {rpc_address} is already occupied by a non-Atho service or an incompatible node; choose a different RPC port or stop the conflicting process."
                ));
            }
        }
    }

    if !rpc_bind_available(rpc_address) {
        return Err(format!(
            "rpc address {rpc_address} is still occupied after managed-node preflight for {}; choose a different RPC port or stop the conflicting process.",
            network.id()
        ));
    }

    if force_stop_owned_local_athod_on_default_p2p(network)? {
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "reclaimed stale managed local-node p2p listener before restart network={}",
                network.id()
            ),
        );
    }

    Ok(())
}

fn spawn_managed_local_node_child(network: Network, rpc_address: &str) -> Result<Child, String> {
    let mut command = managed_local_node_command(network, rpc_address);
    let (stdout, stderr) = local_node_stdio(network)?;
    if let Some(p2p_addr) = managed_local_node_p2p_bind_address(network) {
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "managed local node p2p bind override network={} addr={}",
                network.id(),
                p2p_addr
            ),
        );
        command.env("ATHO_P2P_ADDR", p2p_addr);
    }
    command
        .env("ATHO_MANAGED_PARENT_PID", managed_parent_pid_env_value())
        .env("ATHO_RPC_ADDR", rpc_address)
        .env("ATHO_NETWORK", network.cli_arg())
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);

    command.spawn().map_err(|err| {
        let startup_error = format!("failed to spawn local node: {err}");
        let _ = atho_node::dev::append_log("atho-qt", &startup_error);
        startup_error
    })
}

fn managed_local_node_command(network: Network, rpc_address: &str) -> Command {
    if let Some(binary) = node_binary_path() {
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "using bundled athod binary for managed local node network={}",
                network.id()
            ),
        );
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
            &format!(
                "athod binary not found; falling back to cargo run for network={}",
                network.id()
            ),
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
    }
}

fn managed_parent_pid_env_value() -> String {
    std::process::id().to_string()
}

fn inspect_existing_rpc_endpoint(network: Network, rpc_address: &str) -> ExistingRpcEndpoint {
    let client = RpcClient::new(rpc_address.to_string());
    if let Ok(RpcResponse::NodeStatus(status)) = client.call(&RpcRequest::GetNodeStatus) {
        if status.network != network {
            return ExistingRpcEndpoint::WrongNetwork(status.network.id().to_string());
        }
        return if status.running {
            ExistingRpcEndpoint::SameNetworkRunning
        } else {
            ExistingRpcEndpoint::SameNetworkStopped
        };
    }

    if let Ok(RpcResponse::Network(label)) = client.call(&RpcRequest::GetNetwork) {
        return if label == network.id() {
            ExistingRpcEndpoint::SameNetworkIncomplete
        } else {
            ExistingRpcEndpoint::WrongNetwork(label)
        };
    }

    if let Some(endpoint) = inspect_legacy_rpc_endpoint(network, rpc_address) {
        return endpoint;
    }

    if rpc_bind_available(rpc_address) {
        ExistingRpcEndpoint::None
    } else {
        ExistingRpcEndpoint::OccupiedByNonAtho
    }
}

fn inspect_legacy_rpc_endpoint(network: Network, rpc_address: &str) -> Option<ExistingRpcEndpoint> {
    if let Ok(RpcResponse::NodeStatus(status)) =
        legacy_rpc_call(rpc_address, &RpcRequest::GetNodeStatus)
    {
        return Some(if status.network == network {
            ExistingRpcEndpoint::SameNetworkLegacy
        } else {
            ExistingRpcEndpoint::WrongNetwork(status.network.id().to_string())
        });
    }

    if let Ok(RpcResponse::Network(label)) = legacy_rpc_call(rpc_address, &RpcRequest::GetNetwork) {
        return Some(if label == network.id() {
            ExistingRpcEndpoint::SameNetworkLegacy
        } else {
            ExistingRpcEndpoint::WrongNetwork(label)
        });
    }

    None
}

fn legacy_rpc_call(rpc_address: &str, request: &RpcRequest) -> Result<RpcResponse, String> {
    let mut stream = legacy_connect_stream(rpc_address).map_err(|err| err.to_string())?;
    let encoded = serde_json::to_string(request).map_err(|err| err.to_string())?;
    stream
        .write_all(encoded.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|err| err.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let bytes = reader
        .read_until(b'\n', &mut line)
        .map_err(|err| err.to_string())?;
    if bytes == 0 {
        return Err(String::from("legacy rpc returned an empty response"));
    }
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    serde_json::from_slice(&line).map_err(|err| err.to_string())
}

fn legacy_connect_stream(rpc_address: &str) -> Result<TcpStream, std::io::Error> {
    let mut last_error = None;
    for socket_addr in rpc_address.to_socket_addrs()? {
        match TcpStream::connect_timeout(&socket_addr, LEGACY_RPC_COMPAT_CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(LEGACY_RPC_COMPAT_IO_TIMEOUT))?;
                stream.set_write_timeout(Some(LEGACY_RPC_COMPAT_IO_TIMEOUT))?;
                return Ok(stream);
            }
            Err(err) => last_error = Some(err),
        }
    }

    match last_error {
        Some(err) => Err(err),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rpc address did not resolve to any socket address",
        )),
    }
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

#[cfg(test)]
fn probe_rpc(rpc_address: &str) -> bool {
    let client = RpcClient::new(rpc_address.to_string());
    matches!(
        client.call(&RpcRequest::GetNetwork),
        Ok(RpcResponse::Network(_))
    )
}

fn rpc_bind_available(rpc_address: &str) -> bool {
    TcpListener::bind(rpc_address).is_ok()
}

fn load_persisted_chain_tip_status(network: Network) -> Option<PersistedChainTipStatus> {
    let database = Database::open(network).ok()?;
    let snapshot = database.load_chainstate_snapshot().ok().flatten()?;
    let tip_timestamp = snapshot
        .tip_header
        .as_ref()
        .map(|header| header.timestamp)
        .or_else(|| {
            database
                .load_block_record_by_height(snapshot.height)
                .ok()
                .flatten()
                .map(|record| record.timestamp)
        })
        .unwrap_or_default();
    Some(PersistedChainTipStatus {
        height: snapshot.height,
        tip_hash: snapshot.tip_hash,
        tip_timestamp,
    })
}

fn stop_existing_local_node(rpc_address: &str, network: Network) -> Result<(), String> {
    let mut invocation = CommandInvocation::new("stop", Vec::new());
    invocation.confirmed = true;
    let stop_request = RpcRequest::ExecuteCommand(invocation);
    let client = RpcClient::new(rpc_address.to_string());

    let current_stop_result = client.call(&stop_request);
    let current_stop = match &current_stop_result {
        Ok(RpcResponse::Command(_)) => true,
        Ok(RpcResponse::Error(err)) => {
            return Err(format!(
                "existing local node at {rpc_address} refused managed restart for {}: {err}",
                network.id()
            ));
        }
        Ok(other) => {
            return Err(format!(
                "existing local node at {rpc_address} returned unexpected stop response for {}: {other:?}",
                network.id()
            ));
        }
        Err(err) => {
            let _ = atho_node::dev::append_log(
                "atho-qt",
                &format!(
                    "stop command transport error before managed restart network={} rpc={} error={err}",
                    network.id(),
                    rpc_address
                ),
            );
            false
        }
    };

    let legacy_stop = if current_stop {
        false
    } else {
        match legacy_rpc_call(rpc_address, &stop_request) {
            Ok(RpcResponse::Command(_)) => true,
            Ok(RpcResponse::Error(err)) => {
                return Err(format!(
                    "existing legacy local node at {rpc_address} refused managed restart for {}: {err}",
                    network.id()
                ));
            }
            Ok(other) => {
                return Err(format!(
                    "existing legacy local node at {rpc_address} returned unexpected stop response for {}: {other:?}",
                    network.id()
                ));
            }
            Err(err) => {
                let _ = atho_node::dev::append_log(
                    "atho-qt",
                    &format!(
                        "legacy stop command transport error before managed restart network={} rpc={} error={err}",
                        network.id(),
                        rpc_address
                    ),
                );
                false
            }
        }
    };

    if !current_stop && !legacy_stop {
        if force_stop_owned_local_athod(rpc_address, network)? {
            return Ok(());
        }
        return Err(format!(
            "rpc address {rpc_address} is occupied but did not respond to Atho stop commands for {}; stop the conflicting process or choose a different RPC port.",
            network.id()
        ));
    }

    if wait_for_rpc_bind_release(rpc_address, LOCAL_RPC_STOP_RETRY_ATTEMPTS) {
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!(
                "existing local node stopped before managed restart network={} rpc={}",
                network.id(),
                rpc_address
            ),
        );
        return Ok(());
    }

    if force_stop_owned_local_athod(rpc_address, network)? {
        return Ok(());
    }

    Err(format!(
        "existing local node at {rpc_address} did not stop for {}; stop the conflicting athod process or use external RPC mode instead of --local-node",
        network.id()
    ))
}

fn wait_for_rpc_bind_release(rpc_address: &str, attempts: usize) -> bool {
    wait_for_bind_release(rpc_address, attempts)
}

fn wait_for_bind_release(bind_address: &str, attempts: usize) -> bool {
    for _ in 0..attempts {
        if rpc_bind_available(bind_address) {
            return true;
        }
        thread::sleep(Duration::from_millis(LOCAL_RPC_READY_RETRY_DELAY_MS));
    }
    false
}

fn force_stop_owned_local_athod(rpc_address: &str, network: Network) -> Result<bool, String> {
    let Some(owner) = listening_process_on_rpc_address(rpc_address) else {
        return Ok(false);
    };
    if !command_matches_managed_local_athod(&owner.command, network, rpc_address) {
        return Ok(false);
    }

    let _ = atho_node::dev::append_log(
        "atho-qt",
        &format!(
            "graceful stop did not release rpc address {}; force-stopping managed athod pid={} command={}",
            rpc_address, owner.pid, owner.command
        ),
    );

    terminate_process(owner.pid, false)?;
    if wait_for_rpc_bind_release(rpc_address, LOCAL_RPC_FORCE_STOP_RETRY_ATTEMPTS) {
        return Ok(true);
    }

    terminate_process(owner.pid, true)?;
    if wait_for_rpc_bind_release(rpc_address, LOCAL_RPC_FORCE_STOP_RETRY_ATTEMPTS) {
        return Ok(true);
    }

    Err(format!(
        "existing local athod pid {} kept rpc address {} after termination attempts for {}",
        owner.pid,
        rpc_address,
        network.id()
    ))
}

fn force_stop_owned_local_athod_on_default_p2p(network: Network) -> Result<bool, String> {
    if std::env::var("ATHO_P2P_ADDR")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(false);
    }

    let bind_address = atho_node::runtime::default_p2p_bind_address(network);
    let Some(port) = bind_address
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next().map(|addr| addr.port()))
    else {
        return Ok(false);
    };
    let Some(owner) = listening_process_on_port(port) else {
        return Ok(false);
    };
    if !command_matches_managed_local_athod(&owner.command, network, "") {
        return Ok(false);
    }

    let _ = atho_node::dev::append_log(
        "atho-qt",
        &format!(
            "force-stopping stale managed athod p2p listener network={} p2p={} pid={} command={}",
            network.id(),
            bind_address,
            owner.pid,
            owner.command
        ),
    );

    terminate_process(owner.pid, false)?;
    if wait_for_bind_release(&bind_address, LOCAL_RPC_FORCE_STOP_RETRY_ATTEMPTS) {
        return Ok(true);
    }

    terminate_process(owner.pid, true)?;
    if wait_for_bind_release(&bind_address, LOCAL_RPC_FORCE_STOP_RETRY_ATTEMPTS) {
        return Ok(true);
    }

    Err(format!(
        "existing local athod pid {} kept p2p address {} after termination attempts for {}",
        owner.pid,
        bind_address,
        network.id()
    ))
}

fn command_matches_managed_local_athod(
    command: &str,
    network: Network,
    _rpc_address: &str,
) -> bool {
    let lower = command.to_ascii_lowercase();
    let explicit_network = lower.contains(&format!("--network {}", network.cli_arg()));
    let implicit_mainnet = network == Network::Mainnet && !lower.contains("--network");
    let same_network = explicit_network || implicit_mainnet;
    lower.contains("athod") && same_network
}

fn listening_process_on_rpc_address(rpc_address: &str) -> Option<PortOwnerProcess> {
    let port = rpc_address.to_socket_addrs().ok()?.next()?.port();
    listening_process_on_port(port)
}

#[cfg(unix)]
fn listening_process_on_port(port: u16) -> Option<PortOwnerProcess> {
    let pid_output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    if !pid_output.status.success() {
        return None;
    }
    let pid = String::from_utf8_lossy(&pid_output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u32>().ok())?;
    let command_output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !command_output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&command_output.stdout)
        .trim()
        .to_string();
    if command.is_empty() {
        return None;
    }
    Some(PortOwnerProcess { pid, command })
}

#[cfg(windows)]
fn listening_process_on_port(port: u16) -> Option<PortOwnerProcess> {
    let output = Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let suffix = format!(":{port}");
    let pid = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            if columns.len() < 5 {
                return None;
            }
            let local_addr = columns[1];
            let state = columns[3];
            if state.eq_ignore_ascii_case("LISTENING") && local_addr.ends_with(&suffix) {
                columns[4].parse::<u32>().ok()
            } else {
                None
            }
        })?;
    let command_output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\").CommandLine"),
        ])
        .output()
        .ok()?;
    if !command_output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&command_output.stdout)
        .trim()
        .to_string();
    if command.is_empty() {
        return None;
    }
    Some(PortOwnerProcess { pid, command })
}

#[cfg(unix)]
fn terminate_process(pid: u32, force: bool) -> Result<(), String> {
    let signal = if force { "-KILL" } else { "-TERM" };
    let status = Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .map_err(|err| err.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "kill {} {} failed with status {}",
            signal, pid, status
        ))
    }
}

#[cfg(windows)]
fn terminate_process(pid: u32, force: bool) -> Result<(), String> {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if force {
        command.arg("/F");
    }
    let status = command.status().map_err(|err| err.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "taskkill pid {} failed with status {}",
            pid, status
        ))
    }
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
        let candidate = PathBuf::from(path);
        return Some(if candidate.is_absolute() {
            candidate
        } else {
            workspace_manifest_path()
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(candidate)
        });
    }

    let exe = std::env::current_exe().ok()?;
    if prefer_workspace_cargo_runner(&exe) {
        return None;
    }
    node_binary_candidates_from_exe(&exe)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn prefer_workspace_cargo_runner(exe: &Path) -> bool {
    let manifest_path = workspace_manifest_path();
    let Some(workspace_root) = manifest_path.parent() else {
        return false;
    };
    let target_root = workspace_root.join("target");
    let Ok(relative_exe) = exe.strip_prefix(&target_root) else {
        return false;
    };
    !relative_exe
        .components()
        .any(|component| component.as_os_str() == "release")
}

fn node_binary_candidates_from_exe(exe: &Path) -> Vec<PathBuf> {
    let name = if cfg!(windows) { "athod.exe" } else { "athod" };
    let mut candidates = Vec::new();

    if let Some(parent) = exe.parent() {
        let candidate_dir = if parent.ends_with("deps") {
            parent.parent().unwrap_or(parent)
        } else {
            parent
        };
        push_unique_candidate(&mut candidates, candidate_dir.join(name));
    }

    if let Some(app_root) = macos_app_bundle_root(exe) {
        push_unique_candidate(
            &mut candidates,
            app_root.join("Contents").join("MacOS").join(name),
        );
        push_unique_candidate(
            &mut candidates,
            app_root.join("Contents").join("Resources").join(name),
        );
        if let Some(install_root) = app_root.parent() {
            push_unique_candidate(&mut candidates, install_root.join(name));
        }
    }

    if let Some(workspace_root) = workspace_manifest_path().parent() {
        push_unique_candidate(
            &mut candidates,
            workspace_root.join("target").join("release").join(name),
        );
        push_unique_candidate(
            &mut candidates,
            workspace_root.join("target").join("debug").join(name),
        );
    }

    candidates
}

fn push_unique_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn macos_app_bundle_root(exe: &Path) -> Option<&Path> {
    let app_root = exe.parent().and_then(Path::parent).and_then(Path::parent)?;
    if app_root.extension().and_then(|ext| ext.to_str()) == Some("app") {
        Some(app_root)
    } else {
        None
    }
}

fn managed_local_node_p2p_bind_address(network: Network) -> Option<String> {
    managed_local_node_p2p_bind_address_for_probe(
        network,
        &atho_node::runtime::default_p2p_bind_address(network),
    )
}

fn managed_local_node_p2p_bind_address_for_probe(
    _network: Network,
    probe_bind_address: &str,
) -> Option<String> {
    if std::env::var("ATHO_P2P_ADDR")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return None;
    }

    if TcpListener::bind(probe_bind_address).is_ok() {
        return None;
    }

    Some(String::from("127.0.0.1:0"))
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
    use crate::test_support::acquire_global_test_lock;
    use atho_node::miner::Miner;
    use atho_rpc::command::{CommandGroup, CommandPermission, CommandResponse};
    use atho_rpc::request::RpcRequest;
    use atho_rpc::response::{
        MempoolInfo, NetworkDiagnostics, NetworkPeerDiagnostics, NetworkPeerDirection, NodeStatus,
        RpcResponse,
    };
    use atho_rpc::transport::{read_message, write_message};
    use atho_storage::db::{ChainstateSnapshot, Database};
    use atho_storage::path::{database_dir, ATHO_DATA_DIR_ENV};
    use atho_storage::utxo::UtxoEntry;
    use lmdb::{Environment, Transaction as LmdbTransaction, WriteFlags};
    use std::ffi::OsString;
    use std::fs;
    use std::io::BufReader;
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
        _lock: crate::test_support::TestLockGuard,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let lock = acquire_global_test_lock();
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock: lock,
            }
        }

        fn set_value(key: &'static str, value: &str) -> Self {
            let lock = acquire_global_test_lock();
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock: lock,
            }
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
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let clone = stream.try_clone().expect("clone");
                let mut reader = BufReader::new(clone);
                let request: RpcRequest = read_message(&mut reader).expect("request");
                let response = match request {
                    RpcRequest::GetNodeStatus => RpcResponse::NodeStatus(NodeStatus {
                        network,
                        block_count,
                        tip_hash: [0; 48],
                        tip_timestamp: 1_777_416_445,
                        estimated_hashrate_hps: 0,
                        mempool_count,
                        mempool_total_fee_atoms: total_fee_atoms,
                        mempool_fingerprint: [0; 32],
                        running: true,
                        headers_synced: true,
                        sync_best_height: block_count,
                        network_diagnostics: NetworkDiagnostics {
                            peer_count: 1,
                            inbound_peer_count: 0,
                            outbound_peer_count: 1,
                            connecting_peer_count: 0,
                            bytes_sent: 2_048,
                            bytes_received: 4_096,
                            peers: vec![NetworkPeerDiagnostics {
                                remote_addr: String::from("74.208.219.116:56000"),
                                direction: NetworkPeerDirection::Outbound,
                                roles: vec![
                                    String::from("OUTBOUND_PEER"),
                                    String::from("FULL_RELAY_PEER"),
                                    String::from("BLOCK_RELAY_PEER"),
                                    String::from("SYNC_PEER"),
                                    String::from("TX_RELAY_PEER"),
                                    String::from("ADDR_RELAY_PEER"),
                                ],
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
                            connecting_peers: Vec::new(),
                            ..NetworkDiagnostics::default()
                        },
                    }),
                    RpcRequest::GetNetwork => RpcResponse::Network(network.id().to_string()),
                    RpcRequest::GetBlockCount => RpcResponse::BlockCount(block_count),
                    RpcRequest::GetMempoolInfo => RpcResponse::MempoolInfo(MempoolInfo {
                        transaction_count: mempool_count,
                        total_fee_atoms,
                    }),
                    other => RpcResponse::Error(RpcError::invalid_request(format!(
                        "unexpected request in mock rpc server: {other:?}"
                    ))),
                };
                write_message(&mut stream, &response).expect("response");
            }
        });
        (address, handle)
    }

    fn spawn_partial_rpc_server(network: Network) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock rpc");
        let address = listener.local_addr().expect("local addr").to_string();
        let handle = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept");
                let clone = stream.try_clone().expect("clone");
                let mut reader = BufReader::new(clone);
                let request: RpcRequest = read_message(&mut reader).expect("request");
                let response = match request {
                    RpcRequest::GetNodeStatus => {
                        RpcResponse::Error(atho_rpc::error::RpcError::method_not_found())
                    }
                    RpcRequest::GetNetwork => RpcResponse::Network(network.id().to_string()),
                    RpcRequest::GetBlockCount => RpcResponse::BlockCount(77),
                    RpcRequest::GetMempoolInfo => RpcResponse::MempoolInfo(MempoolInfo {
                        transaction_count: 5,
                        total_fee_atoms: 9,
                    }),
                    other => RpcResponse::Error(RpcError::invalid_request(format!(
                        "unexpected request in mock rpc server: {other:?}"
                    ))),
                };
                write_message(&mut stream, &response).expect("response");
            }
        });
        (address, handle)
    }

    fn spawn_stoppable_rpc_server(network: Network) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stoppable rpc");
        let address = listener.local_addr().expect("local addr").to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept getnetwork");
            let clone = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(clone);
            let request: RpcRequest = read_message(&mut reader).expect("request");
            match request {
                RpcRequest::GetNetwork => {
                    write_message(&mut stream, &RpcResponse::Network(network.id().to_string()))
                        .expect("getnetwork response")
                }
                other => panic!("unexpected initial request: {other:?}"),
            }

            let (mut stream, _) = listener.accept().expect("accept stop");
            let clone = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(clone);
            let request: RpcRequest = read_message(&mut reader).expect("request");
            match request {
                RpcRequest::ExecuteCommand(invocation) => {
                    assert_eq!(invocation.name, "stop");
                    assert!(invocation.confirmed);
                    write_message(
                        &mut stream,
                        &RpcResponse::Command(CommandResponse {
                            command: String::from("stop"),
                            group: CommandGroup::Control,
                            permission: CommandPermission::NodeAdmin,
                            dangerous: true,
                            network: network.id().to_string(),
                            data: serde_json::json!({
                                "stopping": true,
                                "network": network.id(),
                                "height": 0,
                            }),
                        }),
                    )
                    .expect("stop response");
                }
                other => panic!("unexpected stop request: {other:?}"),
            }
        });
        (address, handle)
    }

    fn spawn_stoppable_running_rpc_server(
        network: Network,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stoppable running rpc");
        let address = listener.local_addr().expect("local addr").to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept status");
            let clone = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(clone);
            let request: RpcRequest = read_message(&mut reader).expect("request");
            match request {
                RpcRequest::GetNodeStatus => write_message(
                    &mut stream,
                    &RpcResponse::NodeStatus(NodeStatus {
                        network,
                        block_count: 3,
                        tip_hash: [0x33; 48],
                        tip_timestamp: 1_777_416_777,
                        estimated_hashrate_hps: 0,
                        mempool_count: 0,
                        mempool_total_fee_atoms: 0,
                        mempool_fingerprint: [0; 32],
                        running: true,
                        headers_synced: true,
                        sync_best_height: 3,
                        network_diagnostics: NetworkDiagnostics::default(),
                    }),
                )
                .expect("status response"),
                other => panic!("unexpected status request: {other:?}"),
            }

            let (mut stream, _) = listener.accept().expect("accept stop");
            let clone = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(clone);
            let request: RpcRequest = read_message(&mut reader).expect("request");
            match request {
                RpcRequest::ExecuteCommand(invocation) => {
                    assert_eq!(invocation.name, "stop");
                    assert!(invocation.confirmed);
                    write_message(
                        &mut stream,
                        &RpcResponse::Command(CommandResponse {
                            command: String::from("stop"),
                            group: CommandGroup::Control,
                            permission: CommandPermission::NodeAdmin,
                            dangerous: true,
                            network: network.id().to_string(),
                            data: serde_json::json!({
                                "stopping": true,
                                "network": network.id(),
                                "height": 3,
                            }),
                        }),
                    )
                    .expect("stop response");
                }
                other => panic!("unexpected stop request: {other:?}"),
            }
        });
        (address, handle)
    }

    fn write_legacy_message(
        stream: &mut std::net::TcpStream,
        message: &RpcResponse,
    ) -> Result<(), String> {
        let encoded = serde_json::to_string(message).map_err(|err| err.to_string())?;
        stream
            .write_all(encoded.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .and_then(|_| stream.flush())
            .map_err(|err| err.to_string())
    }

    fn read_legacy_request(
        reader: &mut BufReader<std::net::TcpStream>,
    ) -> Result<RpcRequest, String> {
        let mut line = Vec::new();
        let bytes = reader
            .read_until(b'\n', &mut line)
            .map_err(|err| err.to_string())?;
        if bytes == 0 {
            return Err(String::from("legacy rpc empty request"));
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        serde_json::from_slice(&line).map_err(|err| err.to_string())
    }

    fn spawn_stoppable_legacy_rpc_server(
        network: Network,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind legacy stoppable rpc");
        let address = listener.local_addr().expect("local addr").to_string();
        let handle = std::thread::spawn(move || {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().expect("accept legacy rpc");
                let clone = stream.try_clone().expect("clone");
                let mut reader = BufReader::new(clone);
                let Ok(request) = read_legacy_request(&mut reader) else {
                    continue;
                };
                match request {
                    RpcRequest::GetNodeStatus => {
                        write_legacy_message(
                            &mut stream,
                            &RpcResponse::NodeStatus(NodeStatus {
                                network,
                                block_count: 0,
                                tip_hash: [0; 48],
                                tip_timestamp: 1_777_416_445,
                                estimated_hashrate_hps: 0,
                                mempool_count: 0,
                                mempool_total_fee_atoms: 0,
                                mempool_fingerprint: [0; 32],
                                running: true,
                                headers_synced: true,
                                sync_best_height: 0,
                                network_diagnostics: NetworkDiagnostics::default(),
                            }),
                        )
                        .expect("legacy status response");
                    }
                    RpcRequest::GetNetwork => {
                        write_legacy_message(
                            &mut stream,
                            &RpcResponse::Network(network.id().to_string()),
                        )
                        .expect("legacy getnetwork response");
                    }
                    RpcRequest::ExecuteCommand(invocation) => {
                        assert_eq!(invocation.name, "stop");
                        assert!(invocation.confirmed);
                        write_legacy_message(
                            &mut stream,
                            &RpcResponse::Command(CommandResponse {
                                command: String::from("stop"),
                                group: CommandGroup::Control,
                                permission: CommandPermission::NodeAdmin,
                                dangerous: true,
                                network: network.id().to_string(),
                                data: serde_json::json!({
                                    "stopping": true,
                                    "network": network.id(),
                                    "height": 0,
                                }),
                            }),
                        )
                        .expect("legacy stop response");
                        break;
                    }
                    other => panic!("unexpected legacy request: {other:?}"),
                }
            }
        });
        (address, handle)
    }

    fn spawn_retryable_node_status_rpc_server(
        network: Network,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock rpc");
        let address = listener.local_addr().expect("local addr").to_string();
        let handle = std::thread::spawn(move || {
            let mut fail_first_status = true;
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let clone = stream.try_clone().expect("clone");
                let mut reader = BufReader::new(clone);
                let request: RpcRequest = read_message(&mut reader).expect("request");
                let response = match request {
                    RpcRequest::GetNodeStatus if fail_first_status => {
                        fail_first_status = false;
                        RpcResponse::Error(atho_rpc::error::RpcError::method_not_found())
                    }
                    RpcRequest::GetNodeStatus => RpcResponse::NodeStatus(NodeStatus {
                        network,
                        block_count: 2,
                        tip_hash: [0x55; 48],
                        tip_timestamp: 1_777_416_555,
                        estimated_hashrate_hps: 0,
                        mempool_count: 0,
                        mempool_total_fee_atoms: 0,
                        mempool_fingerprint: [0x77; 32],
                        running: true,
                        headers_synced: true,
                        sync_best_height: 2,
                        network_diagnostics: NetworkDiagnostics {
                            peer_count: 1,
                            inbound_peer_count: 0,
                            outbound_peer_count: 1,
                            connecting_peer_count: 0,
                            bytes_sent: 512,
                            bytes_received: 1_024,
                            peers: vec![NetworkPeerDiagnostics {
                                remote_addr: String::from("74.208.219.116:56000"),
                                direction: NetworkPeerDirection::Outbound,
                                roles: vec![
                                    String::from("OUTBOUND_PEER"),
                                    String::from("FULL_RELAY_PEER"),
                                    String::from("BLOCK_RELAY_PEER"),
                                    String::from("SYNC_PEER"),
                                    String::from("TX_RELAY_PEER"),
                                    String::from("ADDR_RELAY_PEER"),
                                ],
                                handshake_ready: true,
                                best_height: Some(2),
                                protocol_version: Some(1),
                                services: Some(9),
                                user_agent: Some(String::from("/Atho:0.1.0/")),
                                ruleset_version: Some(1),
                                bytes_sent: 512,
                                bytes_received: 1_024,
                                last_send_unix: Some(1_777_416_555),
                                last_receive_unix: Some(1_777_416_555),
                                quality_score: Some(100),
                                consecutive_failures: Some(0),
                            }],
                            connecting_peers: Vec::new(),
                            ..NetworkDiagnostics::default()
                        },
                    }),
                    RpcRequest::GetNetwork => RpcResponse::Network(network.id().to_string()),
                    RpcRequest::GetBlockCount => RpcResponse::BlockCount(2),
                    RpcRequest::GetMempoolInfo => RpcResponse::MempoolInfo(MempoolInfo {
                        transaction_count: 0,
                        total_fee_atoms: 0,
                    }),
                    other => RpcResponse::Error(RpcError::invalid_request(format!(
                        "unexpected request in retryable mock rpc server: {other:?}"
                    ))),
                };
                write_message(&mut stream, &response).expect("response");
            }
        });
        (address, handle)
    }

    fn spawn_single_node_status_rpc_server(
        network: Network,
        running: bool,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock rpc");
        let address = listener.local_addr().expect("local addr").to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept status");
            let clone = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(clone);
            let request: RpcRequest = read_message(&mut reader).expect("request");
            let response = match request {
                RpcRequest::GetNodeStatus => RpcResponse::NodeStatus(NodeStatus {
                    network,
                    block_count: 3,
                    tip_hash: [0x33; 48],
                    tip_timestamp: 1_777_416_777,
                    estimated_hashrate_hps: 0,
                    mempool_count: 0,
                    mempool_total_fee_atoms: 0,
                    mempool_fingerprint: [0; 32],
                    running,
                    headers_synced: true,
                    sync_best_height: 3,
                    network_diagnostics: NetworkDiagnostics::default(),
                }),
                other => RpcResponse::Error(RpcError::invalid_request(format!(
                    "unexpected request in single status server: {other:?}"
                ))),
            };
            write_message(&mut stream, &response).expect("response");
        });
        (address, handle)
    }

    fn wait_for_status(
        conn: &ReadOnlyNodeConnection,
        predicate: impl Fn(&ConnectionStatus) -> bool,
    ) -> ConnectionStatus {
        // Local-node startup can be slow in release builds and on busy CI runners.
        for _attempt in 0..600 {
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
        let _lock = acquire_global_test_lock();
        let root = temp_data_dir("read-only-connection");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

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

        let db = Database::open(Network::Testnet).expect("database");
        let db_path = database_dir(Network::Testnet);
        drop(db);

        let mut builder = Environment::new();
        builder
            .set_max_readers(128)
            .set_max_dbs(10)
            .set_map_size(1 << 30);
        let env = builder.open(&db_path).expect("open env");
        let meta = env.open_db(Some("meta")).expect("meta db");
        let mut txn = env.begin_rw_txn().expect("rw txn");
        let snapshot_bytes = bincode::serialize(&ChainstateSnapshot {
            height: 1,
            tip_hash: [9; 48],
            tip_header: None,
        })
        .expect("serialize snapshot");
        txn.put(meta, b"chainstate", &snapshot_bytes, WriteFlags::empty())
            .expect("put snapshot");
        txn.commit().expect("commit fixture");

        let conn = ReadOnlyNodeConnection::new(Network::Testnet);
        let status = conn.status();
        assert!(status.connected);
        assert!(status.running);
        assert_eq!(status.block_count, 0);
        assert_eq!(status.startup_error, None);
    }

    #[test]
    fn rpc_status_path_uses_available_rpc_server_without_local_node_mode() {
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "0");
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
    fn rpc_status_without_node_status_does_not_claim_readiness() {
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "0");
        let (rpc_address, handle) = spawn_partial_rpc_server(Network::Mainnet);

        let conn = ReadOnlyNodeConnection::with_rpc_address(Network::Mainnet, rpc_address);
        let status = conn.status();

        assert!(!status.connected);
        assert!(!status.running);
        assert!(!status.headers_synced);
        assert_eq!(status.block_count, 77);
        assert_eq!(status.mempool_count, 5);
        assert_eq!(status.mempool_total_fee_atoms, 9);

        handle.join().expect("mock rpc server");
    }

    #[test]
    fn managed_local_node_status_degrades_without_zeroing_last_known_sync_target() {
        let degraded = degrade_rpc_status(DegradedRpcStatusInput {
            network: Network::Mainnet,
            rpc_address: "127.0.0.1:9010",
            block_count_reply: Some(0),
            mempool_reply: Some((0, 0)),
            network_ok: true,
            rpc_reachable: true,
            managed_local: true,
            last_known: Some(&ConnectionStatus {
                network: Network::Mainnet,
                rpc_address: String::from("127.0.0.1:9010"),
                block_count: 42,
                tip_hash: [0; 48],
                tip_timestamp: 1_777_416_445,
                estimated_hashrate_hps: 0,
                mempool_count: 3,
                mempool_total_fee_atoms: 55,
                mempool_fingerprint: [0; 32],
                peer_count: 1,
                inbound_peer_count: 0,
                outbound_peer_count: 1,
                connecting_peer_count: 0,
                bytes_sent: 2_048,
                bytes_received: 4_096,
                peers: Vec::new(),
                connecting_peers: Vec::new(),
                running: true,
                headers_synced: true,
                sync_best_height: 128,
                connected: true,
                startup_error: None,
            }),
        });
        assert!(degraded.connected);
        assert!(degraded.running);
        assert_eq!(degraded.block_count, 0);
        assert_eq!(degraded.sync_best_height, 128);
        assert_eq!(degraded.tip_timestamp, 1_777_416_445);
    }

    #[test]
    fn managed_local_degraded_status_uses_persisted_chainstate_height() {
        let root = temp_data_dir("managed-persisted-height");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let db = Database::open(Network::Mainnet).expect("database");
        let db_path = database_dir(Network::Mainnet);
        drop(db);

        let mut builder = Environment::new();
        builder
            .set_max_readers(128)
            .set_max_dbs(10)
            .set_map_size(1 << 30);
        let env = builder.open(&db_path).expect("open env");
        let meta = env.open_db(Some("meta")).expect("meta db");
        let mut txn = env.begin_rw_txn().expect("rw txn");
        let snapshot_bytes = bincode::serialize(&ChainstateSnapshot {
            height: 7,
            tip_hash: [9; 48],
            tip_header: None,
        })
        .expect("serialize snapshot");
        txn.put(meta, b"chainstate", &snapshot_bytes, WriteFlags::empty())
            .expect("put snapshot");
        txn.commit().expect("commit fixture");

        let degraded = degrade_rpc_status(DegradedRpcStatusInput {
            network: Network::Mainnet,
            rpc_address: "127.0.0.1:9010",
            block_count_reply: None,
            mempool_reply: None,
            network_ok: false,
            rpc_reachable: false,
            managed_local: true,
            last_known: Some(&ConnectionStatus {
                network: Network::Mainnet,
                rpc_address: String::from("127.0.0.1:9010"),
                block_count: 42,
                tip_hash: [4; 48],
                tip_timestamp: 1_777_416_445,
                estimated_hashrate_hps: 0,
                mempool_count: 3,
                mempool_total_fee_atoms: 55,
                mempool_fingerprint: [8; 32],
                peer_count: 1,
                inbound_peer_count: 0,
                outbound_peer_count: 1,
                connecting_peer_count: 0,
                bytes_sent: 2_048,
                bytes_received: 4_096,
                peers: Vec::new(),
                connecting_peers: Vec::new(),
                running: true,
                headers_synced: true,
                sync_best_height: 128,
                connected: true,
                startup_error: None,
            }),
        });

        assert_eq!(degraded.block_count, 7);
        assert_eq!(degraded.tip_hash, [9; 48]);
        assert_eq!(degraded.tip_timestamp, 0);
        assert_eq!(degraded.sync_best_height, 128);
        assert!(!degraded.running);
        assert!(!degraded.connected);
        assert!(!degraded.headers_synced);
        assert_eq!(degraded.mempool_count, 0);
        assert_eq!(degraded.peer_count, 0);
    }

    #[test]
    fn managed_local_stop_command_reclaims_existing_rpc_endpoint() {
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let (rpc_address, handle) = spawn_stoppable_rpc_server(Network::Mainnet);

        assert!(probe_rpc(&rpc_address));
        stop_existing_local_node(&rpc_address, Network::Mainnet).expect("stop existing node");
        assert!(!probe_rpc(&rpc_address));

        handle.join().expect("stoppable mock rpc server");
    }

    #[test]
    fn managed_local_stop_command_reclaims_legacy_rpc_endpoint() {
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let (rpc_address, handle) = spawn_stoppable_legacy_rpc_server(Network::Mainnet);

        assert!(matches!(
            inspect_existing_rpc_endpoint(Network::Mainnet, &rpc_address),
            ExistingRpcEndpoint::SameNetworkLegacy
        ));
        stop_existing_local_node(&rpc_address, Network::Mainnet).expect("stop legacy node");
        assert!(rpc_bind_available(&rpc_address));

        handle.join().expect("stoppable legacy mock rpc server");
    }

    #[test]
    fn managed_local_athod_matcher_accepts_same_network_commands() {
        assert!(command_matches_managed_local_athod(
            "target/debug/athod --network testnet --rpc-addr 127.0.0.1:9110",
            Network::Testnet,
            "127.0.0.1:9110"
        ));
        assert!(command_matches_managed_local_athod(
            "cargo run --manifest-path /repo/Cargo.toml -p atho-node --bin athod -- --network testnet --rpc-addr 127.0.0.1:9110",
            Network::Testnet,
            "127.0.0.1:9110"
        ));
        assert!(command_matches_managed_local_athod(
            "/opt/atho/bin/athod --rpc-addr 127.0.0.1:9010",
            Network::Mainnet,
            "127.0.0.1:9010"
        ));
    }

    #[test]
    fn managed_local_athod_matcher_rejects_wrong_process_or_network() {
        assert!(!command_matches_managed_local_athod(
            "python some_server.py --network testnet",
            Network::Testnet,
            "127.0.0.1:9110"
        ));
        assert!(!command_matches_managed_local_athod(
            "target/debug/athod --network regnet --rpc-addr 127.0.0.1:9110",
            Network::Testnet,
            "127.0.0.1:9110"
        ));
    }

    #[test]
    fn managed_node_exit_retry_policy_covers_cargo_status_101() {
        assert!(managed_node_exit_status_is_retryable(false, Some(101)));
        assert!(managed_node_exit_status_is_retryable(false, Some(1)));
        assert!(managed_node_exit_status_is_retryable(false, None));
        assert!(!managed_node_exit_status_is_retryable(true, Some(0)));
    }

    #[test]
    fn managed_parent_pid_env_value_tracks_current_client() {
        assert_eq!(
            managed_parent_pid_env_value(),
            std::process::id().to_string()
        );
    }

    #[test]
    fn managed_local_start_reclaims_running_same_network_endpoint_before_restart() {
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let _node_bin = EnvVarGuard::set_value(
            "ATHO_NODE_BIN",
            &std::env::var("CARGO").unwrap_or_else(|_| String::from("cargo")),
        );
        let (rpc_address, handle) = spawn_stoppable_running_rpc_server(Network::Testnet);

        let startup =
            start_local_node_if_needed(Network::Testnet, &rpc_address).expect("restart managed");

        assert!(startup.local_node);
        assert!(startup.node.is_some());
        handle.join().expect("running stoppable mock rpc server");
    }

    #[test]
    fn managed_local_start_rejects_wrong_network_endpoint_without_stopping_it() {
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let (rpc_address, handle) = spawn_single_node_status_rpc_server(Network::Mainnet, true);

        let err = start_local_node_if_needed(Network::Testnet, &rpc_address)
            .expect_err("wrong network should be rejected");

        assert!(err.contains("already serving atho-mainnet"));
        assert!(err.contains("expected atho-testnet"));
        handle.join().expect("status mock rpc server");
    }

    #[test]
    fn managed_local_node_status_retries_transient_node_status_failures() {
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let (rpc_address, handle) = spawn_retryable_node_status_rpc_server(Network::Mainnet);
        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 2")
            .spawn()
            .expect("spawn managed child");
        let managed = Arc::new(ManagedNodeState::new(
            Network::Mainnet,
            rpc_address.clone(),
            child,
        ));
        let client = RpcClient::new(rpc_address.clone());

        let status = collect_rpc_status(
            Network::Mainnet,
            &rpc_address,
            &client,
            Some(&managed),
            true,
            None,
        );

        assert!(status.connected);
        assert!(status.running);
        assert!(status.headers_synced);
        assert_eq!(status.block_count, 2);
        assert_eq!(status.sync_best_height, 2);
        assert_eq!(status.tip_timestamp, 1_777_416_555);

        handle.join().expect("retryable mock rpc server");
    }

    #[test]
    fn local_flag_prefers_embedded_backend_by_default() {
        let root = temp_data_dir("embedded-local");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "0");

        let conn = ReadOnlyNodeConnection::with_rpc_address(Network::Regnet, free_rpc_address());
        let status = wait_for_status(&conn, |status| status.connected && status.running);
        assert!(status.connected);
        assert!(status.running);
        assert!(conn.with_local_system_for_test(|_| true).is_some());
    }

    #[test]
    fn managed_local_node_rpc_status_tracks_real_chain_tip() {
        let root = temp_data_dir("real-rpc");
        fs::create_dir_all(&root).expect("root");
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "1");
        let _local = EnvVarGuard::set_value(ATHO_QT_LOCAL_ENV, "1");
        let rpc_address = free_rpc_address();
        let api_address = free_rpc_address();
        let (_, api_port) = api_address
            .rsplit_once(':')
            .expect("free API address includes port");
        let _api_port = EnvVarGuard::set_value("ATHO_API_PORT", api_port);

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
        let _force_rpc = EnvVarGuard::set_value(ATHO_QT_FORCE_RPC_ENV, "0");

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
                        20 * atho_core::constants::atoms_per_atho_for_network(Network::Regnet),
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

    #[test]
    fn macos_bundle_node_binary_candidates_include_bundle_and_install_root() {
        let app_executable = Path::new("/Applications/Atho/Atho.app/Contents/MacOS/Atho");
        let candidates = node_binary_candidates_from_exe(app_executable);
        assert!(candidates.contains(&PathBuf::from(
            "/Applications/Atho/Atho.app/Contents/MacOS/athod"
        )));
        assert!(candidates.contains(&PathBuf::from("/Applications/Atho/athod")));
    }

    #[test]
    fn workspace_target_executables_prefer_cargo_runner() {
        let exe = workspace_manifest_path()
            .parent()
            .expect("workspace root")
            .join("target")
            .join("debug")
            .join("atho-qt");
        assert!(prefer_workspace_cargo_runner(&exe));
    }

    #[test]
    fn workspace_release_executables_use_sibling_node_binary() {
        let exe = workspace_manifest_path()
            .parent()
            .expect("workspace root")
            .join("target")
            .join("release")
            .join("atho-qt");
        assert!(!prefer_workspace_cargo_runner(&exe));
    }

    #[test]
    fn packaged_executables_do_not_force_cargo_runner() {
        let exe = Path::new("/Applications/Atho/Atho.app/Contents/MacOS/Atho");
        assert!(!prefer_workspace_cargo_runner(exe));
    }

    #[test]
    fn managed_local_node_p2p_bind_address_falls_back_when_default_port_is_busy() {
        let _lock = acquire_global_test_lock();
        let previous_addr = std::env::var_os("ATHO_P2P_ADDR");
        std::env::remove_var("ATHO_P2P_ADDR");
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve test p2p port");
        let probe_addr = listener.local_addr().expect("probe addr").to_string();

        let addr = managed_local_node_p2p_bind_address_for_probe(Network::Mainnet, &probe_addr);
        assert_eq!(addr.as_deref(), Some("127.0.0.1:0"));

        if let Some(previous) = previous_addr {
            std::env::set_var("ATHO_P2P_ADDR", previous);
        } else {
            std::env::remove_var("ATHO_P2P_ADDR");
        }
        drop(listener);
    }
}
