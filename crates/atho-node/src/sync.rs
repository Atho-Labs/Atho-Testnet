//! Node sync notices and background synchronization bookkeeping.
use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use crate::validation::{finalize_witness_commit_refs, ValidationError};
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::network::Network;
use atho_p2p::config::network_params;
use atho_p2p::connection::{ConnectionDirection, ConnectionEvent, ConnectionManager};
use atho_p2p::downloader::{BlockDownloadScheduler, DownloadAssignment};
use atho_p2p::protocol::{
    compact_block_from_block, compact_short_id, reconstruct_compact_block, BlockTxnMessage,
    CompactBlockMessage, CompactBlockReconstruction, GetBlockTxnMessage, Hash48, InventoryKind,
    InventoryVector, MessagePayload, NetworkMessage, PeerAddress, ProtocolError,
};
use atho_p2p::relay::RelayLoop;
use atho_p2p::sync::SyncState;
use atho_storage::chainstate::ChainSelectionOutcome;
use atho_storage::error::StorageError;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const BLOCK_REQUEST_RETRY_TIMEOUT: Duration = Duration::from_secs(8);
const BLOCK_DOWNLOAD_LOOKAHEAD: u64 = 256;
const BLOCK_REQUEST_BATCH_LIMIT: usize = 64;
const HEADERS_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_SIDE_BRANCH_BLOCKS: usize = 4_096;
const MAX_PENDING_COMPACT_BLOCKS: usize = 256;
const PENDING_COMPACT_BLOCK_TIMEOUT: Duration = Duration::from_secs(30);
const POST_HANDSHAKE_PROTOCOL_ERROR_SCORE: u32 = 50;
const ADDR_SPAM_MISBEHAVIOR_SCORE: u32 = 10;
const ADDR_DISCOVERY_INTERVAL_SECS: u64 = 5 * 60;
const ADDR_RELAY_INTERVAL_SECS: u64 = 5 * 60;

#[derive(Debug, Error)]
pub enum NodeSyncError {
    #[error(transparent)]
    Node(#[from] NodeError),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Connection(#[from] atho_p2p::connection::ConnectionError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncNotice {
    Ready { peer: String, best_height: u64 },
    Disconnected { peer: String, reason: String },
}

#[derive(Debug)]
pub struct NodeSync {
    network: Network,
    relay: RelayLoop,
    connections: ConnectionManager,
    downloader: BlockDownloadScheduler,
    mempool_snapshot_peers: BTreeSet<String>,
    side_branches: SideBranchPool,
    pending_compact_blocks: BTreeMap<[u8; 48], PendingCompactBlock>,
    pending_header_blocks: BTreeMap<u64, BTreeMap<[u8; 48], BTreeSet<String>>>,
    header_requests_started: BTreeMap<String, SystemTime>,
    addr_rate_windows: BTreeMap<String, AddrRateWindow>,
    last_addr_received_unix: Option<u64>,
    getaddr_last_sent_unix: BTreeMap<String, u64>,
    addr_relay_last_sent_unix: BTreeMap<String, u64>,
}

#[derive(Debug, Clone)]
struct PendingCompactBlock {
    message: CompactBlockMessage,
    overrides: BTreeMap<u32, atho_core::transaction::Transaction>,
    received_at: SystemTime,
}

#[derive(Debug, Clone, Default)]
struct SideBranchPool {
    blocks: BTreeMap<[u8; 48], SideBranchBlock>,
    children_by_parent: BTreeMap<[u8; 48], BTreeSet<[u8; 48]>>,
    next_seen_order: u64,
}

#[derive(Debug, Clone)]
struct SideBranchBlock {
    block: Block,
    peers: BTreeSet<String>,
    first_seen_order: u64,
    last_seen_order: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct AddrRateWindow {
    window_start_unix: u64,
    messages: u32,
}

impl SideBranchPool {
    fn insert(&mut self, peer: &str, block: Block) {
        let block_hash = block.header.block_hash();
        let previous_hash = block.header.previous_block_hash;
        self.next_seen_order = self.next_seen_order.saturating_add(1);
        let seen_order = self.next_seen_order;

        if let Some(entry) = self.blocks.get_mut(&block_hash) {
            entry.peers.insert(peer.to_string());
            entry.last_seen_order = seen_order;
            return;
        }

        self.children_by_parent
            .entry(previous_hash)
            .or_default()
            .insert(block_hash);
        self.blocks.insert(
            block_hash,
            SideBranchBlock {
                block,
                peers: BTreeSet::from([peer.to_string()]),
                first_seen_order: seen_order,
                last_seen_order: seen_order,
            },
        );
        self.enforce_limit();
    }

    fn remove(&mut self, block_hash: &[u8; 48]) -> Option<Block> {
        let entry = self.blocks.remove(block_hash)?;
        let previous_hash = entry.block.header.previous_block_hash;
        let remove_parent_entry =
            if let Some(children) = self.children_by_parent.get_mut(&previous_hash) {
                children.remove(block_hash);
                children.is_empty()
            } else {
                false
            };
        if remove_parent_entry {
            self.children_by_parent.remove(&previous_hash);
        }
        self.children_by_parent.remove(block_hash);
        Some(entry.block)
    }

    fn get(&self, block_hash: &[u8; 48]) -> Option<&Block> {
        self.blocks.get(block_hash).map(|entry| &entry.block)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.blocks.len()
    }

    fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    fn block_hashes(&self) -> Vec<[u8; 48]> {
        self.blocks.keys().copied().collect()
    }

    fn leaf_hashes(&self) -> Vec<[u8; 48]> {
        let leaves = self
            .blocks
            .keys()
            .filter(|hash| {
                self.children_by_parent
                    .get(*hash)
                    .is_none_or(BTreeSet::is_empty)
            })
            .copied()
            .collect::<Vec<_>>();
        if leaves.is_empty() {
            self.block_hashes()
        } else {
            leaves
        }
    }

    fn remove_canonical_blocks(&mut self, node: &Node) {
        let canonical_hashes = self
            .blocks
            .keys()
            .filter(|hash| node.is_canonical_block(hash))
            .copied()
            .collect::<Vec<_>>();
        for hash in canonical_hashes {
            self.remove(&hash);
        }
    }

    fn enforce_limit(&mut self) {
        while self.blocks.len() > MAX_SIDE_BRANCH_BLOCKS {
            let Some(evict_hash) = self.eviction_candidate() else {
                break;
            };
            self.remove(&evict_hash);
        }
    }

    fn eviction_candidate(&self) -> Option<[u8; 48]> {
        self.blocks
            .iter()
            .filter(|(hash, _)| {
                self.children_by_parent
                    .get(*hash)
                    .is_none_or(BTreeSet::is_empty)
            })
            .max_by_key(|(hash, entry)| {
                (
                    entry.block.header.height,
                    entry.last_seen_order,
                    entry.first_seen_order,
                    **hash,
                )
            })
            .map(|(hash, _)| *hash)
            .or_else(|| {
                self.blocks
                    .iter()
                    .max_by_key(|(hash, entry)| {
                        (
                            entry.block.header.height,
                            entry.last_seen_order,
                            entry.first_seen_order,
                            **hash,
                        )
                    })
                    .map(|(hash, _)| *hash)
            })
    }
}

impl NodeSync {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            relay: RelayLoop::new(network),
            connections: ConnectionManager::new(network),
            downloader: BlockDownloadScheduler::default(),
            mempool_snapshot_peers: BTreeSet::new(),
            side_branches: SideBranchPool::default(),
            pending_compact_blocks: BTreeMap::new(),
            pending_header_blocks: BTreeMap::new(),
            header_requests_started: BTreeMap::new(),
            addr_rate_windows: BTreeMap::new(),
            last_addr_received_unix: None,
            getaddr_last_sent_unix: BTreeMap::new(),
            addr_relay_last_sent_unix: BTreeMap::new(),
        }
    }

    pub fn prime(&mut self, node: &Node) {
        self.prime_relay_from_node_locator(node);
    }

    pub fn sync_state(&self) -> &SyncState {
        self.relay.sync_state()
    }

    pub fn connections(&self) -> &ConnectionManager {
        &self.connections
    }

    pub fn last_addr_received_unix(&self) -> Option<u64> {
        self.last_addr_received_unix
    }

    pub fn last_getaddr_time_unix(&self) -> Option<u64> {
        self.connections.last_getaddr_time_unix()
    }

    pub fn known_peer_count(&self) -> usize {
        self.connections.known_peer_count()
    }

    pub fn fresh_peer_count(&self) -> usize {
        self.connections.fresh_peer_count()
    }

    pub fn stale_peer_count(&self) -> usize {
        self.connections.stale_peer_count()
    }

    pub fn banned_peer_count(&self) -> usize {
        self.connections.banned_count()
    }

    pub fn has_peer(&self, remote_addr: &str) -> bool {
        self.connections.has_peer(remote_addr)
    }

    pub fn add_manual_peer(&mut self, remote_addr: impl Into<String>) {
        self.connections.add_manual_peer(remote_addr);
    }

    pub fn seed_peer_addresses(
        &mut self,
        addresses: &[PeerAddress],
    ) -> Result<Vec<PeerAddress>, NodeSyncError> {
        let public_source = !matches!(self.network, Network::Regnet);
        Ok(self
            .connections
            .note_gossip_addresses(addresses, public_source)?)
    }

    pub fn accept_inbound(&mut self, remote_addr: impl Into<String>) -> Result<(), NodeSyncError> {
        self.connections.accept_inbound(remote_addr)?;
        Ok(())
    }

    pub fn open_outbound(
        &mut self,
        remote_addr: impl Into<String>,
        node: &Node,
    ) -> Result<Vec<ConnectionEvent>, NodeSyncError> {
        let local_version = self.local_version_message(node);
        Ok(self.connections.open_outbound(remote_addr, local_version)?)
    }

    pub fn receive(
        &mut self,
        remote_addr: &str,
        message: NetworkMessage,
        node: &mut Node,
    ) -> Result<(Vec<ConnectionEvent>, Vec<SyncNotice>), NodeSyncError> {
        let local_version = self.local_version_message(node);
        let events = match self
            .connections
            .receive(remote_addr, message, &local_version)
        {
            Ok(events) => events,
            Err(atho_p2p::connection::ConnectionError::Protocol(error)) => {
                self.mempool_snapshot_peers.remove(remote_addr);
                self.refresh_sync_target_from_live_peers(node);
                return Ok((
                    Vec::new(),
                    vec![SyncNotice::Disconnected {
                        peer: remote_addr.to_string(),
                        reason: error.to_string(),
                    }],
                ));
            }
            Err(error) => return Err(error.into()),
        };
        match self.expand_events(events, node) {
            Ok(result) => Ok(result),
            Err(NodeSyncError::Protocol(error)) => {
                let banned = self.connections.record_misbehavior(
                    remote_addr.to_string(),
                    POST_HANDSHAKE_PROTOCOL_ERROR_SCORE,
                );
                let _ = dev::append_log(
                    "p2p",
                    &format!(
                        "post-handshake protocol error peer={} banned={} error={}",
                        remote_addr, banned, error
                    ),
                );
                Err(NodeSyncError::Protocol(error))
            }
            Err(error) => Err(error),
        }
    }

    pub fn relay_block_message(&self, block: &Block) -> NetworkMessage {
        self.relay
            .relay_block(&block.header.block_hash(), block.transactions.len())
    }

    pub fn relay_transaction_message(&self, txid: [u8; 48]) -> NetworkMessage {
        self.relay.relay_transaction(&txid)
    }

    pub fn relay_compact_block_message(&self, block: &Block) -> NetworkMessage {
        NetworkMessage::new(
            self.network,
            MessagePayload::CompactBlock(compact_block_from_block(block)),
        )
    }

    pub fn disconnect_peer(
        &mut self,
        remote_addr: &str,
        reason: String,
        node: &Node,
    ) -> Option<SyncNotice> {
        if !self.connections.disconnect(remote_addr) {
            return None;
        }
        self.note_peer_disconnected(remote_addr, node);
        Some(SyncNotice::Disconnected {
            peer: remote_addr.to_string(),
            reason,
        })
    }

    pub fn maintain_peer_sync(
        &mut self,
        remote_addr: &str,
        node: &Node,
    ) -> Result<Vec<ConnectionEvent>, NodeSyncError> {
        let mut outbound = Vec::new();
        self.prune_pending_compact_blocks();
        self.refresh_sync_target_from_live_peers(node);
        if let Some(event) = self.header_timeout_disconnect_event(remote_addr, node) {
            outbound.push(event);
            return Ok(outbound);
        }
        let stale_downloads = self
            .downloader
            .requeue_stale_inflight_for_peer(remote_addr, BLOCK_REQUEST_RETRY_TIMEOUT);
        if !stale_downloads.is_empty() {
            let first_hash = stale_downloads
                .first()
                .map(|download| short_hash(&download.hash.into_inner()))
                .unwrap_or_else(|| String::from("<none>"));
            if self.ready_peer_count() <= 1 {
                let _ = dev::append_log(
                    "p2p",
                    &format!(
                        "sync maintenance retrying stale block requests in low-peer mode peer={} count={} first_hash={}",
                        remote_addr,
                        stale_downloads.len(),
                        first_hash
                    ),
                );
                self.heal_buffered_branch_parents(Some(remote_addr), node, &mut outbound);
                self.push_block_download_work_for_peer(remote_addr, node, &mut outbound);
                if self.should_rehydrate_headers_from_local_tip(remote_addr, node) {
                    self.reseed_locator_from_node(node);
                    self.queue_getheaders(remote_addr, &mut outbound);
                }
                self.push_address_discovery_events(remote_addr, node, &mut outbound);
                return Ok(outbound);
            }
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "sync maintenance timed out block requests peer={} count={} first_hash={}",
                    remote_addr,
                    stale_downloads.len(),
                    first_hash
                ),
            );
            outbound.push(ConnectionEvent::Disconnect {
                peer: remote_addr.to_string(),
                reason: format!(
                    "block download timeout count={} first_hash={first_hash}",
                    stale_downloads.len()
                ),
            });
            return Ok(outbound);
        }
        self.heal_buffered_branch_parents(Some(remote_addr), node, &mut outbound);
        self.push_block_download_work_for_peer(remote_addr, node, &mut outbound);

        if self.should_rehydrate_headers_from_local_tip(remote_addr, node) {
            self.reseed_locator_from_node(node);
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "sync maintenance re-requesting headers peer={} local_height={} target_height={}",
                    remote_addr,
                    node.height(),
                    self.sync_state().best_height
                ),
            );
            self.queue_getheaders(remote_addr, &mut outbound);
        }
        self.push_address_discovery_events(remote_addr, node, &mut outbound);

        Ok(outbound)
    }

    fn ready_peer_count(&self) -> usize {
        self.connections
            .peer_snapshots()
            .into_iter()
            .filter(|peer| peer.handshake_ready)
            .count()
    }

    fn expand_events(
        &mut self,
        events: Vec<ConnectionEvent>,
        node: &mut Node,
    ) -> Result<(Vec<ConnectionEvent>, Vec<SyncNotice>), NodeSyncError> {
        let mut outbound = Vec::new();
        let mut notices = Vec::new();

        for event in events {
            match event {
                ConnectionEvent::Send { .. } => outbound.push(event),
                ConnectionEvent::Disconnect { peer, reason } => {
                    self.note_peer_disconnected(&peer, node);
                    notices.push(SyncNotice::Disconnected { peer, reason });
                }
                ConnectionEvent::Ready { peer, best_height } => {
                    if let Some(version) = self.connections.remote_version(&peer).cloned() {
                        self.relay.accept_version(peer.clone(), &version)?;
                    }
                    self.downloader.note_peer_ready(peer.clone());
                    self.record_peer_observation(node, &peer, best_height)?;
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "sync ready peer={} local_height={} peer_best_height={} target_height={}",
                            peer,
                            node.height(),
                            best_height,
                            self.sync_state().best_height
                        ),
                    );
                    notices.push(SyncNotice::Ready {
                        peer: peer.clone(),
                        best_height,
                    });
                    self.queue_getheaders(&peer, &mut outbound);
                }
                ConnectionEvent::Message { peer, message } => {
                    self.handle_message(&peer, message, node, &mut outbound)?;
                }
            }
        }

        self.maybe_request_mempool_snapshots(node, &mut outbound);
        Ok((outbound, notices))
    }

    fn refresh_sync_target_from_live_peers(&mut self, node: &Node) {
        let peer_best_height = self
            .connections
            .peer_snapshots()
            .into_iter()
            .filter(|peer| peer.handshake_ready)
            .filter_map(|peer| peer.best_height)
            .max();
        self.relay.refresh_sync_target_at(
            node.height(),
            Some(Hash48::from(node.tip_hash())),
            peer_best_height,
        );
    }

    fn note_peer_disconnected(&mut self, peer: &str, node: &Node) {
        self.downloader.note_peer_disconnected(peer);
        self.mempool_snapshot_peers.remove(peer);
        self.header_requests_started.remove(peer);
        self.getaddr_last_sent_unix.remove(peer);
        self.addr_relay_last_sent_unix.remove(peer);
        // Buffered blocks are already marked received by the downloader. Keep
        // them across disconnects so a reconnect or another peer can still
        // complete the branch instead of leaving an unrecoverable gap.
        self.refresh_sync_target_from_live_peers(node);
    }

    fn note_local_chain_progress(&mut self, node: &Node) {
        let peer_best_height = self
            .connections
            .peer_snapshots()
            .into_iter()
            .filter(|peer| peer.handshake_ready)
            .filter_map(|peer| peer.best_height)
            .max();
        self.relay.note_local_chain_progress_at(
            node.height(),
            Some(Hash48::from(node.tip_hash())),
            peer_best_height,
        );
    }

    fn local_version_message(&self, node: &Node) -> NetworkMessage {
        self.relay
            .build_version_message(node.height(), node.tip_hash(), node.chainwork_bytes())
    }

    fn prime_relay_from_node_locator(&mut self, node: &Node) {
        self.relay.prime_with_locator(
            node.height(),
            Some(Hash48::from(node.tip_hash())),
            node.block_locator_hashes()
                .into_iter()
                .map(Hash48::from)
                .collect(),
        );
    }

    fn reseed_locator_from_node(&mut self, node: &Node) {
        self.relay.reseed_locator_hashes(
            node.block_locator_hashes()
                .into_iter()
                .map(Hash48::from)
                .collect(),
        );
    }

    fn handle_message(
        &mut self,
        peer: &str,
        message: NetworkMessage,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        match message.payload {
            MessagePayload::Inv { inventory } => {
                let chain_synced = self.chain_synced(node);
                for vector in &inventory {
                    if vector.kind == InventoryKind::Block {
                        self.downloader
                            .note_inventory(peer, vector.hash.into_inner());
                    }
                }
                let requests = self.missing_inventory_requests(node, &inventory, chain_synced);
                if !requests.is_empty() {
                    outbound.push(ConnectionEvent::Send {
                        peer: peer.to_string(),
                        message: NetworkMessage::new(
                            self.network,
                            MessagePayload::GetData {
                                inventory: requests,
                            },
                        ),
                    });
                }
            }
            MessagePayload::GetData { inventory } => {
                self.serve_getdata(peer, &inventory, node, outbound);
            }
            MessagePayload::GetHeaders(request) => {
                let headers = node.headers_after_locator(
                    &request
                        .locator_hashes
                        .iter()
                        .copied()
                        .map(Into::into)
                        .collect::<Vec<[u8; 48]>>(),
                    request.stop_hash.into_inner(),
                    network_params(self.network).limits.max_headers_per_message,
                );
                let first_height = headers.first().map(|header| header.height);
                let last_height = headers.last().map(|header| header.height);
                let _ = dev::append_log(
                    "p2p",
                    &format!(
                        "serving headers peer={} locator_len={} count={} first_height={:?} last_height={:?}",
                        peer,
                        request.locator_hashes.len(),
                        headers.len(),
                        first_height,
                        last_height
                    ),
                );
                outbound.push(ConnectionEvent::Send {
                    peer: peer.to_string(),
                    message: NetworkMessage::new(self.network, MessagePayload::Headers { headers }),
                });
            }
            MessagePayload::Headers { headers } => {
                self.header_requests_started.remove(peer);
                let first_height = headers.first().map(|header| header.height);
                let last_height = headers.last().map(|header| header.height);
                let header_count = headers.len();
                if let Some(first) = headers.first() {
                    if let Some(parent_height) = node.known_block_height(&first.previous_block_hash)
                    {
                        let expected_height = parent_height.saturating_add(1);
                        if first.height != expected_height {
                            let _ = dev::append_log(
                                "p2p",
                                &format!(
                                    "rejecting headers peer={} first_height={} expected_height={} prev={} target_height={}",
                                    peer,
                                    first.height,
                                    expected_height,
                                    short_hash(&first.previous_block_hash),
                                    self.relay.sync_state().best_height
                                ),
                            );
                            return Err(NodeSyncError::Protocol(
                                ProtocolError::InvalidHeadersSequence,
                            ));
                        }
                    }
                }
                self.relay.accept_headers(&headers)?;
                if header_count == 0 && node.height() < self.relay.sync_state().best_height {
                    self.reseed_locator_from_node(node);
                    self.relay.mark_headers_unsynced();
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "empty headers while behind peer={} local_height={} target_height={} locator_len={}",
                            peer,
                            node.height(),
                            self.relay.sync_state().best_height,
                            self.relay.sync_state().locator_hashes.len()
                        ),
                    );
                    self.queue_getheaders(peer, outbound);
                    return Ok(());
                }
                if let Some(last_header) = headers.last() {
                    self.note_observed_peer_tip(peer, last_header, node);
                }
                let headers_synced = self.relay.sync_state().headers_synced;
                let _ = dev::append_log(
                    "p2p",
                    &format!(
                        "received headers peer={} count={} first_height={:?} last_height={:?} headers_synced={} target_height={}",
                        peer,
                        header_count,
                        first_height,
                        last_height,
                        headers_synced,
                        self.relay.sync_state().best_height
                    ),
                );
                let mut known_noncanonical_blocks = Vec::new();
                for header in &headers {
                    let hash = header.block_hash();
                    if node.is_canonical_block(&hash) {
                        self.remove_pending_header_block(header.height, &hash);
                        continue;
                    }
                    if let Some(block) = node.block_by_hash(hash) {
                        known_noncanonical_blocks.push(block);
                    } else {
                        self.note_pending_header_block(peer, header.height, hash);
                    }
                }
                self.push_block_download_work_for_peer(peer, node, outbound);
                for block in known_noncanonical_blocks {
                    self.handle_received_block(peer, block, node, outbound)?;
                }
                if !self.relay.sync_state().headers_synced {
                    self.queue_getheaders(peer, outbound);
                } else {
                    self.record_getaddr_sent(peer, now_unix());
                    outbound.push(ConnectionEvent::Send {
                        peer: peer.to_string(),
                        message: NetworkMessage::new(self.network, MessagePayload::GetAddr),
                    });
                }
            }
            MessagePayload::Block(block) => {
                self.handle_received_block(peer, block, node, outbound)?;
            }
            MessagePayload::CompactBlock(message) => {
                self.handle_compact_block(peer, message, node, outbound)?;
            }
            MessagePayload::GetBlockTxn(request) => {
                self.serve_getblocktxn(peer, request, node, outbound);
            }
            MessagePayload::BlockTxn(response) => {
                self.handle_blocktxn(peer, response, node, outbound)?;
            }
            MessagePayload::Tx(transaction) => {
                if !self.chain_synced(node) {
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "ignoring relayed tx during catch-up peer={} txid={} local_height={} target_height={}",
                            peer,
                            short_hash(&transaction.txid()),
                            node.height(),
                            self.sync_state().best_height
                        ),
                    );
                    return Ok(());
                }
                let txid = transaction.txid();
                if !node.mempool_contains(&txid) {
                    match node.accept_relayed_transaction(transaction) {
                        Ok(_) => {}
                        Err(err) if Self::recoverable_relay_transaction_error(&err) => {
                            let _ = dev::append_log(
                                "p2p",
                                &format!(
                                    "ignoring recoverable relay tx peer={} txid={} error={}",
                                    peer,
                                    hex::encode(txid),
                                    err
                                ),
                            );
                        }
                        Err(err) => return Err(err.into()),
                    }
                }
            }
            MessagePayload::MemPool => {
                if !self.chain_synced(node) {
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "ignoring mempool snapshot request during catch-up peer={} local_height={} target_height={}",
                            peer,
                            node.height(),
                            self.sync_state().best_height
                        ),
                    );
                    return Ok(());
                }
                let inventory = node
                    .mempool_transactions()
                    .into_iter()
                    .take(network_params(self.network).limits.max_inv_per_message)
                    .map(|transaction| InventoryVector {
                        kind: InventoryKind::Transaction,
                        hash: Hash48::from(transaction.txid()),
                    })
                    .collect::<Vec<_>>();
                if !inventory.is_empty() {
                    outbound.push(ConnectionEvent::Send {
                        peer: peer.to_string(),
                        message: NetworkMessage::new(
                            self.network,
                            MessagePayload::Inv { inventory },
                        ),
                    });
                }
            }
            MessagePayload::NotFound { inventory } => {
                let hashes = inventory
                    .iter()
                    .filter(|vector| vector.kind == InventoryKind::Block)
                    .map(|vector| vector.hash.into_inner())
                    .collect::<Vec<_>>();
                self.downloader.note_not_found(peer, &hashes);
                self.push_block_download_work_for_peer(peer, node, outbound);
            }
            MessagePayload::Pong { .. } => {}
            MessagePayload::Version(_)
            | MessagePayload::Verack
            | MessagePayload::Ping { .. }
            | MessagePayload::GetAddr => {
                return Err(NodeSyncError::Protocol(ProtocolError::UnexpectedPayload));
            }
            MessagePayload::Addr { addresses } => {
                if !self.allow_addr_message(peer) {
                    let banned = self
                        .connections
                        .record_misbehavior(peer.to_string(), ADDR_SPAM_MISBEHAVIOR_SCORE);
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "addr relay rate-limited peer={} banned={} count={}",
                            peer,
                            banned,
                            addresses.len()
                        ),
                    );
                    return Ok(());
                }
                self.last_addr_received_unix = Some(now_unix());
                let accepted = self
                    .connections
                    .note_gossip_addresses_from_source(
                        peer,
                        &addresses,
                        !matches!(self.network, Network::Regnet),
                    )
                    .map_err(NodeSyncError::from)?;
                let observed_height = self
                    .connections
                    .remote_best_height(peer)
                    .unwrap_or_else(|| node.height());
                let observed_unix = now_unix();
                node.observe_peer(peer.to_string(), observed_height, observed_unix)?;
                let accepted_count = accepted.len();
                for address in accepted {
                    node.observe_peer_address(&address, observed_height, observed_unix)?;
                }
                if accepted_count > 0 {
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "addr accepted peer={} count={} known_peers={}",
                            peer,
                            accepted_count,
                            self.known_peer_count()
                        ),
                    );
                }
            }
        }
        Ok(())
    }

    fn push_address_discovery_events(
        &mut self,
        remote_addr: &str,
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        if !self.chain_synced(node) || !self.peer_is_ready(remote_addr) {
            return;
        }

        let now = now_unix();
        if self.should_send_periodic_getaddr(remote_addr, now) {
            self.record_getaddr_sent(remote_addr, now);
            let _ = dev::append_log("p2p", &format!("periodic getaddr peer={remote_addr}"));
            outbound.push(ConnectionEvent::Send {
                peer: remote_addr.to_string(),
                message: NetworkMessage::new(self.network, MessagePayload::GetAddr),
            });
        }

        if !self.should_send_periodic_addr_relay(remote_addr, now) {
            return;
        }
        let addresses = self.connections.relay_addresses_for_peer(remote_addr);
        if addresses.is_empty() {
            return;
        }
        self.record_addr_relay_sent(remote_addr, now);
        let _ = dev::append_log(
            "p2p",
            &format!(
                "periodic addr relay peer={} count={}",
                remote_addr,
                addresses.len()
            ),
        );
        outbound.push(ConnectionEvent::Send {
            peer: remote_addr.to_string(),
            message: NetworkMessage::new(self.network, MessagePayload::Addr { addresses }),
        });
    }

    fn peer_is_ready(&self, remote_addr: &str) -> bool {
        self.connections
            .peer_snapshots()
            .into_iter()
            .any(|peer| peer.remote_addr == remote_addr && peer.handshake_ready)
    }

    fn should_send_periodic_getaddr(&self, remote_addr: &str, now: u64) -> bool {
        self.getaddr_last_sent_unix
            .get(remote_addr)
            .is_none_or(|last| now.saturating_sub(*last) >= ADDR_DISCOVERY_INTERVAL_SECS)
    }

    fn record_getaddr_sent(&mut self, remote_addr: &str, now: u64) {
        self.getaddr_last_sent_unix
            .insert(remote_addr.to_string(), now);
    }

    fn should_send_periodic_addr_relay(&self, remote_addr: &str, now: u64) -> bool {
        self.addr_relay_last_sent_unix
            .get(remote_addr)
            .is_none_or(|last| now.saturating_sub(*last) >= ADDR_RELAY_INTERVAL_SECS)
    }

    fn record_addr_relay_sent(&mut self, remote_addr: &str, now: u64) {
        self.addr_relay_last_sent_unix
            .insert(remote_addr.to_string(), now);
    }

    fn allow_addr_message(&mut self, peer: &str) -> bool {
        let now = now_unix();
        let limits = network_params(self.network).limits;
        let window = self.addr_rate_windows.entry(peer.to_string()).or_default();
        if now.saturating_sub(window.window_start_unix) >= limits.addr_rate_limit_window_secs {
            window.window_start_unix = now;
            window.messages = 0;
        }
        if window.messages >= limits.max_addr_messages_per_window {
            return false;
        }
        window.messages = window.messages.saturating_add(1);
        true
    }

    fn note_observed_peer_tip(&mut self, peer: &str, header: &BlockHeader, node: &Node) {
        if header.network_id != self.network {
            return;
        }
        let observed_tip = header.block_hash();
        self.connections
            .note_peer_tip(peer, header.height, Hash48::from(observed_tip));
        let previous_target = self.sync_state().best_height;
        let previous_headers_synced = self.sync_state().headers_synced;
        self.relay.note_observed_tip_at(
            node.height(),
            Some(Hash48::from(node.tip_hash())),
            header.height,
            observed_tip,
        );
        if header.height > node.height()
            && (header.height > previous_target || previous_headers_synced)
        {
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "observed peer tip peer={} observed_height={} local_height={} target_height={} headers_synced={}",
                    peer,
                    header.height,
                    node.height(),
                    self.sync_state().best_height,
                    self.sync_state().headers_synced
                ),
            );
        }
    }

    fn record_peer_observation(
        &self,
        node: &mut Node,
        peer: &str,
        observed_height: u64,
    ) -> Result<(), NodeSyncError> {
        if matches!(
            self.connections.direction(peer),
            Some(ConnectionDirection::Outbound)
        ) {
            node.observe_peer(peer.to_string(), observed_height, now_unix())?;
        }
        Ok(())
    }

    fn push_scheduled_block_requests(&mut self, outbound: &mut Vec<ConnectionEvent>) {
        let limits = network_params(self.network).limits;
        for assignment in self
            .downloader
            .assignments(limits.max_blocks_in_flight, limits.max_requests_per_peer)
        {
            self.push_download_assignment(assignment, outbound);
        }
    }

    fn push_block_download_work_for_peer(
        &mut self,
        peer: &str,
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        self.stage_header_blocks_near_tip(node);
        let limits = network_params(self.network).limits;
        if let Some(assignment) = self.downloader.assignment_for_peer_limited(
            peer,
            limits.max_blocks_in_flight,
            limits.max_requests_per_peer,
            BLOCK_REQUEST_BATCH_LIMIT,
        ) {
            self.push_download_assignment(assignment, outbound);
        }
    }

    fn push_download_assignment(
        &self,
        assignment: DownloadAssignment,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        let requested = assignment
            .inventory
            .iter()
            .map(|item| short_hash(&item.hash.into_inner()))
            .collect::<Vec<_>>();
        let _ = dev::append_log(
            "p2p",
            &format!(
                "requesting blocks peer={} count={} hashes=[{}]",
                assignment.peer,
                assignment.inventory.len(),
                requested.join(",")
            ),
        );
        outbound.push(ConnectionEvent::Send {
            peer: assignment.peer,
            message: NetworkMessage::new(
                self.network,
                MessagePayload::GetData {
                    inventory: assignment.inventory,
                },
            ),
        });
    }

    fn should_rehydrate_headers_from_local_tip(&self, remote_addr: &str, node: &Node) -> bool {
        if self.header_request_inflight(remote_addr) {
            return false;
        }
        if !self
            .connections
            .peer_snapshots()
            .into_iter()
            .any(|peer| peer.remote_addr == remote_addr && peer.handshake_ready)
        {
            return false;
        }
        let target_height = self.sync_state().best_height.max(
            self.connections
                .remote_best_height(remote_addr)
                .unwrap_or(0),
        );
        if node.height() >= target_height {
            return false;
        }
        !self.sync_state().headers_synced || self.downloader.is_idle()
    }

    fn header_request_inflight(&self, remote_addr: &str) -> bool {
        self.header_requests_started
            .get(remote_addr)
            .is_some_and(|started| started.elapsed().unwrap_or_default() < HEADERS_REQUEST_TIMEOUT)
    }

    fn header_timeout_disconnect_event(
        &mut self,
        remote_addr: &str,
        node: &Node,
    ) -> Option<ConnectionEvent> {
        let target_height = self.sync_state().best_height.max(
            self.connections
                .remote_best_height(remote_addr)
                .unwrap_or(node.height()),
        );
        if node.height() >= target_height {
            self.header_requests_started.remove(remote_addr);
            return None;
        }
        let timed_out = self
            .header_requests_started
            .get(remote_addr)
            .is_some_and(|started| {
                started.elapsed().unwrap_or_default() >= HEADERS_REQUEST_TIMEOUT
            });
        if !timed_out {
            return None;
        }
        self.header_requests_started.remove(remote_addr);
        let _ = dev::append_log(
            "p2p",
            &format!(
                "headers request timed out peer={} local_height={} target_height={}",
                remote_addr,
                node.height(),
                target_height
            ),
        );
        Some(ConnectionEvent::Disconnect {
            peer: remote_addr.to_string(),
            reason: format!(
                "headers download timeout local_height={} target_height={target_height}",
                node.height()
            ),
        })
    }

    fn queue_getheaders(&mut self, peer: &str, outbound: &mut Vec<ConnectionEvent>) {
        self.header_requests_started
            .insert(peer.to_string(), SystemTime::now());
        outbound.push(ConnectionEvent::Send {
            peer: peer.to_string(),
            message: self.relay.build_getheaders(peer.to_string()),
        });
    }

    fn should_defer_unsolicited_future_block(
        &self,
        header: &BlockHeader,
        node: &Node,
        requested: bool,
    ) -> bool {
        if requested || header.network_id != self.network {
            return false;
        }
        if header.previous_block_hash == node.tip_hash() {
            return false;
        }
        header.height > node.height().saturating_add(BLOCK_DOWNLOAD_LOOKAHEAD)
    }

    fn defer_unsolicited_future_block(
        &mut self,
        peer: &str,
        header: &BlockHeader,
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
        source: &str,
        requested: bool,
    ) -> bool {
        if !self.should_defer_unsolicited_future_block(header, node, requested) {
            return false;
        }

        let block_hash = header.block_hash();
        self.note_observed_peer_tip(peer, header, node);
        if !self.sync_state().headers_synced && !self.header_request_inflight(peer) {
            self.reseed_locator_from_node(node);
            self.queue_getheaders(peer, outbound);
        }
        let _ = dev::append_log(
            "p2p",
            &format!(
                "deferred far-ahead unsolicited block source={} peer={} height={} hash={} local_height={} target_height={}",
                source,
                peer,
                header.height,
                short_hash(&block_hash),
                node.height(),
                self.sync_state().best_height
            ),
        );
        true
    }

    fn handle_received_block(
        &mut self,
        peer: &str,
        block: Block,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        let block_hash = block.header.block_hash();
        let block_height = block.header.height;
        if let Err(validation) =
            crate::validation::validate_block(&block, block.header.height, self.network)
        {
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "rejecting block envelope peer={} height={} hash={} error={}",
                    peer,
                    block.header.height,
                    short_hash(&block_hash),
                    validation
                ),
            );
            return Err(NodeError::Validation(validation).into());
        }
        let requested = self.downloader.is_inflight(block_hash);
        if self.defer_unsolicited_future_block(
            peer,
            &block.header,
            node,
            outbound,
            "block",
            requested,
        ) {
            self.pending_compact_blocks.remove(&block_hash);
            return Ok(());
        }
        if node.is_canonical_block(&block_hash) {
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "received duplicate block peer={} height={} hash={}",
                    peer,
                    block.header.height,
                    short_hash(&block_hash)
                ),
            );
            self.downloader.note_block_received(block_hash);
            self.pending_compact_blocks.remove(&block_hash);
            self.side_branches.remove(&block_hash);
            self.remove_pending_header_block(block.header.height, &block_hash);
            self.push_block_download_work_for_peer(peer, node, outbound);
            self.process_buffered_branches(Some(peer), node, outbound)?;
            return Ok(());
        }

        if block.header.previous_block_hash == node.tip_hash() {
            match node.submit_block(&block) {
                Ok(()) => {
                    self.note_observed_peer_tip(peer, &block.header, node);
                    self.note_local_chain_progress(node);
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "accepted block peer={} height={} hash={} new_local_height={} target_height={}",
                            peer,
                            block.header.height,
                            short_hash(&block_hash),
                            node.height(),
                            self.sync_state().best_height
                        ),
                    );
                    self.downloader.note_block_received(block_hash);
                    self.pending_compact_blocks.remove(&block_hash);
                    self.side_branches.remove(&block_hash);
                    self.side_branches.remove_canonical_blocks(node);
                    self.remove_pending_header_block(block.header.height, &block_hash);
                    self.push_block_download_work_for_peer(peer, node, outbound);
                    self.process_buffered_branches(Some(peer), node, outbound)?;
                    return Ok(());
                }
                Err(NodeError::Validation(validation))
                    if Self::recoverable_tip_validation(&validation) =>
                {
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "buffering recoverable block peer={} height={} hash={} reason={}",
                            peer,
                            block.header.height,
                            short_hash(&block_hash),
                            validation
                        ),
                    );
                    // Keep the block buffered so fork-choice can re-evaluate it once the
                    // branch is complete enough to compare by cumulative work.
                }
                Err(err) => {
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "rejecting block peer={} height={} hash={} error={}",
                            peer,
                            block.header.height,
                            short_hash(&block_hash),
                            err
                        ),
                    );
                    return Err(err.into());
                }
            }
        }

        self.note_observed_peer_tip(peer, &block.header, node);
        let previous_hash = block.header.previous_block_hash;
        let _ = dev::append_log(
            "p2p",
            &format!(
                "buffering block peer={} height={} hash={} prev={} local_tip={}",
                peer,
                block.header.height,
                short_hash(&block_hash),
                short_hash(&block.header.previous_block_hash),
                short_hash(&node.tip_hash())
            ),
        );
        self.buffer_peer_block_with_known_ancestors(peer, block, node);
        self.downloader.note_block_received(block_hash);
        self.pending_compact_blocks.remove(&block_hash);
        self.remove_pending_header_block(block_height, &block_hash);
        if !node.is_canonical_block(&previous_hash)
            && self.side_branches.get(&previous_hash).is_none()
        {
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "orphan parent needed peer={} parent={} child={}",
                    peer,
                    short_hash(&previous_hash),
                    short_hash(&block_hash)
                ),
            );
        }
        self.heal_buffered_branch_parents(Some(peer), node, outbound);
        self.push_block_download_work_for_peer(peer, node, outbound);
        self.process_buffered_branches(Some(peer), node, outbound)?;

        Ok(())
    }

    #[cfg(test)]
    fn branch_is_preferred_over_current(branch: &[Block], current_blocks: &[Block]) -> bool {
        let Some(first) = branch.first() else {
            return false;
        };
        let Some(fork_index) = current_blocks
            .iter()
            .position(|block| block.header.block_hash() == first.header.previous_block_hash)
        else {
            return false;
        };
        pow::branch_is_preferred(branch, &current_blocks[fork_index + 1..])
    }

    fn buffer_peer_block_with_known_ancestors(&mut self, peer: &str, block: Block, node: &Node) {
        let mut previous_hash = block.header.previous_block_hash;
        self.side_branches.insert(peer, block);

        let mut visited = BTreeSet::new();
        while !node.is_canonical_block(&previous_hash) && visited.insert(previous_hash) {
            let parent = self
                .side_branches
                .get(&previous_hash)
                .cloned()
                .or_else(|| node.block_by_hash(previous_hash));
            let Some(parent) = parent else {
                break;
            };
            previous_hash = parent.header.previous_block_hash;
            self.side_branches.insert(peer, parent);
        }
    }

    fn heal_buffered_branch_parents(
        &mut self,
        preferred_peer: Option<&str>,
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        let mut discovered_local_parent = true;
        while discovered_local_parent {
            discovered_local_parent = false;
            for parent_hash in self.missing_buffered_parent_hashes(node) {
                if let Some(parent) = node.block_by_hash(parent_hash) {
                    let parent_peer = preferred_peer.unwrap_or("local-archive");
                    let _ = dev::append_log(
                        "p2p",
                        &format!(
                            "rehydrating buffered branch parent source=local-archive height={} hash={}",
                            parent.header.height,
                            short_hash(&parent_hash)
                        ),
                    );
                    self.side_branches.insert(parent_peer, parent);
                    self.downloader.note_block_received(parent_hash);
                    discovered_local_parent = true;
                }
            }
        }

        for parent_hash in self.missing_buffered_parent_hashes(node) {
            self.downloader
                .queue_priority_block(preferred_peer, parent_hash);
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "queued orphan parent request peer={} parent={}",
                    preferred_peer.unwrap_or("<any>"),
                    short_hash(&parent_hash)
                ),
            );
        }
        if let Some(peer) = preferred_peer {
            self.push_block_download_work_for_peer(peer, node, outbound);
        } else {
            self.stage_header_blocks_near_tip(node);
            self.push_scheduled_block_requests(outbound);
        }
    }

    fn note_pending_header_block(&mut self, peer: &str, height: u64, hash: [u8; 48]) {
        self.pending_header_blocks
            .entry(height)
            .or_default()
            .entry(hash)
            .or_default()
            .insert(peer.to_string());
    }

    fn remove_pending_header_block(&mut self, height: u64, hash: &[u8; 48]) {
        if let Some(blocks_at_height) = self.pending_header_blocks.get_mut(&height) {
            blocks_at_height.remove(hash);
            if blocks_at_height.is_empty() {
                self.pending_header_blocks.remove(&height);
            }
        }
    }

    fn block_source_peers_for_height(
        &self,
        height: u64,
        mut peers: BTreeSet<String>,
    ) -> BTreeSet<String> {
        for snapshot in self.connections.peer_snapshots() {
            if !snapshot.handshake_ready {
                continue;
            }
            if snapshot
                .best_height
                .is_some_and(|best_height| best_height >= height)
            {
                peers.insert(snapshot.remote_addr);
            }
        }
        peers
    }

    fn stage_header_blocks_near_tip(&mut self, node: &Node) {
        let max_height = node.height().saturating_add(BLOCK_DOWNLOAD_LOOKAHEAD);
        let heights = self
            .pending_header_blocks
            .range(..=max_height)
            .map(|(height, _)| *height)
            .collect::<Vec<_>>();
        for height in heights {
            let Some(blocks_at_height) = self.pending_header_blocks.remove(&height) else {
                continue;
            };
            for (hash, peers) in blocks_at_height {
                if node.is_canonical_block(&hash) || node.block_by_hash(hash).is_some() {
                    self.downloader.note_block_received(hash);
                    continue;
                }
                for peer in self.block_source_peers_for_height(height, peers) {
                    self.downloader.note_headers(&peer, [hash]);
                }
            }
        }
    }

    fn missing_buffered_parent_hashes(&self, node: &Node) -> Vec<[u8; 48]> {
        let mut missing = BTreeSet::new();
        for entry in self.side_branches.blocks.values() {
            let parent_hash = entry.block.header.previous_block_hash;
            if node.is_canonical_block(&parent_hash)
                || self.side_branches.get(&parent_hash).is_some()
            {
                continue;
            }
            missing.insert(parent_hash);
        }
        missing.into_iter().collect()
    }

    fn buffered_branch_from_tip(&self, node: &Node, tip_hash: [u8; 48]) -> Option<Vec<Block>> {
        let mut branch_reversed = Vec::new();
        let mut current_hash = tip_hash;
        let mut visited = BTreeSet::new();

        loop {
            if !visited.insert(current_hash) {
                return None;
            }
            let block = self.side_branches.get(&current_hash)?.clone();
            let previous_hash = block.header.previous_block_hash;
            branch_reversed.push(block);
            if node.is_canonical_block(&previous_hash) {
                break;
            }
            if self.side_branches.get(&previous_hash).is_some() {
                current_hash = previous_hash;
                continue;
            }
            return None;
        }

        branch_reversed.reverse();
        Some(branch_reversed)
    }

    fn best_buffered_branch(&self, node: &Node) -> Option<Vec<Block>> {
        let mut best: Option<Vec<Block>> = None;
        for tip_hash in self.side_branches.leaf_hashes() {
            let Some(candidate) = self.buffered_branch_from_tip(node, tip_hash) else {
                continue;
            };
            if !node.branch_is_preferred_over_current(&candidate) {
                continue;
            }
            match &best {
                None => best = Some(candidate),
                Some(current) if pow::branch_is_preferred(&candidate, current) => {
                    best = Some(candidate)
                }
                _ => {}
            }
        }
        best
    }

    fn process_buffered_branch_once(
        &mut self,
        preferred_peer: Option<&str>,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<bool, NodeSyncError> {
        if self.side_branches.is_empty() {
            return Ok(false);
        }

        let mut progressed = false;
        loop {
            let Some(candidate_branch) = self.best_buffered_branch(node) else {
                return Ok(progressed);
            };

            let candidate_hashes = candidate_branch
                .iter()
                .map(|candidate| candidate.header.block_hash())
                .collect::<Vec<_>>();
            match node.consider_branch(&candidate_branch) {
                Ok(selection) if selection.outcome != ChainSelectionOutcome::KeptCurrent => {
                    self.note_local_chain_progress(node);
                    for hash in candidate_hashes {
                        self.downloader.note_block_received(hash);
                        self.pending_compact_blocks.remove(&hash);
                        self.side_branches.remove(&hash);
                    }
                    self.side_branches.remove_canonical_blocks(node);
                    if let Some(peer) = preferred_peer {
                        self.push_block_download_work_for_peer(peer, node, outbound);
                    } else {
                        self.stage_header_blocks_near_tip(node);
                        self.push_scheduled_block_requests(outbound);
                    }
                    progressed = true;
                }
                Ok(_) => return Ok(progressed),
                Err(NodeError::Storage(StorageError::ForkPointUnavailable)) => {
                    if let Some(tip_hash) = candidate_hashes.last().copied() {
                        self.side_branches.remove(&tip_hash);
                    }
                }
                Err(err) if Self::recoverable_branch_error(&err) => return Ok(progressed),
                Err(err) => {
                    let tip = candidate_hashes
                        .last()
                        .copied()
                        .map(|hash| short_hash(&hash))
                        .unwrap_or_else(|| String::from("<empty>"));
                    let _ = dev::append_log(
                        "p2p",
                        &format!("dropping invalid side branch tip={} error={}", tip, err),
                    );
                    for hash in candidate_hashes {
                        self.downloader.note_block_received(hash);
                        self.pending_compact_blocks.remove(&hash);
                        self.side_branches.remove(&hash);
                    }
                    progressed = true;
                }
            }
        }
    }

    fn process_buffered_branches(
        &mut self,
        preferred_peer: Option<&str>,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        loop {
            if !self.process_buffered_branch_once(preferred_peer, node, outbound)? {
                return Ok(());
            }
        }
    }

    fn recoverable_tip_validation(validation: &ValidationError) -> bool {
        matches!(
            validation,
            ValidationError::InvalidBlockHeight | ValidationError::BlockParentHashMismatch
        )
    }

    fn recoverable_branch_error(error: &NodeError) -> bool {
        match error {
            NodeError::Validation(validation) => Self::recoverable_tip_validation(validation),
            NodeError::Storage(StorageError::InvalidBranchSequence) => true,
            _ => false,
        }
    }

    fn recoverable_relay_transaction_error(error: &NodeError) -> bool {
        matches!(
            error,
            NodeError::Validation(
                ValidationError::MissingUtxo
                    | ValidationError::InsufficientConfirmations
                    | ValidationError::MempoolConflict
            )
        )
    }

    fn handle_compact_block(
        &mut self,
        peer: &str,
        message: CompactBlockMessage,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        self.prune_pending_compact_blocks();
        let block_hash = message.header.block_hash();
        let requested = self.downloader.is_inflight(block_hash);
        if self.defer_unsolicited_future_block(
            peer,
            &message.header,
            node,
            outbound,
            "compact",
            requested,
        ) {
            return Ok(());
        }
        let mempool_by_short_id = node
            .mempool_transactions()
            .into_iter()
            .map(|tx| (compact_short_id(tx.txid()), tx))
            .collect::<BTreeMap<_, _>>();
        match reconstruct_compact_block(
            &message,
            |short_id| mempool_by_short_id.get(&short_id).cloned(),
            &BTreeMap::new(),
        )? {
            CompactBlockReconstruction::Complete(block) => {
                let block = finalize_compact_block_witness_refs(*block);
                self.handle_received_block(peer, block, node, outbound)?;
            }
            CompactBlockReconstruction::Missing { indexes, .. } => {
                self.note_observed_peer_tip(peer, &message.header, node);
                self.pending_compact_blocks.insert(
                    block_hash,
                    PendingCompactBlock {
                        message,
                        overrides: BTreeMap::new(),
                        received_at: SystemTime::now(),
                    },
                );
                self.prune_pending_compact_blocks();
                outbound.push(ConnectionEvent::Send {
                    peer: peer.to_string(),
                    message: NetworkMessage::new(
                        self.network,
                        MessagePayload::GetBlockTxn(GetBlockTxnMessage {
                            block_hash: Hash48::from(block_hash),
                            indexes,
                        }),
                    ),
                });
            }
        }
        Ok(())
    }

    fn serve_getblocktxn(
        &self,
        peer: &str,
        request: GetBlockTxnMessage,
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        let block_hash = request.block_hash.into_inner();
        let Some(block) = node.block_by_hash(block_hash) else {
            outbound.push(ConnectionEvent::Send {
                peer: peer.to_string(),
                message: NetworkMessage::new(
                    self.network,
                    MessagePayload::NotFound {
                        inventory: vec![InventoryVector {
                            kind: InventoryKind::Block,
                            hash: Hash48::from(block_hash),
                        }],
                    },
                ),
            });
            return;
        };

        let mut indexes = Vec::new();
        let mut transactions = Vec::new();
        for index in request.indexes {
            let Some(transaction) = block.transactions.get(index as usize).cloned() else {
                continue;
            };
            indexes.push(index);
            transactions.push(transaction);
        }
        outbound.push(ConnectionEvent::Send {
            peer: peer.to_string(),
            message: NetworkMessage::new(
                self.network,
                MessagePayload::BlockTxn(BlockTxnMessage {
                    block_hash: Hash48::from(block_hash),
                    indexes,
                    transactions,
                }),
            ),
        });
    }

    fn handle_blocktxn(
        &mut self,
        peer: &str,
        response: BlockTxnMessage,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        if response.indexes.len() != response.transactions.len() {
            return Err(NodeSyncError::Protocol(ProtocolError::InvalidCompactBlock));
        }
        self.prune_pending_compact_blocks();
        let block_hash = response.block_hash.into_inner();
        let Some(pending) = self.pending_compact_blocks.get_mut(&block_hash) else {
            return Err(NodeSyncError::Protocol(ProtocolError::UnexpectedPayload));
        };
        for (index, transaction) in response.indexes.into_iter().zip(response.transactions) {
            pending.overrides.insert(index, transaction);
        }
        let mempool_by_short_id = node
            .mempool_transactions()
            .into_iter()
            .map(|tx| (compact_short_id(tx.txid()), tx))
            .collect::<BTreeMap<_, _>>();
        match reconstruct_compact_block(
            &pending.message,
            |short_id| mempool_by_short_id.get(&short_id).cloned(),
            &pending.overrides,
        )? {
            CompactBlockReconstruction::Complete(block) => {
                self.pending_compact_blocks.remove(&block_hash);
                let block = finalize_compact_block_witness_refs(*block);
                self.handle_received_block(peer, block, node, outbound)?;
            }
            CompactBlockReconstruction::Missing { indexes, .. } => {
                outbound.push(ConnectionEvent::Send {
                    peer: peer.to_string(),
                    message: NetworkMessage::new(
                        self.network,
                        MessagePayload::GetBlockTxn(GetBlockTxnMessage {
                            block_hash: Hash48::from(block_hash),
                            indexes,
                        }),
                    ),
                });
            }
        }
        Ok(())
    }

    fn prune_pending_compact_blocks(&mut self) {
        let now = SystemTime::now();
        self.pending_compact_blocks.retain(|_, pending| {
            now.duration_since(pending.received_at).unwrap_or_default()
                < PENDING_COMPACT_BLOCK_TIMEOUT
        });
        while self.pending_compact_blocks.len() > MAX_PENDING_COMPACT_BLOCKS {
            let Some(oldest) = self
                .pending_compact_blocks
                .iter()
                .min_by_key(|(_, pending)| pending.received_at)
                .map(|(hash, _)| *hash)
            else {
                break;
            };
            self.pending_compact_blocks.remove(&oldest);
        }
    }

    fn missing_inventory_requests(
        &self,
        node: &Node,
        inventory: &[InventoryVector],
        chain_synced: bool,
    ) -> Vec<InventoryVector> {
        inventory
            .iter()
            .filter(|vector| match vector.kind {
                InventoryKind::Block => {
                    chain_synced && !node.is_canonical_block(&vector.hash.into_inner())
                }
                // During initial catch-up, ignore mempool relay entirely so a policy-only
                // transaction error cannot disconnect the sync peer or starve block download.
                InventoryKind::Transaction => {
                    chain_synced && !node.mempool_contains(&vector.hash.into_inner())
                }
            })
            .take(network_params(self.network).limits.max_requests_per_peer)
            .cloned()
            .collect()
    }

    fn chain_synced(&self, node: &Node) -> bool {
        self.sync_state().headers_synced && node.height() >= self.sync_state().best_height
    }

    fn maybe_request_mempool_snapshots(
        &mut self,
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        if !self.chain_synced(node) {
            self.mempool_snapshot_peers.clear();
            return;
        }

        for peer in self
            .connections
            .peer_snapshots()
            .into_iter()
            .filter(|snapshot| snapshot.handshake_ready)
            .map(|snapshot| snapshot.remote_addr)
        {
            if !self.mempool_snapshot_peers.insert(peer.clone()) {
                continue;
            }
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "requesting mempool snapshot peer={} local_height={} target_height={}",
                    peer,
                    node.height(),
                    self.sync_state().best_height
                ),
            );
            outbound.push(ConnectionEvent::Send {
                peer,
                message: NetworkMessage::new(self.network, MessagePayload::MemPool),
            });
        }
    }

    fn serve_getdata(
        &self,
        peer: &str,
        inventory: &[InventoryVector],
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        let mut not_found = Vec::new();
        let chain_synced = self.chain_synced(node);
        for vector in inventory
            .iter()
            .take(network_params(self.network).limits.max_requests_per_peer)
        {
            match vector.kind {
                InventoryKind::Block => {
                    if let Some(block) = node.block_by_hash(vector.hash.into_inner()) {
                        let _ = dev::append_log(
                            "p2p",
                            &format!(
                                "serving block peer={} height={} hash={}",
                                peer,
                                block.header.height,
                                short_hash(&block.header.block_hash())
                            ),
                        );
                        outbound.push(ConnectionEvent::Send {
                            peer: peer.to_string(),
                            message: NetworkMessage::new(
                                self.network,
                                MessagePayload::Block(block),
                            ),
                        });
                    } else {
                        not_found.push(vector.clone());
                    }
                }
                InventoryKind::Transaction => {
                    if !chain_synced {
                        not_found.push(vector.clone());
                        continue;
                    }
                    if let Some(transaction) = node.mempool_transaction(&vector.hash.into_inner()) {
                        outbound.push(ConnectionEvent::Send {
                            peer: peer.to_string(),
                            message: NetworkMessage::new(
                                self.network,
                                MessagePayload::Tx(transaction),
                            ),
                        });
                    } else {
                        not_found.push(vector.clone());
                    }
                }
            }
        }
        if !not_found.is_empty() {
            let missing = not_found
                .iter()
                .map(|item| short_hash(&item.hash.into_inner()))
                .collect::<Vec<_>>();
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "getdata notfound peer={} count={} hashes=[{}]",
                    peer,
                    not_found.len(),
                    missing.join(",")
                ),
            );
            outbound.push(ConnectionEvent::Send {
                peer: peer.to_string(),
                message: NetworkMessage::new(
                    self.network,
                    MessagePayload::NotFound {
                        inventory: not_found,
                    },
                ),
            });
        }
    }
}

fn finalize_compact_block_witness_refs(block: Block) -> Block {
    let witness_root = block.header.witness_root;
    let transactions = block
        .transactions
        .iter()
        .map(|tx| finalize_witness_commit_refs(tx, witness_root))
        .collect::<Vec<_>>();
    let witnesses = transactions
        .iter()
        .filter_map(|tx| tx.witness_payload().map(|witness| (tx.txid(), witness)))
        .collect();
    Block {
        transactions,
        witnesses,
        ..block
    }
}

impl Default for NodeSync {
    fn default() -> Self {
        Self::new(Network::Mainnet)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn short_hash(hash: &[u8; 48]) -> String {
    hex::encode(hash)[..12].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NodeConfig;
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use crate::test_support::acquire_global_test_lock;
    use crate::validation::{derive_sig_ref_short, derive_witness_commit_ref};
    use atho_core::block::{merkle_root, witness_root, BlockHeader};
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::consensus::tx_policy::{minimum_required_fee_atoms, solve_transaction_pow};
    use atho_core::consensus::{pow, subsidy};
    use atho_core::genesis;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};
    use atho_p2p::protocol::PeerAddress;
    use atho_storage::db::{ChainstateSnapshot, Database};
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use atho_storage::utxo::UtxoEntry;
    use std::collections::VecDeque;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug)]
    struct SandboxPeer {
        id: String,
        node: Node,
        sync: NodeSync,
    }

    #[derive(Debug)]
    struct QueuedSend {
        from: String,
        to: String,
        message: NetworkMessage,
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        _lock: crate::test_support::TestLockGuard,
    }

    impl SandboxPeer {
        fn new(id: &str, network: Network) -> Self {
            let node = Node::new(NodeConfig::new(network));
            let mut sync = NodeSync::new(network);
            sync.prime(&node);
            Self {
                id: id.to_string(),
                node,
                sync,
            }
        }

        fn new_persistent(id: &str, network: Network) -> Self {
            let node = Node::load_or_new(NodeConfig::new(network));
            let mut sync = NodeSync::new(network);
            sync.prime(&node);
            Self {
                id: id.to_string(),
                node,
                sync,
            }
        }
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let lock = acquire_global_test_lock();
            let previous = env::var_os(key);
            env::set_var(key, value);
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
                env::set_var(self.key, previous);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn temp_data_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atho-sync-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn witness_bytes_for_tx(tx: &Transaction) -> Vec<u8> {
        let keypair = generate_from_seed(b"atho-node-sync-test").expect("falcon keypair");
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &transaction_signing_digest(Network::Regnet, tx),
        )
        .expect("falcon signature")
        .0;
        let pubkey = keypair.public_key.0;
        let txid = tx.txid();
        let staged = TxWitness {
            signature: signature.clone(),
            pubkey: pubkey.clone(),
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    input_index: index as u32,
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
            additional_signers: vec![],
        };
        let staged_tx = Transaction {
            witness: staged.canonical_bytes(),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx.clone()
        };
        let witness_root = staged_tx.witness_commitment_hash();
        TxWitness {
            signature: signature.clone(),
            pubkey,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    input_index: index as u32,
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                    witness_commit_ref: derive_witness_commit_ref(
                        &txid,
                        &witness_root,
                        index as u32,
                    ),
                })
                .collect(),
            additional_signers: vec![],
        }
        .canonical_bytes()
    }

    fn coinbase_block(
        network: Network,
        height: u64,
        previous_block_hash: [u8; 48],
        target: [u8; 48],
        timestamp: u64,
    ) -> Block {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms_for_network(network, height),
                locking_script: vec![1],
            }],
            lock_time: height as u32,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let transactions = vec![coinbase];
        Block::new(
            BlockHeader {
                version: 1,
                network_id: network,
                height,
                previous_block_hash,
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp,
                difficulty_target_or_bits: target,
                nonce: 0,
            },
            transactions,
        )
    }

    fn synthetic_coinbase_block(
        network: Network,
        height: u64,
        previous_block_hash: [u8; 48],
        salt: u8,
    ) -> Block {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms_for_network(network, height),
                locking_script: vec![salt, height as u8],
            }],
            lock_time: height as u32,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let transactions = vec![coinbase];
        Block::new(
            BlockHeader {
                version: 1,
                network_id: network,
                height,
                previous_block_hash,
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 1_700_000_000 + u64::from(salt) * 10_000 + height * 75,
                difficulty_target_or_bits: pow::target_for_height(network, height),
                nonce: u64::from(salt) << 32 | height,
            },
            transactions,
        )
    }

    fn persist_synthetic_chain(network: Network, height: u64, salt: u8) -> [u8; 48] {
        let genesis = genesis::genesis_state(network).block;
        let mut blocks = vec![genesis];
        let mut previous_hash = blocks[0].header.block_hash();
        for next_height in 1..=height {
            let block = synthetic_coinbase_block(network, next_height, previous_hash, salt);
            previous_hash = block.header.block_hash();
            blocks.push(block);
        }
        let snapshot = ChainstateSnapshot {
            height,
            tip_hash: previous_hash,
            tip_header: blocks.last().map(|block| block.header.clone()),
        };
        Database::open(network)
            .expect("database")
            .replace_chainstate(&snapshot, &[], &blocks)
            .expect("replace synthetic chainstate");
        previous_hash
    }

    fn mine_reference_blocks(network: Network, count: usize) -> Vec<Block> {
        let miner = Miner::new(1);
        let mut node = Node::new(NodeConfig::new(network));
        let mut blocks = Vec::new();
        for height in 1..=count {
            blocks.push(
                node.mine_and_connect_candidate_block(&miner)
                    .unwrap_or_else(|_| panic!("mine reference block {height}")),
            );
        }
        blocks
    }

    fn mine_with_timestamp_offset(node: &mut Node, miner: &Miner, offset: u64) -> Block {
        let mut candidate = node.build_candidate_block().expect("candidate block");
        candidate.header.timestamp = candidate.header.timestamp.saturating_add(offset);
        let block = miner.solve_block(candidate);
        node.connect_block(&block).expect("connect mined block");
        block
    }

    fn signed_missing_utxo_transaction(previous_txid: [u8; 48]) -> Transaction {
        let template = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid,
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        Transaction {
            witness: witness_bytes_for_tx(&template),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..template
        }
    }

    fn sign_and_solve_transaction(
        network: Network,
        tx: Transaction,
        fee_atoms: u64,
    ) -> Transaction {
        let mut tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx
        };
        solve_transaction_pow(network, &mut tx, fee_atoms);
        tx
    }

    fn collect_events(
        queue: &mut VecDeque<QueuedSend>,
        notices: &mut Vec<SyncNotice>,
        from: &str,
        events: Vec<ConnectionEvent>,
    ) {
        for event in events {
            match event {
                ConnectionEvent::Send { peer, message } => queue.push_back(QueuedSend {
                    from: from.to_string(),
                    to: peer,
                    message,
                }),
                ConnectionEvent::Ready { peer, best_height } => {
                    notices.push(SyncNotice::Ready { peer, best_height });
                }
                ConnectionEvent::Disconnect { peer, reason } => {
                    notices.push(SyncNotice::Disconnected { peer, reason });
                }
                ConnectionEvent::Message { .. } => panic!("unexpected raw message event"),
            }
        }
    }

    fn collect_handshake_events(
        queue: &mut VecDeque<QueuedSend>,
        notices: &mut Vec<SyncNotice>,
        from: &str,
        events: Vec<ConnectionEvent>,
    ) {
        for event in events {
            match event {
                ConnectionEvent::Send { peer, message }
                    if matches!(
                        message.payload,
                        MessagePayload::Version(_) | MessagePayload::Verack
                    ) =>
                {
                    queue.push_back(QueuedSend {
                        from: from.to_string(),
                        to: peer,
                        message,
                    });
                }
                ConnectionEvent::Send { .. } => {}
                ConnectionEvent::Ready { peer, best_height } => {
                    notices.push(SyncNotice::Ready { peer, best_height });
                }
                ConnectionEvent::Disconnect { peer, reason } => {
                    notices.push(SyncNotice::Disconnected { peer, reason });
                }
                ConnectionEvent::Message { .. } => panic!("unexpected raw message event"),
            }
        }
    }

    fn outbound_getdata_hashes(events: &[ConnectionEvent]) -> Vec<[u8; 48]> {
        events
            .iter()
            .flat_map(|event| match event {
                ConnectionEvent::Send {
                    message:
                        NetworkMessage {
                            payload: MessagePayload::GetData { inventory },
                            ..
                        },
                    ..
                } => inventory
                    .iter()
                    .filter(|item| item.kind == InventoryKind::Block)
                    .map(|item| item.hash.into_inner())
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn outbound_getdata_peers(events: &[ConnectionEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ConnectionEvent::Send {
                    peer,
                    message:
                        NetworkMessage {
                            payload: MessagePayload::GetData { .. },
                            ..
                        },
                } => Some(peer.clone()),
                _ => None,
            })
            .collect()
    }

    fn synthetic_header_hash(height: u64, salt: u8) -> [u8; 48] {
        let mut hash = [salt; 48];
        hash[..8].copy_from_slice(&height.to_be_bytes());
        hash[47] = salt ^ (height as u8);
        hash
    }

    fn drain(
        left: &mut SandboxPeer,
        right: &mut SandboxPeer,
        queue: &mut VecDeque<QueuedSend>,
    ) -> Vec<SyncNotice> {
        let mut notices = Vec::new();
        while let Some(queued) = queue.pop_front() {
            if queued.to == left.id {
                let (events, mut new_notices) = left
                    .sync
                    .receive(&queued.from, queued.message, &mut left.node)
                    .expect("left receive");
                collect_events(queue, &mut notices, &left.id, events);
                notices.append(&mut new_notices);
            } else if queued.to == right.id {
                let (events, mut new_notices) = right
                    .sync
                    .receive(&queued.from, queued.message, &mut right.node)
                    .expect("right receive");
                collect_events(queue, &mut notices, &right.id, events);
                notices.append(&mut new_notices);
            } else {
                panic!("unknown peer {}", queued.to);
            }
        }
        notices
    }

    fn connect(left: &mut SandboxPeer, right: &mut SandboxPeer) -> Vec<SyncNotice> {
        left.sync.add_manual_peer(right.id.clone());
        right.sync.add_manual_peer(left.id.clone());
        right
            .sync
            .accept_inbound(left.id.clone())
            .expect("accept inbound");
        let events = left
            .sync
            .open_outbound(right.id.clone(), &left.node)
            .expect("open outbound");
        let mut queue = VecDeque::new();
        let mut notices = Vec::new();
        collect_events(&mut queue, &mut notices, &left.id, events);
        let mut drained = drain(left, right, &mut queue);
        notices.append(&mut drained);
        notices
    }

    fn connect_handshake_only(left: &mut SandboxPeer, right: &mut SandboxPeer) -> Vec<SyncNotice> {
        left.sync.add_manual_peer(right.id.clone());
        right.sync.add_manual_peer(left.id.clone());
        right
            .sync
            .accept_inbound(left.id.clone())
            .expect("accept inbound");
        let events = left
            .sync
            .open_outbound(right.id.clone(), &left.node)
            .expect("open outbound");
        let mut queue = VecDeque::new();
        let mut notices = Vec::new();
        collect_handshake_events(&mut queue, &mut notices, &left.id, events);
        while let Some(queued) = queue.pop_front() {
            if queued.to == left.id {
                let (events, mut new_notices) = left
                    .sync
                    .receive(&queued.from, queued.message, &mut left.node)
                    .expect("left receive");
                collect_handshake_events(&mut queue, &mut notices, &left.id, events);
                notices.append(&mut new_notices);
            } else if queued.to == right.id {
                let (events, mut new_notices) = right
                    .sync
                    .receive(&queued.from, queued.message, &mut right.node)
                    .expect("right receive");
                collect_handshake_events(&mut queue, &mut notices, &right.id, events);
                notices.append(&mut new_notices);
            } else {
                panic!("unknown peer {}", queued.to);
            }
        }
        notices
    }

    #[test]
    fn sandbox_nodes_complete_handshake_and_share_addresses() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        right.sync.connections.add_manual_peer("8.8.8.8:9200");

        let notices = connect(&mut left, &mut right);

        assert!(notices
            .iter()
            .any(|notice| matches!(notice, SyncNotice::Ready { peer, .. } if peer == "right")));
        assert_eq!(left.sync.connections().peer_count(), 1);
        let addresses = left
            .sync
            .connections()
            .address_manager()
            .advertisable_addresses(8);
        assert!(addresses.iter().any(|address| address.host == "8.8.8.8"));
    }

    #[test]
    fn synced_peer_periodically_relays_known_peer_addresses() {
        let mut leaf = SandboxPeer::new("leaf", Network::Regnet);
        let mut bootstrap = SandboxPeer::new("bootstrap", Network::Regnet);
        let _ = connect(&mut leaf, &mut bootstrap);

        bootstrap
            .sync
            .seed_peer_addresses(&[PeerAddress {
                host: String::from("127.0.0.42"),
                port: 18445,
                services: 0,
                last_seen_unix: 0,
            }])
            .expect("seed relay address");

        let events = bootstrap
            .sync
            .maintain_peer_sync(&leaf.id, &bootstrap.node)
            .expect("maintain peer sync");

        assert!(events.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                peer,
                message:
                    NetworkMessage {
                        payload: MessagePayload::Addr { addresses },
                        ..
                    },
            } if peer == "leaf"
                && addresses
                    .iter()
                    .any(|address| address.host == "127.0.0.42" && address.port == 18445)
        )));
    }

    #[test]
    fn ready_peer_requests_headers_before_addr_gossip() {
        let mut node = Node::new(NodeConfig::new(Network::Regnet));
        let mut sync = NodeSync::new(Network::Regnet);
        sync.prime(&node);

        let (events, notices) = sync
            .expand_events(
                vec![ConnectionEvent::Ready {
                    peer: String::from("right"),
                    best_height: 2,
                }],
                &mut node,
            )
            .expect("expand ready");

        assert_eq!(
            notices,
            vec![SyncNotice::Ready {
                peer: String::from("right"),
                best_height: 2
            }]
        );
        let payloads = events
            .into_iter()
            .map(|event| match event {
                ConnectionEvent::Send { message, .. } => message.payload,
                _ => panic!("unexpected event"),
            })
            .collect::<Vec<_>>();
        assert_eq!(payloads.len(), 1);
        assert!(matches!(payloads[0], MessagePayload::GetHeaders(_)));
    }

    #[test]
    fn addr_gossip_waits_until_headers_response() {
        let mut node = Node::new(NodeConfig::new(Network::Regnet));
        let mut sync = NodeSync::new(Network::Regnet);
        sync.prime(&node);
        let _ = sync
            .expand_events(
                vec![ConnectionEvent::Ready {
                    peer: String::from("right"),
                    best_height: 0,
                }],
                &mut node,
            )
            .expect("expand ready");

        let mut outbound = Vec::new();
        sync.handle_message(
            "right",
            NetworkMessage::new(
                Network::Regnet,
                MessagePayload::Headers {
                    headers: Vec::new(),
                },
            ),
            &mut node,
            &mut outbound,
        )
        .expect("headers response");

        assert!(outbound.into_iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message: NetworkMessage {
                    payload: MessagePayload::GetAddr,
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn headers_batch_must_start_at_the_next_known_height() {
        let mut node = Node::new(NodeConfig::new(Network::Regnet));
        let mut sync = NodeSync::new(Network::Regnet);
        sync.prime(&node);
        let _ = sync
            .expand_events(
                vec![ConnectionEvent::Ready {
                    peer: String::from("right"),
                    best_height: 10,
                }],
                &mut node,
            )
            .expect("expand ready");

        let target = pow::initial_target_for_network(Network::Regnet);
        let jumped = coinbase_block(Network::Regnet, 5, node.tip_hash(), target, 1_700_000_005);
        let mut outbound = Vec::new();

        let err = sync
            .handle_message(
                "right",
                NetworkMessage::new(
                    Network::Regnet,
                    MessagePayload::Headers {
                        headers: vec![jumped.header.clone()],
                    },
                ),
                &mut node,
                &mut outbound,
            )
            .expect_err("jumped headers must be rejected");

        assert!(matches!(
            err,
            NodeSyncError::Protocol(ProtocolError::InvalidHeadersSequence)
        ));
    }

    #[test]
    fn repeated_post_handshake_protocol_errors_ban_peer() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let target = pow::initial_target_for_network(Network::Regnet);
        let jumped = coinbase_block(
            Network::Regnet,
            5,
            left.node.tip_hash(),
            target,
            1_700_000_005,
        )
        .header;

        for _ in 0..2 {
            let err = left
                .sync
                .receive(
                    &right.id,
                    NetworkMessage::new(
                        Network::Regnet,
                        MessagePayload::Headers {
                            headers: vec![jumped.clone()],
                        },
                    ),
                    &mut left.node,
                )
                .expect_err("invalid headers should score peer");
            assert!(matches!(
                err,
                NodeSyncError::Protocol(ProtocolError::InvalidHeadersSequence)
            ));
        }

        let err = left
            .sync
            .receive(
                &right.id,
                NetworkMessage::new(Network::Regnet, MessagePayload::Ping { nonce: 1 }),
                &mut left.node,
            )
            .expect_err("repeat offender should be banned");
        assert!(matches!(
            err,
            NodeSyncError::Connection(atho_p2p::connection::ConnectionError::BannedPeer)
        ));
    }

    #[test]
    fn buffered_out_of_order_block_frees_download_slot() {
        let mut node = Node::new(NodeConfig::new(Network::Regnet));
        let mut sync = NodeSync::new(Network::Regnet);
        sync.prime(&node);
        sync.downloader.note_peer_ready("right");

        let target = pow::initial_target_for_network(Network::Regnet);
        let miner = Miner::new(1);
        let mut previous_hash = node.tip_hash();
        let mut blocks = Vec::new();
        let request_limit = network_params(Network::Regnet).limits.max_requests_per_peer;
        for height in 1..=(request_limit as u64 + 1) {
            let mut block = coinbase_block(
                Network::Regnet,
                height,
                previous_hash,
                target,
                1_700_000_000 + height,
            );
            if height == request_limit as u64 {
                block = miner.solve_block(block);
            }
            previous_hash = block.header.block_hash();
            blocks.push(block);
        }
        sync.downloader.note_headers(
            "right",
            blocks.iter().map(|block| block.header.block_hash()),
        );

        let mut outbound = Vec::new();
        sync.push_scheduled_block_requests(&mut outbound);
        let requested_before = outbound
            .iter()
            .map(|event| match event {
                ConnectionEvent::Send {
                    message:
                        NetworkMessage {
                            payload: MessagePayload::GetData { inventory },
                            ..
                        },
                    ..
                } => inventory.len(),
                _ => 0,
            })
            .sum::<usize>();
        assert_eq!(requested_before, request_limit);

        outbound.clear();
        sync.handle_received_block(
            "right",
            blocks[request_limit - 1].clone(),
            &mut node,
            &mut outbound,
        )
        .expect("buffer future block");

        assert_eq!(node.height(), 0);
        let requested_after = outbound
            .iter()
            .map(|event| match event {
                ConnectionEvent::Send {
                    message:
                        NetworkMessage {
                            payload: MessagePayload::GetData { inventory },
                            ..
                        },
                    ..
                } => inventory.len(),
                _ => 0,
            })
            .sum::<usize>();
        assert_eq!(requested_after, 1);
    }

    #[test]
    fn block_refill_stays_on_current_peer_socket() {
        let mut node = Node::new(NodeConfig::new(Network::Regnet));
        let mut sync = NodeSync::new(Network::Regnet);
        sync.prime(&node);
        sync.downloader.note_peer_ready("peer-a");
        sync.downloader.note_peer_ready("peer-b");

        let miner = Miner::new(1);
        let first_block = miner.solve_block(node.build_candidate_block().expect("candidate"));
        let request_limit = network_params(Network::Regnet).limits.max_requests_per_peer;
        let mut hashes = vec![first_block.header.block_hash()];
        for index in 0..=request_limit {
            let mut hash = [0u8; 48];
            hash[0..8].copy_from_slice(&(index as u64 + 100).to_be_bytes());
            hashes.push(hash);
        }
        sync.downloader
            .note_headers("peer-a", hashes.iter().copied());
        sync.downloader
            .note_headers("peer-b", hashes.iter().copied());

        let mut outbound = Vec::new();
        sync.push_block_download_work_for_peer("peer-a", &node, &mut outbound);
        assert_eq!(
            outbound_getdata_peers(&outbound),
            vec![String::from("peer-a")]
        );

        outbound.clear();
        sync.handle_received_block("peer-a", first_block, &mut node, &mut outbound)
            .expect("accepted first block refills peer-a");

        assert_eq!(node.height(), 1);
        assert_eq!(
            outbound_getdata_peers(&outbound),
            vec![String::from("peer-a")],
            "a peer receive path must not assign block requests to another peer thread"
        );
    }

    #[test]
    fn headers_stage_only_near_tip_blocks_in_small_batches() {
        let mut node = Node::new(NodeConfig::new(Network::Regnet));
        let mut sync = NodeSync::new(Network::Regnet);
        sync.prime(&node);
        let _ = sync
            .expand_events(
                vec![ConnectionEvent::Ready {
                    peer: String::from("right"),
                    best_height: BLOCK_DOWNLOAD_LOOKAHEAD + 8,
                }],
                &mut node,
            )
            .expect("ready peer");

        let hashes = (1..=BLOCK_DOWNLOAD_LOOKAHEAD + 8)
            .map(|height| synthetic_header_hash(height, 0x91))
            .collect::<Vec<_>>();
        for (height, hash) in (1..=BLOCK_DOWNLOAD_LOOKAHEAD + 8).zip(hashes.iter().copied()) {
            sync.note_pending_header_block("right", height, hash);
        }

        let mut outbound = Vec::new();
        sync.push_block_download_work_for_peer("right", &node, &mut outbound);

        let requested = outbound_getdata_hashes(&outbound);
        assert_eq!(requested.len(), BLOCK_REQUEST_BATCH_LIMIT);
        assert_eq!(
            requested,
            hashes
                .into_iter()
                .take(BLOCK_REQUEST_BATCH_LIMIT)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            sync.pending_header_blocks.keys().next().copied(),
            Some(BLOCK_DOWNLOAD_LOOKAHEAD + 1)
        );
    }

    #[test]
    fn headers_from_one_peer_keep_other_ready_peer_pipeline_full() {
        let mut local = SandboxPeer::new("local", Network::Regnet);
        let mut header_peer = SandboxPeer::new("header-peer", Network::Regnet);
        let mut fast_peer = SandboxPeer::new("fast-peer", Network::Regnet);
        header_peer
            .node
            .dev_seed_chainstate(512, [11; 48], Vec::<UtxoEntry>::new())
            .expect("header peer height");
        fast_peer
            .node
            .dev_seed_chainstate(512, [22; 48], Vec::<UtxoEntry>::new())
            .expect("fast peer height");
        let _ = connect_handshake_only(&mut local, &mut header_peer);
        let _ = connect_handshake_only(&mut local, &mut fast_peer);

        let hashes = (1..=BLOCK_DOWNLOAD_LOOKAHEAD + 16)
            .map(|height| synthetic_header_hash(height, 0xA7))
            .collect::<Vec<_>>();
        for (height, hash) in (1..=BLOCK_DOWNLOAD_LOOKAHEAD + 16).zip(hashes) {
            local
                .sync
                .note_pending_header_block(&header_peer.id, height, hash);
        }

        let mut outbound = Vec::new();
        local
            .sync
            .push_block_download_work_for_peer(&header_peer.id, &local.node, &mut outbound);
        assert_eq!(
            outbound_getdata_peers(&outbound),
            vec![header_peer.id.clone()],
            "the header sender still gets the first pipeline refill on its own socket"
        );

        outbound.clear();
        local
            .sync
            .push_block_download_work_for_peer(&fast_peer.id, &local.node, &mut outbound);
        let requested = outbound_getdata_hashes(&outbound);
        assert_eq!(requested.len(), BLOCK_REQUEST_BATCH_LIMIT);
        assert_eq!(
            outbound_getdata_peers(&outbound),
            vec![fast_peer.id.clone()],
            "a ready peer that claims the advertised height must be able to keep its own block window full"
        );
    }

    #[test]
    fn low_peer_stale_block_request_retries_without_disconnect() {
        let mut local = SandboxPeer::new("local", Network::Regnet);
        let mut only_peer = SandboxPeer::new("only-peer", Network::Regnet);
        only_peer
            .node
            .dev_seed_chainstate(64, [64; 48], Vec::<UtxoEntry>::new())
            .expect("peer height");
        let _ = connect_handshake_only(&mut local, &mut only_peer);

        let wanted_hash = [7; 48];
        local
            .sync
            .downloader
            .note_headers(&only_peer.id, [wanted_hash]);
        let mut outbound = Vec::new();
        local
            .sync
            .push_block_download_work_for_peer(&only_peer.id, &local.node, &mut outbound);
        assert_eq!(outbound_getdata_hashes(&outbound), vec![wanted_hash]);

        local.sync.downloader.backdate_inflight_for_peer(
            &only_peer.id,
            BLOCK_REQUEST_RETRY_TIMEOUT + Duration::from_secs(1),
        );
        let retry = local
            .sync
            .maintain_peer_sync(&only_peer.id, &local.node)
            .expect("maintenance retry");

        assert!(
            !retry
                .iter()
                .any(|event| matches!(event, ConnectionEvent::Disconnect { .. })),
            "low-peer mode must not tear down the only useful peer for a stale block response"
        );
        assert_eq!(outbound_getdata_hashes(&retry), vec![wanted_hash]);
        assert!(local.sync.downloader.is_inflight(wanted_hash));
    }

    #[test]
    fn future_block_observation_advances_sync_target_and_rehydrates_headers() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let target = pow::initial_target_for_network(Network::Regnet);
        let block_1 = coinbase_block(
            Network::Regnet,
            1,
            left.node.tip_hash(),
            target,
            1_700_000_001,
        );
        let block_2 = Miner::new(1).solve_block(coinbase_block(
            Network::Regnet,
            2,
            block_1.header.block_hash(),
            target,
            1_700_000_002,
        ));

        let mut outbound = Vec::new();
        left.sync
            .handle_received_block(&right.id, block_2, &mut left.node, &mut outbound)
            .expect("buffer future tip");

        assert_eq!(left.node.height(), 0);
        assert_eq!(left.sync.sync_state().best_height, 2);
        assert!(!left.sync.sync_state().headers_synced);
        assert_eq!(
            left.sync.connections().remote_best_height(&right.id),
            Some(2)
        );

        let maintenance = left
            .sync
            .maintain_peer_sync(&right.id, &left.node)
            .expect("maintain sync");
        assert!(maintenance.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message: NetworkMessage {
                    payload: MessagePayload::GetHeaders(_),
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn sandbox_addr_gossip_persists_peer_records_and_seed_graph() {
        let root = temp_data_dir("peer-gossip");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut left = SandboxPeer::new_persistent("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let gossip = PeerAddress {
            host: String::from("9.9.9.9"),
            port: 9200,
            services: 0,
            last_seen_unix: 1_700_000_123,
        };
        let (events, notices) = left
            .sync
            .receive(
                &right.id,
                NetworkMessage::new(
                    Network::Regnet,
                    MessagePayload::Addr {
                        addresses: vec![gossip],
                    },
                ),
                &mut left.node,
            )
            .expect("addr gossip");

        assert!(events.is_empty());
        assert!(notices.is_empty());
        let stored = left
            .node
            .load_peer_record("9.9.9.9:9200")
            .expect("load peer record")
            .expect("peer record present");
        assert_eq!(stored.remote_addr, "9.9.9.9:9200");
        assert!(stored.last_seen_unix > 0);
        let addresses = left
            .sync
            .connections()
            .address_manager()
            .advertisable_addresses(8);
        assert!(addresses
            .iter()
            .any(|address| address.host == "9.9.9.9" && address.port == 9200));
    }

    #[test]
    fn addr_messages_are_rate_limited_per_peer() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);
        let limit = network_params(Network::Regnet)
            .limits
            .max_addr_messages_per_window;

        for index in 0..limit {
            let (events, notices) = left
                .sync
                .receive(
                    &right.id,
                    NetworkMessage::new(
                        Network::Regnet,
                        MessagePayload::Addr {
                            addresses: vec![PeerAddress {
                                host: format!("127.0.0.{}", index + 2),
                                port: 18445,
                                services: 0,
                                last_seen_unix: 1_700_000_000 + u64::from(index),
                            }],
                        },
                    ),
                    &mut left.node,
                )
                .expect("addr gossip");
            assert!(events.is_empty());
            assert!(notices.is_empty());
        }

        let (events, notices) = left
            .sync
            .receive(
                &right.id,
                NetworkMessage::new(
                    Network::Regnet,
                    MessagePayload::Addr {
                        addresses: vec![PeerAddress {
                            host: String::from("127.0.0.250"),
                            port: 18445,
                            services: 0,
                            last_seen_unix: 1_700_000_999,
                        }],
                    },
                ),
                &mut left.node,
            )
            .expect("rate-limited addr gossip");

        assert!(events.is_empty());
        assert!(notices.is_empty());
        assert!(
            left.sync.connections().ban_score(&right.id) >= ADDR_SPAM_MISBEHAVIOR_SCORE,
            "addr spam should penalize the peer"
        );
    }

    #[test]
    fn sandbox_headers_first_sync_downloads_missing_blocks() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let miner = Miner::new(1);
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine first");
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine second");
        right.sync.prime(&right.node);

        let _ = connect(&mut left, &mut right);

        assert_eq!(left.node.height(), right.node.height());
        assert_eq!(left.node.tip_hash(), right.node.tip_hash());
        assert_eq!(left.sync.sync_state().best_height, right.node.height());
    }

    #[test]
    fn disconnecting_last_ready_peer_preserves_observed_target_while_local_is_behind() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let miner = Miner::new(1);
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine first");
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine second");
        right.sync.prime(&right.node);

        right
            .sync
            .accept_inbound(left.id.clone())
            .expect("accept inbound");

        let mut queue = VecDeque::new();
        let mut notices = Vec::new();
        let events = left
            .sync
            .open_outbound(right.id.clone(), &left.node)
            .expect("open outbound");
        collect_events(&mut queue, &mut notices, &left.id, events);

        let left_version = queue.pop_front().expect("left version");
        let (events, mut new_notices) = right
            .sync
            .receive(&left_version.from, left_version.message, &mut right.node)
            .expect("right receives version");
        collect_events(&mut queue, &mut notices, &right.id, events);
        notices.append(&mut new_notices);

        for _ in 0..2 {
            let queued = queue.pop_front().expect("right handshake reply");
            let (events, mut new_notices) = left
                .sync
                .receive(&queued.from, queued.message, &mut left.node)
                .expect("left receives handshake reply");
            collect_events(&mut queue, &mut notices, &left.id, events);
            notices.append(&mut new_notices);
        }

        assert_eq!(left.node.height(), 0);
        assert_eq!(left.sync.sync_state().best_height, right.node.height());
        assert!(!left.sync.sync_state().headers_synced);

        let notice = left
            .sync
            .disconnect_peer(&right.id, String::from("peer dropped"), &left.node)
            .expect("disconnect notice");
        assert!(matches!(
            notice,
            SyncNotice::Disconnected { peer, .. } if peer == right.id
        ));
        assert_eq!(left.sync.sync_state().best_height, right.node.height());
        assert!(!left.sync.sync_state().headers_synced);
    }

    #[test]
    fn stalled_headers_request_disconnects_peer_instead_of_spinning() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        right
            .node
            .dev_seed_chainstate(2, [2; 48], Vec::<UtxoEntry>::new())
            .expect("seed remote height");
        right.sync.prime(&right.node);

        right
            .sync
            .accept_inbound(left.id.clone())
            .expect("accept inbound");

        let mut queue = VecDeque::new();
        let mut notices = Vec::new();
        let events = left
            .sync
            .open_outbound(right.id.clone(), &left.node)
            .expect("open outbound");
        collect_events(&mut queue, &mut notices, &left.id, events);

        let left_version = queue.pop_front().expect("left version");
        let (events, mut new_notices) = right
            .sync
            .receive(&left_version.from, left_version.message, &mut right.node)
            .expect("right receives version");
        collect_events(&mut queue, &mut notices, &right.id, events);
        notices.append(&mut new_notices);

        for _ in 0..2 {
            let queued = queue.pop_front().expect("right handshake reply");
            let (events, mut new_notices) = left
                .sync
                .receive(&queued.from, queued.message, &mut left.node)
                .expect("left receives handshake reply");
            collect_events(&mut queue, &mut notices, &left.id, events);
            notices.append(&mut new_notices);
        }

        assert!(left.sync.header_requests_started.contains_key(&right.id));
        left.sync.header_requests_started.insert(
            right.id.clone(),
            SystemTime::now() - HEADERS_REQUEST_TIMEOUT - Duration::from_secs(1),
        );

        let events = left
            .sync
            .maintain_peer_sync(&right.id, &left.node)
            .expect("maintenance");
        assert!(events.iter().any(|event| matches!(
            event,
            ConnectionEvent::Disconnect { peer, reason }
                if peer == &right.id && reason.contains("headers download timeout")
        )));
    }

    #[test]
    fn sandbox_block_inventory_relays_new_blocks() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let miner = Miner::new(1);
        let block = right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine");
        right.sync.prime(&right.node);

        let mut queue = VecDeque::from([QueuedSend {
            from: right.id.clone(),
            to: left.id.clone(),
            message: right.sync.relay_block_message(&block),
        }]);
        let _ = drain(&mut left, &mut right, &mut queue);

        assert_eq!(left.node.height(), right.node.height());
        assert_eq!(left.node.tip_hash(), right.node.tip_hash());
    }

    #[test]
    fn block_inventory_does_not_trigger_body_requests_while_chain_sync_is_incomplete() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let miner = Miner::new(1);
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine first");
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine second");
        right.sync.prime(&right.node);

        right
            .sync
            .accept_inbound(left.id.clone())
            .expect("accept inbound");

        let mut queue = VecDeque::new();
        let mut notices = Vec::new();
        let events = left
            .sync
            .open_outbound(right.id.clone(), &left.node)
            .expect("open outbound");
        collect_events(&mut queue, &mut notices, &left.id, events);

        let left_version = queue.pop_front().expect("left version");
        let (events, mut new_notices) = right
            .sync
            .receive(&left_version.from, left_version.message, &mut right.node)
            .expect("right receives version");
        collect_events(&mut queue, &mut notices, &right.id, events);
        notices.append(&mut new_notices);

        for _ in 0..2 {
            let queued = queue.pop_front().expect("right handshake reply");
            let (events, mut new_notices) = left
                .sync
                .receive(&queued.from, queued.message, &mut left.node)
                .expect("left receives handshake reply");
            collect_events(&mut queue, &mut notices, &left.id, events);
            notices.append(&mut new_notices);
        }

        assert!(!left.sync.sync_state().headers_synced);
        let block = right.node.blocks().last().cloned().expect("tip block");
        let (events, _) = left
            .sync
            .receive(
                &right.id,
                right.sync.relay_block_message(&block),
                &mut left.node,
            )
            .expect("left receives inv");

        assert!(
            !events.iter().any(|event| matches!(
                event,
                ConnectionEvent::Send {
                    message: NetworkMessage {
                        payload: MessagePayload::GetData { .. },
                        ..
                    },
                    ..
                }
            )),
            "block inventory should not bypass headers-first sync"
        );
    }

    #[test]
    fn synced_nodes_pull_peer_mempool_snapshots_after_handshake() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);

        let seed_txid = [7; 48];
        let seed_value = 2_000u64;
        let seed_script = vec![1];
        for peer in [&mut left, &mut right] {
            peer.node
                .dev_seed_chainstate(
                    6,
                    peer.node.tip_hash(),
                    [UtxoEntry::new(
                        Network::Regnet,
                        seed_txid,
                        0,
                        seed_value,
                        seed_script.clone(),
                        0,
                        false,
                    )],
                )
                .expect("seed utxo");
            peer.sync.prime(&peer.node);
        }

        let template = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: seed_txid,
                output_index: 0,
                unlocking_script: seed_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let provisional = Transaction {
            witness: witness_bytes_for_tx(&template),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..template
        };
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &provisional);
        let signed = Transaction {
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(fee_atoms),
                locking_script: vec![2],
            }],
            ..Transaction {
                witness: vec![],
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
                ..provisional
            }
        };
        let signed = sign_and_solve_transaction(Network::Regnet, signed, fee_atoms);
        let txid = right
            .node
            .submit_transaction(MempoolEntry::new(signed, fee_atoms))
            .expect("submit tx");

        let _ = connect(&mut left, &mut right);

        assert!(left.node.mempool_contains(&txid));
    }

    #[test]
    fn relayed_transactions_are_ignored_while_chain_sync_is_incomplete() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let miner = Miner::new(1);
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine first");
        right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine second");
        right.sync.prime(&right.node);

        right
            .sync
            .accept_inbound(left.id.clone())
            .expect("accept inbound");

        let mut queue = VecDeque::new();
        let mut notices = Vec::new();
        let events = left
            .sync
            .open_outbound(right.id.clone(), &left.node)
            .expect("open outbound");
        collect_events(&mut queue, &mut notices, &left.id, events);

        let left_version = queue.pop_front().expect("left version");
        let (events, mut new_notices) = right
            .sync
            .receive(&left_version.from, left_version.message, &mut right.node)
            .expect("right receives version");
        collect_events(&mut queue, &mut notices, &right.id, events);
        notices.append(&mut new_notices);

        for _ in 0..2 {
            let queued = queue.pop_front().expect("right handshake reply");
            let (events, mut new_notices) = left
                .sync
                .receive(&queued.from, queued.message, &mut left.node)
                .expect("left receives handshake reply");
            collect_events(&mut queue, &mut notices, &left.id, events);
            notices.append(&mut new_notices);
        }

        assert!(!left.sync.sync_state().headers_synced);

        let seed_txid = [7; 48];
        let seed_value = 2_000u64;
        let seed_script = vec![1];
        right
            .node
            .dev_seed_chainstate(
                6,
                right.node.tip_hash(),
                [UtxoEntry::new(
                    Network::Regnet,
                    seed_txid,
                    0,
                    seed_value,
                    seed_script.clone(),
                    0,
                    false,
                )],
            )
            .expect("seed utxo");

        let template = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: seed_txid,
                output_index: 0,
                unlocking_script: seed_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let provisional = Transaction {
            witness: witness_bytes_for_tx(&template),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..template
        };
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &provisional);
        let signed = Transaction {
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(fee_atoms),
                locking_script: vec![2],
            }],
            ..Transaction {
                witness: vec![],
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
                ..provisional
            }
        };
        let signed = sign_and_solve_transaction(Network::Regnet, signed, fee_atoms);
        let txid = signed.txid();

        let (events, new_notices) = left
            .sync
            .receive(
                &right.id,
                NetworkMessage::new(Network::Regnet, MessagePayload::Tx(signed)),
                &mut left.node,
            )
            .expect("ignore relayed tx while behind");

        assert!(events.is_empty());
        assert!(new_notices.is_empty());
        assert!(!left.node.mempool_contains(&txid));
    }

    #[test]
    fn mempool_snapshot_requests_are_ignored_while_chain_sync_is_incomplete() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let (seed_txid, seed_value, seed_script) = crate::dev::seed_utxo(Network::Regnet);
        left.node
            .dev_seed_chainstate(
                6,
                left.node.tip_hash(),
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
            .expect("seed utxo");
        let transaction = crate::dev::signed_spend_transaction(
            Network::Regnet,
            seed_txid,
            seed_value,
            seed_script,
        )
        .expect("signed transaction");
        let txid = transaction.txid();
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &transaction);
        left.node
            .submit_transaction(MempoolEntry::new(transaction, fee_atoms))
            .expect("submit tx");
        left.sync.prime(&left.node);

        let future_header = coinbase_block(
            Network::Regnet,
            left.node.height() + 2,
            left.node.tip_hash(),
            pow::initial_target_for_network(Network::Regnet),
            1_700_001_000,
        )
        .header;
        left.sync
            .note_observed_peer_tip(&right.id, &future_header, &left.node);
        assert!(!left.sync.chain_synced(&left.node));
        assert!(left.node.mempool_contains(&txid));

        let (events, notices) = left
            .sync
            .receive(
                &right.id,
                NetworkMessage::new(Network::Regnet, MessagePayload::MemPool),
                &mut left.node,
            )
            .expect("mempool request while behind should be ignored");

        assert!(events.is_empty());
        assert!(notices.is_empty());
    }

    #[test]
    fn getdata_transaction_requests_return_notfound_while_chain_sync_is_incomplete() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let (seed_txid, seed_value, seed_script) = crate::dev::seed_utxo(Network::Regnet);
        peer.node
            .dev_seed_chainstate(
                6,
                peer.node.tip_hash(),
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
            .expect("seed utxo");
        let transaction = crate::dev::signed_spend_transaction(
            Network::Regnet,
            seed_txid,
            seed_value,
            seed_script,
        )
        .expect("signed transaction");
        let txid = transaction.txid();
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &transaction);
        peer.node
            .submit_transaction(MempoolEntry::new(transaction, fee_atoms))
            .expect("submit tx");

        let future_header = coinbase_block(
            Network::Regnet,
            peer.node.height() + 2,
            peer.node.tip_hash(),
            pow::initial_target_for_network(Network::Regnet),
            1_700_000_999,
        )
        .header;
        peer.sync
            .note_observed_peer_tip("remote", &future_header, &peer.node);
        assert!(!peer.sync.chain_synced(&peer.node));

        let mut outbound = Vec::new();
        peer.sync.serve_getdata(
            "remote",
            &[InventoryVector {
                kind: InventoryKind::Transaction,
                hash: Hash48::from(txid),
            }],
            &peer.node,
            &mut outbound,
        );

        assert!(outbound.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message:
                    NetworkMessage {
                        payload: MessagePayload::NotFound { inventory },
                        ..
                    },
                ..
            } if inventory == &[InventoryVector {
                kind: InventoryKind::Transaction,
                hash: Hash48::from(txid),
            }]
        )));
        assert!(!outbound.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message: NetworkMessage {
                    payload: MessagePayload::Tx(_),
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn sandbox_transaction_inventory_relays_to_mempool() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let seed_txid = [7; 48];
        let seed_value = 2_000u64;
        let seed_script = vec![1];
        for peer in [&mut left, &mut right] {
            peer.node
                .dev_seed_chainstate(
                    6,
                    peer.node.tip_hash(),
                    [UtxoEntry::new(
                        Network::Regnet,
                        seed_txid,
                        0,
                        seed_value,
                        seed_script.clone(),
                        0,
                        false,
                    )],
                )
                .expect("seed utxo");
            peer.sync.prime(&peer.node);
        }

        let template = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: seed_txid,
                output_index: 0,
                unlocking_script: seed_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let provisional = Transaction {
            witness: witness_bytes_for_tx(&template),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..template
        };
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &provisional);
        let signed = Transaction {
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(fee_atoms),
                locking_script: vec![2],
            }],
            ..Transaction {
                witness: vec![],
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
                ..provisional
            }
        };
        let signed = sign_and_solve_transaction(Network::Regnet, signed, fee_atoms);
        let txid = right
            .node
            .submit_transaction(MempoolEntry::new(signed.clone(), fee_atoms))
            .expect("submit tx");

        let mut queue = VecDeque::from([QueuedSend {
            from: right.id.clone(),
            to: left.id.clone(),
            message: right.sync.relay_transaction_message(txid),
        }]);
        let _ = drain(&mut left, &mut right, &mut queue);

        assert!(left.node.mempool_contains(&txid));
    }

    #[test]
    fn future_chain_relay_tx_does_not_disconnect_or_drop_buffered_blocks() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let blocks = mine_reference_blocks(Network::Regnet, 3);

        let mut outbound = Vec::new();
        peer.sync
            .handle_received_block("remote", blocks[2].clone(), &mut peer.node, &mut outbound)
            .expect("buffer future tip");
        assert_eq!(peer.node.height(), 0);
        assert_eq!(Some(peer.sync.side_branches.len()), Some(1));

        let tx = signed_missing_utxo_transaction([8; 48]);
        let txid = tx.txid();
        peer.sync
            .handle_message(
                "remote",
                NetworkMessage::new(Network::Regnet, MessagePayload::Tx(tx)),
                &mut peer.node,
                &mut outbound,
            )
            .expect("future-chain tx is ignored until its inputs exist locally");

        assert!(!peer.node.mempool_contains(&txid));
        assert_eq!(Some(peer.sync.side_branches.len()), Some(1));

        peer.sync
            .handle_received_block("remote", blocks[1].clone(), &mut peer.node, &mut outbound)
            .expect("buffer parent block");
        peer.sync
            .handle_received_block("remote", blocks[0].clone(), &mut peer.node, &mut outbound)
            .expect("rebuild buffered branch");

        assert_eq!(peer.node.height(), 3);
        assert_eq!(peer.node.tip_hash(), blocks[2].header.block_hash());
        assert!(peer.sync.side_branches.is_empty());
    }

    #[test]
    fn invalid_pre_handshake_message_disconnects_peer() {
        let left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        right
            .sync
            .accept_inbound(left.id.clone())
            .expect("accept inbound");

        let (events, notices) = right
            .sync
            .receive(
                &left.id,
                NetworkMessage::new(Network::Regnet, MessagePayload::Ping { nonce: 7 }),
                &mut right.node,
            )
            .expect("receive");
        assert!(events.is_empty());
        assert!(notices.iter().any(|notice| matches!(
            notice,
            SyncNotice::Disconnected { peer, .. } if peer == "left"
        )));
    }

    #[test]
    fn sandbox_compact_block_recovers_missing_transactions() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let seed_txid = [5; 48];
        let seed_value = 2_000u64;
        let seed_script = vec![1];
        for peer in [&mut left, &mut right] {
            peer.node
                .dev_seed_chainstate(
                    6,
                    peer.node.tip_hash(),
                    [UtxoEntry::new(
                        Network::Regnet,
                        seed_txid,
                        0,
                        seed_value,
                        seed_script.clone(),
                        0,
                        false,
                    )],
                )
                .expect("seed utxo");
            peer.sync.prime(&peer.node);
        }

        let template = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: seed_txid,
                output_index: 0,
                unlocking_script: seed_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let provisional = Transaction {
            witness: witness_bytes_for_tx(&template),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..template
        };
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &provisional);
        let signed = Transaction {
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(fee_atoms),
                locking_script: vec![2],
            }],
            ..Transaction {
                witness: vec![],
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
                ..provisional
            }
        };
        let signed = sign_and_solve_transaction(Network::Regnet, signed, fee_atoms);
        right
            .node
            .submit_transaction(MempoolEntry::new(signed.clone(), fee_atoms))
            .expect("submit tx");

        let miner = Miner::new(1);
        let block = right
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine compact block");
        right.sync.prime(&right.node);

        let mut queue = VecDeque::from([QueuedSend {
            from: right.id.clone(),
            to: left.id.clone(),
            message: right.sync.relay_compact_block_message(&block),
        }]);
        let _ = drain(&mut left, &mut right, &mut queue);

        assert_eq!(left.node.height(), right.node.height());
        assert_eq!(left.node.tip_hash(), right.node.tip_hash());
        assert!(left.node.block_by_hash(block.header.block_hash()).is_some());
    }

    #[test]
    fn compact_future_header_advances_sync_target_before_body_reconstruction() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let target = pow::initial_target_for_network(Network::Regnet);
        let block_1 = coinbase_block(
            Network::Regnet,
            1,
            left.node.tip_hash(),
            target,
            1_700_000_001,
        );
        let mut block_2 = coinbase_block(
            Network::Regnet,
            2,
            block_1.header.block_hash(),
            target,
            1_700_000_002,
        );
        block_2.transactions.push(Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [4; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: Vec::new(),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        });

        let mut outbound = Vec::new();
        left.sync
            .handle_compact_block(
                &right.id,
                compact_block_from_block(&block_2),
                &mut left.node,
                &mut outbound,
            )
            .expect("observe compact future tip");

        assert_eq!(left.node.height(), 0);
        assert_eq!(left.sync.sync_state().best_height, 2);
        assert!(!left.sync.sync_state().headers_synced);
        assert_eq!(
            left.sync.connections().remote_best_height(&right.id),
            Some(2)
        );
        assert!(left
            .sync
            .pending_compact_blocks
            .contains_key(&block_2.header.block_hash()));
        assert!(outbound.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message: NetworkMessage {
                    payload: MessagePayload::GetBlockTxn(_),
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn far_ahead_compact_block_is_header_signal_not_orphan_work() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let far_height = BLOCK_DOWNLOAD_LOOKAHEAD + 32;
        let block = coinbase_block(
            Network::Regnet,
            far_height,
            [42; 48],
            pow::target_for_height(Network::Regnet, far_height),
            1_700_000_000 + far_height,
        );

        let mut outbound = Vec::new();
        left.sync
            .handle_compact_block(
                &right.id,
                compact_block_from_block(&block),
                &mut left.node,
                &mut outbound,
            )
            .expect("defer far-ahead compact block");

        assert_eq!(left.node.height(), 0);
        assert_eq!(left.sync.sync_state().best_height, far_height);
        assert!(!left.sync.sync_state().headers_synced);
        assert!(left.sync.side_branches.is_empty());
        assert!(left.sync.pending_compact_blocks.is_empty());
        assert!(outbound_getdata_hashes(&outbound).is_empty());
        assert!(outbound.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message: NetworkMessage {
                    payload: MessagePayload::GetHeaders(_),
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn far_ahead_unsolicited_block_is_not_buffered_as_orphan() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);

        let far_height = BLOCK_DOWNLOAD_LOOKAHEAD + 64;
        let block = Miner::new(1).solve_block(coinbase_block(
            Network::Regnet,
            far_height,
            [43; 48],
            pow::target_for_height(Network::Regnet, far_height),
            1_700_000_000 + far_height,
        ));

        let mut outbound = Vec::new();
        left.sync
            .handle_received_block(&right.id, block, &mut left.node, &mut outbound)
            .expect("defer far-ahead full block");

        assert_eq!(left.node.height(), 0);
        assert_eq!(left.sync.sync_state().best_height, far_height);
        assert!(!left.sync.sync_state().headers_synced);
        assert!(left.sync.side_branches.is_empty());
        assert!(outbound_getdata_hashes(&outbound).is_empty());
        assert!(outbound.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message: NetworkMessage {
                    payload: MessagePayload::GetHeaders(_),
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn pending_compact_blocks_are_bounded_and_stale_entries_are_pruned() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);
        let target = pow::initial_target_for_network(Network::Regnet);
        let now = SystemTime::now();

        for index in 0..(MAX_PENDING_COMPACT_BLOCKS + 8) {
            let height = index as u64 + 1;
            let mut block = coinbase_block(
                Network::Regnet,
                height,
                [index as u8; 48],
                target,
                1_700_100_000 + height,
            );
            block.transactions.push(Transaction {
                version: 1,
                inputs: vec![TxInput {
                    previous_txid: [height as u8; 48],
                    output_index: 0,
                    unlocking_script: vec![1],
                }],
                outputs: vec![TxOutput {
                    value_atoms: 1_000,
                    locking_script: vec![2],
                }],
                lock_time: height as u32,
                witness: Vec::new(),
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
            });
            block.header.merkle_root = merkle_root(&block.transactions);
            block.header.witness_root = witness_root(&block.transactions);

            let mut outbound = Vec::new();
            left.sync
                .handle_compact_block(
                    &right.id,
                    compact_block_from_block(&block),
                    &mut left.node,
                    &mut outbound,
                )
                .expect("buffer missing compact block");
        }

        assert!(left.sync.pending_compact_blocks.len() <= MAX_PENDING_COMPACT_BLOCKS);
        for pending in left.sync.pending_compact_blocks.values_mut() {
            pending.received_at = now - PENDING_COMPACT_BLOCK_TIMEOUT - Duration::from_secs(1);
        }
        left.sync.prune_pending_compact_blocks();
        assert!(left.sync.pending_compact_blocks.is_empty());
    }

    #[test]
    fn sandbox_longer_branch_reorgs_to_the_preferred_tip() {
        let mut canonical = SandboxPeer::new("canonical", Network::Regnet);
        let mut fork = SandboxPeer::new("fork", Network::Regnet);
        let miner = Miner::new(1);

        canonical
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine canonical block 1");
        canonical
            .node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine canonical block 2");
        canonical.sync.prime(&canonical.node);

        fork.node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine fork block 1");
        fork.node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine fork block 2");
        fork.node
            .mine_and_connect_candidate_block(&miner)
            .expect("mine fork block 3");
        fork.sync.prime(&fork.node);

        let notices = connect(&mut canonical, &mut fork);

        assert!(notices.iter().any(|notice| matches!(
            notice,
            SyncNotice::Ready { peer, .. } if peer == "fork"
        )));
        assert_eq!(canonical.node.height(), fork.node.height());
        assert_eq!(canonical.node.tip_hash(), fork.node.tip_hash());
        assert_eq!(canonical.sync.sync_state().best_height, fork.node.height());
    }

    #[test]
    fn restarted_node_locator_finds_common_ancestor_beyond_recent_reload_window() {
        let left_root = temp_data_dir("deep-fork-left");
        let right_root = temp_data_dir("deep-fork-right");
        fs::create_dir_all(&left_root).expect("left root");
        fs::create_dir_all(&right_root).expect("right root");

        {
            let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &left_root);
            persist_synthetic_chain(Network::Regnet, 30, 11);
        }
        {
            let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &right_root);
            persist_synthetic_chain(Network::Regnet, 32, 22);
        }

        let _left_guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &left_root);
        let left = SandboxPeer::new_persistent("left", Network::Regnet);
        assert!(
            left.node.blocks_len() < left.node.height() as usize,
            "reloaded node must only have recent blocks in memory"
        );
        let genesis_hash = genesis::genesis_hash(Network::Regnet);
        assert!(
            !atho_p2p::sync::block_locator(left.node.blocks())
                .iter()
                .any(|hash| (*hash).into_inner() == genesis_hash),
            "old recent-only locator would not contain a common ancestor"
        );
        assert!(
            left.node.block_locator_hashes().contains(&genesis_hash),
            "full persisted locator must include genesis as a fallback ancestor"
        );

        let locator_hashes = left
            .sync
            .sync_state()
            .locator_hashes
            .iter()
            .copied()
            .map(Hash48::into_inner)
            .collect::<Vec<_>>();

        let _right_guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &right_root);
        let right = Node::load_or_new(NodeConfig::new(Network::Regnet));
        let headers = right.headers_after_locator(
            &locator_hashes,
            [0; 48],
            network_params(Network::Regnet)
                .limits
                .max_headers_per_message,
        );

        assert!(
            !headers.is_empty(),
            "a restarted forked node must request from a locator the remote can anchor"
        );
        assert_eq!(headers.first().map(|header| header.height), Some(1));
    }

    #[test]
    fn restarted_sync_selects_branch_from_fork_point_outside_recent_reload_window() {
        let root = temp_data_dir("deep-reorg-after-restart");
        fs::create_dir_all(&root).expect("data root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        persist_synthetic_chain(Network::Regnet, 32, 1);

        let mut local = SandboxPeer::new_persistent("local", Network::Regnet);
        assert_eq!(local.node.height(), 32);
        assert!(
            !local
                .node
                .blocks()
                .iter()
                .any(|block| block.header.height == 0),
            "reload keeps only the recent tail, so sync must consult persisted history"
        );

        let mut remote_branch = Vec::new();
        let mut previous_hash = genesis::genesis_hash(Network::Regnet);
        for height in 1..=33 {
            let block = synthetic_coinbase_block(Network::Regnet, height, previous_hash, 2);
            previous_hash = block.header.block_hash();
            local.sync.side_branches.insert("remote", block.clone());
            remote_branch.push(block);
        }
        let remote_tip = remote_branch
            .last()
            .expect("remote branch")
            .header
            .block_hash();

        let best = local
            .sync
            .best_buffered_branch(&local.node)
            .expect("preferred remote branch");
        assert_eq!(best.len(), 33);
        assert_eq!(
            best.last().map(|block| block.header.block_hash()),
            Some(remote_tip)
        );
        assert!(local.node.branch_is_preferred_over_current(&best));
    }

    #[test]
    fn archived_side_branch_headers_replay_known_blocks_and_finish_reorg() {
        let root = temp_data_dir("archived-side-branch-reorg");
        fs::create_dir_all(&root).expect("data root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let miner = Miner::new(1);

        let mut archived_branch_node = Node::new(NodeConfig::new(Network::Regnet));
        let archived_1 = mine_with_timestamp_offset(&mut archived_branch_node, &miner, 10);
        let archived_2 = mine_with_timestamp_offset(&mut archived_branch_node, &miner, 20);

        let mut local = SandboxPeer::new_persistent("local", Network::Regnet);
        local
            .node
            .connect_block(&archived_1)
            .expect("archive branch 1");
        local
            .node
            .connect_block(&archived_2)
            .expect("archive branch 2");

        let mut local_fork_node = Node::new(NodeConfig::new(Network::Regnet));
        let local_fork = (0..3)
            .map(|index| mine_with_timestamp_offset(&mut local_fork_node, &miner, 1_000 + index))
            .collect::<Vec<_>>();
        local
            .node
            .consider_branch(&local_fork)
            .expect("switch to local fork");
        local.sync.prime(&local.node);
        assert_eq!(local.node.tip_hash(), local_fork[2].header.block_hash());
        assert!(local.node.contains_block(&archived_1.header.block_hash()));
        assert!(!local
            .node
            .is_canonical_block(&archived_1.header.block_hash()));
        assert!(local.node.contains_block(&archived_2.header.block_hash()));
        assert!(!local
            .node
            .is_canonical_block(&archived_2.header.block_hash()));

        let remote_extension = (0..3)
            .map(|index| {
                mine_with_timestamp_offset(&mut archived_branch_node, &miner, 2_000 + index)
            })
            .collect::<Vec<_>>();
        let remote_tip = remote_extension
            .last()
            .expect("remote extension")
            .header
            .block_hash();
        let mut remote = SandboxPeer::new("remote", Network::Regnet);
        remote.node = archived_branch_node;
        remote.sync.prime(&remote.node);

        let _ = connect(&mut local, &mut remote);

        assert_eq!(local.node.height(), remote.node.height());
        assert_eq!(local.node.tip_hash(), remote_tip);
        assert_eq!(local.node.tip_hash(), remote.node.tip_hash());
        assert!(local.sync.side_branches.is_empty());
    }

    #[test]
    fn branch_fork_choice_uses_work_not_raw_height() {
        let genesis = genesis::genesis_state(Network::Regnet).block;
        let genesis_hash = genesis.header.block_hash();
        let easy_target = pow::DIFFICULTY_PROFILE.min_difficulty_target;
        let hard_target = pow::DIFFICULTY_PROFILE.max_difficulty_target;

        let current_1 = coinbase_block(Network::Regnet, 1, genesis_hash, easy_target, 1_000);
        let current_2 = coinbase_block(
            Network::Regnet,
            2,
            current_1.header.block_hash(),
            easy_target,
            1_075,
        );
        let current_chain = vec![genesis, current_1, current_2];
        let candidate_branch = vec![coinbase_block(
            Network::Regnet,
            1,
            current_chain[0].header.block_hash(),
            hard_target,
            1_000,
        )];

        assert!(NodeSync::branch_is_preferred_over_current(
            &candidate_branch,
            &current_chain
        ));
    }

    #[test]
    fn invalid_network_future_block_does_not_advance_target_or_buffer() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let mut block = mine_reference_blocks(Network::Regnet, 1)
            .pop()
            .expect("reference block");
        block.header.network_id = Network::Testnet;

        let mut outbound = Vec::new();
        let err = peer
            .sync
            .handle_received_block("remote", block, &mut peer.node, &mut outbound)
            .expect_err("wrong-network block rejected");

        assert!(err.to_string().contains("block network mismatch"));
        assert_eq!(peer.node.height(), 0);
        assert_eq!(peer.sync.sync_state().best_height, 0);
        assert!(peer.sync.side_branches.is_empty());
        assert!(outbound.is_empty());
    }

    #[test]
    fn higher_valid_network_branch_rebuilds_over_local_fork() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let miner = Miner::new(1);
        mine_with_timestamp_offset(&mut peer.node, &miner, 0);
        mine_with_timestamp_offset(&mut peer.node, &miner, 1);
        peer.sync.prime(&peer.node);

        let mut reference = Node::new(NodeConfig::new(Network::Regnet));
        let remote_blocks = (0..4)
            .map(|index| mine_with_timestamp_offset(&mut reference, &miner, 10_000 + index))
            .collect::<Vec<_>>();
        assert_ne!(peer.node.tip_hash(), remote_blocks[1].header.block_hash());

        let mut outbound = Vec::new();
        for block in &remote_blocks {
            peer.sync
                .handle_received_block("remote", block.clone(), &mut peer.node, &mut outbound)
                .expect("valid network branch block");
        }

        assert_eq!(peer.node.height(), 4);
        assert_eq!(
            peer.node.tip_hash(),
            remote_blocks
                .last()
                .expect("remote tip")
                .header
                .block_hash()
        );
        assert_eq!(peer.sync.sync_state().best_height, 4);
        assert!(peer.sync.side_branches.is_empty());
    }

    #[test]
    fn recoverable_tip_height_mismatch_stays_buffered() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let arbitrary_tip = [9; 48];
        peer.node
            .dev_seed_chainstate(5, arbitrary_tip, Vec::<UtxoEntry>::new())
            .expect("seed chainstate");

        let block = Miner::new(1).solve_block(coinbase_block(
            Network::Regnet,
            3,
            arbitrary_tip,
            pow::initial_target_for_network(Network::Regnet),
            1_000,
        ));

        let mut outbound = Vec::new();
        peer.sync.downloader.note_peer_ready("peer");
        peer.sync
            .handle_received_block("peer", block, &mut peer.node, &mut outbound)
            .expect("recoverable branch mismatch");

        assert!(outbound_getdata_hashes(&outbound).contains(&arbitrary_tip));
        assert_eq!(peer.sync.side_branches.len(), 1);
    }

    #[test]
    fn side_branch_pool_is_bounded_and_preserves_low_bridge_blocks() {
        let mut sync = NodeSync::new(Network::Regnet);
        for height in 1..=(MAX_SIDE_BRANCH_BLOCKS as u64 + 8) {
            sync.side_branches.insert(
                "peer",
                coinbase_block(
                    Network::Regnet,
                    height,
                    [height as u8; 48],
                    pow::initial_target_for_network(Network::Regnet),
                    1_000 + height,
                ),
            );
        }

        assert_eq!(sync.side_branches.len(), MAX_SIDE_BRANCH_BLOCKS);
        assert!(sync
            .side_branches
            .blocks
            .values()
            .all(|entry| entry.block.header.height <= MAX_SIDE_BRANCH_BLOCKS as u64));
    }

    #[test]
    fn out_of_order_branch_blocks_reconstruct_and_reorg() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let miner = Miner::new(1);
        let mut reference_node = Node::new(NodeConfig::new(Network::Regnet));

        let block_1 = miner.solve_block(
            reference_node
                .build_candidate_block()
                .expect("candidate block 1"),
        );
        reference_node
            .connect_block(&block_1)
            .expect("connect reference block 1");
        let block_2 = miner.solve_block(
            reference_node
                .build_candidate_block()
                .expect("candidate block 2"),
        );
        reference_node
            .connect_block(&block_2)
            .expect("connect reference block 2");
        let block_3 = miner.solve_block(
            reference_node
                .build_candidate_block()
                .expect("candidate block 3"),
        );

        let mut outbound = Vec::new();
        peer.sync.downloader.note_peer_ready("peer");
        peer.sync
            .handle_received_block("peer", block_3.clone(), &mut peer.node, &mut outbound)
            .expect("buffer tip block");
        assert!(outbound_getdata_hashes(&outbound).contains(&block_2.header.block_hash()));
        outbound.clear();
        peer.sync
            .handle_received_block("peer", block_2.clone(), &mut peer.node, &mut outbound)
            .expect("buffer middle block");
        assert!(outbound_getdata_hashes(&outbound).contains(&block_1.header.block_hash()));
        outbound.clear();
        peer.sync
            .handle_received_block("peer", block_1.clone(), &mut peer.node, &mut outbound)
            .expect("reconstruct buffered branch");

        assert!(outbound.is_empty());
        assert_eq!(peer.node.height(), 3);
        assert_eq!(peer.node.tip_hash(), block_3.header.block_hash());
        assert!(peer.sync.side_branches.is_empty());
    }

    #[test]
    fn disconnect_preserves_buffered_blocks_for_later_chain_rebuild() {
        let mut left = SandboxPeer::new("left", Network::Regnet);
        let mut right = SandboxPeer::new("right", Network::Regnet);
        let _ = connect(&mut left, &mut right);
        let blocks = mine_reference_blocks(Network::Regnet, 3);

        let mut outbound = Vec::new();
        left.sync
            .handle_received_block(&right.id, blocks[2].clone(), &mut left.node, &mut outbound)
            .expect("buffer future tip");
        assert_eq!(left.sync.side_branches.len(), 1);
        assert!(outbound_getdata_hashes(&outbound).contains(&blocks[1].header.block_hash()));

        let notice = left
            .sync
            .disconnect_peer(&right.id, String::from("network hiccup"), &left.node)
            .expect("disconnect connected peer");
        assert!(matches!(
            notice,
            SyncNotice::Disconnected { peer, .. } if peer == right.id
        ));
        assert_eq!(left.sync.side_branches.len(), 1);

        left.sync
            .handle_received_block(&right.id, blocks[1].clone(), &mut left.node, &mut outbound)
            .expect("buffer parent");
        left.sync
            .handle_received_block(&right.id, blocks[0].clone(), &mut left.node, &mut outbound)
            .expect("connect preserved buffered branch");

        assert_eq!(left.node.height(), 3);
        assert_eq!(left.node.tip_hash(), blocks[2].header.block_hash());
        assert!(left.sync.side_branches.is_empty());
    }

    #[test]
    fn cross_peer_side_branch_blocks_reconstruct_over_local_fork() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let miner = Miner::new(1);
        mine_with_timestamp_offset(&mut peer.node, &miner, 0);
        mine_with_timestamp_offset(&mut peer.node, &miner, 1);
        peer.sync.prime(&peer.node);

        let mut reference_node = Node::new(NodeConfig::new(Network::Regnet));

        let block_1 = miner.solve_block(
            reference_node
                .build_candidate_block()
                .expect("candidate block 1"),
        );
        reference_node
            .connect_block(&block_1)
            .expect("connect reference block 1");
        let block_2 = miner.solve_block(
            reference_node
                .build_candidate_block()
                .expect("candidate block 2"),
        );
        reference_node
            .connect_block(&block_2)
            .expect("connect reference block 2");
        let block_3 = miner.solve_block(
            reference_node
                .build_candidate_block()
                .expect("candidate block 3"),
        );

        let mut outbound = Vec::new();
        peer.sync.downloader.note_peer_ready("peer-a");
        peer.sync.downloader.note_peer_ready("peer-b");
        peer.sync.downloader.note_peer_ready("peer-c");
        peer.sync
            .handle_received_block("peer-a", block_3.clone(), &mut peer.node, &mut outbound)
            .expect("buffer tip block from peer-a");
        assert!(outbound_getdata_hashes(&outbound).contains(&block_2.header.block_hash()));
        outbound.clear();
        peer.sync
            .handle_received_block("peer-b", block_2.clone(), &mut peer.node, &mut outbound)
            .expect("buffer middle block from peer-b");
        assert!(outbound_getdata_hashes(&outbound).contains(&block_1.header.block_hash()));
        outbound.clear();
        peer.sync
            .handle_received_block("peer-c", block_1.clone(), &mut peer.node, &mut outbound)
            .expect("reconstruct buffered branch across peers");

        assert!(outbound.is_empty());
        assert_eq!(peer.node.height(), 3);
        assert_eq!(peer.node.tip_hash(), block_3.header.block_hash());
        assert!(peer.sync.side_branches.is_empty());
    }
}
