use crate::error::NodeError;
use crate::node::Node;
use crate::validation::ValidationError;
use atho_core::block::Block;
use atho_core::consensus::pow;
use atho_core::network::Network;
use atho_p2p::config::network_params;
use atho_p2p::connection::{ConnectionEvent, ConnectionManager};
use atho_p2p::downloader::BlockDownloadScheduler;
use atho_p2p::protocol::{
    compact_block_from_block, compact_short_id, reconstruct_compact_block, BlockTxnMessage,
    CompactBlockMessage, CompactBlockReconstruction, GetBlockTxnMessage, Hash48, InventoryKind,
    InventoryVector, MessagePayload, NetworkMessage, PeerAddress, ProtocolError,
};
use atho_p2p::relay::RelayLoop;
use atho_p2p::sync::SyncState;
use atho_storage::chainstate::ChainSelectionOutcome;
use atho_storage::error::StorageError;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

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
    branch_buffers: BTreeMap<String, BufferedBranch>,
    pending_compact_blocks: BTreeMap<[u8; 48], PendingCompactBlock>,
}

#[derive(Debug, Clone)]
struct PendingCompactBlock {
    message: CompactBlockMessage,
    overrides: BTreeMap<u32, atho_core::transaction::Transaction>,
}

#[derive(Debug, Clone, Default)]
struct BufferedBranch {
    blocks: BTreeMap<[u8; 48], Block>,
}

impl NodeSync {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            relay: RelayLoop::new(network),
            connections: ConnectionManager::new(network),
            downloader: BlockDownloadScheduler::default(),
            branch_buffers: BTreeMap::new(),
            pending_compact_blocks: BTreeMap::new(),
        }
    }

    pub fn prime(&mut self, node: &Node) {
        self.relay.prime(node.blocks());
    }

    pub fn sync_state(&self) -> &SyncState {
        self.relay.sync_state()
    }

    pub fn connections(&self) -> &ConnectionManager {
        &self.connections
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
        let local_version = self.relay.build_version_message(node.blocks());
        Ok(self.connections.open_outbound(remote_addr, local_version)?)
    }

    pub fn receive(
        &mut self,
        remote_addr: &str,
        message: NetworkMessage,
        node: &mut Node,
    ) -> Result<(Vec<ConnectionEvent>, Vec<SyncNotice>), NodeSyncError> {
        let local_version = self.relay.build_version_message(node.blocks());
        let events = match self
            .connections
            .receive(remote_addr, message, &local_version)
        {
            Ok(events) => events,
            Err(atho_p2p::connection::ConnectionError::Protocol(error)) => {
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
        self.expand_events(events, node)
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

    pub fn disconnect_peer(&mut self, remote_addr: &str, reason: String) -> Option<SyncNotice> {
        if !self.connections.disconnect(remote_addr) {
            return None;
        }
        self.downloader.note_peer_disconnected(remote_addr);
        self.branch_buffers.remove(remote_addr);
        Some(SyncNotice::Disconnected {
            peer: remote_addr.to_string(),
            reason,
        })
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
                    self.downloader.note_peer_disconnected(&peer);
                    self.branch_buffers.remove(&peer);
                    notices.push(SyncNotice::Disconnected { peer, reason });
                }
                ConnectionEvent::Ready { peer, best_height } => {
                    if let Some(version) = self.connections.remote_version(&peer).cloned() {
                        self.relay.accept_version(peer.clone(), &version)?;
                    }
                    self.downloader.note_peer_ready(peer.clone());
                    self.record_peer_observation(node, &peer, best_height)?;
                    notices.push(SyncNotice::Ready {
                        peer: peer.clone(),
                        best_height,
                    });
                    outbound.push(ConnectionEvent::Send {
                        peer: peer.clone(),
                        message: self.relay.build_getheaders(peer.clone()),
                    });
                    outbound.push(ConnectionEvent::Send {
                        peer,
                        message: NetworkMessage::new(self.network, MessagePayload::GetAddr),
                    });
                }
                ConnectionEvent::Message { peer, message } => {
                    self.handle_message(&peer, message, node, &mut outbound)?;
                }
            }
        }

        Ok((outbound, notices))
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
                for vector in &inventory {
                    if vector.kind == InventoryKind::Block {
                        self.downloader
                            .note_inventory(peer, vector.hash.into_inner());
                    }
                }
                let requests = self.missing_inventory_requests(node, &inventory);
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
                outbound.push(ConnectionEvent::Send {
                    peer: peer.to_string(),
                    message: NetworkMessage::new(self.network, MessagePayload::Headers { headers }),
                });
            }
            MessagePayload::Headers { headers } => {
                self.relay.accept_headers(&headers)?;
                self.downloader.note_headers(
                    peer,
                    headers
                        .iter()
                        .map(|header| header.block_hash())
                        .filter(|hash| !node.contains_block(hash)),
                );
                self.push_scheduled_block_requests(outbound);
                if !self.relay.sync_state().headers_synced {
                    outbound.push(ConnectionEvent::Send {
                        peer: peer.to_string(),
                        message: self.relay.build_getheaders(peer.to_string()),
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
                let txid = transaction.txid();
                if !node.mempool_contains(&txid) {
                    node.accept_relayed_transaction(transaction)?;
                }
            }
            MessagePayload::MemPool => {
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
                self.push_scheduled_block_requests(outbound);
            }
            MessagePayload::Pong { .. } => {}
            MessagePayload::Version(_)
            | MessagePayload::Verack
            | MessagePayload::Ping { .. }
            | MessagePayload::GetAddr => {
                return Err(NodeSyncError::Protocol(ProtocolError::UnexpectedPayload));
            }
            MessagePayload::Addr { addresses } => {
                let accepted = self
                    .connections
                    .note_gossip_addresses(&addresses, !matches!(self.network, Network::Regnet))
                    .map_err(NodeSyncError::from)?;
                let observed_height = self
                    .connections
                    .remote_best_height(peer)
                    .unwrap_or_else(|| node.height());
                let observed_unix = now_unix();
                node.observe_peer(peer.to_string(), observed_height, observed_unix)?;
                for address in accepted {
                    node.observe_peer_address(&address, observed_height, observed_unix)?;
                }
            }
        }
        Ok(())
    }

    fn record_peer_observation(
        &self,
        node: &mut Node,
        peer: &str,
        observed_height: u64,
    ) -> Result<(), NodeSyncError> {
        node.observe_peer(peer.to_string(), observed_height, now_unix())?;
        Ok(())
    }

    fn push_scheduled_block_requests(&mut self, outbound: &mut Vec<ConnectionEvent>) {
        let limits = network_params(self.network).limits;
        for assignment in self
            .downloader
            .assignments(limits.max_blocks_in_flight, limits.max_requests_per_peer)
        {
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
    }

    fn handle_received_block(
        &mut self,
        peer: &str,
        block: Block,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        let block_hash = block.header.block_hash();
        if node.contains_block(&block_hash) {
            self.downloader.note_block_received(block_hash);
            self.pending_compact_blocks.remove(&block_hash);
            self.remove_buffered_block(peer, &block_hash);
            self.push_scheduled_block_requests(outbound);
            self.process_buffered_branches(peer, node, outbound)?;
            return Ok(());
        }

        if block.header.previous_block_hash == node.tip_hash() {
            match node.submit_block(&block) {
                Ok(()) => {
                    self.relay.prime(node.blocks());
                    self.downloader.note_block_received(block_hash);
                    self.pending_compact_blocks.remove(&block_hash);
                    self.remove_buffered_block(peer, &block_hash);
                    self.push_scheduled_block_requests(outbound);
                    self.process_buffered_branches(peer, node, outbound)?;
                    return Ok(());
                }
                Err(NodeError::Validation(validation))
                    if Self::recoverable_tip_validation(&validation) =>
                {
                    // Keep the block buffered so fork-choice can re-evaluate it once the
                    // branch is complete enough to compare by cumulative work.
                }
                Err(err) => return Err(err.into()),
            }
        }

        self.buffer_peer_block(peer, block);
        self.process_buffered_branches(peer, node, outbound)?;

        Ok(())
    }

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

    fn buffer_peer_block(&mut self, peer: &str, block: Block) {
        self.branch_buffers
            .entry(peer.to_string())
            .or_default()
            .blocks
            .insert(block.header.block_hash(), block);
    }

    fn remove_buffered_block(&mut self, peer: &str, block_hash: &[u8; 48]) {
        let mut remove_peer = false;
        if let Some(buffer) = self.branch_buffers.get_mut(peer) {
            buffer.blocks.remove(block_hash);
            remove_peer = buffer.blocks.is_empty();
        }
        if remove_peer {
            self.branch_buffers.remove(peer);
        }
    }

    fn buffered_branch_from_tip(
        &self,
        peer: &str,
        node: &Node,
        tip_hash: [u8; 48],
    ) -> Option<Vec<Block>> {
        let buffer = self.branch_buffers.get(peer)?;
        let mut branch_reversed = Vec::new();
        let mut current_hash = tip_hash;

        loop {
            let block = buffer.blocks.get(&current_hash)?.clone();
            let previous_hash = block.header.previous_block_hash;
            branch_reversed.push(block);
            if node.contains_block(&previous_hash) {
                break;
            }
            if buffer.blocks.contains_key(&previous_hash) {
                current_hash = previous_hash;
                continue;
            }
            return None;
        }

        branch_reversed.reverse();
        Some(branch_reversed)
    }

    fn best_buffered_branch(&self, peer: &str, node: &Node) -> Option<Vec<Block>> {
        let buffer = self.branch_buffers.get(peer)?;
        let mut best: Option<Vec<Block>> = None;
        let tip_hashes = buffer.blocks.keys().copied().collect::<Vec<_>>();
        for tip_hash in tip_hashes {
            let Some(candidate) = self.buffered_branch_from_tip(peer, node, tip_hash) else {
                continue;
            };
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

    fn process_buffered_branches(
        &mut self,
        peer: &str,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        loop {
            let Some(candidate_branch) = self.best_buffered_branch(peer, node) else {
                return Ok(());
            };
            if !Self::branch_is_preferred_over_current(&candidate_branch, node.blocks()) {
                return Ok(());
            }

            let candidate_hashes = candidate_branch
                .iter()
                .map(|candidate| candidate.header.block_hash())
                .collect::<Vec<_>>();
            match node.consider_branch(&candidate_branch) {
                Ok(selection) if selection.outcome != ChainSelectionOutcome::KeptCurrent => {
                    self.relay.prime(node.blocks());
                    for hash in candidate_hashes {
                        self.downloader.note_block_received(hash);
                        self.pending_compact_blocks.remove(&hash);
                        self.remove_buffered_block(peer, &hash);
                    }
                    self.push_scheduled_block_requests(outbound);
                }
                Ok(_) => return Ok(()),
                Err(err) if Self::recoverable_branch_error(&err) => return Ok(()),
                Err(err) => return Err(err.into()),
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
            NodeError::Storage(
                StorageError::ForkPointUnavailable | StorageError::InvalidBranchSequence,
            ) => true,
            _ => false,
        }
    }

    fn handle_compact_block(
        &mut self,
        peer: &str,
        message: CompactBlockMessage,
        node: &mut Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) -> Result<(), NodeSyncError> {
        let block_hash = message.header.block_hash();
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
                self.handle_received_block(peer, block, node, outbound)?;
            }
            CompactBlockReconstruction::Missing { indexes, .. } => {
                self.pending_compact_blocks.insert(
                    block_hash,
                    PendingCompactBlock {
                        message,
                        overrides: BTreeMap::new(),
                    },
                );
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

    fn missing_inventory_requests(
        &self,
        node: &Node,
        inventory: &[InventoryVector],
    ) -> Vec<InventoryVector> {
        inventory
            .iter()
            .filter(|vector| match vector.kind {
                InventoryKind::Block => !node.contains_block(&vector.hash.into_inner()),
                InventoryKind::Transaction => !node.mempool_contains(&vector.hash.into_inner()),
            })
            .take(network_params(self.network).limits.max_requests_per_peer)
            .cloned()
            .collect()
    }

    fn serve_getdata(
        &self,
        peer: &str,
        inventory: &[InventoryVector],
        node: &Node,
        outbound: &mut Vec<ConnectionEvent>,
    ) {
        let mut not_found = Vec::new();
        for vector in inventory
            .iter()
            .take(network_params(self.network).limits.max_requests_per_peer)
        {
            match vector.kind {
                InventoryKind::Block => {
                    if let Some(block) = node.block_by_hash(vector.hash.into_inner()) {
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
    use atho_core::consensus::{pow, subsidy};
    use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
    use atho_core::genesis;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};
    use atho_p2p::protocol::PeerAddress;
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
            &transaction_signing_digest(tx),
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
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        };
        let staged_tx = Transaction {
            witness: staged.canonical_bytes(),
            ..tx.clone()
        };
        let witness_root = staged_tx.witness_commitment_hash();
        TxWitness {
            signature: signature.clone(),
            pubkey,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                    witness_commit_ref: derive_witness_commit_ref(
                        &txid,
                        &witness_root,
                        index as u32,
                    ),
                })
                .collect(),
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
                value_atoms: subsidy::block_subsidy_atoms(height),
                locking_script: vec![1],
            }],
            lock_time: height as u32,
            witness: vec![],
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
                value_atoms: seed_value - MIN_TX_FEE_PER_VBYTE_ATOMS,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let provisional = Transaction {
            witness: witness_bytes_for_tx(&template),
            ..template
        };
        let fee_atoms = provisional.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let signed = Transaction {
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(fee_atoms),
                locking_script: vec![2],
            }],
            ..Transaction {
                witness: vec![],
                ..provisional
            }
        };
        let signed = Transaction {
            witness: witness_bytes_for_tx(&signed),
            ..signed
        };
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
                value_atoms: seed_value - MIN_TX_FEE_PER_VBYTE_ATOMS,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let provisional = Transaction {
            witness: witness_bytes_for_tx(&template),
            ..template
        };
        let fee_atoms = provisional.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let signed = Transaction {
            outputs: vec![TxOutput {
                value_atoms: seed_value.saturating_sub(fee_atoms),
                locking_script: vec![2],
            }],
            ..Transaction {
                witness: vec![],
                ..provisional
            }
        };
        let signed = Transaction {
            witness: witness_bytes_for_tx(&signed),
            ..signed
        };
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
    fn recoverable_tip_height_mismatch_stays_buffered() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let arbitrary_tip = [9; 48];
        peer.node
            .dev_seed_chainstate(5, arbitrary_tip, Vec::<UtxoEntry>::new())
            .expect("seed chainstate");

        let block = coinbase_block(
            Network::Regnet,
            3,
            arbitrary_tip,
            pow::initial_target_for_network(Network::Regnet),
            1_000,
        );

        let mut outbound = Vec::new();
        peer.sync
            .handle_received_block("peer", block, &mut peer.node, &mut outbound)
            .expect("recoverable branch mismatch");

        assert!(outbound.is_empty());
        assert_eq!(
            peer.sync
                .branch_buffers
                .get("peer")
                .map(|buffer| buffer.blocks.len()),
            Some(1)
        );
    }

    #[test]
    fn out_of_order_branch_blocks_reconstruct_and_reorg() {
        let mut peer = SandboxPeer::new("peer", Network::Regnet);
        let miner = Miner::new(1);
        let mut reference_node = Node::new(NodeConfig::new(Network::Regnet));

        let block_1 = miner.solve_block(
            reference_node
                .build_candidate_block(&miner)
                .expect("candidate block 1"),
        );
        reference_node
            .connect_block(&block_1)
            .expect("connect reference block 1");
        let block_2 = miner.solve_block(
            reference_node
                .build_candidate_block(&miner)
                .expect("candidate block 2"),
        );
        reference_node
            .connect_block(&block_2)
            .expect("connect reference block 2");
        let block_3 = miner.solve_block(
            reference_node
                .build_candidate_block(&miner)
                .expect("candidate block 3"),
        );

        let mut outbound = Vec::new();
        peer.sync
            .handle_received_block("peer", block_3.clone(), &mut peer.node, &mut outbound)
            .expect("buffer tip block");
        peer.sync
            .handle_received_block("peer", block_2.clone(), &mut peer.node, &mut outbound)
            .expect("buffer middle block");
        peer.sync
            .handle_received_block("peer", block_1.clone(), &mut peer.node, &mut outbound)
            .expect("reconstruct buffered branch");

        assert!(outbound.is_empty());
        assert_eq!(peer.node.height(), 3);
        assert_eq!(peer.node.tip_hash(), block_3.header.block_hash());
        assert!(peer.sync.branch_buffers.get("peer").is_none());
    }
}
