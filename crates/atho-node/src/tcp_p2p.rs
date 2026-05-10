//! TCP-backed P2P runtime integration for the Atho node.
use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::service::NodeService;
use crate::sync::SyncNotice;
use atho_core::network::Network;
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, LAUNCH_INVALID_PEER_ADDRESS, LAUNCH_P2P_BIND_FAILED,
    P2P_IO_FAILURE,
};
use atho_p2p::codec::{CodecError, WireCodec};
use atho_p2p::config::network_params;
use atho_p2p::connection::ConnectionEvent;
use atho_p2p::protocol::{Hash48, InventoryKind, InventoryVector, MessagePayload, NetworkMessage};
use atho_storage::db::PeerHealthRecord;
use get_if_addrs::get_if_addrs;
use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const FRAME_HEADER_BYTES: usize = 24;
const OUTBOUND_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const OUTBOUND_MAX_RETRY_INTERVAL: Duration = Duration::from_secs(32);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const PEER_DISCOVERY_INTERVAL: Duration = Duration::from_secs(5);
const PEER_IO_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FRAME_READ_STALL_TIMEOUT: Duration = Duration::from_secs(120);
const PEER_QUALITY_MAX_SCORE: u32 = 100;
const PEER_QUALITY_FAILURE_PENALTY: u32 = 15;

#[derive(Debug, Error)]
pub enum TcpP2pError {
    #[error("bind failed: {0}")]
    Bind(String),
    #[error("invalid peer address: {0}")]
    InvalidPeerAddress(String),
    #[error(transparent)]
    Node(#[from] NodeError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Codec(#[from] CodecError),
}

impl AthoErrorMeta for TcpP2pError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Bind(_) => &LAUNCH_P2P_BIND_FAILED,
            Self::InvalidPeerAddress(_) => &LAUNCH_INVALID_PEER_ADDRESS,
            Self::Node(error) => error.descriptor(),
            Self::Io(_) => &P2P_IO_FAILURE,
            Self::Codec(error) => error.descriptor(),
        }
    }

    fn source_module(&self) -> &'static str {
        match self {
            Self::Node(error) => error.source_module(),
            Self::Codec(error) => error.source_module(),
            _ => "atho-node::tcp_p2p",
        }
    }

    fn safe_details(&self) -> Option<String> {
        match self {
            Self::Bind(value) | Self::InvalidPeerAddress(value) => Some(value.clone()),
            Self::Io(error) => Some(error.to_string()),
            Self::Node(error) => error.safe_details(),
            Self::Codec(error) => error.safe_details(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpP2pSnapshot {
    pub network: Network,
    pub bind_addr: SocketAddr,
    pub height: u64,
    pub tip_hash: [u8; 48],
    pub peer_count: usize,
    pub sync_best_height: u64,
    pub headers_synced: bool,
}

#[derive(Debug)]
pub struct TcpP2pRuntime {
    network: Network,
    bind_addr: SocketAddr,
    state: Arc<Mutex<NodeService>>,
    stop_requested: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(dead_code))]
    active_inbound_threads: Arc<AtomicUsize>,
    outbound_targets: Arc<Mutex<BTreeSet<String>>>,
    listener_thread: Option<JoinHandle<()>>,
    peer_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
    maintenance_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl TcpP2pRuntime {
    pub fn bind_shared(
        network: Network,
        state: Arc<Mutex<NodeService>>,
        bind_addr: impl AsRef<str>,
    ) -> Result<Self, TcpP2pError> {
        {
            let mut service = state.lock().expect("p2p runtime state poisoned");
            if !service.is_running() {
                service.start();
            } else {
                service.p2p_prime();
            }
        }
        let listener = TcpListener::bind(bind_addr.as_ref())
            .map_err(|err| TcpP2pError::Bind(err.to_string()))?;
        listener.set_nonblocking(true).map_err(TcpP2pError::Io)?;
        let local_addr = listener.local_addr().map_err(TcpP2pError::Io)?;
        let stop_requested = Arc::new(AtomicBool::new(false));
        let active_inbound_threads = Arc::new(AtomicUsize::new(0));
        let outbound_targets = Arc::new(Mutex::new(BTreeSet::new()));
        let peer_threads = Arc::new(Mutex::new(Vec::new()));
        let maintenance_threads = Arc::new(Mutex::new(Vec::new()));
        let listener_state = Arc::clone(&state);
        let listener_stop = Arc::clone(&stop_requested);
        let listener_active_inbound = Arc::clone(&active_inbound_threads);
        let listener_peers = Arc::clone(&peer_threads);
        let inbound_limit = network_params(network).limits.max_inbound_peers;
        let listener_thread = thread::spawn(move || loop {
            if listener_stop.load(Ordering::Acquire) {
                break;
            }
            reap_finished_threads(&listener_peers);
            match listener.accept() {
                Ok((stream, remote_addr)) => {
                    // Bound raw inbound socket handling before protocol handshake. Public internet
                    // noise can otherwise create an unbounded number of worker threads before the
                    // higher-level connection manager has a chance to reject the peer.
                    if !try_acquire_inbound_slot(&listener_active_inbound, inbound_limit) {
                        drop(stream);
                        continue;
                    }
                    let thread = spawn_peer_thread(
                        ConnectionRole::Inbound,
                        stream,
                        remote_addr,
                        remote_addr.to_string(),
                        Arc::clone(&listener_state),
                        Arc::clone(&listener_stop),
                        Some(Arc::clone(&listener_active_inbound)),
                    );
                    listener_peers
                        .lock()
                        .expect("peer thread list poisoned")
                        .push(thread);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(err) => {
                    let _ = dev::append_log("p2p", &format!("listener accept error: {err}"));
                    thread::sleep(Duration::from_millis(50));
                }
            }
        });

        let runtime = Self {
            network,
            bind_addr: local_addr,
            state,
            stop_requested,
            active_inbound_threads,
            outbound_targets,
            listener_thread: Some(listener_thread),
            peer_threads,
            maintenance_threads,
        };
        let discovery_thread = spawn_peer_discovery(
            runtime.network,
            runtime.bind_addr,
            Arc::clone(&runtime.state),
            Arc::clone(&runtime.stop_requested),
            Arc::clone(&runtime.peer_threads),
            Arc::clone(&runtime.outbound_targets),
        );
        runtime
            .maintenance_threads
            .lock()
            .expect("maintenance thread list poisoned")
            .push(discovery_thread);
        Ok(runtime)
    }

    pub fn bind_service(
        network: Network,
        service: NodeService,
        bind_addr: impl AsRef<str>,
    ) -> Result<Self, TcpP2pError> {
        Self::bind_shared(network, Arc::new(Mutex::new(service)), bind_addr)
    }

    pub fn new_in_memory(
        network: Network,
        bind_addr: impl AsRef<str>,
    ) -> Result<Self, TcpP2pError> {
        Self::bind_service(
            network,
            NodeService::new_ephemeral(NodeConfig::new(network)),
            bind_addr,
        )
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn connect_outbound(&self, remote_addr: impl AsRef<str>) -> Result<(), TcpP2pError> {
        connect_outbound_target(
            remote_addr.as_ref(),
            Arc::clone(&self.state),
            Arc::clone(&self.stop_requested),
            Arc::clone(&self.peer_threads),
        )
    }

    pub fn maintain_outbound(&self, remote_addr: impl Into<String>) {
        let remote_addr = remote_addr.into();
        if is_self_outbound_target(self.bind_addr, &remote_addr) {
            let _ = dev::append_log(
                "p2p",
                &format!("ignoring self outbound bootstrap target peer={remote_addr}"),
            );
            return;
        }
        let Some(target_key) = track_outbound_target(&self.outbound_targets, &remote_addr) else {
            return;
        };
        let thread = spawn_outbound_maintainer(
            remote_addr,
            target_key,
            Arc::clone(&self.state),
            Arc::clone(&self.stop_requested),
            Arc::clone(&self.peer_threads),
            Arc::clone(&self.outbound_targets),
        );
        self.maintenance_threads
            .lock()
            .expect("maintenance thread list poisoned")
            .push(thread);
    }

    pub fn snapshot(&self) -> TcpP2pSnapshot {
        let state = self.state.lock().expect("p2p runtime state poisoned");
        TcpP2pSnapshot {
            network: self.network,
            bind_addr: self.bind_addr,
            height: state.height(),
            tip_hash: state.tip_hash(),
            peer_count: state.p2p_peer_count(),
            sync_best_height: state.p2p_sync_best_height(),
            headers_synced: state.p2p_headers_synced(),
        }
    }

    #[cfg(test)]
    pub fn mine_local_block(&self) -> Result<[u8; 48], NodeError> {
        let mut state = self.state.lock().expect("p2p runtime state poisoned");
        state.p2p_mine_local_block()
    }

    #[cfg(test)]
    fn active_inbound_threads(&self) -> usize {
        self.active_inbound_threads.load(Ordering::Acquire)
    }
}

impl Drop for TcpP2pRuntime {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.bind_addr);
        if let Some(listener) = self.listener_thread.take() {
            let _ = listener.join();
        }
        let mut maintenance_threads = self
            .maintenance_threads
            .lock()
            .expect("maintenance thread list poisoned");
        while let Some(thread) = maintenance_threads.pop() {
            let _ = thread.join();
        }
        drop(maintenance_threads);
        let mut peer_threads = self.peer_threads.lock().expect("peer thread list poisoned");
        while let Some(thread) = peer_threads.pop() {
            let _ = thread.join();
        }
    }
}

fn spawn_outbound_session(
    peer_id: String,
    remote_addr: SocketAddr,
    state: Arc<Mutex<NodeService>>,
    stop_requested: Arc<AtomicBool>,
    peer_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
) -> Result<(), TcpP2pError> {
    let stream = TcpStream::connect(remote_addr)?;
    let thread = spawn_peer_thread(
        ConnectionRole::Outbound,
        stream,
        remote_addr,
        peer_id,
        state,
        stop_requested,
        None,
    );
    peer_threads
        .lock()
        .expect("peer thread list poisoned")
        .push(thread);
    Ok(())
}

fn spawn_outbound_maintainer(
    remote_addr: String,
    target_key: String,
    state: Arc<Mutex<NodeService>>,
    stop_requested: Arc<AtomicBool>,
    peer_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
    outbound_targets: Arc<Mutex<BTreeSet<String>>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let _target_guard = OutboundTargetGuard::new(Arc::clone(&outbound_targets), target_key);
        let network = {
            let state = state.lock().expect("p2p runtime state poisoned");
            state.network()
        };
        let mut last_failure = None::<String>;
        while !stop_requested.load(Ordering::Acquire) {
            let mut health = load_peer_health_snapshot(&state, network, &remote_addr);
            let now_unix = unix_timestamp();
            if health.backoff_until_unix > now_unix {
                let remaining = health.backoff_until_unix.saturating_sub(now_unix).max(1);
                sleep_with_stop(&stop_requested, Duration::from_secs(remaining));
                continue;
            }

            // Keep configured outbound peers sticky. This closes the common startup-order race
            // where one node launches before another and would otherwise never retry.
            let already_connected = {
                let state = state.lock().expect("p2p runtime state poisoned");
                state.p2p_has_peer(&remote_addr)
            };
            if already_connected {
                last_failure = None;
                sleep_with_stop(&stop_requested, OUTBOUND_RETRY_INTERVAL);
                continue;
            }

            match connect_outbound_target(
                &remote_addr,
                Arc::clone(&state),
                Arc::clone(&stop_requested),
                Arc::clone(&peer_threads),
            ) {
                Ok(()) => {
                    sleep_with_stop(&stop_requested, Duration::from_millis(250));
                }
                Err(err) => {
                    let failure = err.to_string();
                    health.consecutive_failures = health.consecutive_failures.saturating_add(1);
                    let retry_delay = next_outbound_retry_delay(health.consecutive_failures);
                    health.backoff_until_unix =
                        now_unix.saturating_add(retry_delay.as_secs().max(1));
                    health.quality_score = health
                        .quality_score
                        .saturating_sub(PEER_QUALITY_FAILURE_PENALTY);
                    health.last_failure_unix = Some(now_unix);
                    persist_peer_health(&state, &health);
                    if last_failure.as_deref() != Some(failure.as_str()) {
                        let _ = dev::append_log(
                            "p2p",
                            &format!(
                                "outbound connect retry failed peer={remote_addr} error={failure} retry_in_secs={} failures={} quality={}",
                                retry_delay.as_secs().max(1),
                                health.consecutive_failures,
                                health.quality_score
                            ),
                        );
                        last_failure = Some(failure);
                    }
                    sleep_with_stop(&stop_requested, retry_delay);
                }
            }
        }
    })
}

fn connect_outbound_target(
    remote_addr: &str,
    state: Arc<Mutex<NodeService>>,
    stop_requested: Arc<AtomicBool>,
    peer_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
) -> Result<(), TcpP2pError> {
    let peer_id = remote_addr.trim();
    let candidates = resolve_outbound_target(peer_id)?;
    let mut last_error = None;
    for socket_addr in candidates {
        match spawn_outbound_session(
            peer_id.to_string(),
            socket_addr,
            Arc::clone(&state),
            Arc::clone(&stop_requested),
            Arc::clone(&peer_threads),
        ) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or_else(|| TcpP2pError::InvalidPeerAddress(peer_id.to_string())))
}

fn resolve_outbound_target(remote_addr: &str) -> Result<Vec<SocketAddr>, TcpP2pError> {
    let remote_addr = remote_addr.trim();
    if remote_addr.is_empty() {
        return Err(TcpP2pError::InvalidPeerAddress(String::new()));
    }
    let unique = remote_addr
        .to_socket_addrs()
        .map_err(|_| TcpP2pError::InvalidPeerAddress(remote_addr.to_string()))?
        .collect::<BTreeSet<_>>();
    if unique.is_empty() {
        return Err(TcpP2pError::InvalidPeerAddress(remote_addr.to_string()));
    }
    Ok(unique.into_iter().collect())
}

fn spawn_peer_discovery(
    network: Network,
    bind_addr: SocketAddr,
    state: Arc<Mutex<NodeService>>,
    stop_requested: Arc<AtomicBool>,
    peer_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
    outbound_targets: Arc<Mutex<BTreeSet<String>>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let candidate_limit = network_params(network)
            .limits
            .max_outbound_peers
            .saturating_mul(4)
            .max(8);
        while !stop_requested.load(Ordering::Acquire) {
            let candidates = {
                let mut state = state.lock().expect("p2p runtime state poisoned");
                state.p2p_bootstrap_peers(candidate_limit)
            };
            for remote_addr in candidates {
                if is_self_outbound_target(bind_addr, &remote_addr) {
                    continue;
                }
                let Some(target_key) = track_outbound_target(&outbound_targets, &remote_addr)
                else {
                    continue;
                };
                let thread = spawn_outbound_maintainer(
                    remote_addr,
                    target_key,
                    Arc::clone(&state),
                    Arc::clone(&stop_requested),
                    Arc::clone(&peer_threads),
                    Arc::clone(&outbound_targets),
                );
                peer_threads
                    .lock()
                    .expect("peer thread list poisoned")
                    .push(thread);
            }
            sleep_with_stop(&stop_requested, PEER_DISCOVERY_INTERVAL);
        }
    })
}

pub(crate) fn outbound_target_dedup_key(remote_addr: &str) -> String {
    match resolve_outbound_target(remote_addr) {
        Ok(resolved) => format!("resolved:{}", resolved[0]),
        Err(_) => remote_addr.trim().to_ascii_lowercase(),
    }
}

fn is_self_outbound_target(bind_addr: SocketAddr, remote_addr: &str) -> bool {
    let Ok(candidates) = resolve_outbound_target(remote_addr) else {
        return false;
    };
    let local_ips = local_listener_ips(bind_addr);
    if local_ips.is_empty() {
        return false;
    }
    candidates.into_iter().any(|candidate| {
        candidate.port() == bind_addr.port() && local_ips.contains(&candidate.ip())
    })
}

fn local_listener_ips(bind_addr: SocketAddr) -> BTreeSet<IpAddr> {
    if !bind_addr.ip().is_unspecified() {
        return BTreeSet::from([bind_addr.ip()]);
    }

    get_if_addrs()
        .map(|interfaces| interfaces.into_iter().map(|iface| iface.ip()).collect())
        .unwrap_or_default()
}

fn track_outbound_target(
    outbound_targets: &Arc<Mutex<BTreeSet<String>>>,
    remote_addr: &str,
) -> Option<String> {
    let target_key = outbound_target_dedup_key(remote_addr);
    let inserted = outbound_targets
        .lock()
        .expect("outbound target registry poisoned")
        .insert(target_key.clone());
    inserted.then_some(target_key)
}

struct OutboundTargetGuard {
    outbound_targets: Arc<Mutex<BTreeSet<String>>>,
    target_key: String,
}

impl OutboundTargetGuard {
    fn new(outbound_targets: Arc<Mutex<BTreeSet<String>>>, target_key: String) -> Self {
        Self {
            outbound_targets,
            target_key,
        }
    }
}

impl Drop for OutboundTargetGuard {
    fn drop(&mut self) {
        let mut targets = self
            .outbound_targets
            .lock()
            .expect("outbound target registry poisoned");
        let _ = targets.remove(&self.target_key);
    }
}

#[derive(Debug, Clone, Copy)]
enum ConnectionRole {
    Inbound,
    Outbound,
}

fn spawn_peer_thread(
    role: ConnectionRole,
    mut stream: TcpStream,
    _remote_addr: SocketAddr,
    peer_id: String,
    state: Arc<Mutex<NodeService>>,
    stop_requested: Arc<AtomicBool>,
    inbound_slot_counter: Option<Arc<AtomicUsize>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let _inbound_slot = inbound_slot_counter.map(InboundThreadGuard::new);
        let disconnect_reason = (|| -> String {
            if let Err(err) = configure_stream(&stream) {
                return format!("stream configure error: {err}");
            }

            match role {
                ConnectionRole::Inbound => {
                    let mut state = state.lock().expect("p2p runtime state poisoned");
                    if let Err(err) = state.p2p_accept_inbound(peer_id.clone()) {
                        return format!("accept inbound rejected peer={peer_id} error={err}");
                    }
                }
                ConnectionRole::Outbound => {
                    let events = {
                        let mut state = state.lock().expect("p2p runtime state poisoned");
                        match state.p2p_open_outbound(peer_id.clone()) {
                            Ok(events) => events,
                            Err(err) => {
                                return format!("open outbound failed peer={peer_id} error={err}");
                            }
                        }
                    };
                    let bytes_sent = match flush_send_events(&mut stream, &peer_id, events) {
                        Ok(bytes_sent) => bytes_sent,
                        Err(err) => {
                            return format!(
                                "outbound handshake send failed peer={peer_id} error={err}"
                            );
                        }
                    };
                    if bytes_sent > 0 {
                        let mut state = state.lock().expect("p2p runtime state poisoned");
                        state.p2p_note_bytes_sent(&peer_id, bytes_sent);
                    }
                }
            }

            let mut last_announced_tip = {
                let state = state.lock().expect("p2p runtime state poisoned");
                state.tip_hash()
            };
            let mut last_announced_mempool = None;
            let mut last_activity = SystemTime::now();
            let peer_network = {
                let state = state.lock().expect("p2p runtime state poisoned");
                state.network()
            };
            let handshake_timeout =
                Duration::from_millis(network_params(peer_network).limits.handshake_timeout_ms);
            let handshake_started = SystemTime::now();
            let mut handshake_ready = false;
            let mut last_sync_maintenance = SystemTime::now();

            loop {
                if stop_requested.load(Ordering::Acquire) {
                    return String::from("runtime stopping");
                }
                let message = match read_message(&mut stream, peer_network) {
                    Ok(Some((message, bytes_received))) => {
                        if bytes_received > 0 {
                            let mut state = state.lock().expect("p2p runtime state poisoned");
                            state.p2p_note_bytes_received(&peer_id, bytes_received);
                            last_activity = SystemTime::now();
                        }
                        log_peer_message("rx", &peer_id, &message, bytes_received);
                        message
                    }
                    Ok(None) => {
                        if !handshake_ready {
                            if handshake_started.elapsed().unwrap_or_default() >= handshake_timeout
                            {
                                return format!("handshake timeout peer={peer_id}");
                            }
                            continue;
                        }
                        if let Some(messages) =
                            poll_tip_announcements(&state, &mut last_announced_tip)
                        {
                            for message in messages {
                                let bytes_sent = match write_message(&mut stream, &message) {
                                    Ok(bytes_sent) => bytes_sent,
                                    Err(err) => {
                                        return format!(
                                            "tip announcement failed peer={peer_id} error={err}"
                                        );
                                    }
                                };
                                if bytes_sent > 0 {
                                    let mut state =
                                        state.lock().expect("p2p runtime state poisoned");
                                    state.p2p_note_bytes_sent(&peer_id, bytes_sent);
                                    last_activity = SystemTime::now();
                                }
                            }
                        }
                        if let Some(messages) =
                            poll_mempool_announcements(&state, &mut last_announced_mempool)
                        {
                            for message in messages {
                                let bytes_sent = match write_message(&mut stream, &message) {
                                    Ok(bytes_sent) => bytes_sent,
                                    Err(err) => {
                                        return format!(
                                            "mempool announcement failed peer={peer_id} error={err}"
                                        );
                                    }
                                };
                                if bytes_sent > 0 {
                                    let mut state =
                                        state.lock().expect("p2p runtime state poisoned");
                                    state.p2p_note_bytes_sent(&peer_id, bytes_sent);
                                    last_activity = SystemTime::now();
                                }
                            }
                        }
                        if handshake_ready
                            && last_sync_maintenance.elapsed().unwrap_or_default()
                                >= sync_maintenance_interval(peer_network)
                        {
                            let events = {
                                let mut state = state.lock().expect("p2p runtime state poisoned");
                                match state.p2p_maintain_peer_sync(&peer_id) {
                                    Ok(events) => events,
                                    Err(err) => {
                                        return format!(
                                            "sync maintenance failed peer={peer_id} error={err}"
                                        );
                                    }
                                }
                            };
                            if let Some(reason) = disconnect_event_for_peer(&events, &peer_id) {
                                return format!(
                                    "sync maintenance disconnect peer={peer_id} reason={reason}"
                                );
                            }
                            let bytes_sent = match flush_send_events(&mut stream, &peer_id, events)
                            {
                                Ok(bytes_sent) => bytes_sent,
                                Err(err) => {
                                    return format!(
                                        "sync maintenance send failed peer={peer_id} error={err}"
                                    );
                                }
                            };
                            if bytes_sent > 0 {
                                let mut state = state.lock().expect("p2p runtime state poisoned");
                                state.p2p_note_bytes_sent(&peer_id, bytes_sent);
                                last_activity = SystemTime::now();
                            }
                            last_sync_maintenance = SystemTime::now();
                        }
                        if handshake_ready
                            && last_activity.elapsed().unwrap_or_default() >= KEEPALIVE_INTERVAL
                        {
                            let keepalive = NetworkMessage::new(
                                peer_network,
                                MessagePayload::Ping {
                                    nonce: unix_timestamp(),
                                },
                            );
                            let bytes_sent = match write_message(&mut stream, &keepalive) {
                                Ok(bytes_sent) => bytes_sent,
                                Err(err) => {
                                    return format!(
                                        "keepalive ping failed peer={peer_id} error={err}"
                                    );
                                }
                            };
                            if bytes_sent > 0 {
                                let mut state = state.lock().expect("p2p runtime state poisoned");
                                state.p2p_note_bytes_sent(&peer_id, bytes_sent);
                                last_activity = SystemTime::now();
                                let _ = dev::append_log(
                                    "p2p",
                                    &format!("keepalive ping sent peer={peer_id}"),
                                );
                            }
                        }
                        continue;
                    }
                    Err(err) => {
                        return format!("peer read failed peer={peer_id} error={err}");
                    }
                };
                let (events, notices) = {
                    let mut state = state.lock().expect("p2p runtime state poisoned");
                    match state.p2p_receive(&peer_id, message) {
                        Ok(result) => result,
                        Err(err) => {
                            return format!("peer receive failed peer={peer_id} error={err}");
                        }
                    }
                };
                for notice in notices {
                    match notice {
                        SyncNotice::Ready { best_height, .. } => {
                            handshake_ready = true;
                            let _ = dev::append_log(
                                "p2p",
                                &format!("peer ready peer={peer_id} best_height={best_height}"),
                            );
                        }
                        SyncNotice::Disconnected { reason, .. } => {
                            return format!("protocol disconnect peer={peer_id} reason={reason}");
                        }
                    }
                }
                let bytes_sent = match flush_send_events(&mut stream, &peer_id, events) {
                    Ok(bytes_sent) => bytes_sent,
                    Err(err) => {
                        return format!("peer send failed peer={peer_id} error={err}");
                    }
                };
                if bytes_sent > 0 {
                    let mut state = state.lock().expect("p2p runtime state poisoned");
                    state.p2p_note_bytes_sent(&peer_id, bytes_sent);
                    last_activity = SystemTime::now();
                }
            }
        })();

        if disconnect_reason != "runtime stopping"
            && should_log_disconnect_reason(&disconnect_reason)
        {
            let _ = dev::append_log("p2p", &disconnect_reason);
        }
        let disconnect_notice = {
            let mut state = state.lock().expect("p2p runtime state poisoned");
            state.p2p_disconnect_peer(&peer_id, disconnect_reason)
        };
        if let Some(SyncNotice::Disconnected { peer, reason }) = disconnect_notice {
            if should_log_disconnect_reason(&reason) {
                let _ = dev::append_log(
                    "p2p",
                    &format!("peer disconnected peer={peer} reason={reason}"),
                );
            }
        }
    })
}

fn poll_tip_announcements(
    state: &Arc<Mutex<NodeService>>,
    last_announced_tip: &mut [u8; 48],
) -> Option<Vec<NetworkMessage>> {
    let state = state.lock().expect("p2p runtime state poisoned");
    let tip_hash = state.tip_hash();
    if !state.p2p_block_relay_ready() {
        *last_announced_tip = tip_hash;
        return None;
    }
    if tip_hash == *last_announced_tip {
        return None;
    }
    let messages = state.p2p_relay_compact_tip_messages_since(*last_announced_tip);
    *last_announced_tip = tip_hash;
    Some(messages)
}

fn poll_mempool_announcements(
    state: &Arc<Mutex<NodeService>>,
    last_announced_fingerprint: &mut Option<[u8; 32]>,
) -> Option<Vec<NetworkMessage>> {
    let (network, txids, fingerprint) = {
        let state = state.lock().expect("p2p runtime state poisoned");
        if !state.p2p_transaction_relay_ready() {
            *last_announced_fingerprint = None;
            return None;
        }
        let fingerprint = state.p2p_mempool_fingerprint();
        if *last_announced_fingerprint == Some(fingerprint) {
            return None;
        }
        (state.network(), state.p2p_mempool_txids(), fingerprint)
    };
    *last_announced_fingerprint = Some(fingerprint);
    if txids.is_empty() {
        return None;
    }

    let max_inventory = network_params(network).limits.max_inv_per_message.max(1);
    let messages = txids
        .chunks(max_inventory)
        .map(|chunk| {
            NetworkMessage::new(
                network,
                MessagePayload::Inv {
                    inventory: chunk
                        .iter()
                        .copied()
                        .map(|txid| InventoryVector {
                            kind: InventoryKind::Transaction,
                            hash: Hash48::from(txid),
                        })
                        .collect(),
                },
            )
        })
        .collect::<Vec<_>>();
    Some(messages)
}

fn configure_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(PEER_IO_POLL_INTERVAL))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(())
}

fn sync_maintenance_interval(network: Network) -> Duration {
    Duration::from_millis(network_params(network).limits.sync_maintenance_interval_ms)
}

fn flush_send_events(
    stream: &mut TcpStream,
    peer_id: &str,
    events: Vec<ConnectionEvent>,
) -> Result<usize, TcpP2pError> {
    let mut bytes_sent = 0usize;
    for event in events {
        if let ConnectionEvent::Send { peer, message } = event {
            if peer == peer_id {
                let sent = write_message(stream, &message)?;
                log_peer_message("tx", peer_id, &message, sent);
                bytes_sent = bytes_sent.saturating_add(sent);
            }
        }
    }
    Ok(bytes_sent)
}

fn disconnect_event_for_peer(events: &[ConnectionEvent], peer_id: &str) -> Option<String> {
    events.iter().find_map(|event| match event {
        ConnectionEvent::Disconnect { peer, reason } if peer == peer_id => Some(reason.clone()),
        _ => None,
    })
}

fn log_peer_message(direction: &str, peer_id: &str, message: &NetworkMessage, bytes: usize) {
    let _ = dev::append_log(
        "p2p",
        &format!(
            "peer {direction} peer={} command={} bytes={} {}",
            peer_id,
            message.command().as_str(),
            bytes,
            message_payload_summary(&message.payload)
        ),
    );
}

fn message_payload_summary(payload: &MessagePayload) -> String {
    match payload {
        MessagePayload::Version(version) => {
            format!(
                "height={} protocol={} user_agent={}",
                version.best_height, version.protocol_version, version.user_agent
            )
        }
        MessagePayload::Verack => String::from("handshake=verack"),
        MessagePayload::Ping { nonce } | MessagePayload::Pong { nonce } => {
            format!("nonce={nonce}")
        }
        MessagePayload::GetAddr => String::from("request=addresses"),
        MessagePayload::Addr { addresses } => format!("count={}", addresses.len()),
        MessagePayload::Inv { inventory }
        | MessagePayload::GetData { inventory }
        | MessagePayload::NotFound { inventory } => format!("count={}", inventory.len()),
        MessagePayload::GetHeaders(message) => {
            format!("locator_len={}", message.locator_hashes.len())
        }
        MessagePayload::Headers { headers } => {
            let first_height = headers.first().map(|header| header.height);
            let last_height = headers.last().map(|header| header.height);
            format!(
                "count={} first_height={first_height:?} last_height={last_height:?}",
                headers.len()
            )
        }
        MessagePayload::Block(block) => format!("height={}", block.header.height),
        MessagePayload::Tx(transaction) => {
            format!(
                "inputs={} outputs={}",
                transaction.inputs.len(),
                transaction.outputs.len()
            )
        }
        MessagePayload::MemPool => String::from("request=mempool"),
        MessagePayload::CompactBlock(message) => {
            format!(
                "height={} tx_count={}",
                message.header.height, message.tx_count
            )
        }
        MessagePayload::GetBlockTxn(message) => format!("indexes={}", message.indexes.len()),
        MessagePayload::BlockTxn(message) => {
            format!(
                "indexes={} txs={}",
                message.indexes.len(),
                message.transactions.len()
            )
        }
    }
}

fn write_message(stream: &mut TcpStream, message: &NetworkMessage) -> Result<usize, TcpP2pError> {
    let bytes = WireCodec::encode(message)?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(bytes.len())
}

fn read_message(
    stream: &mut TcpStream,
    expected_network: Network,
) -> Result<Option<(NetworkMessage, usize)>, TcpP2pError> {
    let mut header = [0u8; FRAME_HEADER_BYTES];
    if !read_exact_with_timeouts(stream, &mut header, true, FRAME_READ_STALL_TIMEOUT)? {
        return Ok(None);
    }

    let payload_len = validated_payload_len(&header, expected_network)?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload_len);
    frame.extend_from_slice(&header);
    frame.resize(FRAME_HEADER_BYTES + payload_len, 0);
    if !read_exact_with_timeouts(
        stream,
        &mut frame[FRAME_HEADER_BYTES..],
        false,
        FRAME_READ_STALL_TIMEOUT,
    )? {
        return Err(TcpP2pError::Io(io::Error::new(
            io::ErrorKind::TimedOut,
            "peer stalled before frame payload",
        )));
    }
    let frame_len = frame.len();
    Ok(Some((WireCodec::decode(&frame)?, frame_len)))
}

fn validated_payload_len(
    header: &[u8; FRAME_HEADER_BYTES],
    expected_network: Network,
) -> Result<usize, TcpP2pError> {
    let magic: [u8; 4] = header[..4].try_into().expect("header magic slice");
    if magic != network_params(expected_network).magic {
        return Err(TcpP2pError::Codec(CodecError::InvalidMagic));
    }
    let payload_len =
        u32::from_le_bytes(header[16..20].try_into().expect("payload length slice")) as usize;
    if payload_len > network_params(expected_network).limits.max_message_size as usize {
        return Err(TcpP2pError::Codec(CodecError::PayloadTooLarge));
    }
    Ok(payload_len)
}

fn read_exact_with_timeouts(
    stream: &mut TcpStream,
    buf: &mut [u8],
    idle_means_no_message: bool,
    stall_timeout: Duration,
) -> Result<bool, TcpP2pError> {
    let mut read = 0usize;
    let mut last_progress = Instant::now();
    while read < buf.len() {
        match stream.read(&mut buf[read..]) {
            Ok(0) if read == 0 => {
                return Err(TcpP2pError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "peer closed connection",
                )));
            }
            Ok(0) => {
                return Err(TcpP2pError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "peer closed connection mid-frame",
                )));
            }
            Ok(count) => {
                read = read.saturating_add(count);
                last_progress = Instant::now();
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) && read == 0
                    && idle_means_no_message =>
            {
                return Ok(false);
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                if last_progress.elapsed() >= stall_timeout {
                    return Err(TcpP2pError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "peer stalled while sending frame",
                    )));
                }
                continue;
            }
            Err(err) => return Err(TcpP2pError::Io(err)),
        }
    }
    Ok(true)
}

fn load_peer_health_snapshot(
    state: &Arc<Mutex<NodeService>>,
    network: Network,
    remote_addr: &str,
) -> PeerHealthRecord {
    let mut state = state.lock().expect("p2p runtime state poisoned");
    state
        .p2p_peer_health(remote_addr)
        .unwrap_or(PeerHealthRecord {
            network,
            remote_addr: remote_addr.to_string(),
            quality_score: PEER_QUALITY_MAX_SCORE,
            consecutive_failures: 0,
            backoff_until_unix: 0,
            last_failure_unix: None,
            last_success_unix: None,
        })
}

fn persist_peer_health(state: &Arc<Mutex<NodeService>>, health: &PeerHealthRecord) {
    let mut state = state.lock().expect("p2p runtime state poisoned");
    state.p2p_save_peer_health(health);
}

fn reap_finished_threads(peer_threads: &Arc<Mutex<Vec<JoinHandle<()>>>>) {
    let mut peer_threads = peer_threads.lock().expect("peer thread list poisoned");
    let mut index = 0usize;
    while index < peer_threads.len() {
        if peer_threads[index].is_finished() {
            let thread = peer_threads.swap_remove(index);
            let _ = thread.join();
        } else {
            index += 1;
        }
    }
}

fn try_acquire_inbound_slot(active_inbound_threads: &Arc<AtomicUsize>, limit: usize) -> bool {
    let mut current = active_inbound_threads.load(Ordering::Acquire);
    loop {
        if current >= limit {
            return false;
        }
        match active_inbound_threads.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(next) => current = next,
        }
    }
}

fn should_log_disconnect_reason(reason: &str) -> bool {
    !reason.contains("invalid network magic") && !reason.starts_with("handshake timeout peer=")
}

struct InboundThreadGuard {
    active_inbound_threads: Arc<AtomicUsize>,
}

impl InboundThreadGuard {
    fn new(active_inbound_threads: Arc<AtomicUsize>) -> Self {
        Self {
            active_inbound_threads,
        }
    }
}

impl Drop for InboundThreadGuard {
    fn drop(&mut self) {
        self.active_inbound_threads.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn next_outbound_retry_delay(consecutive_failures: u32) -> Duration {
    let shift = consecutive_failures.saturating_sub(1).min(5);
    let factor = 1u64 << shift;
    OUTBOUND_RETRY_INTERVAL
        .checked_mul(factor as u32)
        .unwrap_or(OUTBOUND_MAX_RETRY_INTERVAL)
        .min(OUTBOUND_MAX_RETRY_INTERVAL)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sleep_with_stop(stop_requested: &Arc<AtomicBool>, duration: Duration) {
    let started = SystemTime::now();
    while !stop_requested.load(Ordering::Acquire) {
        let elapsed = started.elapsed().unwrap_or_default();
        if elapsed >= duration {
            break;
        }
        let remaining = duration.saturating_sub(elapsed);
        thread::sleep(remaining.min(Duration::from_millis(250)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::acquire_global_test_lock;
    use atho_core::consensus::rules;
    use atho_core::consensus::tx_policy::minimum_required_fee_atoms;
    use atho_core::genesis;
    use atho_p2p::config::MIN_SUPPORTED_PROTOCOL_VERSION;
    use atho_p2p::protocol::{VersionMessage, LOCAL_NODE_SERVICES};
    use atho_rpc::request::RpcRequest;
    use atho_rpc::response::RpcResponse;
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use atho_storage::utxo::UtxoEntry;
    use std::ffi::OsString;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

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

    fn wait_until(label: &str, timeout: Duration, predicate: impl Fn() -> bool) {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for tcp p2p condition: {label}");
    }

    fn test_version_message(network: Network, best_height: u64) -> NetworkMessage {
        NetworkMessage::new(
            network,
            MessagePayload::Version(VersionMessage {
                protocol_version: rules::PROTOCOL_VERSION,
                min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
                services: LOCAL_NODE_SERVICES,
                timestamp_unix: unix_timestamp() as i64,
                network,
                user_agent: String::from("/Atho-Test:0.1.0/"),
                best_height,
                ruleset_version: rules::RULESET_VERSION_V1,
                relay: true,
                genesis_hash: Hash48::from(genesis::genesis_hash(network)),
                tip_hash: Hash48::ZERO,
                chainwork: Hash48::ZERO,
            }),
        )
    }

    fn spawn_ready_then_drop_peer(
        network: Network,
        best_height: u64,
    ) -> (SocketAddr, Arc<AtomicBool>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("ready-drop listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = listener.local_addr().expect("ready-drop addr");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = configure_stream(&stream);
                        if read_message(&mut stream, network).is_ok() {
                            let _ = write_message(
                                &mut stream,
                                &test_version_message(network, best_height),
                            );
                            let _ = write_message(
                                &mut stream,
                                &NetworkMessage::new(network, MessagePayload::Verack),
                            );
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });
        (address, stop, handle)
    }

    #[test]
    fn tcp_runtime_syncs_two_nodes_from_genesis_over_real_sockets() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");

        left.mine_local_block().expect("mine left block");
        right
            .connect_outbound(left.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("right sync height", Duration::from_secs(30), || {
            let left_snapshot = left.snapshot();
            let right_snapshot = right.snapshot();
            right_snapshot.height == left_snapshot.height
                && right_snapshot.tip_hash == left_snapshot.tip_hash
                && left_snapshot.headers_synced
                && right_snapshot.headers_synced
        });
    }

    #[test]
    fn tcp_runtime_connect_outbound_accepts_hostname_targets() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");

        right
            .connect_outbound(format!("localhost:{}", left.bind_addr().port()))
            .expect("connect outbound via hostname");

        wait_until("hostname peer handshake", Duration::from_secs(10), || {
            left.snapshot().peer_count == 1 && right.snapshot().peer_count == 1
        });
    }

    #[test]
    fn outbound_target_dedup_key_collapses_hostname_and_ip_aliases() {
        let hostname = outbound_target_dedup_key("localhost:9100");
        let loopback = outbound_target_dedup_key("127.0.0.1:9100");
        let other = outbound_target_dedup_key("127.0.0.1:9101");

        assert_eq!(hostname, loopback);
        assert_ne!(hostname, other);
    }

    #[test]
    fn self_outbound_detection_matches_local_loopback_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let bind_addr = listener.local_addr().expect("local addr");

        assert!(is_self_outbound_target(
            bind_addr,
            &format!("localhost:{}", bind_addr.port())
        ));
        assert!(!is_self_outbound_target(
            bind_addr,
            &format!("127.0.0.1:{}", bind_addr.port() + 1)
        ));
    }

    #[test]
    fn tcp_runtime_shared_service_keeps_status_and_tip_in_lockstep() {
        let root = std::env::temp_dir().join(format!(
            "atho-tcp-shared-service-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let service = Arc::new(Mutex::new(NodeService::new(NodeConfig::new(
            Network::Regnet,
        ))));
        let runtime =
            TcpP2pRuntime::bind_shared(Network::Regnet, Arc::clone(&service), "127.0.0.1:0")
                .expect("bind shared runtime");

        runtime.mine_local_block().expect("mine shared block");

        let snapshot = runtime.snapshot();
        let status = service.lock().expect("service lock").status();
        assert!(snapshot.height >= 1);
        assert_eq!(snapshot.height, status.block_count);
        assert_eq!(
            snapshot.tip_hash,
            service.lock().expect("service lock").tip_hash()
        );
    }

    #[test]
    fn tcp_runtime_announces_new_tip_to_connected_peers() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");

        right
            .connect_outbound(left.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("peer handshake", Duration::from_secs(10), || {
            let left_snapshot = left.snapshot();
            let right_snapshot = right.snapshot();
            left_snapshot.peer_count == 1
                && right_snapshot.peer_count == 1
                && left_snapshot.headers_synced
                && right_snapshot.headers_synced
                && left_snapshot.height >= left_snapshot.sync_best_height
                && right_snapshot.height >= right_snapshot.sync_best_height
        });

        let announced_at = Instant::now();
        let mined_hash = left.mine_local_block().expect("mine after connect");

        wait_until("tip announcement", Duration::from_secs(30), || {
            let right_snapshot = right.snapshot();
            right_snapshot.tip_hash == mined_hash
                && right_snapshot.height == 1
                && right_snapshot.headers_synced
        });
        eprintln!(
            "tcp_tip_announcement_ms={}",
            announced_at.elapsed().as_millis()
        );
    }

    #[test]
    fn tcp_runtime_reports_peer_directions_and_traffic_in_network_diagnostics() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");

        right
            .connect_outbound(left.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("peer handshake", Duration::from_secs(10), || {
            let left_snapshot = left.snapshot();
            let right_snapshot = right.snapshot();
            left_snapshot.peer_count == 1
                && right_snapshot.peer_count == 1
                && left_snapshot.headers_synced
                && right_snapshot.headers_synced
                && left_snapshot.height >= left_snapshot.sync_best_height
                && right_snapshot.height >= right_snapshot.sync_best_height
        });

        let left_diagnostics = {
            let state = left.state.lock().expect("left state");
            state.network_diagnostics()
        };
        let right_diagnostics = {
            let state = right.state.lock().expect("right state");
            state.network_diagnostics()
        };

        assert_eq!(left_diagnostics.peer_count, 1);
        assert_eq!(left_diagnostics.inbound_peer_count, 1);
        assert_eq!(left_diagnostics.outbound_peer_count, 0);
        assert_eq!(
            left_diagnostics.peers[0].direction,
            atho_rpc::response::NetworkPeerDirection::Inbound
        );
        assert!(left_diagnostics.bytes_received > 0);
        assert!(left_diagnostics.peers[0].bytes_received > 0);

        assert_eq!(right_diagnostics.peer_count, 1);
        assert_eq!(right_diagnostics.inbound_peer_count, 0);
        assert_eq!(right_diagnostics.outbound_peer_count, 1);
        assert_eq!(
            right_diagnostics.peers[0].direction,
            atho_rpc::response::NetworkPeerDirection::Outbound
        );
        assert!(right_diagnostics.bytes_sent > 0);
        assert!(right_diagnostics.peers[0].bytes_sent > 0);
    }

    #[test]
    fn tcp_runtime_retries_outbound_until_peer_comes_online() {
        let reserved = TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let delayed_addr = reserved.local_addr().expect("reserved addr");
        drop(reserved);

        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");
        right.maintain_outbound(delayed_addr.to_string());

        // Allow at least one failed dial before the peer becomes available.
        thread::sleep(Duration::from_millis(1200));

        let left =
            TcpP2pRuntime::new_in_memory(Network::Regnet, delayed_addr.to_string()).expect("left");
        left.mine_local_block().expect("mine delayed peer block");
        let recovery_started = Instant::now();

        wait_until(
            "delayed outbound reconnect",
            Duration::from_secs(30),
            || {
                let left_snapshot = left.snapshot();
                let right_snapshot = right.snapshot();
                left_snapshot.peer_count == 1
                    && right_snapshot.peer_count == 1
                    && right_snapshot.height == left_snapshot.height
                    && right_snapshot.tip_hash == left_snapshot.tip_hash
                    && right_snapshot.headers_synced
            },
        );
        eprintln!(
            "tcp_outbound_reconnect_ms={}",
            recovery_started.elapsed().as_millis()
        );
    }

    #[test]
    fn tcp_runtime_penalizes_peer_health_after_disconnect() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");

        right
            .connect_outbound(left.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("peer handshake", Duration::from_secs(10), || {
            left.snapshot().peer_count == 1 && right.snapshot().peer_count == 1
        });

        let peer_addr = left.bind_addr().to_string();
        {
            let mut state = right.state.lock().expect("right state");
            let notice =
                state.p2p_disconnect_peer(&peer_addr, String::from("peer closed connection"));
            assert!(notice.is_some(), "disconnect should return a notice");
        }

        wait_until(
            "peer health updated after disconnect",
            Duration::from_secs(5),
            || {
                let mut state = right.state.lock().expect("right state");
                state.p2p_peer_health(&peer_addr).is_some_and(|record| {
                    record.consecutive_failures >= 1
                        && record.quality_score < PEER_QUALITY_MAX_SCORE
                        && record.backoff_until_unix >= unix_timestamp()
                })
            },
        );
    }

    #[test]
    fn outbound_retry_delay_grows_exponentially_and_caps() {
        assert_eq!(next_outbound_retry_delay(1), Duration::from_secs(1));
        assert_eq!(next_outbound_retry_delay(2), Duration::from_secs(2));
        assert_eq!(next_outbound_retry_delay(3), Duration::from_secs(4));
        assert_eq!(next_outbound_retry_delay(4), Duration::from_secs(8));
        assert_eq!(next_outbound_retry_delay(8), Duration::from_secs(32));
    }

    #[test]
    fn tcp_runtime_persists_peer_health_after_failed_outbound_attempts() {
        let root = std::env::temp_dir().join(format!(
            "atho-p2p-health-{}-{}",
            std::process::id(),
            unix_timestamp()
        ));
        std::fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let reserved = TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let delayed_addr = reserved.local_addr().expect("reserved addr");
        drop(reserved);

        let right = TcpP2pRuntime::bind_service(
            Network::Regnet,
            NodeService::try_new(NodeConfig::new(Network::Regnet)).expect("service"),
            "127.0.0.1:0",
        )
        .expect("runtime");
        right.maintain_outbound(delayed_addr.to_string());

        wait_until(
            "peer health persisted after failures",
            Duration::from_secs(5),
            || {
                let mut state = right.state.lock().expect("state");
                state
                    .p2p_peer_health(&delayed_addr.to_string())
                    .is_some_and(|record| {
                        record.consecutive_failures >= 1
                            && record.quality_score < PEER_QUALITY_MAX_SCORE
                            && record.backoff_until_unix >= unix_timestamp()
                    })
            },
        );
    }

    #[test]
    fn tcp_runtime_backoff_grows_when_peer_accepts_tcp_then_drops_handshake() {
        let root = std::env::temp_dir().join(format!(
            "atho-p2p-drop-health-{}-{}",
            std::process::id(),
            unix_timestamp()
        ));
        std::fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let listener = TcpListener::bind("127.0.0.1:0").expect("dropping listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let dropping_addr = listener.local_addr().expect("dropping addr");
        let stop_dropping_peer = Arc::new(AtomicBool::new(false));
        let dropping_peer_stop = Arc::clone(&stop_dropping_peer);
        let dropping_peer = thread::spawn(move || {
            while !dropping_peer_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        drop(stream);
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });

        let runtime = TcpP2pRuntime::bind_service(
            Network::Regnet,
            NodeService::try_new(NodeConfig::new(Network::Regnet)).expect("service"),
            "127.0.0.1:0",
        )
        .expect("runtime");
        runtime.maintain_outbound(dropping_addr.to_string());

        wait_until(
            "peer health backoff grows after protocol-level drops",
            Duration::from_secs(10),
            || {
                let mut state = runtime.state.lock().expect("state");
                state
                    .p2p_peer_health(&dropping_addr.to_string())
                    .is_some_and(|record| {
                        record.consecutive_failures >= 2
                            && record.quality_score
                                <= PEER_QUALITY_MAX_SCORE
                                    .saturating_sub(PEER_QUALITY_FAILURE_PENALTY * 2)
                            && record.backoff_until_unix >= unix_timestamp()
                    })
            },
        );

        drop(runtime);
        stop_dropping_peer.store(true, Ordering::Release);
        let _ = TcpStream::connect(dropping_addr);
        dropping_peer.join().expect("dropping peer join");
    }

    #[test]
    fn tcp_runtime_backoff_grows_when_ready_peer_repeatedly_resets() {
        let root = std::env::temp_dir().join(format!(
            "atho-p2p-ready-reset-health-{}-{}",
            std::process::id(),
            unix_timestamp()
        ));
        std::fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let (peer_addr, stop_peer, peer_thread) = spawn_ready_then_drop_peer(Network::Regnet, 42);
        let runtime = TcpP2pRuntime::bind_service(
            Network::Regnet,
            NodeService::try_new(NodeConfig::new(Network::Regnet)).expect("service"),
            "127.0.0.1:0",
        )
        .expect("runtime");
        runtime.maintain_outbound(peer_addr.to_string());

        wait_until(
            "peer health backoff grows after ready peer resets",
            Duration::from_secs(10),
            || {
                let mut state = runtime.state.lock().expect("state");
                state
                    .p2p_peer_health(&peer_addr.to_string())
                    .is_some_and(|record| {
                        record.consecutive_failures >= 2
                            && record.quality_score
                                <= PEER_QUALITY_MAX_SCORE
                                    .saturating_sub(PEER_QUALITY_FAILURE_PENALTY * 2)
                            && record.backoff_until_unix >= unix_timestamp()
                    })
            },
        );

        drop(runtime);
        stop_peer.store(true, Ordering::Release);
        let _ = TcpStream::connect(peer_addr);
        peer_thread.join().expect("ready-drop peer join");
    }

    #[test]
    fn tcp_runtime_three_nodes_follow_same_tip_after_burst_mining() {
        let leader = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("leader");
        let follower_a =
            TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("follower a");
        let follower_b =
            TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("follower b");

        follower_a.maintain_outbound(leader.bind_addr().to_string());
        follower_b.maintain_outbound(leader.bind_addr().to_string());

        wait_until("followers connected", Duration::from_secs(20), || {
            let leader_snapshot = leader.snapshot();
            let a_snapshot = follower_a.snapshot();
            let b_snapshot = follower_b.snapshot();
            leader_snapshot.peer_count == 2
                && a_snapshot.peer_count == 1
                && b_snapshot.peer_count == 1
        });

        for _ in 0..3 {
            leader.mine_local_block().expect("mine burst block");
        }

        wait_until("followers catch burst tip", Duration::from_secs(30), || {
            let leader_snapshot = leader.snapshot();
            let a_snapshot = follower_a.snapshot();
            let b_snapshot = follower_b.snapshot();
            a_snapshot.height == leader_snapshot.height
                && b_snapshot.height == leader_snapshot.height
                && a_snapshot.tip_hash == leader_snapshot.tip_hash
                && b_snapshot.tip_hash == leader_snapshot.tip_hash
                && a_snapshot.headers_synced
                && b_snapshot.headers_synced
        });
    }

    #[test]
    fn payload_length_is_bounded_before_frame_allocation() {
        let mut header = [0u8; FRAME_HEADER_BYTES];
        header[..4].copy_from_slice(&network_params(Network::Mainnet).magic);
        let oversized = network_params(Network::Mainnet)
            .limits
            .max_message_size
            .saturating_add(1);
        header[16..20].copy_from_slice(&oversized.to_le_bytes());
        assert!(matches!(
            validated_payload_len(&header, Network::Mainnet),
            Err(TcpP2pError::Codec(CodecError::PayloadTooLarge))
        ));
    }

    #[test]
    fn read_message_waits_for_slow_payload_after_header() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("listener addr");
        let message = NetworkMessage::new(Network::Regnet, MessagePayload::Ping { nonce: 42 });
        let frame = WireCodec::encode(&message).expect("encode ping");
        let expected_len = frame.len();
        let sender = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            configure_stream(&stream).expect("configure sender");
            stream
                .write_all(&frame[..FRAME_HEADER_BYTES])
                .expect("write header");
            stream.flush().expect("flush header");
            thread::sleep(PEER_IO_POLL_INTERVAL + Duration::from_millis(50));
            stream
                .write_all(&frame[FRAME_HEADER_BYTES..])
                .expect("write payload");
            stream.flush().expect("flush payload");
        });
        let (mut stream, _) = listener.accept().expect("accept");
        configure_stream(&stream).expect("configure receiver");

        let (received, bytes) = read_message(&mut stream, Network::Regnet)
            .expect("read message")
            .expect("message");

        sender.join().expect("sender join");
        assert_eq!(bytes, expected_len);
        assert!(matches!(
            received.payload,
            MessagePayload::Ping { nonce: 42 }
        ));
    }

    #[test]
    fn established_idle_peer_does_not_hold_state_mutex() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");

        right
            .connect_outbound(left.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("peer handshake", Duration::from_secs(10), || {
            left.snapshot().peer_count == 1 && right.snapshot().peer_count == 1
        });

        // Allow the idle read path to cycle after handshake. This used to deadlock the peer
        // thread by holding the shared state mutex across the `read_message(...).network()`
        // temporary and then re-locking inside `poll_tip_announcement`.
        thread::sleep(Duration::from_millis(600));

        wait_until("state mutex released", Duration::from_secs(5), || {
            right.state.try_lock().is_ok()
        });
        assert!(right.snapshot().headers_synced);
    }

    #[test]
    fn silent_inbound_peer_times_out_before_handshake() {
        let runtime = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("node");
        let _silent_peer = TcpStream::connect(runtime.bind_addr()).expect("connect silent peer");

        wait_until("silent inbound accepted", Duration::from_secs(2), || {
            runtime.active_inbound_threads() == 1 && runtime.snapshot().peer_count == 1
        });

        wait_until(
            "silent inbound disconnected after handshake timeout",
            Duration::from_secs(8),
            || runtime.active_inbound_threads() == 0 && runtime.snapshot().peer_count == 0,
        );
    }

    #[test]
    fn raw_inbound_threads_are_capped_before_protocol_handshake() {
        let runtime = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("node");
        let limit = network_params(Network::Regnet).limits.max_inbound_peers;
        let mut peers = Vec::new();
        for _ in 0..(limit + 16) {
            peers.push(TcpStream::connect(runtime.bind_addr()).expect("connect raw inbound"));
        }

        wait_until("inbound slots filled", Duration::from_secs(2), || {
            runtime.active_inbound_threads() > 0
        });

        assert!(runtime.active_inbound_threads() <= limit);
        assert!(runtime.snapshot().peer_count <= limit);

        drop(peers);
        wait_until("raw inbound peers drained", Duration::from_secs(8), || {
            runtime.active_inbound_threads() == 0 && runtime.snapshot().peer_count == 0
        });
    }

    #[test]
    fn tcp_runtime_reorgs_to_longer_branch_after_reconnect() {
        let canonical =
            TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("canonical");
        let fork = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("fork");

        canonical.mine_local_block().expect("canonical block 1");
        canonical.mine_local_block().expect("canonical block 2");
        fork.mine_local_block().expect("fork block 1");
        fork.mine_local_block().expect("fork block 2");
        fork.mine_local_block().expect("fork block 3");

        fork.connect_outbound(canonical.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("fork reorg", Duration::from_secs(30), || {
            let canonical_snapshot = canonical.snapshot();
            let fork_snapshot = fork.snapshot();
            canonical_snapshot.height == fork_snapshot.height
                && canonical_snapshot.tip_hash == fork_snapshot.tip_hash
                && canonical_snapshot.headers_synced
                && fork_snapshot.headers_synced
        });
    }

    #[test]
    fn tcp_runtime_relays_transactions_over_real_sockets() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");
        let (seed_txid, seed_value, seed_script) = crate::dev::seed_utxo(Network::Regnet);

        for runtime in [&left, &right] {
            let mut state = runtime.state.lock().expect("runtime state");
            state.sandbox_with_node_mut(|node| {
                node.dev_seed_chainstate(
                    6,
                    node.tip_hash(),
                    [UtxoEntry::new(
                        Network::Regnet,
                        seed_txid,
                        0,
                        seed_value,
                        vec![seed_script],
                        0,
                        false,
                    )],
                )
                .expect("seed chainstate");
            });
        }

        right
            .connect_outbound(left.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("peer handshake", Duration::from_secs(10), || {
            left.snapshot().peer_count == 1 && right.snapshot().peer_count == 1
        });

        let relayed_at = Instant::now();
        let transaction = crate::dev::signed_spend_transaction(
            Network::Regnet,
            seed_txid,
            seed_value,
            seed_script,
        )
        .expect("signed transaction");
        let txid = transaction.txid();
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &transaction);
        let response = {
            let mut state = left.state.lock().expect("left state");
            state.handle_mut(RpcRequest::SubmitTransaction {
                transaction: transaction.clone(),
                fee_atoms,
            })
        };
        assert!(
            matches!(response, RpcResponse::TransactionSubmitted(submitted) if submitted == txid),
            "unexpected submit response: {response:?}"
        );

        wait_until("relayed transaction", Duration::from_secs(30), || {
            let left_has_tx = {
                let state = left.state.lock().expect("left state");
                state.p2p_mempool_txids().contains(&txid)
            };
            let right_has_tx = {
                let state = right.state.lock().expect("right state");
                state.p2p_mempool_txids().contains(&txid)
            };
            left_has_tx && right_has_tx
        });
        eprintln!(
            "tcp_transaction_relay_ms={}",
            relayed_at.elapsed().as_millis()
        );
    }

    #[test]
    fn mempool_announcements_wait_until_public_node_has_ready_peer() {
        let service = Arc::new(Mutex::new(NodeService::new_ephemeral(NodeConfig::new(
            Network::Testnet,
        ))));
        let (seed_txid, seed_value, seed_script) = crate::dev::seed_utxo(Network::Testnet);
        let transaction = crate::dev::signed_spend_transaction(
            Network::Testnet,
            seed_txid,
            seed_value,
            seed_script,
        )
        .expect("signed transaction");
        let txid = transaction.txid();
        let fee_atoms = minimum_required_fee_atoms(Network::Testnet, &transaction);
        {
            let mut state = service.lock().expect("service lock");
            state.start();
            state.sandbox_with_node_mut(|node| {
                node.dev_seed_chainstate(
                    6,
                    node.tip_hash(),
                    [UtxoEntry::new(
                        Network::Testnet,
                        seed_txid,
                        0,
                        seed_value,
                        vec![seed_script],
                        0,
                        false,
                    )],
                )
                .expect("seed chainstate");
                node.submit_transaction(crate::mempool::MempoolEntry::new(transaction, fee_atoms))
                    .expect("submit tx");
            });
            assert!(state.p2p_mempool_txids().contains(&txid));
            assert!(!state.p2p_transaction_relay_ready());
        }

        let mut last_announced_fingerprint = {
            let state = service.lock().expect("service lock");
            Some(state.p2p_mempool_fingerprint())
        };
        assert!(poll_mempool_announcements(&service, &mut last_announced_fingerprint).is_none());
        assert!(
            last_announced_fingerprint.is_none(),
            "stale announcement cache must clear so txids rebroadcast after sync catches up"
        );
    }

    #[test]
    fn tip_announcements_wait_until_public_node_has_ready_peer() {
        let service = Arc::new(Mutex::new(NodeService::new_ephemeral(NodeConfig::new(
            Network::Testnet,
        ))));
        let tip_hash = {
            let mut state = service.lock().expect("service lock");
            state.start();
            state.tip_hash()
        };
        let mut last_announced_tip = [0x99; 48];

        assert!(poll_tip_announcements(&service, &mut last_announced_tip).is_none());
        assert_eq!(
            last_announced_tip, tip_hash,
            "catch-up tips should be marked seen without relaying stale intermediate blocks"
        );
    }

    #[test]
    fn tcp_runtime_25_node_cluster_converges_restarts_and_recovers() {
        let started = Instant::now();
        let leader = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("leader");
        let leader_addr = leader.bind_addr().to_string();
        let mut followers = Vec::new();

        for _ in 0..24 {
            let follower =
                TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("follower");
            follower.maintain_outbound(leader_addr.clone());
            followers.push(follower);
        }

        wait_until("cluster connected", Duration::from_secs(20), || {
            let leader_snapshot = leader.snapshot();
            leader_snapshot.peer_count == followers.len()
                && followers
                    .iter()
                    .all(|follower| follower.snapshot().peer_count == 1)
        });

        for _ in 0..3 {
            leader.mine_local_block().expect("mine cluster block");
        }

        wait_until("cluster synchronized", Duration::from_secs(30), || {
            let leader_snapshot = leader.snapshot();
            followers.iter().all(|follower| {
                let snapshot = follower.snapshot();
                snapshot.height == leader_snapshot.height
                    && snapshot.tip_hash == leader_snapshot.tip_hash
                    && snapshot.headers_synced
            })
        });

        let restart_targets: Vec<_> = followers.drain(0..5).collect();
        drop(restart_targets);

        wait_until("cluster shrink", Duration::from_secs(10), || {
            leader.snapshot().peer_count == followers.len()
        });

        let mut restarted_followers = Vec::new();
        for _ in 0..5 {
            let follower =
                TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("restart");
            follower.maintain_outbound(leader_addr.clone());
            restarted_followers.push(follower);
        }

        wait_until("cluster restored", Duration::from_secs(30), || {
            let leader_snapshot = leader.snapshot();
            leader_snapshot.peer_count == followers.len() + restarted_followers.len()
                && followers
                    .iter()
                    .chain(restarted_followers.iter())
                    .all(|follower| {
                        let snapshot = follower.snapshot();
                        snapshot.height == leader_snapshot.height
                            && snapshot.tip_hash == leader_snapshot.tip_hash
                            && snapshot.headers_synced
                    })
        });

        eprintln!(
            "tcp_cluster_25_nodes startup_ms={} height={} peer_count={}",
            started.elapsed().as_millis(),
            leader.snapshot().height,
            leader.snapshot().peer_count
        );
    }
}
