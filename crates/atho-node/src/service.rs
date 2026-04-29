use crate::config::NodeConfig;
use crate::dev;
use crate::error::rpc_error_from_node;
use crate::error::NodeError;
use crate::mempool::MempoolEntry;
use crate::miner::Miner;
use crate::orchestrator::NodeOrchestrator;
use crate::sync::{NodeSyncError, SyncNotice};
use crate::tcp_p2p::next_outbound_retry_delay;
use crate::wallet_history;
use atho_core::block::Block;
use atho_core::network::Network;
use atho_p2p::address_manager::format_remote_addr;
use atho_p2p::connection::{ConnectionDirection, ConnectionEvent};
use atho_p2p::protocol::NetworkMessage;
use atho_rpc::request::{RpcRequest, WalletHistoryAddress};
use atho_rpc::response::{
    BlockTemplate, MempoolInfo, MempoolSpentInput, NetworkDiagnostics, NetworkPeerDiagnostics,
    NetworkPeerDirection, NodeStatus, RpcResponse, WalletActivityEntry,
};
use atho_storage::db::PeerHealthRecord;
use atho_wallet::snapshot::WalletSnapshot;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStatus {
    pub network: Network,
    pub block_count: u64,
    pub tip_hash: [u8; 48],
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub wallet_snapshot: WalletSnapshot,
    pub running: bool,
    pub headers_synced: bool,
    pub sync_best_height: u64,
}

#[derive(Debug, Clone, Default)]
struct PeerTrafficStats {
    bytes_sent: u64,
    bytes_received: u64,
    last_send_unix: Option<u64>,
    last_receive_unix: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct NetworkRuntimeView {
    bytes_sent: u64,
    bytes_received: u64,
    peers: BTreeMap<String, PeerTrafficStats>,
}

#[derive(Debug)]
pub struct NodeService {
    orchestrator: NodeOrchestrator,
    wallet_snapshot: WalletSnapshot,
    network_runtime: NetworkRuntimeView,
    peer_health_cache: BTreeMap<String, PeerHealthRecord>,
}

impl NodeService {
    pub fn new(config: NodeConfig) -> Self {
        Self {
            orchestrator: NodeOrchestrator::new(config),
            wallet_snapshot: WalletSnapshot::default(),
            network_runtime: NetworkRuntimeView::default(),
            peer_health_cache: BTreeMap::new(),
        }
    }

    pub fn new_ephemeral(config: NodeConfig) -> Self {
        let network = config.network;
        Self {
            orchestrator: NodeOrchestrator {
                runtime: crate::runtime::NodeRuntime::new(config),
                sync: crate::sync::NodeSync::new(network),
                rpc_server: atho_rpc::server::RpcServer::new(network),
            },
            wallet_snapshot: WalletSnapshot::default(),
            network_runtime: NetworkRuntimeView::default(),
            peer_health_cache: BTreeMap::new(),
        }
    }

    pub fn try_new(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            orchestrator: NodeOrchestrator::try_new(config)?,
            wallet_snapshot: WalletSnapshot::default(),
            network_runtime: NetworkRuntimeView::default(),
            peer_health_cache: BTreeMap::new(),
        })
    }

    pub fn start(&mut self) {
        self.orchestrator.start();
        self.seed_peer_graph();
        self.refresh_runtime_views();
    }

    pub fn stop(&mut self) {
        self.orchestrator.stop();
    }

    pub fn network(&self) -> Network {
        self.orchestrator.runtime.node.config.network
    }

    pub fn handle(&self, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::GetNetwork => RpcResponse::Network(self.network().id().to_string()),
            RpcRequest::GetBlockCount => {
                RpcResponse::BlockCount(self.orchestrator.runtime.node.height())
            }
            RpcRequest::GetNodeStatus => RpcResponse::NodeStatus(self.node_status()),
            RpcRequest::GetMempoolInfo => RpcResponse::MempoolInfo(MempoolInfo {
                transaction_count: self.orchestrator.runtime.node.mempool_len(),
                total_fee_atoms: self.orchestrator.runtime.node.mempool_total_fee_atoms(),
            }),
            RpcRequest::GetMempoolSpentInputs => RpcResponse::MempoolSpentInputs(
                self.orchestrator
                    .runtime
                    .node
                    .mempool_spent_inputs()
                    .into_iter()
                    .map(|(txid, output_index)| MempoolSpentInput { txid, output_index })
                    .collect(),
            ),
            RpcRequest::ListUtxos => RpcResponse::Utxos(self.list_utxos()),
            RpcRequest::GetWalletActivity { addresses } => match self.wallet_activity(&addresses) {
                Ok(activity) => RpcResponse::WalletActivity(activity),
                Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
            },
            RpcRequest::GetBlockTemplate => {
                let miner = Miner::new(1);
                match self.orchestrator.runtime.node.build_candidate_block(&miner) {
                    Ok(block) => {
                        let _ = dev::append_log(
                            "athod",
                            &format!(
                                "rpc template height={} mempool={} fees={}",
                                block.header.height,
                                self.orchestrator.runtime.node.mempool_len(),
                                block.fees_total_atoms
                            ),
                        );
                        RpcResponse::BlockTemplate(self.block_template(block))
                    }
                    Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
                }
            }
            RpcRequest::SubmitBlock(_) | RpcRequest::SubmitTransaction { .. } => {
                RpcResponse::Error(atho_rpc::error::RpcError::InvalidRequest(String::from(
                    "submit requests require mutable RPC handling",
                )))
            }
        }
    }

    pub fn handle_mut(&mut self, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::SubmitTransaction {
                transaction,
                fee_atoms,
            } => {
                let tx_summary = dev::summarize_transaction(&transaction, Some(fee_atoms));
                let response = match self
                    .orchestrator
                    .runtime
                    .node
                    .submit_transaction(MempoolEntry::new(transaction, fee_atoms))
                {
                    Ok(txid) => {
                        self.refresh_runtime_views();
                        RpcResponse::TransactionSubmitted(txid)
                    }
                    Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
                };
                match &response {
                    RpcResponse::TransactionSubmitted(_) => {
                        let _ = dev::append_log(
                            "athod",
                            &format!(
                                "rpc tx submitted mempool={} {tx_summary}",
                                self.orchestrator.runtime.node.mempool_len()
                            ),
                        );
                    }
                    RpcResponse::Error(err) => {
                        let _ = dev::append_log(
                            "athod",
                            &format!("rpc tx rejected error={err} {tx_summary}"),
                        );
                    }
                    _ => {}
                }
                response
            }
            RpcRequest::SubmitBlock(block) => {
                let block_hash = block.header.block_hash();
                let block_summary = dev::summarize_block(&block);
                let response = match self.orchestrator.runtime.node.submit_block(&block) {
                    Ok(()) => {
                        self.orchestrator
                            .sync
                            .prime(&self.orchestrator.runtime.node);
                        self.refresh_runtime_views();
                        RpcResponse::BlockSubmitted {
                            accepted: true,
                            block_hash,
                        }
                    }
                    Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
                };
                match &response {
                    RpcResponse::BlockSubmitted { accepted: true, .. } => {
                        let _ = dev::append_log(
                            "athod",
                            &format!(
                                "rpc block accepted height={} mempool={} {block_summary}",
                                self.orchestrator.runtime.node.height(),
                                self.orchestrator.runtime.node.mempool_len(),
                            ),
                        );
                    }
                    RpcResponse::Error(err) => {
                        let _ = dev::append_log(
                            "athod",
                            &format!("rpc block rejected error={err} {block_summary}"),
                        );
                    }
                    _ => {}
                }
                response
            }
            other => self.handle(other),
        }
    }

    fn block_template(&self, block: Block) -> BlockTemplate {
        BlockTemplate {
            network: self.network(),
            height: block.header.height,
            previous_block_hash: block.header.previous_block_hash,
            target: block.header.difficulty_target_or_bits,
            transaction_count: block.transactions.len(),
            fees_atoms: block.fees_total_atoms,
            block,
        }
    }

    fn list_utxos(&self) -> Vec<atho_storage::utxo::UtxoEntry> {
        self.orchestrator
            .runtime
            .node
            .utxo_snapshot()
            .entries()
            .cloned()
            .collect()
    }

    fn wallet_activity(
        &self,
        addresses: &[WalletHistoryAddress],
    ) -> Result<Vec<WalletActivityEntry>, NodeError> {
        let blocks = self.orchestrator.runtime.node.canonical_blocks()?;
        Ok(wallet_history::derive_wallet_activity(&blocks, addresses))
    }

    pub fn status(&self) -> SystemStatus {
        SystemStatus {
            network: self.network(),
            block_count: self.orchestrator.runtime.node.height(),
            tip_hash: self.orchestrator.runtime.node.tip_hash(),
            mempool_count: self.orchestrator.runtime.node.mempool_len(),
            mempool_total_fee_atoms: self.orchestrator.runtime.node.mempool_total_fee_atoms(),
            wallet_snapshot: self.wallet_snapshot.clone(),
            running: self.orchestrator.runtime.running,
            headers_synced: self.orchestrator.sync.sync_state().headers_synced,
            sync_best_height: self.orchestrator.sync.sync_state().best_height,
        }
    }

    pub fn node_status(&self) -> NodeStatus {
        let status = self.status();
        NodeStatus {
            network: status.network,
            block_count: status.block_count,
            tip_hash: status.tip_hash,
            mempool_count: status.mempool_count,
            mempool_total_fee_atoms: status.mempool_total_fee_atoms,
            mempool_fingerprint: self.orchestrator.runtime.node.mempool_fingerprint(),
            running: status.running,
            headers_synced: status.headers_synced,
            sync_best_height: status.sync_best_height,
            network_diagnostics: self.network_diagnostics(),
        }
    }

    pub fn network_diagnostics(&self) -> NetworkDiagnostics {
        let connections = self.orchestrator.sync.connections();
        let peers = connections
            .peer_snapshots()
            .into_iter()
            .map(|peer| {
                let traffic = self
                    .network_runtime
                    .peers
                    .get(&peer.remote_addr)
                    .cloned()
                    .unwrap_or_default();
                let health = self.peer_health_cache.get(&peer.remote_addr);
                NetworkPeerDiagnostics {
                    remote_addr: peer.remote_addr,
                    direction: match peer.direction {
                        ConnectionDirection::Inbound => NetworkPeerDirection::Inbound,
                        ConnectionDirection::Outbound => NetworkPeerDirection::Outbound,
                    },
                    handshake_ready: peer.handshake_ready,
                    best_height: peer.best_height,
                    protocol_version: peer.protocol_version,
                    services: peer.services,
                    user_agent: peer.user_agent,
                    ruleset_version: peer.ruleset_version,
                    bytes_sent: traffic.bytes_sent,
                    bytes_received: traffic.bytes_received,
                    last_send_unix: traffic.last_send_unix,
                    last_receive_unix: traffic.last_receive_unix,
                    quality_score: health.map(|record| record.quality_score),
                    consecutive_failures: health.map(|record| record.consecutive_failures),
                }
            })
            .collect();

        NetworkDiagnostics {
            peer_count: connections.peer_count(),
            inbound_peer_count: connections.inbound_count(),
            outbound_peer_count: connections.outbound_count(),
            bytes_sent: self.network_runtime.bytes_sent,
            bytes_received: self.network_runtime.bytes_received,
            peers,
        }
    }

    pub fn wallet_snapshot(&self) -> &WalletSnapshot {
        &self.wallet_snapshot
    }

    pub fn is_running(&self) -> bool {
        self.orchestrator.runtime.running
    }

    pub fn height(&self) -> u64 {
        self.orchestrator.runtime.node.height()
    }

    pub fn tip_hash(&self) -> [u8; 48] {
        self.orchestrator.runtime.node.tip_hash()
    }

    pub fn p2p_peer_count(&self) -> usize {
        self.orchestrator.sync.connections().peer_count()
    }

    pub fn p2p_has_peer(&self, remote_addr: &str) -> bool {
        self.orchestrator.sync.has_peer(remote_addr)
    }

    pub fn p2p_sync_best_height(&self) -> u64 {
        self.orchestrator.sync.sync_state().best_height
    }

    pub fn p2p_headers_synced(&self) -> bool {
        self.orchestrator.sync.sync_state().headers_synced
    }

    pub fn p2p_mempool_txids(&self) -> Vec<[u8; 48]> {
        self.orchestrator.runtime.node.mempool_txids()
    }

    pub fn p2p_peer_health(&mut self, remote_addr: &str) -> Option<PeerHealthRecord> {
        if let Some(record) = self.peer_health_cache.get(remote_addr) {
            return Some(record.clone());
        }
        let record = self
            .orchestrator
            .runtime
            .node
            .load_peer_health(remote_addr)
            .ok()
            .flatten();
        if let Some(record) = record.as_ref() {
            self.peer_health_cache
                .insert(remote_addr.to_string(), record.clone());
        }
        record
    }

    pub fn p2p_save_peer_health(&mut self, record: &PeerHealthRecord) {
        self.peer_health_cache
            .insert(record.remote_addr.clone(), record.clone());
        if let Err(err) = self.orchestrator.runtime.node.save_peer_health(record) {
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "peer health persist failed peer={} error={err}",
                    record.remote_addr
                ),
            );
        }
    }

    pub fn p2p_note_bytes_sent(&mut self, remote_addr: &str, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let now = unix_timestamp();
        let bytes = bytes as u64;
        self.network_runtime.bytes_sent = self.network_runtime.bytes_sent.saturating_add(bytes);
        let peer = self
            .network_runtime
            .peers
            .entry(remote_addr.to_string())
            .or_default();
        peer.bytes_sent = peer.bytes_sent.saturating_add(bytes);
        peer.last_send_unix = Some(now);
    }

    pub fn p2p_note_bytes_received(&mut self, remote_addr: &str, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let now = unix_timestamp();
        let bytes = bytes as u64;
        self.network_runtime.bytes_received =
            self.network_runtime.bytes_received.saturating_add(bytes);
        let peer = self
            .network_runtime
            .peers
            .entry(remote_addr.to_string())
            .or_default();
        peer.bytes_received = peer.bytes_received.saturating_add(bytes);
        peer.last_receive_unix = Some(now);
    }

    pub fn p2p_relay_compact_tip_messages_since(
        &self,
        last_announced_tip: [u8; 48],
    ) -> Vec<NetworkMessage> {
        let blocks = self.orchestrator.runtime.node.blocks();
        if blocks.len() <= 1 {
            return Vec::new();
        }

        let start_index = blocks
            .iter()
            .position(|block| block.header.block_hash() == last_announced_tip)
            .map(|index| index.saturating_add(1))
            .unwrap_or(1);

        blocks
            .iter()
            .skip(start_index)
            .map(|block| self.orchestrator.sync.relay_compact_block_message(block))
            .collect()
    }

    pub fn p2p_accept_inbound(&mut self, remote_addr: impl Into<String>) -> Result<(), NodeError> {
        self.orchestrator
            .sync
            .accept_inbound(remote_addr)
            .map_err(sync_error_into_node)?;
        self.refresh_runtime_views();
        Ok(())
    }

    pub fn p2p_open_outbound(
        &mut self,
        remote_addr: impl Into<String>,
    ) -> Result<Vec<ConnectionEvent>, NodeError> {
        let events = self
            .orchestrator
            .sync
            .open_outbound(remote_addr, &self.orchestrator.runtime.node)
            .map_err(sync_error_into_node)?;
        self.refresh_runtime_views();
        Ok(events)
    }

    pub fn p2p_receive(
        &mut self,
        remote_addr: &str,
        message: NetworkMessage,
    ) -> Result<(Vec<ConnectionEvent>, Vec<SyncNotice>), NodeError> {
        let result = self
            .orchestrator
            .sync
            .receive(remote_addr, message, &mut self.orchestrator.runtime.node)
            .map_err(sync_error_into_node)?;
        self.refresh_runtime_views();
        Ok(result)
    }

    pub fn p2p_disconnect_peer(&mut self, remote_addr: &str, reason: String) -> Option<SyncNotice> {
        let notice = self
            .orchestrator
            .sync
            .disconnect_peer(remote_addr, reason.clone());
        self.network_runtime.peers.remove(remote_addr);
        if notice.is_some() && reason != "runtime stopping" {
            let now = unix_timestamp();
            let mut health = self
                .p2p_peer_health(remote_addr)
                .unwrap_or(PeerHealthRecord {
                    network: self.network(),
                    remote_addr: remote_addr.to_string(),
                    quality_score: 100,
                    consecutive_failures: 0,
                    backoff_until_unix: 0,
                    last_failure_unix: None,
                    last_success_unix: None,
                });
            health.consecutive_failures = health.consecutive_failures.saturating_add(1);
            health.quality_score = health.quality_score.saturating_sub(15);
            health.last_failure_unix = Some(now);
            let retry_delay = next_outbound_retry_delay(health.consecutive_failures);
            health.backoff_until_unix = now.saturating_add(retry_delay.as_secs().max(1));
            self.p2p_save_peer_health(&health);
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "peer health penalized peer={} reason={} retry_in_secs={} failures={} quality={}",
                    remote_addr,
                    reason,
                    retry_delay.as_secs().max(1),
                    health.consecutive_failures,
                    health.quality_score
                ),
            );
        }
        self.refresh_runtime_views();
        notice
    }

    pub fn p2p_prime(&mut self) {
        self.orchestrator
            .sync
            .prime(&self.orchestrator.runtime.node);
        self.seed_peer_graph();
        self.refresh_runtime_views();
    }

    #[cfg(test)]
    pub fn p2p_mine_local_block(&mut self) -> Result<[u8; 48], NodeError> {
        let block = self
            .orchestrator
            .runtime
            .node
            .mine_and_connect_candidate_block(&Miner::new(1))?;
        self.orchestrator
            .sync
            .prime(&self.orchestrator.runtime.node);
        self.refresh_runtime_views();
        Ok(block.header.block_hash())
    }

    fn refresh_runtime_views(&mut self) {
        self.orchestrator.rpc_server.block_count = self.orchestrator.runtime.node.height();
        self.orchestrator.rpc_server.tip_hash = self.orchestrator.runtime.node.tip_hash();
        self.orchestrator.rpc_server.mempool_count = self.orchestrator.runtime.node.mempool_len();
        self.orchestrator.rpc_server.mempool_total_fee_atoms =
            self.orchestrator.runtime.node.mempool_total_fee_atoms();
        self.orchestrator.rpc_server.running = self.orchestrator.runtime.running;
        self.orchestrator.rpc_server.headers_synced =
            self.orchestrator.sync.sync_state().headers_synced;
        self.orchestrator.rpc_server.sync_best_height =
            self.orchestrator.sync.sync_state().best_height;
    }

    pub fn p2p_bootstrap_peers(&mut self, max: usize) -> Vec<String> {
        let now = unix_timestamp();
        let connected_peers = self
            .orchestrator
            .sync
            .connections()
            .peer_snapshots()
            .into_iter()
            .map(|peer| peer.remote_addr)
            .collect::<BTreeSet<_>>();
        let mut candidates = self
            .orchestrator
            .sync
            .connections()
            .address_manager()
            .advertisable_addresses(max.saturating_mul(4));
        let mut scored = Vec::new();

        for address in candidates.drain(..) {
            let remote_addr = format_remote_addr(&address);
            if connected_peers.contains(&remote_addr) {
                continue;
            }
            let health = self.p2p_peer_health(&remote_addr);
            if health
                .as_ref()
                .is_some_and(|record| record.backoff_until_unix > now)
            {
                continue;
            }
            scored.push((address, remote_addr, health));
        }

        scored.sort_by(|left, right| {
            let left_health = left.2.as_ref();
            let right_health = right.2.as_ref();
            right_health
                .map(|record| record.quality_score)
                .cmp(&left_health.map(|record| record.quality_score))
                .then(
                    left_health
                        .map(|record| record.consecutive_failures)
                        .cmp(&right_health.map(|record| record.consecutive_failures)),
                )
                .then(
                    right_health
                        .and_then(|record| record.last_success_unix)
                        .cmp(&left_health.and_then(|record| record.last_success_unix)),
                )
                .then(right.0.last_seen_unix.cmp(&left.0.last_seen_unix))
                .then(left.0.host.cmp(&right.0.host))
                .then(left.0.port.cmp(&right.0.port))
        });

        let mut seen = BTreeSet::new();
        let mut peers = Vec::new();

        for (_, remote_addr, _) in scored {
            if seen.insert(remote_addr.clone()) {
                peers.push(remote_addr);
            }
            if peers.len() >= max {
                return peers;
            }
        }

        peers
    }

    #[doc(hidden)]
    pub fn sandbox_with_node_mut<T>(&mut self, f: impl FnOnce(&mut crate::node::Node) -> T) -> T {
        let result = f(&mut self.orchestrator.runtime.node);
        self.orchestrator
            .sync
            .prime(&self.orchestrator.runtime.node);
        self.refresh_runtime_views();
        result
    }
}

impl NodeService {
    fn seed_peer_graph(&mut self) {
        let network = self.network();
        let public_source = !matches!(network, Network::Regnet);
        let peer_addresses = match self.orchestrator.runtime.node.peer_addresses() {
            Ok(peer_addresses) => peer_addresses,
            Err(err) => {
                let _ = dev::append_log(
                    "p2p",
                    &format!("peer seed load failed network={} error={err}", network.id()),
                );
                return;
            }
        };
        let accepted = match self.orchestrator.sync.seed_peer_addresses(&peer_addresses) {
            Ok(accepted) => accepted,
            Err(err) => {
                let _ = dev::append_log(
                    "p2p",
                    &format!("peer seed failed network={} error={err}", network.id()),
                );
                return;
            }
        };
        if !accepted.is_empty() && public_source {
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "seeded {} persisted peer address(es) into discovery graph",
                    accepted.len()
                ),
            );
        }
    }
}

impl Drop for NodeService {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::acquire_global_test_lock;
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use std::ffi::OsString;
    use std::fs;
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
            "atho-service-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn bootstrap_peers_include_persisted_peer_records() {
        let root = temp_data_dir("bootstrap-peers");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.sandbox_with_node_mut(|node| {
            node.observe_peer("8.8.8.8:9200", 12, 1_700_000_000)
                .expect("peer observation");
        });
        service.p2p_prime();

        let peers = service.p2p_bootstrap_peers(8);
        assert!(peers.iter().any(|peer| peer == "8.8.8.8:9200"));
    }

    #[test]
    fn bootstrap_peers_prefer_healthy_records_and_skip_backed_off_peers() {
        let root = temp_data_dir("bootstrap-health");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.sandbox_with_node_mut(|node| {
            node.observe_peer("9.9.9.9:9200", 12, 1_700_000_000)
                .expect("peer observation");
            node.observe_peer("8.8.8.8:9200", 12, 1_700_000_000)
                .expect("peer observation");
            node.observe_peer("7.7.7.7:9200", 12, 1_700_000_000)
                .expect("peer observation");
        });

        let now = unix_timestamp();
        service.p2p_save_peer_health(&PeerHealthRecord {
            network: Network::Regnet,
            remote_addr: String::from("9.9.9.9:9200"),
            quality_score: 40,
            consecutive_failures: 2,
            backoff_until_unix: 0,
            last_failure_unix: Some(now.saturating_sub(60)),
            last_success_unix: Some(now.saturating_sub(120)),
        });
        service.p2p_save_peer_health(&PeerHealthRecord {
            network: Network::Regnet,
            remote_addr: String::from("8.8.8.8:9200"),
            quality_score: 95,
            consecutive_failures: 0,
            backoff_until_unix: 0,
            last_failure_unix: None,
            last_success_unix: Some(now.saturating_sub(5)),
        });
        service.p2p_save_peer_health(&PeerHealthRecord {
            network: Network::Regnet,
            remote_addr: String::from("7.7.7.7:9200"),
            quality_score: 100,
            consecutive_failures: 8,
            backoff_until_unix: now.saturating_add(3_600),
            last_failure_unix: Some(now),
            last_success_unix: None,
        });

        service.p2p_prime();

        let peers = service.p2p_bootstrap_peers(8);
        assert_eq!(peers.first().map(String::as_str), Some("8.8.8.8:9200"));
        assert!(peers.iter().any(|peer| peer == "9.9.9.9:9200"));
        assert!(!peers.iter().any(|peer| peer == "7.7.7.7:9200"));
    }
}

fn sync_error_into_node(error: NodeSyncError) -> NodeError {
    match error {
        NodeSyncError::Node(error) => error,
        NodeSyncError::Protocol(error) => NodeError::P2pProtocol(error),
        NodeSyncError::Connection(error) => NodeError::P2pConnection(error),
    }
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
