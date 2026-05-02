//! Node service layer for RPC, CLI, and GUI command execution.
//!
//! This module converts high-level operator requests into validated node
//! actions, gathers diagnostics, and formats human-readable command results.
//!
//! SECURITY: External callers never mutate chainstate directly. Every write path
//! still flows through the node's validated transaction or block submission APIs.
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
use atho_core::address::{decode_base56_address, encode_base56_address};
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::{pow, rules, subsidy};
use atho_core::crypto::hash::sha3_384;
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput};
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
use atho_storage::utxo::{UtxoEntry, UtxoSet};
use atho_wallet::snapshot::WalletSnapshot;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Snapshot of the local node state used by operator interfaces.
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

/// Mutable service façade around a running [`NodeOrchestrator`].
#[derive(Debug)]
pub struct NodeService {
    orchestrator: NodeOrchestrator,
    wallet_snapshot: WalletSnapshot,
    network_runtime: NetworkRuntimeView,
    peer_health_cache: BTreeMap<String, PeerHealthRecord>,
}

impl NodeService {
    /// Creates a new service with a fresh orchestrator.
    pub fn new(config: NodeConfig) -> Self {
        Self {
            orchestrator: NodeOrchestrator::new(config),
            wallet_snapshot: WalletSnapshot::default(),
            network_runtime: NetworkRuntimeView::default(),
            peer_health_cache: BTreeMap::new(),
        }
    }

    /// Creates an ephemeral service for tests and local tooling.
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

    /// Starts the orchestrator and seeds the initial peer graph view.
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

    /// Handles read-only RPC requests.
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

    /// Handles mutable RPC requests that may update node state.
    pub fn handle_mut(&mut self, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::ExecuteCommand(invocation) => self.execute_command_mut(invocation),
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
        let definition = match self.prepare_command_execution(&invocation) {
            Ok(definition) => definition,
            Err(err) => return RpcResponse::Error(err),
        };
        if definition.permission.requires_mutable_access() {
            return RpcResponse::Error(atho_rpc::error::RpcError::invalid_request(format!(
                "command {} requires mutable RPC handling",
                definition.name
            )));
        }
        let data = match self.execute_command_data(definition, &invocation.args) {
            Ok(data) => data,
            Err(err) => return RpcResponse::Error(err),
        };
        self.command_response(definition, data)
    }

    fn execute_command_mut(&mut self, invocation: CommandInvocation) -> RpcResponse {
        let definition = match self.prepare_command_execution(&invocation) {
            Ok(definition) => definition,
            Err(err) => return RpcResponse::Error(err),
        };
        let data = if definition.permission.requires_mutable_access() {
            match self.execute_command_data_mut(definition, &invocation.args) {
                Ok(data) => data,
                Err(err) => return RpcResponse::Error(err),
            }
        } else {
            match self.execute_command_data(definition, &invocation.args) {
                Ok(data) => data,
                Err(err) => return RpcResponse::Error(err),
            }
        };
        self.command_response(definition, data)
    }

    fn command_response(&self, definition: &'static CommandDefinition, data: Value) -> RpcResponse {
        RpcResponse::Command(CommandResponse {
            command: definition.name.to_string(),
            group: definition.group,
            permission: definition.permission,
            dangerous: definition.dangerous,
            network: self.network().id().to_string(),
            data,
        })
    }

    fn prepare_command_execution(
        &self,
        invocation: &CommandInvocation,
    ) -> Result<&'static CommandDefinition, atho_rpc::error::RpcError> {
        let name = invocation.name.trim();
        let Some(definition) = command_definition(name) else {
            return Err(atho_rpc::error::RpcError::method_not_found());
        };

        if definition.dangerous && !invocation.confirmed {
            return Err(atho_rpc::error::RpcError::invalid_request(format!(
                "command {} requires confirmation",
                definition.name
            )));
        }
        if !definition.mainnet_allowed && self.network() == Network::Mainnet {
            return Err(atho_rpc::error::RpcError::invalid_request(format!(
                "command {} is blocked on mainnet",
                definition.name
            )));
        }
        if definition.test_only && !matches!(self.network(), Network::Regnet | Network::Prunetest) {
            return Err(atho_rpc::error::RpcError::invalid_request(format!(
                "command {} is only available on test networks",
                definition.name
            )));
        }
        Ok(definition)
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
            "getrpcinfo" => self.command_getrpcinfo(args),
            "getmemoryinfo" => self.command_getmemoryinfo(args),
            "uptime" => self.command_uptime(args),
            "getblockcount" => self.command_getblockcount(args),
            "getbestblockhash" => self.command_getbestblockhash(args),
            "getblockhash" => self.command_getblockhash(args),
            "getblock" => self.command_getblock(args),
            "getblockheader" => self.command_getblockheader(args),
            "getblockchaininfo" => self.command_getblockchaininfo(args),
            "getblockstats" => self.command_getblockstats(args),
            "getchaintips" => self.command_getchaintips(args),
            "getchaintxstats" => self.command_getchaintxstats(args),
            "getdifficulty" => self.command_getdifficulty(args),
            "gettxout" => self.command_gettxout(args),
            "gettxoutsetinfo" => self.command_gettxoutsetinfo(args),
            "verifychain" => self.command_verifychain(args),
            "getchainwork" => self.command_getchainwork(args),
            "getrulesetinfo" => self.command_getrulesetinfo(args),
            "getconsensusstatus" => self.command_getconsensusstatus(args),
            "getnetworkinfo" => self.command_getnetworkinfo(args),
            "getconnectioncount" => self.command_getconnectioncount(args),
            "getnettotals" => self.command_getnettotals(args),
            "getnodeaddresses" => self.command_getnodeaddresses(args),
            "getaddednodeinfo" => self.command_getaddednodeinfo(args),
            "getpeerinfo" => self.command_getpeerinfo(args),
            "getmempoolinfo" => self.command_getmempoolinfo(args),
            "getrawmempool" => self.command_getrawmempool(args),
            "getmempoolentry" => self.command_getmempoolentry(args),
            "getmempoolancestors" => self.command_getmempoolancestors(args),
            "getmempooldescendants" => self.command_getmempooldescendants(args),
            "getblocktemplate" => self.command_getblocktemplate(args),
            "gettemplateinfo" => self.command_gettemplateinfo(args),
            "getmininginfo" => self.command_getmininginfo(args),
            "getnetworkhashps" => self.command_getnetworkhashps(args),
            "getnetworkparams" => self.command_getnetworkparams(args),
            "getgenesisinfo" => self.command_getgenesisinfo(args),
            "getrawtransaction" => self.command_getrawtransaction(args),
            "validateathoaddress" | "validateaddress" => self.command_validate_athoaddress(args),
            "sha3_384" => self.command_sha3_384(args),
            _ => Err(atho_rpc::error::RpcError::method_not_found()),
        }
    }

    fn execute_command_data_mut(
        &mut self,
        definition: &'static CommandDefinition,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        match definition.name {
            "stop" => self.command_stop(args),
            "addnode" => self.command_addnode(args),
            "disconnectnode" => self.command_disconnectnode(args),
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
        let tip_timestamp = self
            .orchestrator
            .runtime
            .node
            .block_record_by_height(status.block_count)
            .map(|record| record.timestamp)
            .or_else(|| {
                self.orchestrator
                    .runtime
                    .node
                    .blocks()
                    .last()
                    .map(|block| block.header.timestamp)
            })
            .unwrap_or_default();
        NodeStatus {
            network: status.network,
            block_count: status.block_count,
            tip_hash: status.tip_hash,
            tip_timestamp,
            mempool_count: status.mempool_count,
            mempool_total_fee_atoms: status.mempool_total_fee_atoms,
            mempool_fingerprint: self.orchestrator.runtime.node.mempool_fingerprint(),
            running: status.running,
            headers_synced: status.headers_synced,
            sync_best_height: status.sync_best_height,
            network_diagnostics: self.network_diagnostics(),
        }
    }

    fn chain_synced(status: &NodeStatus) -> bool {
        status.running && status.headers_synced && status.block_count >= status.sync_best_height
    }

    fn render_status_value(status: &NodeStatus) -> Value {
        json!({
            "network": status.network.id(),
            "running": status.running,
            "local_height": status.block_count,
            "block_count": status.block_count,
            "sync_target_height": status.sync_best_height,
            "sync_best_height": status.sync_best_height,
            "headers_synced": status.headers_synced,
            "chain_synced": Self::chain_synced(status),
            "tip_hash": hex::encode(status.tip_hash),
            "tip_timestamp": status.tip_timestamp,
            "peer_count": status.network_diagnostics.peer_count,
            "connecting_peer_count": status.network_diagnostics.connecting_peer_count,
            "mempool_count": status.mempool_count,
            "mempool_total_fee_atoms": status.mempool_total_fee_atoms,
        })
    }

    fn health_warnings(status: &NodeStatus) -> Vec<&'static str> {
        let mut warnings = Vec::new();
        let chain_synced = Self::chain_synced(status);
        if !status.running {
            warnings.push("node runtime is not running");
        }
        if !status.headers_synced {
            warnings.push("headers are not fully synced");
        } else if !chain_synced {
            warnings.push("local chain tip is behind the advertised network target");
        }
        if status.network_diagnostics.peer_count == 0
            && !matches!(status.network, Network::Regnet | Network::Prunetest)
        {
            warnings.push("no peers are connected");
        }
        warnings
    }

    pub fn network_diagnostics(&self) -> NetworkDiagnostics {
        let connections = self.orchestrator.sync.connections();
        let (peers, connecting_peers): (Vec<_>, Vec<_>) = connections
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
            .partition(|peer| peer.handshake_ready);

        let peer_count = peers.len();
        let inbound_peer_count = peers
            .iter()
            .filter(|peer| peer.direction == NetworkPeerDirection::Inbound)
            .count();
        let outbound_peer_count = peers
            .iter()
            .filter(|peer| peer.direction == NetworkPeerDirection::Outbound)
            .count();

        NetworkDiagnostics {
            peer_count,
            inbound_peer_count,
            outbound_peer_count,
            connecting_peer_count: connecting_peers.len(),
            bytes_sent: self.network_runtime.bytes_sent,
            bytes_received: self.network_runtime.bytes_received,
            peers,
            connecting_peers,
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
        Ok(Self::render_status_value(&self.node_status()))
    }

    fn command_gethealth(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("gethealth", args)?;
        let status = self.node_status();
        let chain_synced = Self::chain_synced(&status);
        let warnings = Self::health_warnings(&status);
        Ok(json!({
            "network": status.network.id(),
            "running": status.running,
            "synced": chain_synced,
            "headers_synced": status.headers_synced,
            "height": status.block_count,
            "sync_target_height": status.sync_best_height,
            "best_block_hash": hex::encode(status.tip_hash),
            "best_block_time": status.tip_timestamp,
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

    fn command_getrpcinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getrpcinfo", args)?;
        Ok(json!({
            "network": self.network().id(),
            "rpc_bind_address": crate::runtime::rpc_bind_address(self.network()),
            "public_rpc_enabled": std::env::var("ATHO_RPC_ALLOW_PUBLIC").ok().as_deref() == Some("1"),
            "command_count": atho_rpc::command::COMMANDS.len(),
            "transport": "local_tcp",
        }))
    }

    fn command_getmemoryinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getmemoryinfo", args)?;
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        let mempool_entries = self.orchestrator.runtime.node.mempool_entries();
        let mempool_bytes: usize = mempool_entries
            .iter()
            .map(MempoolEntry::full_size_bytes)
            .sum();
        let chain_bytes: usize = blocks.iter().map(Block::full_size_bytes).sum();
        Ok(json!({
            "network": self.network().id(),
            "mempool_transactions": mempool_entries.len(),
            "mempool_estimated_bytes": mempool_bytes,
            "canonical_blocks": blocks.len(),
            "canonical_chain_estimated_bytes": chain_bytes,
            "utxo_count": self.orchestrator.runtime.node.utxo_count(),
            "peer_count": self.network_diagnostics().peer_count,
        }))
    }

    fn command_uptime(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("uptime", args)?;
        let now = unix_timestamp();
        let started_at = self.orchestrator.runtime.started_at_unix.unwrap_or(now);
        Ok(json!({
            "network": self.network().id(),
            "running": self.orchestrator.runtime.running,
            "started_at_unix": self.orchestrator.runtime.started_at_unix,
            "uptime_seconds": if self.orchestrator.runtime.running {
                now.saturating_sub(started_at)
            } else {
                0
            },
        }))
    }

    fn command_stop(&mut self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("stop", args)?;
        let network = self.network().id().to_string();
        let height = self.orchestrator.runtime.node.height();
        self.stop();
        Ok(json!({
            "stopping": true,
            "network": network,
            "height": height,
        }))
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
        let block = self.block_record_by_height(height)?.ok_or_else(|| {
            atho_rpc::error::RpcError::invalid_request(format!("unknown block height {height}"))
        })?;
        Ok(json!({
            "height": height,
            "block_hash": hex::encode(block.block_hash),
        }))
    }

    fn command_getblock(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let block = self.parse_single_block_arg("getblock", args)?;
        Ok(render_block_value(&block))
    }

    fn command_getblockheader(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let block = self.parse_single_block_arg("getblockheader", args)?;
        Ok(render_block_header_value(&block.header))
    }

    fn command_getblockchaininfo(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getblockchaininfo", args)?;
        let status = self.node_status();
        let tip = self
            .block_record_by_height(status.block_count)?
            .ok_or_else(atho_rpc::error::RpcError::internal)?;
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
            "chainwork": render_chainwork_hex(&tip.chainwork),
            "ruleset_id": format!("atho-ruleset-v{}", ruleset.ruleset_version),
            "ruleset_version": ruleset.ruleset_version,
            "genesis_hash": hex::encode(genesis::genesis_hash(status.network)),
            "pruned": self.orchestrator.runtime.node.has_pruned_history(),
            "prune_depth_blocks": self.orchestrator.runtime.node.prune_depth(),
            "tip_raw_block_pruned": tip.pruned,
            "verification_progress": verification_progress,
        }))
    }

    fn command_getblockstats(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let block = self.parse_single_block_arg("getblockstats", args)?;
        let transaction_count = block.transactions.len();
        let input_count: usize = block.transactions.iter().map(|tx| tx.inputs.len()).sum();
        let output_count: usize = block.transactions.iter().map(|tx| tx.outputs.len()).sum();
        let total_output_atoms: u64 = block
            .transactions
            .iter()
            .flat_map(|tx| tx.outputs.iter())
            .map(|output| output.value_atoms)
            .sum();
        Ok(json!({
            "hash": hex::encode(block.header.block_hash()),
            "height": block.header.height,
            "time": block.header.timestamp,
            "txs": transaction_count,
            "inputs": input_count,
            "outputs": output_count,
            "size_bytes": block.size_bytes(),
            "weight_bytes": block.weight_bytes(),
            "vsize_bytes": block.vsize_bytes(),
            "subsidy_atoms": subsidy::block_subsidy_atoms(block.header.height),
            "fees_atoms": block.fees_total_atoms,
            "total_output_atoms": total_output_atoms,
            "avg_tx_size_bytes": if transaction_count == 0 {
                0
            } else {
                block.size_bytes() / transaction_count
            },
        }))
    }

    fn command_getchaintips(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getchaintips", args)?;
        let block = self
            .block_record_by_height(self.orchestrator.runtime.node.height())?
            .ok_or_else(atho_rpc::error::RpcError::internal)?;
        Ok(json!([{
            "height": block.height,
            "hash": hex::encode(block.block_hash),
            "branchlen": 0,
            "status": "active",
        }]))
    }

    fn command_getchaintxstats(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        if args.len() > 2 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "getchaintxstats accepts at most [nblocks] [blockhash]",
            ));
        }
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        let Some(mut end_index) = blocks.len().checked_sub(1) else {
            return Err(atho_rpc::error::RpcError::internal());
        };
        let mut window_blocks = blocks.len().saturating_sub(1).min(30);
        if let Some(value) = args.first() {
            window_blocks = value.parse::<usize>().map_err(|_| {
                atho_rpc::error::RpcError::invalid_request(
                    "getchaintxstats expected nblocks as unsigned integer",
                )
            })?;
        }
        if let Some(value) = args.get(1) {
            let hash = parse_hash48(value)?;
            end_index = blocks
                .iter()
                .position(|block| block.header.block_hash() == hash)
                .ok_or_else(|| {
                    atho_rpc::error::RpcError::invalid_request(format!(
                        "unknown block hash {value}"
                    ))
                })?;
        }
        let end_height = blocks[end_index].header.height;
        let txcount: usize = blocks[..=end_index]
            .iter()
            .map(|block| block.transactions.len())
            .sum();
        let start_index = end_index.saturating_sub(window_blocks.saturating_sub(1));
        let window = &blocks[start_index..=end_index];
        let window_tx_count: usize = window.iter().map(|block| block.transactions.len()).sum();
        let window_interval = window
            .last()
            .map(|block| block.header.timestamp)
            .unwrap_or_default()
            .saturating_sub(
                window
                    .first()
                    .map(|block| block.header.timestamp)
                    .unwrap_or_default(),
            );
        Ok(json!({
            "time": blocks[end_index].header.timestamp,
            "txcount": txcount,
            "window_final_block_hash": hex::encode(blocks[end_index].header.block_hash()),
            "window_final_block_height": end_height,
            "window_block_count": window.len(),
            "window_tx_count": window_tx_count,
            "window_interval_seconds": window_interval,
            "txrate": if window_interval == 0 {
                0.0
            } else {
                window_tx_count as f64 / window_interval as f64
            },
        }))
    }

    fn command_getdifficulty(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getdifficulty", args)?;
        let tip = self
            .block_record_by_height(self.orchestrator.runtime.node.height())?
            .ok_or_else(atho_rpc::error::RpcError::internal)?;
        let scaled = pow::difficulty_ratio_scaled(&tip.difficulty_target_or_bits, 100_000_000);
        Ok(json!({
            "network": self.network().id(),
            "height": tip.height,
            "target": hex::encode(tip.difficulty_target_or_bits),
            "next_target": hex::encode(self.orchestrator.runtime.node.difficulty_target_for_next_block()),
            "difficulty": format_scaled_decimal(scaled, 8),
        }))
    }

    fn command_gettxout(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "gettxout expects <txid> <vout> [include_mempool]",
            ));
        }
        let txid = parse_hash48(&args[0])?;
        let output_index = args[1].parse::<u32>().map_err(|_| {
            atho_rpc::error::RpcError::invalid_request("gettxout expected vout as unsigned integer")
        })?;
        let include_mempool = self.parse_optional_bool_arg(args.get(2), true)?;
        let entry = self
            .orchestrator
            .runtime
            .node
            .utxo_entry(txid, output_index);
        let Some(entry) = entry else {
            return Ok(Value::Null);
        };
        let spent_in_mempool = include_mempool
            && self
                .orchestrator
                .runtime
                .node
                .mempool_spent_inputs()
                .into_iter()
                .any(|(spent_txid, spent_index)| spent_txid == txid && spent_index == output_index);
        if spent_in_mempool {
            return Ok(Value::Null);
        }
        Ok(render_utxo_value(
            self.orchestrator.runtime.node.height(),
            &entry,
        ))
    }

    fn command_gettxoutsetinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("gettxoutsetinfo", args)?;
        let snapshot = self.orchestrator.runtime.node.utxo_snapshot();
        let entries: Vec<UtxoEntry> = snapshot.entries().cloned().collect();
        let total_amount_atoms: u64 = entries.iter().map(|entry| entry.value_atoms).sum();
        Ok(json!({
            "height": self.orchestrator.runtime.node.height(),
            "best_block_hash": hex::encode(self.orchestrator.runtime.node.tip_hash()),
            "txouts": entries.len(),
            "bogosize": entries.iter().map(|entry| 48usize + 4 + 8 + entry.locking_script.len()).sum::<usize>(),
            "total_amount_atoms": total_amount_atoms,
            "total_amount_atho": format_atoms_decimal(total_amount_atoms),
            "utxo_set_hash": hex::encode(utxo_set_hash(&entries)),
        }))
    }

    fn command_verifychain(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        if args.len() > 2 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "verifychain accepts optional [checklevel] [nblocks]",
            ));
        }
        let requested_nblocks = match args.get(1) {
            Some(value) => Some(value.parse::<usize>().map_err(|_| {
                atho_rpc::error::RpcError::invalid_request(
                    "verifychain expected nblocks as unsigned integer",
                )
            })?),
            None => None,
        };
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        let checked =
            verify_canonical_chain(self.network(), &blocks).map_err(rpc_error_from_node)?;
        Ok(json!({
            "verified": true,
            "height": self.orchestrator.runtime.node.height(),
            "blocks_checked": requested_nblocks.unwrap_or(checked).min(checked),
            "canonical_blocks": checked,
        }))
    }

    fn command_getchainwork(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getchainwork", args)?;
        let tip = self
            .block_record_by_height(self.orchestrator.runtime.node.height())?
            .ok_or_else(atho_rpc::error::RpcError::internal)?;
        Ok(json!({
            "height": self.orchestrator.runtime.node.height(),
            "chainwork": render_chainwork_hex(&tip.chainwork),
        }))
    }

    fn command_getrulesetinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getrulesetinfo", args)?;
        let active = rules::rules_at_height(self.orchestrator.runtime.node.height());
        Ok(json!({
            "network": self.network().id(),
            "active": {
                "protocol_version": active.protocol_version,
                "ruleset_version": active.ruleset_version,
                "block_version": active.block_version,
                "transaction_version": active.transaction_version,
                "activation_height": active.activation_height,
            },
            "scheduled": rules::SCHEDULED_ACTIVATIONS.iter().map(|activation| json!({
                "name": activation.name,
                "ruleset_version": activation.ruleset_version,
                "block_version": activation.block_version,
                "transaction_version": activation.transaction_version,
                "activation_height": activation.activation_height,
            })).collect::<Vec<_>>(),
        }))
    }

    fn command_getconsensusstatus(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getconsensusstatus", args)?;
        let height = self.orchestrator.runtime.node.height();
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        let ruleset = rules::rules_at_height(height);
        let next_target = self
            .orchestrator
            .runtime
            .node
            .difficulty_target_for_next_block();
        Ok(json!({
            "network": self.network().id(),
            "height": height,
            "best_block_hash": hex::encode(self.orchestrator.runtime.node.tip_hash()),
            "chainwork": pow::accumulated_chain_work(&blocks).to_str_radix(16),
            "ruleset_version": ruleset.ruleset_version,
            "protocol_version": ruleset.protocol_version,
            "block_version": ruleset.block_version,
            "transaction_version": ruleset.transaction_version,
            "next_target": hex::encode(next_target),
            "difficulty": format_scaled_decimal(pow::difficulty_ratio_scaled(&next_target, 100_000_000), 8),
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
            "connecting_peer_count": status.network_diagnostics.connecting_peer_count,
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

    fn command_getnettotals(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getnettotals", args)?;
        let diagnostics = self.network_diagnostics();
        Ok(json!({
            "totalbytesrecv": diagnostics.bytes_received,
            "totalbytessent": diagnostics.bytes_sent,
            "timemillis": unix_timestamp().saturating_mul(1_000),
        }))
    }

    fn command_getnodeaddresses(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        if args.len() > 1 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "getnodeaddresses accepts at most one count argument",
            ));
        }
        let limit = match args.first() {
            Some(value) => Some(value.parse::<usize>().map_err(|_| {
                atho_rpc::error::RpcError::invalid_request(
                    "getnodeaddresses expected count as unsigned integer",
                )
            })?),
            None => None,
        };
        let mut peers = self
            .orchestrator
            .runtime
            .node
            .peer_addresses()
            .map_err(rpc_error_from_node)?
            .into_iter()
            .map(|address| {
                json!({
                    "address": format_remote_addr(&address),
                    "host": address.host,
                    "port": address.port,
                    "services": address.services,
                    "last_seen_unix": address.last_seen_unix,
                })
            })
            .collect::<Vec<_>>();
        if let Some(limit) = limit {
            peers.truncate(limit);
        }
        Ok(Value::Array(peers))
    }

    fn command_getaddednodeinfo(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getaddednodeinfo", args)?;
        let peers = self
            .orchestrator
            .sync
            .connections()
            .address_manager()
            .manual_peers()
            .into_iter()
            .map(|peer| {
                json!({
                    "address": peer,
                    "connected": self.p2p_has_peer(&peer),
                })
            })
            .collect::<Vec<_>>();
        Ok(Value::Array(peers))
    }

    fn command_getpeerinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getpeerinfo", args)?;
        let diagnostics = self.network_diagnostics();
        serde_json::to_value(json!({
            "connected_peers": diagnostics.peers,
            "connecting_peers": diagnostics.connecting_peers,
        }))
        .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()))
    }

    fn command_getmempoolinfo(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getmempoolinfo", args)?;
        Ok(json!({
            "transaction_count": self.orchestrator.runtime.node.mempool_len(),
            "total_fee_atoms": self.orchestrator.runtime.node.mempool_total_fee_atoms(),
            "spent_inputs_count": self.orchestrator.runtime.node.mempool_spent_inputs().len(),
            "dust_relay_value_atoms": atho_core::constants::DUST_RELAY_VALUE_ATOMS,
            "min_fee_rate_atoms_per_vbyte": atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS,
        }))
    }

    fn command_getrawmempool(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        if args.len() > 1 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "getrawmempool accepts at most one [verbose] argument",
            ));
        }
        let verbose = self.parse_optional_bool_arg(args.first(), false)?;
        let entries = self.orchestrator.runtime.node.mempool_entries();
        if !verbose {
            return Ok(Value::Array(
                entries
                    .into_iter()
                    .map(|entry| Value::String(hex::encode(entry.txid())))
                    .collect(),
            ));
        }
        let relation_entries = entries.clone();
        let map = entries
            .into_iter()
            .map(|entry| {
                (
                    hex::encode(entry.txid()),
                    render_mempool_entry_value(
                        &entry,
                        &mempool_dependencies(entry.txid(), &relation_entries),
                        &mempool_descendants(entry.txid(), &relation_entries),
                    ),
                )
            })
            .collect::<serde_json::Map<String, Value>>();
        Ok(Value::Object(map))
    }

    fn command_getmempoolentry(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let txid = parse_hash48(&self.parse_single_string_arg("getmempoolentry", args, "txid")?)?;
        let entries = self.orchestrator.runtime.node.mempool_entries();
        let entry = entries
            .iter()
            .find(|entry| entry.txid() == txid)
            .cloned()
            .ok_or_else(|| {
                atho_rpc::error::RpcError::invalid_request("transaction is not in the mempool")
            })?;
        let depends = mempool_dependencies(txid, &entries);
        let descendants = mempool_descendants(txid, &entries);
        Ok(render_mempool_entry_value(&entry, &depends, &descendants))
    }

    fn command_getmempoolancestors(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        if args.is_empty() || args.len() > 2 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "getmempoolancestors expects <txid> [verbose]",
            ));
        }
        let txid = parse_hash48(&args[0])?;
        let verbose = self.parse_optional_bool_arg(args.get(1), false)?;
        let entries = self.orchestrator.runtime.node.mempool_entries();
        if !entries.iter().any(|entry| entry.txid() == txid) {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "transaction is not in the mempool",
            ));
        }
        let ancestors = mempool_dependencies(txid, &entries);
        Ok(render_mempool_relation_value(&entries, &ancestors, verbose))
    }

    fn command_getmempooldescendants(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        if args.is_empty() || args.len() > 2 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "getmempooldescendants expects <txid> [verbose]",
            ));
        }
        let txid = parse_hash48(&args[0])?;
        let verbose = self.parse_optional_bool_arg(args.get(1), false)?;
        let entries = self.orchestrator.runtime.node.mempool_entries();
        if !entries.iter().any(|entry| entry.txid() == txid) {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "transaction is not in the mempool",
            ));
        }
        let descendants = mempool_descendants(txid, &entries);
        Ok(render_mempool_relation_value(
            &entries,
            &descendants,
            verbose,
        ))
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
        Ok(render_block_template_value(&template))
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

    fn command_getnetworkhashps(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        if args.len() > 2 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "getnetworkhashps accepts [nblocks] [height]",
            ));
        }
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        let Some(mut end_index) = blocks.len().checked_sub(1) else {
            return Err(atho_rpc::error::RpcError::internal());
        };
        let mut window_blocks = blocks.len().saturating_sub(1).min(120);
        if let Some(value) = args.first() {
            window_blocks = value.parse::<usize>().map_err(|_| {
                atho_rpc::error::RpcError::invalid_request(
                    "getnetworkhashps expected nblocks as unsigned integer",
                )
            })?;
        }
        if let Some(value) = args.get(1) {
            let height = value.parse::<u64>().map_err(|_| {
                atho_rpc::error::RpcError::invalid_request(
                    "getnetworkhashps expected height as unsigned integer",
                )
            })?;
            end_index = blocks
                .iter()
                .position(|block| block.header.height == height)
                .ok_or_else(|| {
                    atho_rpc::error::RpcError::invalid_request(format!(
                        "unknown block height {height}"
                    ))
                })?;
        }
        let start_index = end_index.saturating_sub(window_blocks.saturating_sub(1));
        let window = &blocks[start_index..=end_index];
        let hashes_per_second = pow::estimated_hashes_per_second(window);
        Ok(json!({
            "height": blocks[end_index].header.height,
            "window_block_count": window.len(),
            "window_start_height": window.first().map(|block| block.header.height).unwrap_or_default(),
            "window_end_height": blocks[end_index].header.height,
            "network_hashes_per_second": hashes_per_second,
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

    fn command_getrawtransaction(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        let txid =
            parse_hash48(&self.parse_single_string_arg("getrawtransaction", args, "txid")?)?;
        if let Some(tx) = self.orchestrator.runtime.node.mempool_transaction(&txid) {
            return Ok(json!({
                "source": "mempool",
                "transaction": render_transaction_value(self.network(), 0, &tx),
            }));
        }
        let blocks = self
            .orchestrator
            .runtime
            .node
            .canonical_blocks()
            .map_err(rpc_error_from_node)?;
        for block in &blocks {
            if let Some((tx_index, tx)) = block
                .transactions
                .iter()
                .enumerate()
                .find(|(_, tx)| tx.txid() == txid)
            {
                return Ok(json!({
                    "source": "chain",
                    "block_hash": hex::encode(block.header.block_hash()),
                    "height": block.header.height,
                    "transaction": render_transaction_value(block.header.network_id, tx_index, tx),
                }));
            }
        }
        Err(atho_rpc::error::RpcError::invalid_request(
            "unknown transaction txid",
        ))
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

    fn command_addnode(&mut self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        if args.is_empty() || args.len() > 2 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "addnode expects <address> [add|onetry]",
            ));
        }
        let address = args[0].clone();
        let mode = args.get(1).map(String::as_str).unwrap_or("add");
        if !matches!(mode, "add" | "onetry") {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "addnode mode must be add or onetry",
            ));
        }
        self.orchestrator.sync.add_manual_peer(address.clone());
        let connection_attempted = matches!(mode, "add" | "onetry");
        let connected = if connection_attempted {
            self.p2p_open_outbound(address.clone())
                .map_err(rpc_error_from_node)?;
            self.p2p_has_peer(&address)
        } else {
            false
        };
        Ok(json!({
            "address": address,
            "mode": mode,
            "connected": connected,
        }))
    }

    fn command_disconnectnode(
        &mut self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        let address = self.parse_single_string_arg("disconnectnode", args, "address")?;
        let disconnected = self
            .p2p_disconnect_peer(&address, String::from("operator disconnect"))
            .is_some();
        Ok(json!({
            "address": address,
            "disconnected": disconnected,
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

    fn parse_optional_bool_arg(
        &self,
        value: Option<&String>,
        default: bool,
    ) -> Result<bool, atho_rpc::error::RpcError> {
        let Some(value) = value else {
            return Ok(default);
        };
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "verbose" => Ok(true),
            "0" | "false" | "no" => Ok(false),
            _ => Err(atho_rpc::error::RpcError::invalid_request(
                "expected bool value",
            )),
        }
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
        Ok(self.orchestrator.runtime.node.block_by_height(height))
    }

    fn block_record_by_height(
        &self,
        height: u64,
    ) -> Result<Option<atho_storage::db::BlockArchiveRecord>, atho_rpc::error::RpcError> {
        Ok(self
            .orchestrator
            .runtime
            .node
            .block_record_by_height(height))
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

fn render_block_template_value(template: &BlockTemplate) -> Value {
    json!({
        "network": template.network.id(),
        "height": template.height,
        "previous_block_hash": hex::encode(template.previous_block_hash),
        "target": hex::encode(template.target),
        "transaction_count": template.transaction_count,
        "fees_atoms": template.fees_atoms,
        "header_bytes_without_nonce": hex::encode(template.header_bytes_without_nonce()),
        "nonce_offset_bytes": template.nonce_offset_bytes(),
        "block": render_block_value(&template.block),
    })
}

fn render_chainwork_hex(bytes: &[u8]) -> String {
    let rendered = hex::encode(bytes);
    let trimmed = rendered.trim_start_matches('0');
    if trimmed.is_empty() {
        String::from("0")
    } else {
        trimmed.to_string()
    }
}

fn render_block_value(block: &Block) -> Value {
    json!({
        "hash": hex::encode(block.header.block_hash()),
        "header": render_block_header_value(&block.header),
        "transaction_count": block.transactions.len(),
        "size_bytes": block.size_bytes(),
        "weight_bytes": block.weight_bytes(),
        "vsize_bytes": block.vsize_bytes(),
        "coinbase_txid": block.transactions.first().map(|tx| hex::encode(tx.txid())).unwrap_or_default(),
        "transactions": block.transactions.iter().enumerate().map(|(index, tx)| render_transaction_value(block.header.network_id, index, tx)).collect::<Vec<_>>(),
    })
}

fn render_block_header_value(header: &BlockHeader) -> Value {
    json!({
        "hash": hex::encode(header.block_hash()),
        "version": header.version,
        "network": header.network_id.id(),
        "height": header.height,
        "previous_block_hash": hex::encode(header.previous_block_hash),
        "merkle_root": hex::encode(header.merkle_root),
        "witness_root": hex::encode(header.witness_root),
        "timestamp": header.timestamp,
        "difficulty_target": hex::encode(header.difficulty_target_or_bits),
        "nonce": header.nonce,
    })
}

fn render_transaction_value(network: Network, index: usize, tx: &Transaction) -> Value {
    json!({
        "index": index,
        "txid": hex::encode(tx.txid()),
        "wtxid": hex::encode(tx.wtxid()),
        "version": tx.version,
        "is_coinbase": tx.is_coinbase(),
        "lock_time": tx.lock_time,
        "size_bytes": tx.full_size_bytes(),
        "weight_bytes": tx.weight_bytes(),
        "vsize_bytes": tx.vsize_bytes(),
        "has_witness": !tx.witness.is_empty(),
        "inputs": tx.inputs.iter().map(render_transaction_input_value).collect::<Vec<_>>(),
        "outputs": tx
            .outputs
            .iter()
            .enumerate()
            .map(|(output_index, output)| {
                render_transaction_output_value(network, output_index, output)
            })
            .collect::<Vec<_>>(),
    })
}

fn render_transaction_input_value(input: &TxInput) -> Value {
    json!({
        "previous_txid": hex::encode(input.previous_txid),
        "output_index": input.output_index,
        "unlocking_script_bytes": input.unlocking_script.len(),
        "unlocking_script_hex": hex::encode(&input.unlocking_script),
    })
}

fn render_transaction_output_value(
    network: Network,
    output_index: usize,
    output: &TxOutput,
) -> Value {
    json!({
        "index": output_index,
        "value_atoms": output.value_atoms,
        "value_atho": format_atoms_decimal(output.value_atoms),
        "locking_script_bytes": output.locking_script.len(),
        "locking_script_hex": hex::encode(&output.locking_script),
        "address_hint": script_address_hint(network, &output.locking_script),
    })
}

fn script_address_hint(network: Network, locking_script: &[u8]) -> Option<String> {
    let digest: [u8; 32] = locking_script.try_into().ok()?;
    Some(encode_base56_address(network, &digest))
}

fn format_atoms_decimal(atoms: u64) -> String {
    let whole = atoms / atho_core::constants::ATOMS_PER_ATHO;
    let fractional = atoms % atho_core::constants::ATOMS_PER_ATHO;
    format!("{whole}.{fractional:08}")
}

fn format_scaled_decimal(value: u64, scale_digits: usize) -> String {
    if scale_digits == 0 {
        return value.to_string();
    }
    let divisor = 10u64.saturating_pow(scale_digits as u32).max(1);
    let whole = value / divisor;
    let fractional = value % divisor;
    format!("{whole}.{fractional:0scale_digits$}")
}

fn render_utxo_value(spend_height: u64, entry: &UtxoEntry) -> Value {
    json!({
        "txid": hex::encode(entry.txid),
        "vout": entry.output_index,
        "value_atoms": entry.value_atoms,
        "value_atho": format_atoms_decimal(entry.value_atoms),
        "confirmations": entry.confirmation_count(spend_height),
        "coinbase": entry.is_coinbase,
        "spendable": entry.is_spendable_at(spend_height),
        "locking_script_hex": hex::encode(&entry.locking_script),
        "address_hint": script_address_hint(entry.network, &entry.locking_script),
        "created_height": entry.created_height,
    })
}

fn utxo_set_hash(entries: &[UtxoEntry]) -> [u8; 48] {
    let mut bytes = Vec::new();
    for entry in entries {
        bytes.extend_from_slice(&entry.txid);
        bytes.extend_from_slice(&entry.output_index.to_le_bytes());
        bytes.extend_from_slice(&entry.value_atoms.to_le_bytes());
        bytes.extend_from_slice(&(entry.locking_script.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&entry.locking_script);
        bytes.extend_from_slice(&entry.created_height.to_le_bytes());
        bytes.push(u8::from(entry.is_coinbase));
        bytes.push(entry.network.consensus_id());
    }
    sha3_384(&bytes)
}

fn render_mempool_entry_value(
    entry: &MempoolEntry,
    depends: &[String],
    descendants: &[String],
) -> Value {
    json!({
        "txid": hex::encode(entry.txid()),
        "wtxid": hex::encode(entry.wtxid()),
        "fee_atoms": entry.fee_atoms,
        "fee_atho": format_atoms_decimal(entry.fee_atoms),
        "base_size_bytes": entry.base_size_bytes(),
        "size_bytes": entry.raw_size_bytes(),
        "vsize_bytes": entry.vsize_bytes(),
        "feerate_atoms_per_vbyte": entry.feerate_atoms_per_vbyte(),
        "depends": depends,
        "ancestor_count": depends.len(),
        "descendant_count": descendants.len(),
        "descendants": descendants,
    })
}

fn render_mempool_relation_value(
    entries: &[MempoolEntry],
    txids: &[String],
    verbose: bool,
) -> Value {
    if !verbose {
        return Value::Array(txids.iter().cloned().map(Value::String).collect());
    }
    let index = entries
        .iter()
        .map(|entry| (hex::encode(entry.txid()), entry))
        .collect::<BTreeMap<_, _>>();
    let map = txids
        .iter()
        .filter_map(|txid| {
            index.get(txid).map(|entry| {
                let depends = mempool_dependencies(entry.txid(), entries);
                let descendants = mempool_descendants(entry.txid(), entries);
                (
                    txid.clone(),
                    render_mempool_entry_value(entry, &depends, &descendants),
                )
            })
        })
        .collect::<serde_json::Map<String, Value>>();
    Value::Object(map)
}

fn mempool_dependencies(target: [u8; 48], entries: &[MempoolEntry]) -> Vec<String> {
    let index = entries
        .iter()
        .map(|entry| (entry.txid(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut visited = BTreeSet::new();
    let mut stack = vec![target];
    while let Some(txid) = stack.pop() {
        let Some(entry) = index.get(&txid) else {
            continue;
        };
        for input in &entry.transaction.inputs {
            if index.contains_key(&input.previous_txid) && visited.insert(input.previous_txid) {
                stack.push(input.previous_txid);
            }
        }
    }
    visited.into_iter().map(hex::encode).collect()
}

fn mempool_descendants(target: [u8; 48], entries: &[MempoolEntry]) -> Vec<String> {
    let reverse = entries.iter().fold(
        BTreeMap::<[u8; 48], Vec<[u8; 48]>>::new(),
        |mut acc, entry| {
            for input in &entry.transaction.inputs {
                acc.entry(input.previous_txid)
                    .or_default()
                    .push(entry.txid());
            }
            acc
        },
    );
    let mut visited = BTreeSet::new();
    let mut stack = reverse.get(&target).cloned().unwrap_or_default();
    while let Some(txid) = stack.pop() {
        if !visited.insert(txid) {
            continue;
        }
        if let Some(children) = reverse.get(&txid) {
            stack.extend(children.iter().copied());
        }
    }
    visited.into_iter().map(hex::encode).collect()
}

fn verify_canonical_chain(network: Network, blocks: &[Block]) -> Result<usize, NodeError> {
    if blocks.is_empty() {
        return Ok(0);
    }
    let mut utxos = UtxoSet::new(network);
    let mut previous_blocks = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let expected_previous_hash = if index == 0 {
            [0; 48]
        } else {
            previous_blocks
                .last()
                .map(|block: &Block| block.header.block_hash())
                .unwrap_or([0; 48])
        };
        let expected_target = if index == 0 {
            block.header.difficulty_target_or_bits
        } else {
            pow::target_for_next_block(network, &previous_blocks)
        };
        atho_storage::validation::validate_block_with_context(
            block,
            block.header.height,
            network,
            expected_previous_hash,
            expected_target,
            &previous_blocks,
            utxos.clone(),
        )
        .map_err(NodeError::Validation)?;
        utxos.apply_block(block).map_err(NodeError::Storage)?;
        previous_blocks.push(block.clone());
    }
    Ok(blocks.len())
}

impl Drop for NodeService {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mempool::MempoolEntry;
    use crate::test_support::acquire_global_test_lock;
    use atho_core::address::encode_base56_address;
    use atho_core::transaction::Transaction;
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

    #[test]
    fn rendered_status_marks_peer_target_above_local_height_as_not_synced() {
        let status = NodeStatus {
            network: Network::Mainnet,
            block_count: 0,
            tip_hash: [0x11; 48],
            tip_timestamp: 1_777_416_445,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x22; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 128,
            network_diagnostics: NetworkDiagnostics::default(),
        };

        let rendered = NodeService::render_status_value(&status);
        assert_eq!(rendered["local_height"], 0);
        assert_eq!(rendered["sync_target_height"], 128);
        assert_eq!(rendered["chain_synced"], false);
        assert_eq!(rendered["headers_synced"], true);
    }

    #[test]
    fn gethealth_warns_when_local_tip_is_behind_sync_target() {
        let status = NodeStatus {
            network: Network::Mainnet,
            block_count: 0,
            tip_hash: [0x33; 48],
            tip_timestamp: 1_777_416_445,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x44; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 128,
            network_diagnostics: NetworkDiagnostics::default(),
        };
        assert!(!NodeService::chain_synced(&status));
        let warnings = NodeService::health_warnings(&status);
        assert_eq!(
            warnings,
            vec![
                "local chain tip is behind the advertised network target",
                "no peers are connected",
            ]
        );
    }

    #[test]
    fn network_diagnostics_excludes_pending_outbound_sessions_from_connected_peer_counts() {
        let root = temp_data_dir("pending-peer-diagnostics");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service
            .p2p_open_outbound("8.8.8.8:9200")
            .expect("open pending outbound");

        let diagnostics = service.network_diagnostics();
        assert_eq!(diagnostics.peer_count, 0);
        assert_eq!(diagnostics.inbound_peer_count, 0);
        assert_eq!(diagnostics.outbound_peer_count, 0);
        assert_eq!(diagnostics.connecting_peer_count, 1);
        assert!(diagnostics.peers.is_empty());
        assert_eq!(diagnostics.connecting_peers.len(), 1);
        assert_eq!(diagnostics.connecting_peers[0].remote_addr, "8.8.8.8:9200");
        assert_eq!(
            diagnostics.connecting_peers[0].direction,
            NetworkPeerDirection::Outbound
        );
        assert!(!diagnostics.connecting_peers[0].handshake_ready);
    }

    #[test]
    fn execute_command_getblock_returns_human_readable_block_shape() {
        let root = temp_data_dir("command-getblock-shape");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let service = NodeService::new(NodeConfig::new(Network::Regnet));
        let response = service.handle(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "getblock",
            vec![String::from("0")],
        )));
        let RpcResponse::Command(command) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(command.command, "getblock");
        assert_eq!(command.data["header"]["network"], "atho-regnet");
        assert_eq!(command.data["transaction_count"], 1);
        assert!(command.data.get("fees_burned_atoms").is_none());
        assert!(command.data["transactions"][0]["outputs"][0]["locking_script_hex"].is_string());
    }

    #[test]
    fn execute_command_getblocktemplate_survives_invalid_mempool_entries() {
        let root = temp_data_dir("command-getblocktemplate-stale");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service
            .orchestrator
            .runtime
            .node
            .mempool
            .insert_unchecked(MempoolEntry::new(
                Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![],
                    lock_time: 0,
                    witness: vec![],
                },
                0,
            ));

        let response = service.handle(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "getblocktemplate",
            Vec::new(),
        )));
        let RpcResponse::Command(command) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(command.command, "getblocktemplate");
        assert_eq!(command.data["block"]["transaction_count"], 1);
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
