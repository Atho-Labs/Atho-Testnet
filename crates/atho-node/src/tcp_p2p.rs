use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::service::NodeService;
use crate::sync::SyncNotice;
use atho_core::network::Network;
use atho_p2p::codec::{CodecError, WireCodec};
use atho_p2p::connection::ConnectionEvent;
use atho_p2p::protocol::NetworkMessage;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;

const FRAME_HEADER_BYTES: usize = 24;

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
    listener_thread: Option<JoinHandle<()>>,
    peer_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
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
        let peer_threads = Arc::new(Mutex::new(Vec::new()));
        let listener_state = Arc::clone(&state);
        let listener_stop = Arc::clone(&stop_requested);
        let listener_peers = Arc::clone(&peer_threads);
        let listener_thread = thread::spawn(move || loop {
            if listener_stop.load(Ordering::Acquire) {
                break;
            }
            match listener.accept() {
                Ok((stream, remote_addr)) => {
                    let thread = spawn_peer_thread(
                        ConnectionRole::Inbound,
                        stream,
                        remote_addr,
                        Arc::clone(&listener_state),
                        Arc::clone(&listener_stop),
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

        Ok(Self {
            network,
            bind_addr: local_addr,
            state,
            stop_requested,
            listener_thread: Some(listener_thread),
            peer_threads,
        })
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
        let remote_addr = SocketAddr::from_str(remote_addr.as_ref())
            .map_err(|_| TcpP2pError::InvalidPeerAddress(remote_addr.as_ref().to_string()))?;
        let stream = TcpStream::connect(remote_addr)?;
        let thread = spawn_peer_thread(
            ConnectionRole::Outbound,
            stream,
            remote_addr,
            Arc::clone(&self.state),
            Arc::clone(&self.stop_requested),
        );
        self.peer_threads
            .lock()
            .expect("peer thread list poisoned")
            .push(thread);
        Ok(())
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
}

impl Drop for TcpP2pRuntime {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.bind_addr);
        if let Some(listener) = self.listener_thread.take() {
            let _ = listener.join();
        }
        let mut peer_threads = self.peer_threads.lock().expect("peer thread list poisoned");
        while let Some(thread) = peer_threads.pop() {
            let _ = thread.join();
        }
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
    remote_addr: SocketAddr,
    state: Arc<Mutex<NodeService>>,
    stop_requested: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if let Err(err) = configure_stream(&stream) {
            let _ = dev::append_log("p2p", &format!("stream configure error: {err}"));
            return;
        }
        let peer_id = remote_addr.to_string();

        match role {
            ConnectionRole::Inbound => {
                let mut state = state.lock().expect("p2p runtime state poisoned");
                if let Err(err) = state.p2p_accept_inbound(peer_id.clone()) {
                    let _ = dev::append_log(
                        "p2p",
                        &format!("accept inbound rejected peer={peer_id} error={err}"),
                    );
                    return;
                }
            }
            ConnectionRole::Outbound => {
                let events = {
                    let mut state = state.lock().expect("p2p runtime state poisoned");
                    match state.p2p_open_outbound(peer_id.clone()) {
                        Ok(events) => events,
                        Err(err) => {
                            let _ = dev::append_log(
                                "p2p",
                                &format!("open outbound failed peer={peer_id} error={err}"),
                            );
                            return;
                        }
                    }
                };
                if let Err(err) = flush_send_events(&mut stream, &peer_id, events) {
                    let _ = dev::append_log(
                        "p2p",
                        &format!("outbound handshake send failed peer={peer_id} error={err}"),
                    );
                    return;
                }
            }
        }

        let mut last_announced_tip = {
            let state = state.lock().expect("p2p runtime state poisoned");
            state.tip_hash()
        };

        loop {
            if stop_requested.load(Ordering::Acquire) {
                break;
            }
            let message = match read_message(&mut stream) {
                Ok(Some(message)) => message,
                Ok(None) => {
                    if let Some(message) = poll_tip_announcement(&state, &mut last_announced_tip) {
                        if let Err(err) = write_message(&mut stream, &message) {
                            let _ = dev::append_log(
                                "p2p",
                                &format!("tip announcement failed peer={peer_id} error={err}"),
                            );
                            break;
                        }
                    }
                    continue;
                }
                Err(err) => {
                    let _ = dev::append_log(
                        "p2p",
                        &format!("peer read failed peer={peer_id} error={err}"),
                    );
                    break;
                }
            };
            let (events, notices) = {
                let mut state = state.lock().expect("p2p runtime state poisoned");
                match state.p2p_receive(&peer_id, message) {
                    Ok(result) => result,
                    Err(err) => {
                        let _ = dev::append_log(
                            "p2p",
                            &format!("peer receive failed peer={peer_id} error={err}"),
                        );
                        break;
                    }
                }
            };
            for notice in notices {
                match notice {
                    SyncNotice::Ready { best_height, .. } => {
                        let _ = dev::append_log(
                            "p2p",
                            &format!("peer ready peer={peer_id} best_height={best_height}"),
                        );
                    }
                    SyncNotice::Disconnected { reason, .. } => {
                        let _ = dev::append_log(
                            "p2p",
                            &format!("peer disconnected peer={peer_id} reason={reason}"),
                        );
                    }
                }
            }
            if let Err(err) = flush_send_events(&mut stream, &peer_id, events) {
                let _ = dev::append_log(
                    "p2p",
                    &format!("peer send failed peer={peer_id} error={err}"),
                );
                break;
            }
        }
    })
}

fn poll_tip_announcement(
    state: &Arc<Mutex<NodeService>>,
    last_announced_tip: &mut [u8; 48],
) -> Option<NetworkMessage> {
    let state = state.lock().expect("p2p runtime state poisoned");
    let tip_hash = state.tip_hash();
    if tip_hash == *last_announced_tip {
        return None;
    }
    let message = state.p2p_relay_compact_tip_message()?;
    *last_announced_tip = tip_hash;
    Some(message)
}

fn configure_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(())
}

fn flush_send_events(
    stream: &mut TcpStream,
    peer_id: &str,
    events: Vec<ConnectionEvent>,
) -> Result<(), TcpP2pError> {
    for event in events {
        if let ConnectionEvent::Send { peer, message } = event {
            if peer == peer_id {
                write_message(stream, &message)?;
            }
        }
    }
    Ok(())
}

fn write_message(stream: &mut TcpStream, message: &NetworkMessage) -> Result<(), TcpP2pError> {
    let bytes = WireCodec::encode(message)?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

fn read_message(stream: &mut TcpStream) -> Result<Option<NetworkMessage>, TcpP2pError> {
    let mut header = [0u8; FRAME_HEADER_BYTES];
    if !read_exact_with_timeouts(stream, &mut header)? {
        return Ok(None);
    }

    let payload_len =
        u32::from_le_bytes(header[16..20].try_into().expect("payload length slice")) as usize;
    let mut frame = header.to_vec();
    frame.resize(FRAME_HEADER_BYTES + payload_len, 0);
    read_exact_with_timeouts(stream, &mut frame[FRAME_HEADER_BYTES..])?;
    Ok(Some(WireCodec::decode(&frame)?))
}

fn read_exact_with_timeouts(stream: &mut TcpStream, buf: &mut [u8]) -> Result<bool, TcpP2pError> {
    let mut read = 0usize;
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
            Ok(count) => read = read.saturating_add(count),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) && read == 0 =>
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
                continue;
            }
            Err(err) => return Err(TcpP2pError::Io(err)),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

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
        });
    }

    #[test]
    fn tcp_runtime_shared_service_keeps_status_and_tip_in_lockstep() {
        let service = Arc::new(Mutex::new(NodeService::new(NodeConfig::new(Network::Regnet))));
        let runtime = TcpP2pRuntime::bind_shared(
            Network::Regnet,
            Arc::clone(&service),
            "127.0.0.1:0",
        )
        .expect("bind shared runtime");

        runtime.mine_local_block().expect("mine shared block");

        let snapshot = runtime.snapshot();
        let status = service.lock().expect("service lock").status();
        assert!(snapshot.height >= 1);
        assert_eq!(snapshot.height, status.block_count);
        assert_eq!(snapshot.tip_hash, service.lock().expect("service lock").tip_hash());
    }

    #[test]
    fn tcp_runtime_announces_new_tip_to_connected_peers() {
        let left = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("left");
        let right = TcpP2pRuntime::new_in_memory(Network::Regnet, "127.0.0.1:0").expect("right");

        right
            .connect_outbound(left.bind_addr().to_string())
            .expect("connect outbound");

        wait_until("peer handshake", Duration::from_secs(10), || {
            left.snapshot().peer_count == 1 && right.snapshot().peer_count == 1
        });

        let mined_hash = left.mine_local_block().expect("mine after connect");

        wait_until("tip announcement", Duration::from_secs(30), || {
            let right_snapshot = right.snapshot();
            right_snapshot.tip_hash == mined_hash && right_snapshot.height == 1
        });
    }
}
