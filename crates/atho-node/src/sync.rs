use crate::error::NodeError;
use crate::node::Node;
use atho_core::block::Block;
use atho_core::network::Network;
use atho_p2p::config::network_params;
use atho_p2p::connection::{ConnectionEvent, ConnectionManager};
use atho_p2p::downloader::BlockDownloadScheduler;
use atho_p2p::protocol::{
    compact_block_from_block, compact_short_id, reconstruct_compact_block, BlockTxnMessage,
    CompactBlockMessage, CompactBlockReconstruction, GetBlockTxnMessage, Hash48, InventoryKind,
    InventoryVector, MessagePayload, NetworkMessage, ProtocolError,
};
use atho_p2p::relay::RelayLoop;
use atho_p2p::sync::SyncState;
use atho_storage::chainstate::ChainSelectionOutcome;
use std::collections::BTreeMap;
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
    branch_buffers: BTreeMap<String, Vec<Block>>,
    pending_compact_blocks: BTreeMap<[u8; 48], PendingCompactBlock>,
}

#[derive(Debug, Clone)]
struct PendingCompactBlock {
    message: CompactBlockMessage,
    overrides: BTreeMap<u32, atho_core::transaction::Transaction>,
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
            | MessagePayload::GetAddr
            | MessagePayload::Addr { .. } => {
                return Err(NodeSyncError::Protocol(ProtocolError::UnexpectedPayload));
            }
        }
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
            self.branch_buffers.remove(peer);
            self.push_scheduled_block_requests(outbound);
            return Ok(());
        }

        if block.header.previous_block_hash == node.tip_hash() {
            node.submit_block(&block)?;
            self.relay.prime(node.blocks());
            self.downloader.note_block_received(block_hash);
            self.pending_compact_blocks.remove(&block_hash);
            self.branch_buffers.remove(peer);
            self.push_scheduled_block_requests(outbound);
            return Ok(());
        }

        let branch = self.branch_buffers.entry(peer.to_string()).or_default();
        if branch.last().is_some_and(|previous| {
            previous.header.block_hash() == block.header.previous_block_hash
        }) {
            branch.push(block);
        } else {
            branch.clear();
            branch.push(block);
        }

        if branch
            .last()
            .is_some_and(|candidate| candidate.header.height > node.height())
        {
            let candidate_branch = branch.clone();
            let candidate_hashes = candidate_branch
                .iter()
                .map(|candidate| candidate.header.block_hash())
                .collect::<Vec<_>>();
            let selection = node.consider_branch(&candidate_branch)?;
            if selection.outcome != ChainSelectionOutcome::KeptCurrent {
                self.relay.prime(node.blocks());
                for hash in candidate_hashes {
                    self.downloader.note_block_received(hash);
                    self.pending_compact_blocks.remove(&hash);
                }
                self.branch_buffers.clear();
                self.push_scheduled_block_requests(outbound);
            }
        }

        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NodeConfig;
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use crate::validation::{derive_sig_ref_short, derive_witness_commit_ref};
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};
    use atho_storage::utxo::UtxoEntry;
    use std::collections::VecDeque;

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
}
