use crate::config::NodeConfig;
use crate::dev;
use crate::error::rpc_error_from_node;
use crate::error::NodeError;
use crate::mempool::MempoolEntry;
#[cfg(test)]
use crate::miner::Miner;
use crate::orchestrator::NodeOrchestrator;
use crate::sync::{NodeSyncError, SyncNotice};
use crate::tcp_p2p::next_outbound_retry_delay;
use crate::wallet_history;
use atho_core::address::decode_base56_address;
use atho_core::block::Block;
use atho_core::consensus::{pow, rules};
use atho_core::crypto::hash::sha3_384;
use atho_core::genesis;
use atho_core::network::Network;
use atho_p2p::address_manager::format_remote_addr;
use atho_p2p::config::network_params;
use atho_p2p::connection::{ConnectionDirection, ConnectionEvent};
use atho_p2p::protocol::NetworkMessage;
use atho_rpc::command::{
    command_definition, help_payload, CommandDefinition, CommandInvocation, CommandResponse,
};
use atho_rpc::request::{RpcRequest, WalletHistoryAddress};
use atho_rpc::response::{
    BlockTemplate, MempoolInfo, MempoolSpentInput, NetworkDiagnostics, NetworkPeerDiagnostics,
    NetworkPeerDirection, NodeStatus, RpcResponse, WalletActivityEntry,
};
use atho_storage::db::PeerHealthRecord;
use atho_wallet::snapshot::WalletSnapshot;
use serde_json::{json, Value};
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
            RpcRequest::ExecuteCommand(invocation) => self.execute_command(invocation),
            RpcRequest::GetBlockTemplate => {
                match self.orchestrator.runtime.node.build_candidate_block() {
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
                RpcResponse::Error(atho_rpc::error::RpcError::invalid_request(
                    "submit requests require mutable RPC handling",
                ))
            }
        }
    }

    pub fn handle_mut(&mut self, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::ExecuteCommand(invocation) => self.execute_command(invocation),
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
                        if let Some(descriptor) = atho_errors::registry_descriptor(&err.code) {
                            let _ = dev::append_atho_error(
                                "athod",
                                &atho_errors::AthoError::new(
                                    descriptor,
                                    "atho-node::service",
                                    err.message.clone(),
                                )
                                .with_safe_details(
                                    err.details.clone().unwrap_or_else(|| tx_summary.clone()),
                                ),
                            );
                        }
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
                        if let Some(descriptor) = atho_errors::registry_descriptor(&err.code) {
                            let _ = dev::append_atho_error(
                                "athod",
                                &atho_errors::AthoError::new(
                                    descriptor,
                                    "atho-node::service",
                                    err.message.clone(),
                                )
                                .with_safe_details(
                                    err.details.clone().unwrap_or_else(|| block_summary.clone()),
                                ),
                            );
                        }
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

    fn execute_command(&self, invocation: CommandInvocation) -> RpcResponse {
        let name = invocation.name.trim();
        let Some(definition) = command_definition(name) else {
            return RpcResponse::Error(atho_rpc::error::RpcError::method_not_found());
        };

        if definition.dangerous && !invocation.confirmed {
            return RpcResponse::Error(atho_rpc::error::RpcError::invalid_request(format!(
                "command {} requires confirmation",
                definition.name
            )));
        }
        if !definition.mainnet_allowed && self.network() == Network::Mainnet {
            return RpcResponse::Error(atho_rpc::error::RpcError::invalid_request(format!(
                "command {} is blocked on mainnet",
                definition.name
            )));
        }
        if definition.test_only && !matches!(self.network(), Network::Regnet | Network::Prunetest) {
            return RpcResponse::Error(atho_rpc::error::RpcError::invalid_request(format!(
                "command {} is only available on test networks",
                definition.name
            )));
        }

        let data = match self.execute_command_data(definition, &invocation.args) {
            Ok(data) => data,
            Err(err) => return RpcResponse::Error(err),
        };
        RpcResponse::Command(CommandResponse {
            command: definition.name.to_string(),
            group: definition.group,
            permission: definition.permission,
            dangerous: definition.dangerous,
            network: self.network().id().to_string(),
            data,
        })
    }

    fn execute_command_data(
        &self,
        definition: &'static CommandDefinition,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        match definition.name {
            "help" => self.command_help(args),
            "getstatus" => self.command_getstatus(args),
            "gethealth" => self.command_gethealth(args),
            "getversion" => self.command_getversion(args),
            "geterrorcodes" => self.command_geterrorcodes(args),
            "getblockcount" => self.command_getblockcount(args),
            "getbestblockhash" => self.command_getbestblockhash(args),
            "getblockhash" => self.command_getblockhash(args),
            "getblock" => self.command_getblock(args),
            "getblockheader" => self.command_getblockheader(args),
            "getblockchaininfo" => self.command_getblockchaininfo(args),
            "getnetworkinfo" => self.command_getnetworkinfo(args),
            "getconnectioncount" => self.command_getconnectioncount(args),
            "getpeerinfo" => self.command_getpeerinfo(args),
            "getmempoolinfo" => self.command_getmempoolinfo(args),
            "getblocktemplate" => self.command_getblocktemplate(args),
            "gettemplateinfo" => self.command_gettemplateinfo(args),
            "getmininginfo" => self.command_getmininginfo(args),
            "getnetworkparams" => self.command_getnetworkparams(args),
            "getgenesisinfo" => self.command_getgenesisinfo(args),
            "validateathoaddress" => self.command_validate_athoaddress(args),
            "sha3_384" => self.command_sha3_384(args),
            _ => Err(atho_rpc::error::RpcError::method_not_found()),
        }
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

    fn command_help(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        if args.len() > 1 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "help accepts at most one query",
            ));
        }
        help_payload(args.first().map(String::as_str))
            .map_err(atho_rpc::error::RpcError::invalid_request)
    }

    fn command_getstatus(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getstatus", args)?;
        Ok(serde_json::to_value(self.node_status()).expect("node status serializes"))
    }

    fn command_gethealth(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("gethealth", args)?;
        let status = self.node_status();
        let mut warnings = Vec::new();
        if !status.running {
            warnings.push("node runtime is not running");
        }
        if !status.headers_synced {
            warnings.push("headers are not fully synced");
        }
        if status.network_diagnostics.peer_count == 0
            && !matches!(status.network, Network::Regnet | Network::Prunetest)
        {
            warnings.push("no peers are connected");
        }
        Ok(json!({
            "network": status.network.id(),
            "running": status.running,
            "synced": status.headers_synced,
            "height": status.block_count,
            "best_block_hash": hex::encode(status.tip_hash),
            "peer_count": status.network_diagnostics.peer_count,
            "mempool_count": status.mempool_count,
            "pruned": false,
            "warning_count": warnings.len(),
            "warnings": warnings,
        }))
    }

    fn command_getversion(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getversion", args)?;
        Ok(json!({
            "client_name": "Atho",
            "version": env!("CARGO_PKG_VERSION"),
            "protocol_version": rules::PROTOCOL_VERSION,
            "storage_schema_version": rules::STORAGE_SCHEMA_VERSION,
            "network": self.network().id(),
        }))
    }

    fn command_geterrorcodes(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("geterrorcodes", args)?;
        let rendered = atho_errors::render_json_registry()
            .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()))?;
        serde_json::from_str(&rendered)
            .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()))
    }

    fn command_getblockcount(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getblockcount", args)?;
        Ok(json!({
            "height": self.orchestrator.runtime.node.height(),
        }))
    }

    fn command_getbestblockhash(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getbestblockhash", args)?;
        Ok(json!({
            "best_block_hash": hex::encode(self.orchestrator.runtime.node.tip_hash()),
        }))
    }

    fn command_getblockhash(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let height = self.parse_single_u64_arg("getblockhash", args, "height")?;
        let block = self.canonical_block_by_height(height)?.ok_or_else(|| {
            atho_rpc::error::RpcError::invalid_request(format!("unknown block height {height}"))
        })?;
        Ok(json!({
            "height": height,
            "block_hash": hex::encode(block.header.block_hash()),
        }))
    }

    fn command_getblock(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let block = self.parse_single_block_arg("getblock", args)?;
        serde_json::to_value(block)
            .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()))
    }

    fn command_getblockheader(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let block = self.parse_single_block_arg("getblockheader", args)?;
        serde_json::to_value(block.header)
            .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()))
    }

    fn command_getblockchaininfo(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getblockchaininfo", args)?;
        let status = self.node_status();
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        let chainwork = pow::accumulated_chain_work(&blocks).to_str_radix(16);
        let tip = blocks
            .last()
            .map(|block| block.header.clone())
            .ok_or_else(|| atho_rpc::error::RpcError::internal())?;
        let ruleset = rules::rules_at_height(status.block_count);
        let verification_progress =
            if status.sync_best_height == 0 || status.block_count >= status.sync_best_height {
                1.0
            } else {
                status.block_count as f64 / status.sync_best_height as f64
            };
        Ok(json!({
            "network": status.network.id(),
            "height": status.block_count,
            "best_block_hash": hex::encode(status.tip_hash),
            "best_block_time": tip.timestamp,
            "difficulty_target": hex::encode(tip.difficulty_target_or_bits),
            "next_target": hex::encode(self.orchestrator.runtime.node.difficulty_target_for_next_block()),
            "chainwork": chainwork,
            "ruleset_id": format!("atho-ruleset-v{}", ruleset.ruleset_version),
            "ruleset_version": ruleset.ruleset_version,
            "genesis_hash": hex::encode(genesis::genesis_hash(status.network)),
            "pruned": false,
            "verification_progress": verification_progress,
        }))
    }

    fn command_getnetworkinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getnetworkinfo", args)?;
        let status = self.node_status();
        let params = network_params(status.network);
        Ok(json!({
            "network": status.network.id(),
            "network_magic": hex::encode(params.magic),
            "default_port": params.default_port,
            "protocol_version": params.protocol_version,
            "min_supported_protocol_version": params.min_supported_protocol_version,
            "peer_count": status.network_diagnostics.peer_count,
            "inbound_peer_count": status.network_diagnostics.inbound_peer_count,
            "outbound_peer_count": status.network_diagnostics.outbound_peer_count,
            "bytes_sent": status.network_diagnostics.bytes_sent,
            "bytes_received": status.network_diagnostics.bytes_received,
            "dns_seeds": params.dns_seeds,
            "network_active": status.running,
        }))
    }

    fn command_getconnectioncount(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getconnectioncount", args)?;
        Ok(json!({
            "connection_count": self.network_diagnostics().peer_count,
        }))
    }

    fn command_getpeerinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getpeerinfo", args)?;
        serde_json::to_value(self.network_diagnostics().peers)
            .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()))
    }

    fn command_getmempoolinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getmempoolinfo", args)?;
        Ok(json!({
            "transaction_count": self.orchestrator.runtime.node.mempool_len(),
            "total_fee_atoms": self.orchestrator.runtime.node.mempool_total_fee_atoms(),
            "spent_inputs_count": self.orchestrator.runtime.node.mempool_spent_inputs().len(),
            "dust_relay_value_atoms": 50u64,
            "min_fee_rate_atoms_per_vbyte": atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS,
        }))
    }

    fn command_getblocktemplate(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getblocktemplate", args)?;
        let block = self
            .orchestrator
            .runtime
            .node
            .build_candidate_block()
            .map_err(rpc_error_from_node)?;
        let template = self.block_template(block);
        serde_json::to_value(template)
            .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()))
    }

    fn command_gettemplateinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("gettemplateinfo", args)?;
        let block = self
            .orchestrator
            .runtime
            .node
            .build_candidate_block()
            .map_err(rpc_error_from_node)?;
        let template = self.block_template(block.clone());
        Ok(json!({
            "network": template.network.id(),
            "height": template.height,
            "previous_block_hash": hex::encode(template.previous_block_hash),
            "target": hex::encode(template.target),
            "transaction_count": template.transaction_count,
            "fees_atoms": template.fees_atoms,
            "header_bytes_without_nonce": hex::encode(template.header_bytes_without_nonce()),
            "nonce_offset_bytes": template.nonce_offset_bytes(),
            "coinbase_txid": hex::encode(block.transactions.first().map(|tx| tx.txid()).unwrap_or([0; 48])),
        }))
    }

    fn command_getmininginfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getmininginfo", args)?;
        let node = &self.orchestrator.runtime.node;
        Ok(json!({
            "network": self.network().id(),
            "height": node.height(),
            "best_block_hash": hex::encode(node.tip_hash()),
            "next_target": hex::encode(node.difficulty_target_for_next_block()),
            "mempool_transaction_count": node.mempool_len(),
            "mempool_total_fee_atoms": node.mempool_total_fee_atoms(),
            "headers_synced": self.orchestrator.sync.sync_state().headers_synced,
        }))
    }

    fn command_getnetworkparams(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getnetworkparams", args)?;
        let network = self.network();
        let params = network_params(network);
        Ok(json!({
            "network": network.id(),
            "consensus_id": network.consensus_id(),
            "domain_tag": network.domain_tag(),
            "p2p_port": network.p2p_port(),
            "rpc_port": network.rpc_port(),
            "visible_prefix": network.visible_prefix().to_string(),
            "internal_hpk_prefix": network.internal_hpk_prefix(),
            "utxo_flag": network.utxo_flag(),
            "magic": hex::encode(params.magic),
            "protocol_version": params.protocol_version,
            "min_supported_protocol_version": params.min_supported_protocol_version,
            "dns_seeds": params.dns_seeds,
        }))
    }

    fn command_getgenesisinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getgenesisinfo", args)?;
        let state = genesis::genesis_state(self.network());
        Ok(json!({
            "network": state.network.id(),
            "genesis_hash": hex::encode(state.block_hash),
            "coinbase_txid": hex::encode(state.coinbase_txid),
            "reward_address": state.reward_address,
            "utxo_flag": state.utxo_flag,
            "timestamp": state.block.header.timestamp,
            "target": hex::encode(state.block.header.difficulty_target_or_bits),
            "nonce": state.block.header.nonce,
        }))
    }

    fn command_validate_athoaddress(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        let address = self.parse_single_string_arg("validateathoaddress", args, "address")?;
        match decode_base56_address(&address) {
            Ok((digest, network)) => Ok(json!({
                "is_valid": true,
                "address": address,
                "network": network.id(),
                "visible_prefix": network.visible_prefix().to_string(),
                "payment_digest": hex::encode(digest),
                "matches_active_network": network == self.network(),
            })),
            Err(err) => Ok(json!({
                "is_valid": false,
                "address": address,
                "error": err.to_string(),
                "active_network": self.network().id(),
            })),
        }
    }

    fn command_sha3_384(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let input = self.parse_single_string_arg("sha3_384", args, "input")?;
        let (input_format, bytes) = if let Some(hex_input) = input.strip_prefix("0x") {
            (
                "hex",
                hex::decode(hex_input).map_err(|_| {
                    atho_rpc::error::RpcError::invalid_request(
                        "sha3_384 expected valid hex after 0x prefix",
                    )
                })?,
            )
        } else {
            ("utf8", input.as_bytes().to_vec())
        };
        Ok(json!({
            "input_format": input_format,
            "digest": hex::encode(sha3_384(&bytes)),
        }))
    }

    fn expect_no_args(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<(), atho_rpc::error::RpcError> {
        if args.is_empty() {
            Ok(())
        } else {
            Err(atho_rpc::error::RpcError::invalid_request(format!(
                "{command} does not accept arguments"
            )))
        }
    }

    fn parse_single_string_arg(
        &self,
        command: &str,
        args: &[String],
        label: &str,
    ) -> Result<String, atho_rpc::error::RpcError> {
        if args.len() != 1 {
            return Err(atho_rpc::error::RpcError::invalid_request(format!(
                "{command} expects exactly one {label} argument"
            )));
        }
        Ok(args[0].clone())
    }

    fn parse_single_u64_arg(
        &self,
        command: &str,
        args: &[String],
        label: &str,
    ) -> Result<u64, atho_rpc::error::RpcError> {
        let value = self.parse_single_string_arg(command, args, label)?;
        value.parse::<u64>().map_err(|_| {
            atho_rpc::error::RpcError::invalid_request(format!(
                "{command} expected {label} as unsigned integer"
            ))
        })
    }

    fn parse_single_block_arg(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<Block, atho_rpc::error::RpcError> {
        let value = self.parse_single_string_arg(command, args, "hash|height")?;
        if let Ok(height) = value.parse::<u64>() {
            return self.canonical_block_by_height(height)?.ok_or_else(|| {
                atho_rpc::error::RpcError::invalid_request(format!("unknown block height {height}"))
            });
        }
        let hash = parse_hash48(&value)?;
        self.orchestrator
            .runtime
            .node
            .block_by_hash(hash)
            .ok_or_else(|| {
                atho_rpc::error::RpcError::invalid_request(format!("unknown block hash {value}"))
            })
    }

    fn canonical_block_by_height(
        &self,
        height: u64,
    ) -> Result<Option<Block>, atho_rpc::error::RpcError> {
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        Ok(blocks
            .into_iter()
            .find(|block| block.header.height == height))
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
    use atho_core::address::encode_base56_address;
    use atho_rpc::request::RpcRequest;
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

    #[test]
    fn block_template_exposes_canonical_header_bytes_for_miners() {
        let root = temp_data_dir("block-template");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let service = NodeService::new(NodeConfig::new(Network::Regnet));
        let response = service.handle(RpcRequest::GetBlockTemplate);
        let RpcResponse::BlockTemplate(template) = response else {
            panic!("unexpected response: {response:?}");
        };

        assert_eq!(
            template.header_bytes_without_nonce(),
            template.block.header.canonical_bytes_without_nonce()
        );
        assert_eq!(
            template.nonce_offset_bytes(),
            template.block.header.canonical_size_bytes_without_nonce()
        );
        assert_eq!(
            template.transaction_count,
            template.block.transactions.len()
        );
        assert_eq!(template.fees_atoms, template.block.fees_total_atoms);
    }

    #[test]
    fn execute_command_help_returns_registry_data() {
        let root = temp_data_dir("command-help");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let service = NodeService::new(NodeConfig::new(Network::Regnet));
        let response = service.handle(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "help",
            Vec::new(),
        )));
        let RpcResponse::Command(command) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(command.command, "help");
        assert!(command.data["count"].as_u64().unwrap_or_default() > 0);
        assert!(command.data["groups"].is_object());
    }

    #[test]
    fn execute_command_getblockchaininfo_reports_active_network() {
        let root = temp_data_dir("command-blockchaininfo");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let service = NodeService::new(NodeConfig::new(Network::Regnet));
        let response = service.handle(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "getblockchaininfo",
            Vec::new(),
        )));
        let RpcResponse::Command(command) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(command.command, "getblockchaininfo");
        assert_eq!(command.data["network"], "atho-regnet");
        assert_eq!(command.data["height"], 0);
    }

    #[test]
    fn execute_command_validate_address_reports_matching_network() {
        let root = temp_data_dir("command-validate-address");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let service = NodeService::new(NodeConfig::new(Network::Regnet));
        let address = encode_base56_address(Network::Regnet, &[7u8; 32]);
        let response = service.handle(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "validateathoaddress",
            vec![address.clone()],
        )));
        let RpcResponse::Command(command) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(command.command, "validateathoaddress");
        assert_eq!(command.data["is_valid"], true);
        assert_eq!(command.data["address"], address);
        assert_eq!(command.data["matches_active_network"], true);
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

fn parse_hash48(value: &str) -> Result<[u8; 48], atho_rpc::error::RpcError> {
    let bytes = hex::decode(value)
        .map_err(|_| atho_rpc::error::RpcError::invalid_request("expected 48-byte hex hash"))?;
    if bytes.len() != 48 {
        return Err(atho_rpc::error::RpcError::invalid_request(
            "expected 48-byte hex hash",
        ));
    }
    let mut hash = [0u8; 48];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}
