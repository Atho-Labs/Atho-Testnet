// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

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
use crate::explorer::ExplorerIndex;
use crate::mempool::MempoolEntry;
#[cfg(test)]
use crate::miner::Miner;
use crate::orchestrator::NodeOrchestrator;
use crate::sync::{NodeSyncError, SyncNotice};
use crate::tcp_p2p::next_outbound_retry_delay;
use crate::validation::ValidationError;
use crate::wallet_history;
use atho_core::address::{decode_base56_address, encode_base56_address};
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::{params::consensus_params_for_network, pow, rules, subsidy};
use atho_core::constants::MAX_TRANSACTION_RAW_BYTES;
use atho_core::crypto::hash::sha3_384;
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput};
use atho_p2p::address_manager::{format_remote_addr, parse_remote_addr};
use atho_p2p::config::{configured_bootstrap_peers, default_bootstrap_peers, network_params};
use atho_p2p::connection::{ConnectionDirection, ConnectionEvent};
use atho_p2p::protocol::{NetworkMessage, NODE_NETWORK};
use atho_rpc::command::{
    command_definition, help_payload, CommandDefinition, CommandInvocation, CommandResponse,
};
use atho_rpc::request::{RpcRequest, WalletHistoryAddress};
use atho_rpc::response::{
    BlockTemplate, MempoolInfo, MempoolSpentInput, NetworkDiagnostics, NetworkPeerDiagnostics,
    NetworkPeerDirection, NodeStatus, RpcResponse, WalletActivityEntry,
};
use atho_storage::db::{BlockArchiveRecord, PeerHealthRecord};
use atho_storage::path::database_dir;
use atho_storage::utxo::{UtxoEntry, UtxoSet};
use atho_wallet::snapshot::WalletSnapshot;

const DIFFICULTY_DISPLAY_SCALE: u64 = 100_000_000;
const EXPLORER_API_SNAPSHOT_VERSION: u32 = 3;
const EXPLORER_API_SNAPSHOT_FILENAME: &str = "explorer-api-snapshot.bin";
const HASHRATE_WINDOW_BLOCKS: usize = 120;
const BLOCKTIME_WINDOW_BLOCKS: usize = 120;
const FEE_WINDOW_BLOCKS: usize = 240;
const FEE_WINDOW_TRANSACTIONS: u64 = 1_000;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone)]
pub(crate) struct CachedPendingTransaction {
    pub(crate) txid: [u8; 48],
    pub(crate) fee_atoms: u64,
    pub(crate) size_bytes: u64,
    pub(crate) size_vbytes: u64,
    pub(crate) feerate_atoms_per_vbyte: u64,
    pub(crate) received_at_unix: u64,
}

impl Default for CachedPendingTransaction {
    fn default() -> Self {
        Self {
            txid: [0u8; 48],
            fee_atoms: 0,
            size_bytes: 0,
            size_vbytes: 0,
            feerate_atoms_per_vbyte: 0,
            received_at_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MempoolSummaryCache {
    pub(crate) network: Option<Network>,
    pub(crate) fingerprint: [u8; 32],
    pub(crate) transaction_count: usize,
    pub(crate) mempool_size_bytes: u64,
    pub(crate) mempool_vsize_bytes: u64,
    pub(crate) total_fee_atoms: u64,
    pub(crate) average_fee_atoms: u64,
    pub(crate) highest_fee: Option<CachedPendingTransaction>,
    pub(crate) lowest_fee: Option<CachedPendingTransaction>,
    pub(crate) estimated_next_block_tx_count: usize,
    pub(crate) status: &'static str,
    pub(crate) recent_transactions: Vec<CachedPendingTransaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChainStatsCache {
    pub(crate) network: Option<Network>,
    pub(crate) tip_height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub(crate) tip_hash: [u8; 48],
    pub(crate) total_transactions: u64,
    pub(crate) total_blocks: u64,
    pub(crate) hashrate_window_blocks: usize,
    pub(crate) estimated_hashrate_hps: u64,
    pub(crate) blocktime_window_blocks: usize,
    pub(crate) average_block_time_millis: u64,
    pub(crate) difficulty_ratio_scaled: u64,
    pub(crate) current_block_reward_atoms: u64,
    pub(crate) total_mined_supply_atoms: u128,
    pub(crate) circulating_supply_atoms: u128,
    pub(crate) average_confirmed_fee_atoms: u64,
    pub(crate) average_fee_window_transactions: u64,
    pub(crate) average_fee_window_blocks: usize,
    pub(crate) genesis_timestamp: u64,
}

impl Default for ChainStatsCache {
    fn default() -> Self {
        Self {
            network: None,
            tip_height: 0,
            tip_hash: [0u8; 48],
            total_transactions: 0,
            total_blocks: 0,
            hashrate_window_blocks: 0,
            estimated_hashrate_hps: 0,
            blocktime_window_blocks: 0,
            average_block_time_millis: 0,
            difficulty_ratio_scaled: 0,
            current_block_reward_atoms: 0,
            total_mined_supply_atoms: 0,
            circulating_supply_atoms: 0,
            average_confirmed_fee_atoms: 0,
            average_fee_window_transactions: 0,
            average_fee_window_blocks: 0,
            genesis_timestamp: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExplorerApiSnapshot {
    version: u32,
    network: Network,
    #[serde(with = "serde_big_array::BigArray")]
    genesis_hash: [u8; 48],
    indexed_height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    indexed_tip_hash: [u8; 48],
    explorer_index: ExplorerIndex,
    chain_stats: ChainStatsCache,
    saved_at_unix: u64,
}

/// Mutable service façade around a running [`NodeOrchestrator`].
#[derive(Debug)]
pub struct NodeService {
    orchestrator: NodeOrchestrator,
    wallet_snapshot: WalletSnapshot,
    network_runtime: NetworkRuntimeView,
    peer_health_cache: BTreeMap<String, PeerHealthRecord>,
    explorer_index: ExplorerIndex,
    explorer_index_ready: bool,
    explorer_index_source: &'static str,
    explorer_snapshot_persisted_unix: Option<u64>,
    explorer_snapshot_height: Option<u64>,
    explorer_snapshot_tip_hash: Option<[u8; 48]>,
    mempool_summary: MempoolSummaryCache,
    chain_stats: ChainStatsCache,
}

impl NodeService {
    /// Creates a new service with a fresh orchestrator.
    pub fn new(config: NodeConfig) -> Self {
        let network = config.network;
        Self {
            orchestrator: NodeOrchestrator::new(config),
            wallet_snapshot: WalletSnapshot::default(),
            network_runtime: NetworkRuntimeView::default(),
            peer_health_cache: BTreeMap::new(),
            explorer_index: ExplorerIndex::default_for_network(network),
            explorer_index_ready: false,
            explorer_index_source: "cold",
            explorer_snapshot_persisted_unix: None,
            explorer_snapshot_height: None,
            explorer_snapshot_tip_hash: None,
            mempool_summary: MempoolSummaryCache {
                network: Some(network),
                ..MempoolSummaryCache::default()
            },
            chain_stats: ChainStatsCache {
                network: Some(network),
                ..ChainStatsCache::default()
            },
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
            explorer_index: ExplorerIndex::default_for_network(network),
            explorer_index_ready: false,
            explorer_index_source: "cold",
            explorer_snapshot_persisted_unix: None,
            explorer_snapshot_height: None,
            explorer_snapshot_tip_hash: None,
            mempool_summary: MempoolSummaryCache {
                network: Some(network),
                ..MempoolSummaryCache::default()
            },
            chain_stats: ChainStatsCache {
                network: Some(network),
                ..ChainStatsCache::default()
            },
        }
    }

    pub fn try_new(config: NodeConfig) -> Result<Self, NodeError> {
        let network = config.network;
        Ok(Self {
            orchestrator: NodeOrchestrator::try_new(config)?,
            wallet_snapshot: WalletSnapshot::default(),
            network_runtime: NetworkRuntimeView::default(),
            peer_health_cache: BTreeMap::new(),
            explorer_index: ExplorerIndex::default_for_network(network),
            explorer_index_ready: false,
            explorer_index_source: "cold",
            explorer_snapshot_persisted_unix: None,
            explorer_snapshot_height: None,
            explorer_snapshot_tip_hash: None,
            mempool_summary: MempoolSummaryCache {
                network: Some(network),
                ..MempoolSummaryCache::default()
            },
            chain_stats: ChainStatsCache {
                network: Some(network),
                ..ChainStatsCache::default()
            },
        })
    }

    /// Starts the orchestrator and seeds the initial peer graph view.
    pub fn start(&mut self) {
        self.orchestrator.start();
        self.seed_peer_graph();
        self.restore_api_snapshot_if_valid();
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
            RpcRequest::Authenticated { request, .. } => self.handle(*request),
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
                let status = self.node_status();
                if let Some(reason) = Self::mining_template_sync_block_reason(&status) {
                    let _ = dev::append_log("athod", &format!("rpc template paused {reason}"));
                    return RpcResponse::Error(atho_rpc::error::RpcError::invalid_request(reason));
                }
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
            RpcRequest::Authenticated { request, .. } => self.handle_mut(*request),
            RpcRequest::ExecuteCommand(invocation) => self.execute_command_mut(invocation),
            RpcRequest::SubmitTransaction {
                transaction,
                fee_atoms,
            } => {
                let tx_summary = dev::summarize_transaction(&transaction, Some(fee_atoms));
                let status = self.node_status();
                let response =
                    if let Some(reason) = Self::transaction_submission_sync_block_reason(&status) {
                        RpcResponse::Error(atho_rpc::error::RpcError::invalid_request(reason))
                    } else {
                        match self
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
                        }
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
                let status = self.node_status();
                let response = if let Some(err) = Self::stale_mined_block_error(&block, &status) {
                    RpcResponse::Error(err)
                } else {
                    match self.orchestrator.runtime.node.submit_block(&block) {
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
                    }
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
            .utxo_entries()
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
            "setminingrewardaddress" => self.command_setminingrewardaddress(args),
            "sendrawtransaction" => self.command_sendrawtransaction(args),
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
            estimated_hashrate_hps: self.chain_stats.estimated_hashrate_hps,
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
        let validation_safe = status
            .network_diagnostics
            .chain_validation_status
            .is_empty()
            || status.network_diagnostics.safe_to_serve;
        status.running
            && status.headers_synced
            && Self::local_height_reached_sync_target(status)
            && status.network_diagnostics.safe_to_serve
            && Self::has_required_ready_peer(status)
            && validation_safe
    }

    fn local_height_reached_sync_target(status: &NodeStatus) -> bool {
        status.sync_best_height == 0 || status.block_count >= status.sync_best_height
    }

    fn public_network_requires_ready_peer(network: Network) -> bool {
        !matches!(network, Network::Regnet | Network::Prunetest)
    }

    fn has_required_ready_peer(status: &NodeStatus) -> bool {
        !Self::public_network_requires_ready_peer(status.network)
            || status.network_diagnostics.peer_count > 0
    }

    fn transaction_submission_sync_block_reason(status: &NodeStatus) -> Option<String> {
        if Self::chain_synced(status) {
            return None;
        }

        let state = if !status.running {
            "node is not running"
        } else if !status.headers_synced {
            "headers are still synchronizing"
        } else if !Self::has_required_ready_peer(status) {
            "no ready network peers are connected"
        } else if !Self::local_height_reached_sync_target(status) {
            "local chain tip is behind the advertised network target"
        } else if !status.network_diagnostics.safe_to_serve {
            "chain validation is still catching up"
        } else {
            "node is not ready"
        };
        Some(format!(
            "transaction submission is paused until the node is synced ({state}; local_height={} sync_target_height={} headers_synced={} running={} peer_count={})",
            status.block_count,
            status.sync_best_height,
            status.headers_synced,
            status.running,
            status.network_diagnostics.peer_count
        ))
    }

    fn mining_template_sync_block_reason(status: &NodeStatus) -> Option<String> {
        if Self::chain_synced(status) && status.network_diagnostics.safe_to_mine {
            return None;
        }

        let state = if !status.running {
            "node is not running"
        } else if !status.headers_synced {
            "headers are still synchronizing"
        } else if !Self::has_required_ready_peer(status) {
            "no ready network peers are connected"
        } else if !Self::local_height_reached_sync_target(status) {
            "local chain tip is behind the advertised network target"
        } else if !status.network_diagnostics.safe_to_mine {
            "chain validation is not safe for mining yet"
        } else if !status.network_diagnostics.safe_to_serve {
            "chain state is not safe to serve yet"
        } else {
            "node is not ready"
        };
        Some(format!(
            "block template generation is paused until the node is synced ({state}; local_height={} sync_target_height={} headers_synced={} running={} peer_count={} safe_to_mine={} safe_to_serve={} validation_lag_blocks={})",
            status.block_count,
            status.sync_best_height,
            status.headers_synced,
            status.running,
            status.network_diagnostics.peer_count,
            status.network_diagnostics.safe_to_mine,
            status.network_diagnostics.safe_to_serve,
            status.network_diagnostics.validation_lag_blocks
        ))
    }

    fn stale_mined_block_error(
        block: &Block,
        status: &NodeStatus,
    ) -> Option<atho_rpc::error::RpcError> {
        let expected_height = status.block_count.saturating_add(1);
        let extends_active_tip = block.header.height == expected_height
            && block.header.previous_block_hash == status.tip_hash;
        if extends_active_tip
            && Self::chain_synced(status)
            && status.network_diagnostics.safe_to_mine
        {
            return None;
        }

        let mut err =
            rpc_error_from_node(NodeError::Validation(ValidationError::InvalidBlockHeight));
        err.details = Some(format!(
            "stale mining template: submitted_height={} expected_height={} submitted_prev={} current_tip={} local_height={} sync_target_height={} headers_synced={} safe_to_mine={}",
            block.header.height,
            expected_height,
            hex::encode(block.header.previous_block_hash),
            hex::encode(status.tip_hash),
            status.block_count,
            status.sync_best_height,
            status.headers_synced,
            status.network_diagnostics.safe_to_mine
        ));
        Some(err)
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
            "known_peer_count": status.network_diagnostics.known_peer_count,
            "healthy_peer_count": status.network_diagnostics.healthy_peer_count,
            "stale_peer_count": status.network_diagnostics.stale_peer_count,
            "banned_peer_count": status.network_diagnostics.banned_peer_count,
            "full_relay_peer_count": status.network_diagnostics.full_relay_peer_count,
            "block_relay_peer_count": status.network_diagnostics.block_relay_peer_count,
            "sync_peer_count": status.network_diagnostics.sync_peer_count,
            "tx_relay_peer_count": status.network_diagnostics.tx_relay_peer_count,
            "addr_relay_peer_count": status.network_diagnostics.addr_relay_peer_count,
            "topology_health_score": status.network_diagnostics.topology_health_score,
            "topology_warnings": status.network_diagnostics.topology_warnings.clone(),
            "best_advertised_peer_height": status.network_diagnostics.best_advertised_peer_height,
            "best_serviceable_peer_height": status.network_diagnostics.best_serviceable_peer_height,
            "unresolved_advertised_height": status.network_diagnostics.unresolved_advertised_height,
            "inconsistent_peer_count": status.network_diagnostics.inconsistent_peer_count,
            "healthy_sync_peer_count": status.network_diagnostics.healthy_sync_peer_count,
            "sync_warning": status.network_diagnostics.sync_warning.clone(),
            "best_header_height": status.network_diagnostics.best_header_height,
            "best_downloaded_body_height": status.network_diagnostics.best_downloaded_body_height,
            "best_validated_height": status.network_diagnostics.best_validated_height,
            "best_connected_height": status.network_diagnostics.best_connected_height,
            "latest_finalized_height": status.network_diagnostics.latest_finalized_height,
            "latest_finalized_hash": hex::encode(status.network_diagnostics.latest_finalized_hash),
            "pending_validation_blocks": status.network_diagnostics.pending_validation_blocks,
            "untrusted_downloaded_blocks": status.network_diagnostics.untrusted_downloaded_blocks,
            "untrusted_downloaded_bytes": status.network_diagnostics.untrusted_downloaded_bytes,
            "fast_download_enabled": status.network_diagnostics.fast_download_enabled,
            "checkpoint_anchored_sync_enabled": status.network_diagnostics.checkpoint_anchored_sync_enabled,
            "background_validation_enabled": status.network_diagnostics.background_validation_enabled,
            "chain_validation_status": status.network_diagnostics.chain_validation_status.clone(),
            "sync_mode": status.network_diagnostics.sync_mode.clone(),
            "safe_to_mine": status.network_diagnostics.safe_to_mine,
            "safe_to_serve": status.network_diagnostics.safe_to_serve,
            "validation_lag_blocks": status.network_diagnostics.validation_lag_blocks,
            "peer_discovery_status": status.network_diagnostics.peer_discovery_status,
            "last_getaddr_time_unix": status.network_diagnostics.last_getaddr_time_unix,
            "last_addr_received_time_unix": status.network_diagnostics.last_addr_received_time_unix,
            "peer_db_path": status.network_diagnostics.peer_db_path,
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
        } else if !Self::has_required_ready_peer(status) {
            warnings.push("no peers are connected");
        } else if !chain_synced {
            warnings.push("local chain tip is behind the advertised network target");
        }
        if !Self::has_required_ready_peer(status) && !warnings.contains(&"no peers are connected") {
            warnings.push("no peers are connected");
        }
        if status.network_diagnostics.topology_health_score < 60
            && !status.network_diagnostics.topology_warnings.is_empty()
        {
            warnings.push("p2p topology health is weak");
        }
        if status.network_diagnostics.unresolved_advertised_height {
            warnings.push("peers advertised heights they did not serve");
        }
        if !status.network_diagnostics.safe_to_mine
            && !status
                .network_diagnostics
                .chain_validation_status
                .is_empty()
            && status.network_diagnostics.validation_lag_blocks > 0
        {
            warnings.push("block bodies are downloaded ahead of validation");
        }
        warnings
    }

    pub fn network_diagnostics(&self) -> NetworkDiagnostics {
        let connections = self.orchestrator.sync.connections();
        let local_height = self.node_ref().height();
        let sync_target_height = self.orchestrator.sync.sync_state().best_height;
        let configured_bootstrap = configured_bootstrap_peers(self.network())
            .into_iter()
            .collect::<BTreeSet<_>>();
        let (peers, connecting_peers): (Vec<_>, Vec<_>) = connections
            .peer_snapshots()
            .into_iter()
            .map(|peer| {
                let remote_addr = peer.remote_addr.clone();
                let traffic = self
                    .network_runtime
                    .peers
                    .get(&remote_addr)
                    .cloned()
                    .unwrap_or_default();
                let health = self.peer_health_cache.get(&remote_addr);
                let direction = match peer.direction {
                    ConnectionDirection::Inbound => NetworkPeerDirection::Inbound,
                    ConnectionDirection::Outbound => NetworkPeerDirection::Outbound,
                };
                let roles = Self::classify_peer_roles(
                    direction,
                    peer.handshake_ready,
                    peer.best_height,
                    peer.services,
                    health,
                    local_height,
                    sync_target_height,
                    configured_bootstrap.contains(&remote_addr),
                );
                NetworkPeerDiagnostics {
                    remote_addr,
                    direction,
                    roles,
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
        let full_relay_peer_count = Self::count_peers_with_role(&peers, "FULL_RELAY_PEER");
        let block_relay_peer_count = Self::count_peers_with_role(&peers, "BLOCK_RELAY_PEER");
        let sync_peer_count = Self::count_peers_with_role(&peers, "SYNC_PEER");
        let tx_relay_peer_count = Self::count_peers_with_role(&peers, "TX_RELAY_PEER");
        let addr_relay_peer_count = Self::count_peers_with_role(&peers, "ADDR_RELAY_PEER");
        let known_peer_count = self.orchestrator.sync.known_peer_count();
        let healthy_peer_count = self.orchestrator.sync.fresh_peer_count().max(peer_count);
        let stale_peer_count = self.orchestrator.sync.stale_peer_count();
        let banned_peer_count = self.orchestrator.sync.banned_peer_count();
        let last_addr_received_time_unix = self.orchestrator.sync.last_addr_received_unix();
        let fast_download = self
            .orchestrator
            .sync
            .fast_download_diagnostics(&self.orchestrator.runtime.node);
        let (mut topology_health_score, mut topology_warnings) = Self::topology_health(
            self.network(),
            &peers,
            known_peer_count,
            healthy_peer_count,
            stale_peer_count,
            banned_peer_count,
            outbound_peer_count,
            full_relay_peer_count,
            block_relay_peer_count,
            sync_peer_count,
            tx_relay_peer_count,
            addr_relay_peer_count,
            last_addr_received_time_unix,
        );
        let peer_sync_inconsistency = self.orchestrator.sync.peer_sync_inconsistency_summary();
        if let Some(sync_warning) = &peer_sync_inconsistency.sync_warning {
            topology_warnings.push(format!(
                "{sync_warning}: best_advertised={} best_serviceable={} inconsistent_peers={}",
                peer_sync_inconsistency.best_advertised_peer_height,
                peer_sync_inconsistency.best_serviceable_peer_height,
                peer_sync_inconsistency.inconsistent_peer_count
            ));
            topology_health_score = topology_health_score.min(40);
        }

        NetworkDiagnostics {
            peer_count,
            inbound_peer_count,
            outbound_peer_count,
            full_relay_peer_count,
            block_relay_peer_count,
            sync_peer_count,
            tx_relay_peer_count,
            addr_relay_peer_count,
            connecting_peer_count: connecting_peers.len(),
            known_peer_count,
            healthy_peer_count,
            stale_peer_count,
            banned_peer_count,
            dns_seed_status: if network_params(self.network()).dns_seeds.is_empty() {
                String::from("none")
            } else {
                String::from("configured")
            },
            bootstrap_status: if default_bootstrap_peers(self.network()).is_empty() {
                String::from("none")
            } else {
                String::from("configured")
            },
            peer_discovery_status: self.peer_discovery_status(peer_count),
            last_getaddr_time_unix: self.orchestrator.sync.last_getaddr_time_unix(),
            last_addr_received_time_unix,
            peer_db_path: database_dir(self.network()).display().to_string(),
            topology_health_score,
            topology_warnings,
            best_advertised_peer_height: peer_sync_inconsistency.best_advertised_peer_height,
            best_serviceable_peer_height: peer_sync_inconsistency.best_serviceable_peer_height,
            unresolved_advertised_height: peer_sync_inconsistency.unresolved_advertised_height,
            inconsistent_peer_count: peer_sync_inconsistency.inconsistent_peer_count,
            healthy_sync_peer_count: peer_sync_inconsistency.healthy_sync_peer_count,
            sync_warning: peer_sync_inconsistency.sync_warning,
            best_header_height: fast_download.best_header_height,
            best_downloaded_body_height: fast_download.best_downloaded_body_height,
            best_validated_height: fast_download.best_validated_height,
            best_connected_height: fast_download.best_connected_height,
            latest_finalized_height: fast_download.latest_finalized_height,
            latest_finalized_hash: fast_download.latest_finalized_hash,
            pending_validation_blocks: fast_download.pending_validation_blocks,
            untrusted_downloaded_blocks: fast_download.untrusted_downloaded_blocks,
            untrusted_downloaded_bytes: fast_download.untrusted_downloaded_bytes,
            fast_download_enabled: fast_download.fast_download_enabled,
            checkpoint_anchored_sync_enabled: fast_download.checkpoint_anchored_sync_enabled,
            background_validation_enabled: fast_download.background_validation_enabled,
            chain_validation_status: fast_download.chain_validation_status,
            sync_mode: fast_download.sync_mode,
            safe_to_mine: fast_download.safe_to_mine,
            safe_to_serve: fast_download.safe_to_serve,
            validation_lag_blocks: fast_download.validation_lag_blocks,
            max_fast_download_ahead: fast_download.max_fast_download_ahead,
            max_untrusted_block_cache: fast_download.max_untrusted_block_cache,
            max_pending_validation_blocks: fast_download.max_pending_validation_blocks,
            bytes_sent: self.network_runtime.bytes_sent,
            bytes_received: self.network_runtime.bytes_received,
            peers,
            connecting_peers,
        }
    }

    fn classify_peer_roles(
        direction: NetworkPeerDirection,
        handshake_ready: bool,
        best_height: Option<u64>,
        services: Option<u64>,
        health: Option<&PeerHealthRecord>,
        local_height: u64,
        sync_target_height: u64,
        is_bootstrap: bool,
    ) -> Vec<String> {
        let mut roles = Vec::new();
        roles.push(match direction {
            NetworkPeerDirection::Inbound => String::from("INBOUND_PEER"),
            NetworkPeerDirection::Outbound => String::from("OUTBOUND_PEER"),
        });
        if is_bootstrap {
            roles.push(String::from("BOOTSTRAP_PEER"));
        }
        if !handshake_ready {
            return roles;
        }

        let supports_network = services.is_none_or(|services| services & NODE_NETWORK != 0);
        let quality = health.map(|record| record.quality_score).unwrap_or(100);
        let failures = health
            .map(|record| record.consecutive_failures)
            .unwrap_or_default();
        let usable = quality >= 50 && failures <= 3;
        let good = quality >= 70 && failures <= 1;
        let peer_height = best_height.unwrap_or_default();

        if supports_network && usable {
            roles.push(String::from("FULL_RELAY_PEER"));
        }
        if supports_network && usable && peer_height >= local_height {
            roles.push(String::from("SYNC_PEER"));
        }
        if supports_network && good && peer_height >= local_height {
            roles.push(String::from("BLOCK_RELAY_PEER"));
        }
        if usable {
            roles.push(String::from("TX_RELAY_PEER"));
            roles.push(String::from("ADDR_RELAY_PEER"));
        }
        if peer_height < sync_target_height.saturating_sub(16) || quality < 50 || failures > 3 {
            roles.push(String::from("DISCOURAGED_PEER"));
        }
        roles
    }

    fn count_peers_with_role(peers: &[NetworkPeerDiagnostics], role: &str) -> usize {
        peers
            .iter()
            .filter(|peer| peer.roles.iter().any(|value| value == role))
            .count()
    }

    #[allow(clippy::too_many_arguments)]
    fn topology_health(
        network: Network,
        peers: &[NetworkPeerDiagnostics],
        known_peer_count: usize,
        healthy_peer_count: usize,
        stale_peer_count: usize,
        banned_peer_count: usize,
        outbound_peer_count: usize,
        _full_relay_peer_count: usize,
        block_relay_peer_count: usize,
        sync_peer_count: usize,
        tx_relay_peer_count: usize,
        addr_relay_peer_count: usize,
        last_addr_received_time_unix: Option<u64>,
    ) -> (u8, Vec<String>) {
        let limits = network_params(network).limits;
        let outbound_target = limits.max_outbound_peers.min(8).max(1);
        let block_relay_target = limits.target_block_relay_peers.min(outbound_target).max(1);
        let sync_target = limits.target_sync_peers.min(outbound_target).max(1);
        let tx_target = limits.target_tx_relay_peers.min(outbound_target).max(1);
        let addr_target = limits.target_addr_relay_peers.min(outbound_target).max(1);
        let mut score = 0u8;
        let mut warnings = Vec::new();

        if outbound_peer_count >= outbound_target {
            score = score.saturating_add(15);
        } else {
            warnings.push(format!(
                "low outbound peer count: {outbound_peer_count}/{outbound_target}"
            ));
        }
        if block_relay_peer_count >= block_relay_target {
            score = score.saturating_add(15);
        } else {
            warnings.push(format!(
                "low block-relay peer count: {block_relay_peer_count}/{block_relay_target}"
            ));
        }
        if sync_peer_count >= sync_target {
            score = score.saturating_add(15);
        } else {
            warnings.push(format!(
                "low sync peer count: {sync_peer_count}/{sync_target}"
            ));
        }
        if tx_relay_peer_count >= tx_target {
            score = score.saturating_add(10);
        } else {
            warnings.push(format!(
                "low tx-relay peer count: {tx_relay_peer_count}/{tx_target}"
            ));
        }
        if addr_relay_peer_count >= addr_target {
            score = score.saturating_add(5);
        } else {
            warnings.push(format!(
                "low address-relay peer count: {addr_relay_peer_count}/{addr_target}"
            ));
        }

        let subnet_groups = peers
            .iter()
            .map(|peer| diagnostic_peer_group(&peer.remote_addr, network.p2p_port()))
            .collect::<BTreeSet<_>>();
        if peers.len() <= 1 || subnet_groups.len() >= peers.len().min(3) {
            score = score.saturating_add(10);
        } else {
            warnings.push(String::from("weak peer subnet diversity"));
        }

        let bootstrap_peers = peers
            .iter()
            .filter(|peer| peer.roles.iter().any(|role| role == "BOOTSTRAP_PEER"))
            .count();
        if peers.is_empty()
            || bootstrap_peers < peers.len()
            || last_addr_received_time_unix.is_some()
        {
            score = score.saturating_add(10);
        } else {
            warnings.push(String::from("topology depends only on bootstrap peers"));
        }

        let high_failure_peers = peers
            .iter()
            .filter(|peer| peer.consecutive_failures.unwrap_or_default() > 2)
            .count();
        if high_failure_peers == 0 {
            score = score.saturating_add(10);
        } else {
            warnings.push(format!("high failure peers: {high_failure_peers}"));
        }

        if banned_peer_count == 0 {
            score = score.saturating_add(5);
        } else {
            warnings.push(format!("banned peers observed: {banned_peer_count}"));
        }
        if known_peer_count > 0 && healthy_peer_count > stale_peer_count {
            score = score.saturating_add(5);
        } else {
            warnings.push(String::from("peer database is weak or stale"));
        }

        (score.min(100), warnings)
    }

    pub fn wallet_snapshot(&self) -> &WalletSnapshot {
        &self.wallet_snapshot
    }

    fn peer_discovery_status(&self, ready_peer_count: usize) -> String {
        if ready_peer_count > 0 {
            return String::from("connected");
        }
        if self.orchestrator.sync.known_peer_count() > 0 {
            return String::from("trying-known-peers");
        }
        let params = network_params(self.network());
        if !params.dns_seeds.is_empty() {
            return String::from("dns-seed-bootstrap");
        }
        if !default_bootstrap_peers(self.network()).is_empty() {
            return String::from("static-bootstrap");
        }
        String::from("manual-or-local")
    }

    pub fn is_running(&self) -> bool {
        self.orchestrator.runtime.running
    }

    pub fn runtime_started_at_unix(&self) -> Option<u64> {
        self.orchestrator.runtime.started_at_unix
    }

    pub fn runtime_uptime_seconds(&self) -> u64 {
        if !self.orchestrator.runtime.running {
            return 0;
        }
        let now = unix_timestamp();
        let started_at = self.orchestrator.runtime.started_at_unix.unwrap_or(now);
        now.saturating_sub(started_at)
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

    pub fn p2p_transaction_relay_ready(&self) -> bool {
        Self::chain_synced(&self.node_status())
    }

    pub fn p2p_block_relay_ready(&self) -> bool {
        Self::chain_synced(&self.node_status())
    }

    pub fn p2p_mempool_fingerprint(&self) -> [u8; 32] {
        self.orchestrator.runtime.node.mempool_fingerprint()
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
        self.refresh_runtime_status_views();
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
        self.refresh_runtime_status_views();
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
        self.refresh_runtime_status_views();
        Ok(result)
    }

    pub fn p2p_maintain_peer_sync(
        &mut self,
        remote_addr: &str,
    ) -> Result<Vec<ConnectionEvent>, NodeError> {
        let events = self
            .orchestrator
            .sync
            .maintain_peer_sync(remote_addr, &self.orchestrator.runtime.node)
            .map_err(sync_error_into_node)?;
        self.refresh_runtime_status_views();
        Ok(events)
    }

    pub fn p2p_disconnect_peer(&mut self, remote_addr: &str, reason: String) -> Option<SyncNotice> {
        let notice = self.orchestrator.sync.disconnect_peer(
            remote_addr,
            reason.clone(),
            &self.orchestrator.runtime.node,
        );
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
        self.refresh_runtime_status_views();
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

    fn refresh_runtime_status_views(&mut self) {
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

    fn refresh_runtime_views(&mut self) {
        self.refresh_runtime_status_views();
        self.refresh_api_views();
    }

    pub(crate) fn refresh_api_views(&mut self) {
        self.refresh_explorer_index();
        self.refresh_mempool_summary();
        self.refresh_chain_stats();
        self.persist_api_snapshot_if_enabled();
    }

    pub(crate) fn refresh_api_light_views(&mut self) {
        self.refresh_mempool_summary();
        self.refresh_chain_stats();
    }

    fn refresh_explorer_index(&mut self) {
        if !self.explorer_index_enabled() {
            self.explorer_index_ready = false;
            self.explorer_index_source = "disabled";
            return;
        }
        let node = &self.orchestrator.runtime.node;
        let tip_height = node.height();
        let tip_hash = node.tip_hash();
        if !self
            .explorer_index
            .needs_refresh(self.network(), tip_height, tip_hash)
        {
            return;
        }
        match self.explorer_index.try_refresh_incremental(node) {
            Ok(true) => {
                self.explorer_index_ready = true;
                self.explorer_index_source = "incremental";
                return;
            }
            Ok(false) => {}
            Err(err) => {
                let _ = dev::append_log(
                    "api",
                    &format!(
                        "explorer index incremental refresh failed network={} height={} error={err}",
                        self.network().id(),
                        tip_height
                    ),
                );
            }
        }
        self.rebuild_explorer_index();
    }

    fn refresh_mempool_summary(&mut self) {
        let node = &self.orchestrator.runtime.node;
        let network = self.network();
        let fingerprint = node.mempool_fingerprint();
        if self.mempool_summary.network == Some(network)
            && self.mempool_summary.fingerprint == fingerprint
        {
            return;
        }

        let mut recent_transactions = node
            .mempool_entries_iter()
            .map(|entry| CachedPendingTransaction {
                txid: entry.txid(),
                fee_atoms: entry.fee_atoms,
                size_bytes: entry.full_size_bytes() as u64,
                size_vbytes: entry.vsize_bytes() as u64,
                feerate_atoms_per_vbyte: entry.feerate_atoms_per_vbyte(),
                received_at_unix: entry.received_at_unix(),
            })
            .collect::<Vec<_>>();
        recent_transactions.sort_by(|left, right| {
            right
                .received_at_unix
                .cmp(&left.received_at_unix)
                .then(right.fee_atoms.cmp(&left.fee_atoms))
                .then(left.txid.cmp(&right.txid))
        });

        let transaction_count = recent_transactions.len();
        let mempool_size_bytes = node.mempool_total_raw_bytes() as u64;
        let mempool_vsize_bytes = node.mempool_total_vbytes() as u64;
        let total_fee_atoms = node.mempool_total_fee_atoms();
        let average_fee_atoms = if transaction_count == 0 {
            0
        } else {
            total_fee_atoms / transaction_count as u64
        };
        let highest_fee = recent_transactions
            .iter()
            .max_by(|left, right| {
                left.fee_atoms
                    .cmp(&right.fee_atoms)
                    .then(
                        left.feerate_atoms_per_vbyte
                            .cmp(&right.feerate_atoms_per_vbyte),
                    )
                    .then(right.received_at_unix.cmp(&left.received_at_unix))
            })
            .cloned();
        let lowest_fee = recent_transactions
            .iter()
            .min_by(|left, right| {
                left.fee_atoms
                    .cmp(&right.fee_atoms)
                    .then(
                        left.feerate_atoms_per_vbyte
                            .cmp(&right.feerate_atoms_per_vbyte),
                    )
                    .then(left.received_at_unix.cmp(&right.received_at_unix))
            })
            .cloned();

        self.mempool_summary = MempoolSummaryCache {
            network: Some(network),
            fingerprint,
            transaction_count,
            mempool_size_bytes,
            mempool_vsize_bytes,
            total_fee_atoms,
            average_fee_atoms,
            highest_fee,
            lowest_fee,
            estimated_next_block_tx_count: transaction_count,
            status: mempool_status_label(transaction_count, mempool_vsize_bytes),
            recent_transactions: recent_transactions.into_iter().take(10).collect(),
        };
    }

    fn refresh_chain_stats(&mut self) {
        let node = &self.orchestrator.runtime.node;
        let network = self.network();
        let tip_height = node.height();
        let tip_hash = node.tip_hash();
        if self.chain_stats.network == Some(network)
            && self.chain_stats.tip_height == tip_height
            && self.chain_stats.tip_hash == tip_hash
        {
            return;
        }

        let tip_record = self
            .block_record_by_height(tip_height)
            .ok()
            .flatten()
            .or_else(|| node.block_record_by_hash(tip_hash));
        let Some(tip_record) = tip_record else {
            let _ = dev::append_log(
                "api",
                &format!(
                    "chain stats refresh failed network={} height={} error=missing_tip_record",
                    network.id(),
                    tip_height
                ),
            );
            return;
        };
        let recent_records = match load_recent_block_records(
            node,
            tip_height,
            HASHRATE_WINDOW_BLOCKS.max(FEE_WINDOW_BLOCKS),
        ) {
            Some(records) => records,
            None => {
                let _ = dev::append_log(
                    "api",
                    &format!(
                        "chain stats refresh failed network={} height={} error=missing_recent_window",
                        network.id(),
                        tip_height
                    ),
                );
                return;
            }
        };
        let total_transactions = self
            .incremental_total_transactions(&tip_record)
            .unwrap_or_else(|| full_transaction_count(node, tip_height).unwrap_or_default());
        let total_blocks = tip_height.saturating_add(1);
        let available_block_intervals = recent_records.len().saturating_sub(1);
        let hashrate_window_blocks = available_block_intervals.min(HASHRATE_WINDOW_BLOCKS);
        let estimated_hashrate_hps =
            estimated_hashrate_from_records(&recent_records, hashrate_window_blocks);

        let blocktime_window_blocks = available_block_intervals.min(BLOCKTIME_WINDOW_BLOCKS);
        let average_block_time_millis =
            average_block_time_from_records(&recent_records, blocktime_window_blocks);
        let difficulty_ratio_scaled = pow::difficulty_ratio_scaled(
            &tip_record.difficulty_target_or_bits,
            DIFFICULTY_DISPLAY_SCALE,
        );
        let current_block_reward_atoms =
            subsidy::block_subsidy_atoms_for_network(network, tip_height.saturating_add(1));
        let total_mined_supply_atoms =
            subsidy::cumulative_issued_through_height_for_network(network, tip_height);
        let coinbase_maturity_blocks =
            consensus_params_for_network(network).coinbase_maturity_blocks;
        let circulating_supply_atoms = if tip_height.saturating_add(1) < coinbase_maturity_blocks {
            0
        } else {
            let mature_height = tip_height
                .saturating_add(1)
                .saturating_sub(coinbase_maturity_blocks);
            subsidy::cumulative_issued_through_height_for_network(network, mature_height)
        };

        let mut average_fee_window_transactions = 0u64;
        let mut average_fee_window_blocks = 0usize;
        let mut average_fee_total_atoms = 0u64;
        for record in recent_records.iter().rev().take(FEE_WINDOW_BLOCKS) {
            let non_coinbase_txs = record.tx_count.saturating_sub(1) as u64;
            if non_coinbase_txs == 0 {
                continue;
            }
            average_fee_total_atoms =
                average_fee_total_atoms.saturating_add(record.fees_total_atoms);
            average_fee_window_transactions =
                average_fee_window_transactions.saturating_add(non_coinbase_txs);
            average_fee_window_blocks = average_fee_window_blocks.saturating_add(1);
            if average_fee_window_transactions >= FEE_WINDOW_TRANSACTIONS {
                break;
            }
        }
        let average_confirmed_fee_atoms = average_fee_total_atoms
            .checked_div(average_fee_window_transactions)
            .unwrap_or(0);

        let genesis_timestamp = node
            .block_record_by_height(0)
            .map(|record| record.timestamp)
            .or_else(|| {
                node.blocks()
                    .first()
                    .filter(|block| block.header.height == 0)
                    .map(|block| block.header.timestamp)
            })
            .unwrap_or_else(|| genesis::genesis_state(network).block.header.timestamp);

        self.chain_stats = ChainStatsCache {
            network: Some(network),
            tip_height,
            tip_hash,
            total_transactions,
            total_blocks,
            hashrate_window_blocks,
            estimated_hashrate_hps,
            blocktime_window_blocks,
            average_block_time_millis,
            difficulty_ratio_scaled,
            current_block_reward_atoms,
            total_mined_supply_atoms,
            circulating_supply_atoms,
            average_confirmed_fee_atoms,
            average_fee_window_transactions,
            average_fee_window_blocks,
            genesis_timestamp,
        };
    }

    fn rebuild_explorer_index(&mut self) {
        let node = &self.orchestrator.runtime.node;
        let tip_height = node.height();
        match ExplorerIndex::rebuild(node) {
            Ok(index) => {
                self.explorer_index = index;
                self.explorer_index_ready = true;
                self.explorer_index_source = "rebuilt";
            }
            Err(err) => {
                self.explorer_index_ready = false;
                let _ = dev::append_log(
                    "api",
                    &format!(
                        "explorer index refresh failed network={} height={} error={err}",
                        self.network().id(),
                        tip_height
                    ),
                );
            }
        }
    }

    fn incremental_total_transactions(&self, tip_record: &BlockArchiveRecord) -> Option<u64> {
        if self.chain_stats.network != Some(self.network()) {
            return None;
        }
        if self.chain_stats.tip_height.saturating_add(1) != tip_record.height {
            return None;
        }
        if self.chain_stats.tip_hash != tip_record.previous_block_hash {
            return None;
        }
        Some(
            self.chain_stats
                .total_transactions
                .saturating_add(tip_record.tx_count as u64),
        )
    }

    fn restore_api_snapshot_if_valid(&mut self) {
        if !self.explorer_index_enabled() || !self.explorer_snapshot_enabled() {
            return;
        }
        let path = explorer_api_snapshot_path(self.network());
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(err) => {
                let _ = dev::append_log(
                    "api",
                    &format!(
                        "explorer snapshot read failed network={} path={} error={err}",
                        self.network().id(),
                        path.display()
                    ),
                );
                return;
            }
        };
        let snapshot: ExplorerApiSnapshot = match bincode::deserialize(&bytes) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                let _ = dev::append_log(
                    "api",
                    &format!(
                        "explorer snapshot decode failed network={} path={} error={err}",
                        self.network().id(),
                        path.display()
                    ),
                );
                return;
            }
        };
        if !self.snapshot_matches_current_chain(&snapshot) {
            let _ = dev::append_log(
                "api",
                &format!(
                    "explorer snapshot ignored network={} height={} reason=mismatch",
                    self.network().id(),
                    snapshot.indexed_height
                ),
            );
            return;
        }

        self.explorer_index = snapshot.explorer_index;
        self.chain_stats = snapshot.chain_stats;
        self.explorer_index_ready = true;
        self.explorer_index_source = "snapshot";
        self.explorer_snapshot_persisted_unix = Some(snapshot.saved_at_unix);
        self.explorer_snapshot_height = Some(snapshot.indexed_height);
        self.explorer_snapshot_tip_hash = Some(snapshot.indexed_tip_hash);
    }

    fn persist_api_snapshot_if_enabled(&mut self) {
        if !self.explorer_index_enabled()
            || !self.explorer_snapshot_enabled()
            || !self.explorer_index_ready
        {
            return;
        }
        let index_tip_height = self.explorer_index.tip_height();
        let index_tip_hash = self.explorer_index.tip_hash();
        if self.chain_stats.network != Some(self.network())
            || self.chain_stats.tip_height != index_tip_height
            || self.chain_stats.tip_hash != index_tip_hash
        {
            return;
        }
        if self.explorer_snapshot_height == Some(index_tip_height)
            && self.explorer_snapshot_tip_hash == Some(index_tip_hash)
        {
            return;
        }

        let saved_at_unix = unix_timestamp();
        let snapshot = ExplorerApiSnapshot {
            version: EXPLORER_API_SNAPSHOT_VERSION,
            network: self.network(),
            genesis_hash: genesis::genesis_hash(self.network()),
            indexed_height: index_tip_height,
            indexed_tip_hash: index_tip_hash,
            explorer_index: self.explorer_index.clone(),
            chain_stats: self.chain_stats.clone(),
            saved_at_unix,
        };
        let path = explorer_api_snapshot_path(self.network());
        if let Err(err) = write_explorer_api_snapshot(&path, &snapshot) {
            let _ = dev::append_log(
                "api",
                &format!(
                    "explorer snapshot persist failed network={} path={} error={err}",
                    self.network().id(),
                    path.display()
                ),
            );
            return;
        }
        self.explorer_snapshot_persisted_unix = Some(saved_at_unix);
        self.explorer_snapshot_height = Some(index_tip_height);
        self.explorer_snapshot_tip_hash = Some(index_tip_hash);
    }

    fn snapshot_matches_current_chain(&self, snapshot: &ExplorerApiSnapshot) -> bool {
        if snapshot.version != EXPLORER_API_SNAPSHOT_VERSION
            || snapshot.network != self.network()
            || snapshot.genesis_hash != genesis::genesis_hash(self.network())
            || snapshot.explorer_index.network() != Some(self.network())
            || snapshot.explorer_index.tip_height() != snapshot.indexed_height
            || snapshot.explorer_index.tip_hash() != snapshot.indexed_tip_hash
            || snapshot.chain_stats.network != Some(self.network())
            || snapshot.chain_stats.tip_height != snapshot.indexed_height
            || snapshot.chain_stats.tip_hash != snapshot.indexed_tip_hash
        {
            return false;
        }
        if self.node_ref().height() != snapshot.indexed_height
            || self.node_ref().tip_hash() != snapshot.indexed_tip_hash
        {
            return false;
        }
        self.node_ref()
            .block_record_by_height(snapshot.indexed_height)
            .map(|record| record.block_hash == snapshot.indexed_tip_hash)
            .unwrap_or(false)
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
        let mut backed_off = Vec::new();

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
                backed_off.push((address, remote_addr, health));
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
        let mut seen_groups = BTreeSet::new();
        let mut peers = Vec::new();

        for (_, remote_addr, _) in &scored {
            let group = diagnostic_peer_group(remote_addr, self.network().p2p_port());
            if !seen_groups.insert(group) {
                continue;
            }
            if seen.insert(remote_addr.clone()) {
                peers.push(remote_addr.clone());
            }
            if peers.len() >= max {
                return peers;
            }
        }

        for (_, remote_addr, _) in scored {
            if seen.insert(remote_addr.clone()) {
                peers.push(remote_addr);
            }
            if peers.len() >= max {
                return peers;
            }
        }

        if peers.is_empty() && connected_peers.is_empty() {
            backed_off.sort_by(|left, right| {
                let left_health = left.2.as_ref();
                let right_health = right.2.as_ref();
                left_health
                    .map(|record| record.backoff_until_unix)
                    .cmp(&right_health.map(|record| record.backoff_until_unix))
                    .then(
                        right_health
                            .map(|record| record.quality_score)
                            .cmp(&left_health.map(|record| record.quality_score)),
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
            for (_, remote_addr, _) in backed_off {
                if seen.insert(remote_addr.clone()) {
                    peers.push(remote_addr);
                }
                if peers.len() >= max {
                    break;
                }
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
        self.rebuild_explorer_index();
        self.refresh_runtime_views();
        result
    }

    pub(crate) fn node_ref(&self) -> &crate::node::Node {
        &self.orchestrator.runtime.node
    }

    pub(crate) fn explorer_index(&self) -> &ExplorerIndex {
        &self.explorer_index
    }

    pub(crate) fn explorer_index_ready(&self) -> bool {
        self.explorer_index_ready
    }

    pub(crate) fn explorer_index_source(&self) -> &'static str {
        self.explorer_index_source
    }

    pub(crate) fn explorer_snapshot_persisted_unix(&self) -> Option<u64> {
        self.explorer_snapshot_persisted_unix
    }

    pub(crate) fn explorer_index_enabled(&self) -> bool {
        self.orchestrator
            .runtime
            .node
            .config
            .api
            .explorer
            .index_enabled
    }

    pub(crate) fn explorer_snapshot_enabled(&self) -> bool {
        self.orchestrator
            .runtime
            .node
            .config
            .api
            .explorer
            .snapshot_enabled
    }

    pub(crate) fn mempool_summary(&self) -> &MempoolSummaryCache {
        &self.mempool_summary
    }

    pub(crate) fn chain_stats(&self) -> &ChainStatsCache {
        &self.chain_stats
    }

    pub(crate) fn known_node_count(&self) -> usize {
        self.orchestrator
            .runtime
            .node
            .peer_addresses()
            .map(|peers| peers.len())
            .unwrap_or_else(|_| self.network_diagnostics().peer_count)
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
        if args.len() > 1 {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "geterrorcodes accepts at most one optional error code",
            ));
        }
        if let Some(query) = args.first() {
            let normalized = query.trim().to_ascii_uppercase();
            let descriptor = atho_errors::REGISTRY
                .iter()
                .find(|descriptor| descriptor.code.as_str().eq_ignore_ascii_case(&normalized))
                .ok_or_else(|| {
                    atho_rpc::error::RpcError::invalid_request(format!(
                        "unknown Atho error code {query}"
                    ))
                })?;
            return serde_json::to_value(descriptor)
                .map_err(|err| atho_rpc::error::RpcError::invalid_request(err.to_string()));
        }
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
        let mempool_bytes = self.orchestrator.runtime.node.mempool_total_raw_bytes();
        let chain_bytes: usize = blocks.iter().map(Block::full_size_bytes).sum();
        Ok(json!({
            "network": self.network().id(),
            "mempool_transactions": self.orchestrator.runtime.node.mempool_len(),
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
        Ok(self.render_block_value_with_fees(&block))
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
            "subsidy_atoms": subsidy::block_subsidy_atoms_for_network(
                block.header.network_id,
                block.header.height,
            ),
            "fees_atoms": block.fees_total_atoms,
            "total_output_atoms": total_output_atoms,
            "avg_tx_size_bytes": block.size_bytes().checked_div(transaction_count).unwrap_or(0),
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
        let scaled =
            pow::difficulty_ratio_scaled(&tip.difficulty_target_or_bits, DIFFICULTY_DISPLAY_SCALE);
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
        let stats = utxo_set_stats(self.orchestrator.runtime.node.utxo_entries());
        Ok(json!({
            "height": self.orchestrator.runtime.node.height(),
            "best_block_hash": hex::encode(self.orchestrator.runtime.node.tip_hash()),
            "txouts": stats.txouts,
            "bogosize": stats.bogosize,
            "total_amount_atoms": stats.total_amount_atoms,
            "total_amount_atho": format_atoms_decimal(self.network(), stats.total_amount_atoms),
            "utxo_set_hash": hex::encode(stats.hash),
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
            "difficulty": format_scaled_decimal(
                pow::difficulty_ratio_scaled(&next_target, DIFFICULTY_DISPLAY_SCALE),
                8
            ),
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
            "full_relay_peer_count": status.network_diagnostics.full_relay_peer_count,
            "block_relay_peer_count": status.network_diagnostics.block_relay_peer_count,
            "sync_peer_count": status.network_diagnostics.sync_peer_count,
            "tx_relay_peer_count": status.network_diagnostics.tx_relay_peer_count,
            "addr_relay_peer_count": status.network_diagnostics.addr_relay_peer_count,
            "connecting_peer_count": status.network_diagnostics.connecting_peer_count,
            "known_peer_count": status.network_diagnostics.known_peer_count,
            "healthy_peer_count": status.network_diagnostics.healthy_peer_count,
            "stale_peer_count": status.network_diagnostics.stale_peer_count,
            "banned_peer_count": status.network_diagnostics.banned_peer_count,
            "dns_seed_status": status.network_diagnostics.dns_seed_status,
            "bootstrap_status": status.network_diagnostics.bootstrap_status,
            "peer_discovery_status": status.network_diagnostics.peer_discovery_status,
            "last_getaddr_time_unix": status.network_diagnostics.last_getaddr_time_unix,
            "last_addr_received_time_unix": status.network_diagnostics.last_addr_received_time_unix,
            "peer_db_path": status.network_diagnostics.peer_db_path,
            "topology_health_score": status.network_diagnostics.topology_health_score,
            "topology_warnings": status.network_diagnostics.topology_warnings.clone(),
            "best_advertised_peer_height": status.network_diagnostics.best_advertised_peer_height,
            "best_serviceable_peer_height": status.network_diagnostics.best_serviceable_peer_height,
            "unresolved_advertised_height": status.network_diagnostics.unresolved_advertised_height,
            "inconsistent_peer_count": status.network_diagnostics.inconsistent_peer_count,
            "healthy_sync_peer_count": status.network_diagnostics.healthy_sync_peer_count,
            "sync_warning": status.network_diagnostics.sync_warning.clone(),
            "sync_mode": status.network_diagnostics.sync_mode.clone(),
            "chain_validation_status": status.network_diagnostics.chain_validation_status.clone(),
            "best_header_height": status.network_diagnostics.best_header_height,
            "best_downloaded_body_height": status.network_diagnostics.best_downloaded_body_height,
            "best_validated_height": status.network_diagnostics.best_validated_height,
            "best_connected_height": status.network_diagnostics.best_connected_height,
            "latest_finalized_height": status.network_diagnostics.latest_finalized_height,
            "latest_finalized_hash": hex::encode(status.network_diagnostics.latest_finalized_hash),
            "pending_validation_blocks": status.network_diagnostics.pending_validation_blocks,
            "untrusted_downloaded_blocks": status.network_diagnostics.untrusted_downloaded_blocks,
            "untrusted_downloaded_bytes": status.network_diagnostics.untrusted_downloaded_bytes,
            "fast_download_enabled": status.network_diagnostics.fast_download_enabled,
            "checkpoint_anchored_sync_enabled": status.network_diagnostics.checkpoint_anchored_sync_enabled,
            "background_validation_enabled": status.network_diagnostics.background_validation_enabled,
            "safe_to_mine": status.network_diagnostics.safe_to_mine,
            "safe_to_serve": status.network_diagnostics.safe_to_serve,
            "validation_lag_blocks": status.network_diagnostics.validation_lag_blocks,
            "max_fast_download_ahead": status.network_diagnostics.max_fast_download_ahead,
            "max_untrusted_block_cache": status.network_diagnostics.max_untrusted_block_cache,
            "max_pending_validation_blocks": status.network_diagnostics.max_pending_validation_blocks,
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
        let node = &self.orchestrator.runtime.node;
        if !verbose {
            return Ok(Value::Array(
                node.mempool_txids()
                    .into_iter()
                    .map(|txid| Value::String(hex::encode(txid)))
                    .collect(),
            ));
        }
        let map = node
            .mempool_entries_iter()
            .map(|entry| {
                let depends = node
                    .mempool_dependency_txids(&entry.txid())
                    .unwrap_or_default()
                    .into_iter()
                    .map(hex::encode)
                    .collect::<Vec<_>>();
                let descendants = node
                    .mempool_descendant_txids(&entry.txid())
                    .unwrap_or_default()
                    .into_iter()
                    .map(hex::encode)
                    .collect::<Vec<_>>();
                (
                    hex::encode(entry.txid()),
                    render_mempool_entry_value(self.network(), &entry, &depends, &descendants),
                )
            })
            .collect::<serde_json::Map<String, Value>>();
        Ok(Value::Object(map))
    }

    fn command_getmempoolentry(&self, args: &[String]) -> Result<Value, atho_rpc::error::RpcError> {
        let txid = parse_hash48(&self.parse_single_string_arg("getmempoolentry", args, "txid")?)?;
        let entry = self
            .orchestrator
            .runtime
            .node
            .mempool_entry(&txid)
            .ok_or_else(|| {
                atho_rpc::error::RpcError::invalid_request("transaction is not in the mempool")
            })?;
        let depends = self
            .orchestrator
            .runtime
            .node
            .mempool_dependency_txids(&txid)
            .unwrap_or_default()
            .into_iter()
            .map(hex::encode)
            .collect::<Vec<_>>();
        let descendants = self
            .orchestrator
            .runtime
            .node
            .mempool_descendant_txids(&txid)
            .unwrap_or_default()
            .into_iter()
            .map(hex::encode)
            .collect::<Vec<_>>();
        Ok(render_mempool_entry_value(
            self.network(),
            &entry,
            &depends,
            &descendants,
        ))
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
        if !self.orchestrator.runtime.node.mempool_contains(&txid) {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "transaction is not in the mempool",
            ));
        }
        let ancestors = self
            .orchestrator
            .runtime
            .node
            .mempool_dependency_txids(&txid)
            .unwrap_or_default();
        Ok(render_mempool_relation_value(
            self.network(),
            &self.orchestrator.runtime.node,
            &ancestors,
            verbose,
        ))
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
        if !self.orchestrator.runtime.node.mempool_contains(&txid) {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "transaction is not in the mempool",
            ));
        }
        let descendants = self
            .orchestrator
            .runtime
            .node
            .mempool_descendant_txids(&txid)
            .unwrap_or_default();
        Ok(render_mempool_relation_value(
            self.network(),
            &self.orchestrator.runtime.node,
            &descendants,
            verbose,
        ))
    }

    fn command_getblocktemplate(
        &self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        self.expect_no_args("getblocktemplate", args)?;
        if let Some(reason) = Self::mining_template_sync_block_reason(&self.node_status()) {
            return Err(atho_rpc::error::RpcError::invalid_request(reason));
        }
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
        if let Some(reason) = Self::mining_template_sync_block_reason(&self.node_status()) {
            return Err(atho_rpc::error::RpcError::invalid_request(reason));
        }
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
        let fast = self.orchestrator.sync.fast_download_diagnostics(node);
        let status = self.node_status();
        let mining_blocked_reason = Self::mining_template_sync_block_reason(&status);
        Ok(json!({
            "network": self.network().id(),
            "height": node.height(),
            "best_block_hash": hex::encode(node.tip_hash()),
            "next_target": hex::encode(node.difficulty_target_for_next_block()),
            "mempool_transaction_count": node.mempool_len(),
            "mempool_total_fee_atoms": node.mempool_total_fee_atoms(),
            "mining_reward_address": node.config.mining_reward_address.as_str(),
            "headers_synced": self.orchestrator.sync.sync_state().headers_synced,
            "chain_synced": Self::chain_synced(&status),
            "safe_to_mine": fast.safe_to_mine,
            "mining_allowed": mining_blocked_reason.is_none(),
            "mining_blocked_reason": mining_blocked_reason,
            "chain_validation_status": fast.chain_validation_status,
            "sync_mode": fast.sync_mode,
            "validation_lag_blocks": fast.validation_lag_blocks,
            "pending_validation_blocks": fast.pending_validation_blocks,
            "best_downloaded_body_height": fast.best_downloaded_body_height,
            "best_validated_height": fast.best_validated_height,
        }))
    }

    fn command_setminingrewardaddress(
        &mut self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        let address = self.parse_single_string_arg("setminingrewardaddress", args, "address")?;
        let address = address.trim();
        if address.is_empty() {
            return Err(atho_rpc::error::RpcError::invalid_request(
                "setminingrewardaddress expects a non-empty address",
            ));
        }
        let (payment_digest, network) = decode_base56_address(address).map_err(|err| {
            atho_rpc::error::RpcError::invalid_request(format!(
                "mining reward address is not a valid Atho address: {err}"
            ))
        })?;
        if network != self.network() {
            return Err(atho_rpc::error::RpcError::invalid_request(format!(
                "mining reward address targets {} but node is on {}",
                network.id(),
                self.network().id()
            )));
        }

        self.orchestrator.runtime.node.config.mining_reward_address = address.to_string();
        let _ = dev::append_log(
            "athod",
            &format!(
                "updated runtime mining reward address network={} address={}",
                network.id(),
                address
            ),
        );
        Ok(json!({
            "address": address,
            "network": network.id(),
            "payment_digest": hex::encode(payment_digest),
            "updated": true,
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
        if let Some(entry) = self.orchestrator.runtime.node.mempool_entry(&txid) {
            let fee_atoms = entry.fee_atoms;
            return Ok(json!({
                "source": "mempool",
                "fee_atoms": fee_atoms,
                "fee_atho": format_atoms_decimal(self.network(), fee_atoms),
                "transaction": render_transaction_value_with_fee(self.network(), 0, &entry.transaction, Some(entry.fee_atoms)),
            }));
        }
        if let Some(record) = self
            .orchestrator
            .runtime
            .node
            .transaction_record_by_txid(txid)
        {
            let fee_atoms = self.estimate_transaction_fee_atoms(&record.transaction);
            let mut response = json!({
                "source": "chain",
                "block_hash": hex::encode(record.block_hash),
                "height": record.height,
                "transaction": render_transaction_value_with_fee(
                    self.network(),
                    record.tx_index as usize,
                    &record.transaction,
                    fee_atoms,
                ),
            });
            if let Some(fee_atoms) = fee_atoms {
                response["fee_atoms"] = json!(fee_atoms);
                response["fee_atho"] = json!(format_atoms_decimal(self.network(), fee_atoms));
            }
            return Ok(response);
        }
        Err(atho_rpc::error::RpcError::invalid_request(
            "unknown transaction txid",
        ))
    }

    fn command_sendrawtransaction(
        &mut self,
        args: &[String],
    ) -> Result<Value, atho_rpc::error::RpcError> {
        let raw_tx_hex = self.parse_single_string_arg("sendrawtransaction", args, "raw_tx_hex")?;
        self.broadcast_raw_transaction_hex_value(&raw_tx_hex)
    }

    pub(crate) fn broadcast_raw_transaction_hex_value(
        &mut self,
        raw_tx_hex: &str,
    ) -> Result<Value, atho_rpc::error::RpcError> {
        let transaction = parse_raw_transaction_hex(raw_tx_hex)?;
        self.broadcast_transaction_value(transaction)
    }

    fn broadcast_transaction_value(
        &mut self,
        transaction: Transaction,
    ) -> Result<Value, atho_rpc::error::RpcError> {
        let txid = transaction.txid();
        let wtxid = transaction.wtxid();
        let size_bytes = transaction.full_size_bytes();
        let vsize_bytes = transaction.vsize_bytes().max(1);
        let tx_summary = dev::summarize_transaction(&transaction, None);
        let status = self.node_status();
        if let Some(reason) = Self::transaction_submission_sync_block_reason(&status) {
            let _ = dev::append_log(
                "athod",
                &format!("raw tx rejected before admission error={reason} {tx_summary}"),
            );
            return Err(atho_rpc::error::RpcError::invalid_request(reason));
        }

        match self
            .orchestrator
            .runtime
            .node
            .accept_relayed_transaction(transaction)
        {
            Ok(submitted) => {
                self.refresh_runtime_views();
                let entry = self
                    .orchestrator
                    .runtime
                    .node
                    .mempool_entry(&submitted)
                    .ok_or_else(atho_rpc::error::RpcError::internal)?;
                let fee_atoms = entry.fee_atoms;
                let feerate_atoms_per_vbyte = entry.feerate_atoms_per_vbyte();
                let relay_ready = self.p2p_transaction_relay_ready();
                let _ = dev::append_log(
                    "athod",
                    &format!(
                        "raw tx submitted txid={} fee_atoms={} mempool={} relay_ready={} {tx_summary}",
                        hex::encode(submitted),
                        fee_atoms,
                        self.orchestrator.runtime.node.mempool_len(),
                        relay_ready
                    ),
                );
                Ok(json!({
                    "accepted": true,
                    "txid": hex::encode(txid),
                    "wtxid": hex::encode(wtxid),
                    "fee_atoms": fee_atoms,
                    "fee_atho": format_atoms_decimal(self.network(), fee_atoms),
                    "size_bytes": size_bytes,
                    "vsize_bytes": vsize_bytes,
                    "feerate_atoms_per_vbyte": feerate_atoms_per_vbyte,
                    "mempool_count": self.orchestrator.runtime.node.mempool_len(),
                    "relay_ready": relay_ready,
                    "relay_status": if relay_ready { "ready" } else { "accepted_pending_sync" },
                }))
            }
            Err(err) => {
                let rpc_error = rpc_error_from_node(err);
                let _ = dev::append_log(
                    "athod",
                    &format!("raw tx rejected error={rpc_error} {tx_summary}"),
                );
                Err(rpc_error)
            }
        }
    }

    fn estimate_transaction_fee_atoms(&self, tx: &Transaction) -> Option<u64> {
        if tx.is_coinbase() {
            return Some(0);
        }

        let mut input_total_atoms = 0u128;
        for input in &tx.inputs {
            let previous = self
                .orchestrator
                .runtime
                .node
                .transaction_record_by_txid(input.previous_txid)?;
            let output = previous
                .transaction
                .outputs
                .get(input.output_index as usize)?;
            input_total_atoms = input_total_atoms.checked_add(output.value_atoms as u128)?;
        }

        let output_total_atoms = tx.outputs.iter().fold(0u128, |acc, output| {
            acc.saturating_add(output.value_atoms as u128)
        });
        let fee_atoms = input_total_atoms.checked_sub(output_total_atoms)?;
        fee_atoms.try_into().ok()
    }

    fn render_block_value_with_fees(&self, block: &Block) -> Value {
        let mut total_fee_atoms = 0u64;
        let transactions = block
            .transactions
            .iter()
            .enumerate()
            .map(|(index, tx)| {
                let fee_atoms = self.estimate_transaction_fee_atoms(tx);
                if !tx.is_coinbase() {
                    if let Some(fee_atoms) = fee_atoms {
                        total_fee_atoms = total_fee_atoms.saturating_add(fee_atoms);
                    }
                }
                render_transaction_value_with_fee(block.header.network_id, index, tx, fee_atoms)
            })
            .collect::<Vec<_>>();
        render_block_value_with_transactions(block, transactions, Some(total_fee_atoms))
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

        let bootstrap_addresses = configured_bootstrap_peers(network)
            .into_iter()
            .filter_map(|peer| parse_remote_addr(&peer, network.p2p_port()))
            .collect::<Vec<_>>();
        if bootstrap_addresses.is_empty() {
            return;
        }
        let accepted = match self
            .orchestrator
            .sync
            .seed_peer_addresses(&bootstrap_addresses)
        {
            Ok(accepted) => accepted,
            Err(err) => {
                let _ = dev::append_log(
                    "p2p",
                    &format!(
                        "bootstrap peer seed failed network={} error={err}",
                        network.id()
                    ),
                );
                return;
            }
        };
        if !accepted.is_empty() && public_source {
            let _ = dev::append_log(
                "p2p",
                &format!(
                    "seeded {} configured bootstrap peer address(es) into discovery graph",
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
    let transactions = block
        .transactions
        .iter()
        .enumerate()
        .map(|(index, tx)| {
            render_transaction_value_with_fee(block.header.network_id, index, tx, None)
        })
        .collect::<Vec<_>>();
    render_block_value_with_transactions(block, transactions, None)
}

fn render_block_value_with_transactions(
    block: &Block,
    transactions: Vec<Value>,
    total_fee_atoms: Option<u64>,
) -> Value {
    let total_fee_atoms = total_fee_atoms.unwrap_or_default();
    json!({
        "hash": hex::encode(block.header.block_hash()),
        "header": render_block_header_value(&block.header),
        "transaction_count": block.transactions.len(),
        "size_bytes": block.size_bytes(),
        "weight_bytes": block.weight_bytes(),
        "vsize_bytes": block.vsize_bytes(),
        "coinbase_txid": block.transactions.first().map(|tx| hex::encode(tx.txid())).unwrap_or_default(),
        "fees_total_atoms": total_fee_atoms,
        "fees_total_atho": format_atoms_decimal(block.header.network_id, total_fee_atoms),
        "transactions": transactions,
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
        "tx_pow_nonce": tx.tx_pow_nonce,
        "tx_pow_bits": tx.tx_pow_bits,
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

fn render_transaction_value_with_fee(
    network: Network,
    index: usize,
    tx: &Transaction,
    fee_atoms: Option<u64>,
) -> Value {
    let mut rendered = render_transaction_value(network, index, tx);
    if let Value::Object(ref mut object) = rendered {
        if let Some(fee_atoms) = fee_atoms {
            object.insert(String::from("fee_atoms"), json!(fee_atoms));
            object.insert(
                String::from("fee_atho"),
                json!(format_atoms_decimal(network, fee_atoms)),
            );
        }
    }
    rendered
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
        "value_atho": format_atoms_decimal(network, output.value_atoms),
        "locking_script_bytes": output.locking_script.len(),
        "locking_script_hex": hex::encode(&output.locking_script),
        "address_hint": script_address_hint(network, &output.locking_script),
    })
}

fn script_address_hint(network: Network, locking_script: &[u8]) -> Option<String> {
    let digest: [u8; 32] = locking_script.try_into().ok()?;
    Some(encode_base56_address(network, &digest))
}

fn format_atoms_decimal(network: Network, atoms: u64) -> String {
    let scale = atho_core::constants::atoms_per_atho_for_network(network);
    let decimals = atho_core::constants::decimals_for_network(network);
    let whole = atoms / scale;
    let fractional = atoms % scale;
    format!("{whole}.{fractional:0decimals$}")
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
    let confirmations = entry.confirmation_count(spend_height);
    let required_confirmations = entry.required_confirmations();
    json!({
        "txid": hex::encode(entry.txid),
        "vout": entry.output_index,
        "value_atoms": entry.value_atoms,
        "value_atho": format_atoms_decimal(entry.network, entry.value_atoms),
        "confirmations": confirmations,
        "coinbase": entry.is_coinbase,
        "required_confirmations": required_confirmations,
        "remaining_confirmations": required_confirmations.saturating_sub(confirmations),
        "spendable": entry.is_spendable_at(spend_height),
        "locking_script_hex": hex::encode(&entry.locking_script),
        "address_hint": script_address_hint(entry.network, &entry.locking_script),
        "created_height": entry.created_height,
    })
}

#[derive(Debug, Clone, Copy)]
struct UtxoSetStats {
    txouts: usize,
    bogosize: usize,
    total_amount_atoms: u64,
    hash: [u8; 48],
}

fn utxo_set_stats<'a, I>(entries: I) -> UtxoSetStats
where
    I: IntoIterator<Item = &'a UtxoEntry>,
{
    let mut txouts = 0usize;
    let mut bogosize = 0usize;
    let mut total_amount_atoms = 0u64;
    let mut bytes = Vec::new();
    for entry in entries {
        txouts = txouts.saturating_add(1);
        bogosize = bogosize.saturating_add(48usize + 4 + 8 + entry.locking_script.len());
        total_amount_atoms = total_amount_atoms.saturating_add(entry.value_atoms);
        bytes.extend_from_slice(&entry.txid);
        bytes.extend_from_slice(&entry.output_index.to_le_bytes());
        bytes.extend_from_slice(&entry.value_atoms.to_le_bytes());
        bytes.extend_from_slice(&(entry.locking_script.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&entry.locking_script);
        bytes.extend_from_slice(&entry.created_height.to_le_bytes());
        bytes.push(u8::from(entry.is_coinbase));
        bytes.push(entry.network.consensus_id());
    }
    UtxoSetStats {
        txouts,
        bogosize,
        total_amount_atoms,
        hash: sha3_384(&bytes),
    }
}

fn render_mempool_entry_value(
    network: Network,
    entry: &MempoolEntry,
    depends: &[String],
    descendants: &[String],
) -> Value {
    json!({
        "txid": hex::encode(entry.txid()),
        "wtxid": hex::encode(entry.wtxid()),
        "fee_atoms": entry.fee_atoms,
        "fee_atho": format_atoms_decimal(network, entry.fee_atoms),
        "base_size_bytes": entry.base_size_bytes(),
        "size_bytes": entry.raw_size_bytes(),
        "vsize_bytes": entry.vsize_bytes(),
        "feerate_atoms_per_vbyte": entry.feerate_atoms_per_vbyte(),
        "received_at_unix": entry.received_at_unix(),
        "depends": depends,
        "ancestor_count": depends.len(),
        "descendant_count": descendants.len(),
        "descendants": descendants,
    })
}

fn mempool_status_label(transaction_count: usize, mempool_vsize_bytes: u64) -> &'static str {
    if transaction_count == 0 {
        "Empty"
    } else if transaction_count >= 100 || mempool_vsize_bytes >= 250_000 {
        "Busy"
    } else {
        "Normal"
    }
}

fn render_mempool_relation_value(
    network: Network,
    node: &crate::node::Node,
    txids: &[[u8; 48]],
    verbose: bool,
) -> Value {
    if !verbose {
        return Value::Array(
            txids
                .iter()
                .map(|txid| Value::String(hex::encode(txid)))
                .collect(),
        );
    }
    let map = txids
        .iter()
        .filter_map(|txid| {
            node.mempool_entry(txid).map(|entry| {
                let depends = node
                    .mempool_dependency_txids(&entry.txid())
                    .unwrap_or_default()
                    .into_iter()
                    .map(hex::encode)
                    .collect::<Vec<_>>();
                let descendants = node
                    .mempool_descendant_txids(&entry.txid())
                    .unwrap_or_default()
                    .into_iter()
                    .map(hex::encode)
                    .collect::<Vec<_>>();
                (
                    hex::encode(txid),
                    render_mempool_entry_value(network, &entry, &depends, &descendants),
                )
            })
        })
        .collect::<serde_json::Map<String, Value>>();
    Value::Object(map)
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
            &utxos,
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
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::mempool::MempoolEntry;
    use crate::test_support::acquire_global_test_lock;
    use atho_core::address::encode_base56_address;
    use atho_core::consensus::tx_policy::minimum_required_fee_atoms;
    use atho_core::network::Network;
    use atho_core::transaction::Transaction;
    use atho_rpc::request::RpcRequest;
    use atho_rpc::response::RpcResponse;
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

    fn synthetic_block_record(height: u64, timestamp: u64) -> BlockArchiveRecord {
        BlockArchiveRecord {
            height,
            block_hash: [height as u8; 48],
            previous_block_hash: [height.saturating_sub(1) as u8; 48],
            network: Network::Mainnet,
            version: 1,
            merkle_root: [0; 48],
            witness_root: [0; 48],
            timestamp,
            difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
            nonce: height,
            file_number: 0,
            record_offset: 0,
            payload_length: 0,
            raw_block_size: 0,
            weight_bytes: 0,
            vsize_bytes: 0,
            tx_count: 1,
            fees_total_atoms: 0,
            fees_miner_atoms: 0,
            chainwork: Vec::new(),
            fully_validated: true,
            main_chain: true,
            pruned: false,
            persisted_unix: timestamp,
        }
    }

    fn sign_seed_spend_with_output_lock(
        network: Network,
        seed_txid: [u8; 48],
        seed_value: u64,
        seed_script: Vec<u8>,
        output_locking_script: Vec<u8>,
    ) -> Transaction {
        let mut seed = Vec::with_capacity(network.id().len() + seed_txid.len());
        seed.extend_from_slice(network.id().as_bytes());
        seed.extend_from_slice(&seed_txid);
        let keypair = atho_crypto::falcon::generate_from_seed(&seed).expect("seed keypair");
        let mut output_atoms = seed_value.saturating_sub(1);

        for _ in 0..4 {
            let mut tx = Transaction {
                version: 1,
                inputs: vec![TxInput {
                    previous_txid: seed_txid,
                    output_index: 0,
                    unlocking_script: seed_script.clone(),
                }],
                outputs: vec![TxOutput {
                    value_atoms: output_atoms,
                    locking_script: output_locking_script.clone(),
                }],
                lock_time: 0,
                witness: vec![],
                tx_pow_nonce: 0,
                tx_pow_bits: 0,
            };
            let digest = atho_core::consensus::signatures::transaction_signing_digest(network, &tx);
            let signature = atho_crypto::falcon::sign(
                atho_core::consensus::signatures::AthoSignatureDomain::Transaction,
                &keypair.secret_key,
                &digest,
            )
            .expect("signature");
            let sig_bytes = signature.0.clone();
            tx.witness = atho_core::transaction::TxWitness {
                signature: sig_bytes.clone(),
                pubkey: keypair.public_key.0.clone(),
                input_refs: vec![atho_core::transaction::WitnessInputRef {
                    input_index: 0,
                    sig_ref_short: crate::validation::derive_sig_ref_short(
                        &tx.txid(),
                        &sig_bytes,
                        0,
                    ),
                    witness_commit_ref: [0; 16],
                }],
                additional_signers: vec![],
            }
            .canonical_bytes();
            let fee_atoms = minimum_required_fee_atoms(network, &tx);
            if seed_value.saturating_sub(fee_atoms) == output_atoms {
                let txid = tx.txid();
                let witness_root = tx.witness_commitment_hash();
                tx.witness = atho_core::transaction::TxWitness {
                    signature: sig_bytes.clone(),
                    pubkey: keypair.public_key.0.clone(),
                    input_refs: vec![atho_core::transaction::WitnessInputRef {
                        input_index: 0,
                        sig_ref_short: crate::validation::derive_sig_ref_short(
                            &txid, &sig_bytes, 0,
                        ),
                        witness_commit_ref: crate::validation::derive_witness_commit_ref(
                            &txid,
                            &witness_root,
                            0,
                        ),
                    }],
                    additional_signers: vec![],
                }
                .canonical_bytes();
                atho_core::consensus::tx_policy::solve_transaction_pow(network, &mut tx, fee_atoms);
                return tx;
            }
            output_atoms = seed_value.saturating_sub(fee_atoms);
        }

        panic!("failed to stabilize legacy-lock test spend fee");
    }

    #[test]
    fn chain_stats_hashrate_uses_completed_block_intervals() {
        let records = vec![
            synthetic_block_record(0, 1_000),
            synthetic_block_record(1, 1_010),
            synthetic_block_record(2, 1_020),
        ];
        let mined_blocks = records[1..]
            .iter()
            .map(|record| Block {
                header: record.header(),
                transactions: Vec::new(),
                witnesses: Default::default(),
                fees_total_atoms: 0,
                fees_miner_atoms: 0,
            })
            .collect::<Vec<_>>();
        let expected =
            pow::accumulated_chain_work(&mined_blocks) / num_bigint::BigUint::from(20u64);

        assert_eq!(
            estimated_hashrate_from_records(&records, 2),
            u64::try_from(expected).unwrap_or(u64::MAX)
        );
        assert_eq!(average_block_time_from_records(&records, 2), 10_000);
    }

    #[test]
    fn explorer_snapshot_restores_cached_index_and_stats_on_restart() {
        let root = temp_data_dir("explorer-snapshot-restore");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        let miner = Miner::new(1);
        service.sandbox_with_node_mut(|node| {
            node.mine_and_connect_candidate_block(&miner)
                .expect("mine cached block");
        });

        let snapshot_path = explorer_api_snapshot_path(Network::Regnet);
        assert!(snapshot_path.exists(), "snapshot should be persisted");
        let expected_height = service.node_ref().height();
        let expected_tip_hash = service.node_ref().tip_hash();
        let expected_total_transactions = service.chain_stats().total_transactions;

        drop(service);

        let mut restored = NodeService::new(NodeConfig::new(Network::Regnet));
        restored.start();
        assert!(restored.explorer_index_ready());
        assert_eq!(restored.explorer_index_source(), "snapshot");
        assert_eq!(restored.explorer_index().tip_height(), expected_height);
        assert_eq!(restored.explorer_index().tip_hash(), expected_tip_hash);
        assert_eq!(restored.chain_stats().tip_hash, expected_tip_hash);
        assert_eq!(
            restored.chain_stats().total_transactions,
            expected_total_transactions
        );
        assert!(restored.explorer_snapshot_persisted_unix().is_some());
    }

    #[test]
    fn invalid_explorer_snapshot_is_ignored_and_live_views_rebuild() {
        let root = temp_data_dir("explorer-snapshot-invalid");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        let miner = Miner::new(1);
        service.sandbox_with_node_mut(|node| {
            node.mine_and_connect_candidate_block(&miner)
                .expect("mine cached block");
        });

        let snapshot_path = explorer_api_snapshot_path(Network::Regnet);
        let bytes = fs::read(&snapshot_path).expect("snapshot bytes");
        let mut snapshot: ExplorerApiSnapshot =
            bincode::deserialize(&bytes).expect("decode snapshot");
        snapshot.genesis_hash = [0xAA; 48];
        write_explorer_api_snapshot(&snapshot_path, &snapshot).expect("rewrite corrupted snapshot");

        drop(service);

        let mut restored = NodeService::new(NodeConfig::new(Network::Regnet));
        restored.start();
        assert!(restored.explorer_index_ready());
        assert_eq!(restored.explorer_index_source(), "rebuilt");
    }

    #[test]
    fn p2p_status_refresh_does_not_rebuild_explorer_index_on_hot_path() {
        let root = temp_data_dir("p2p-status-refresh-no-explorer-rebuild");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.refresh_api_views();
        assert!(service.explorer_index_ready());
        assert_eq!(service.explorer_index_source(), "rebuilt");
        let indexed_tip = service.explorer_index().tip_hash();

        service
            .orchestrator
            .runtime
            .node
            .mine_and_connect_candidate_block(&Miner::new(1))
            .expect("mine block without API refresh");
        let new_tip = service.orchestrator.runtime.node.tip_hash();
        assert_ne!(indexed_tip, new_tip);

        service.refresh_runtime_status_views();
        assert_eq!(
            service.explorer_index().tip_hash(),
            indexed_tip,
            "P2P status refresh must not rebuild the full explorer index"
        );
        assert_eq!(service.orchestrator.rpc_server.block_count, 1);

        service.refresh_api_views();
        assert_eq!(service.explorer_index().tip_hash(), new_tip);
        assert_eq!(service.explorer_index_source(), "incremental");
    }

    #[test]
    fn explorer_index_incremental_refresh_matches_full_rebuild() {
        let root = temp_data_dir("explorer-incremental-refresh");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.refresh_api_views();
        assert!(service.explorer_index_ready());
        assert_eq!(service.explorer_index_source(), "rebuilt");

        service
            .orchestrator
            .runtime
            .node
            .mine_and_connect_candidate_block(&Miner::new(1))
            .expect("mine block");

        service.refresh_api_views();
        assert_eq!(service.explorer_index_source(), "incremental");

        let rebuilt = ExplorerIndex::rebuild(service.node_ref()).expect("rebuild");
        assert_eq!(service.explorer_index(), &rebuilt);
    }

    #[test]
    fn gettxoutsetinfo_matches_listed_utxo_stats() {
        let root = temp_data_dir("gettxoutsetinfo-stats");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service
            .orchestrator
            .runtime
            .node
            .mine_and_connect_candidate_block(&Miner::new(1))
            .expect("mine block");

        let listed = service.list_utxos();
        let stats = utxo_set_stats(listed.iter());
        let value = service
            .command_gettxoutsetinfo(&[])
            .expect("gettxoutsetinfo");

        assert_eq!(value["txouts"], stats.txouts);
        assert_eq!(value["bogosize"], stats.bogosize);
        assert_eq!(value["total_amount_atoms"], stats.total_amount_atoms);
        assert_eq!(value["utxo_set_hash"], hex::encode(stats.hash));
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
    fn p2p_prime_seeds_configured_testnet_bootstrap_peers_for_relay() {
        let root = temp_data_dir("configured-testnet-bootstrap-peers");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Testnet));
        service.p2p_prime();

        let peers = service.p2p_bootstrap_peers(16);
        assert!(peers
            .iter()
            .any(|peer| peer == "testnet-node2.atho.io:9100"));
        assert!(peers.iter().any(|peer| peer == "74.208.219.116:9100"));
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
    fn bootstrap_peers_rescue_backed_off_records_when_no_other_peer_is_available() {
        let root = temp_data_dir("bootstrap-health-rescue");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.sandbox_with_node_mut(|node| {
            node.observe_peer("6.6.6.6:9200", 12, 1_700_000_000)
                .expect("peer observation");
        });

        let now = unix_timestamp();
        service.p2p_save_peer_health(&PeerHealthRecord {
            network: Network::Regnet,
            remote_addr: String::from("6.6.6.6:9200"),
            quality_score: 40,
            consecutive_failures: 8,
            backoff_until_unix: now.saturating_add(3_600),
            last_failure_unix: Some(now),
            last_success_unix: Some(now.saturating_sub(600)),
        });
        service.p2p_prime();

        let peers = service.p2p_bootstrap_peers(8);
        assert!(
            peers.iter().any(|peer| peer == "6.6.6.6:9200"),
            "a node with no live peers must keep a rescue dial path even if every known peer is backed off"
        );
    }

    #[test]
    fn bootstrap_peers_prefer_subnet_diversity_before_same_subnet_fill() {
        let root = temp_data_dir("bootstrap-subnet-diversity");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.sandbox_with_node_mut(|node| {
            node.observe_peer("8.8.1.1:9200", 12, 1_700_000_010)
                .expect("peer observation");
            node.observe_peer("8.8.1.2:9200", 12, 1_700_000_009)
                .expect("peer observation");
            node.observe_peer("9.9.9.9:9200", 12, 1_700_000_008)
                .expect("peer observation");
        });

        let now = unix_timestamp();
        for (remote_addr, quality_score) in [
            ("8.8.1.1:9200", 100),
            ("8.8.1.2:9200", 99),
            ("9.9.9.9:9200", 80),
        ] {
            service.p2p_save_peer_health(&PeerHealthRecord {
                network: Network::Regnet,
                remote_addr: String::from(remote_addr),
                quality_score,
                consecutive_failures: 0,
                backoff_until_unix: 0,
                last_failure_unix: None,
                last_success_unix: Some(now),
            });
        }

        service.p2p_prime();

        let peers = service.p2p_bootstrap_peers(3);
        assert_eq!(peers.first().map(String::as_str), Some("8.8.1.1:9200"));
        assert_eq!(peers.get(1).map(String::as_str), Some("9.9.9.9:9200"));
        assert_eq!(peers.get(2).map(String::as_str), Some("8.8.1.2:9200"));
    }

    #[test]
    fn topology_roles_classify_fast_ready_outbound_peer() {
        let roles = NodeService::classify_peer_roles(
            NetworkPeerDirection::Outbound,
            true,
            Some(128),
            Some(NODE_NETWORK),
            None,
            64,
            128,
            false,
        );

        for role in [
            "OUTBOUND_PEER",
            "FULL_RELAY_PEER",
            "SYNC_PEER",
            "BLOCK_RELAY_PEER",
            "TX_RELAY_PEER",
            "ADDR_RELAY_PEER",
        ] {
            assert!(roles.iter().any(|value| value == role), "missing {role}");
        }
    }

    #[test]
    fn topology_health_score_warns_on_weak_peer_shape() {
        let (score, warnings) =
            NodeService::topology_health(Network::Mainnet, &[], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, None);

        assert!(score < 60);
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("low outbound peer count")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("peer database")));
    }

    #[test]
    fn block_template_exposes_canonical_header_bytes_for_miners() {
        let root = temp_data_dir("block-template");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.start();
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
    fn getblocktemplate_pauses_until_public_node_has_ready_peer() {
        let root = temp_data_dir("block-template-public-sync");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Testnet));
        service.start();

        let response = service.handle(RpcRequest::GetBlockTemplate);
        let RpcResponse::Error(error) = response else {
            panic!("expected getblocktemplate sync error, got {response:?}");
        };
        assert!(error
            .details
            .unwrap_or_default()
            .contains("no ready network peers"));
    }

    #[test]
    fn stale_mined_block_submit_is_rejected_before_creating_rpc_fork() {
        let root = temp_data_dir("stale-rpc-mined-block");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.start();
        let response = service.handle(RpcRequest::GetBlockTemplate);
        let RpcResponse::BlockTemplate(template) = response else {
            panic!("expected block template, got {response:?}");
        };
        let stale_block = Miner::new(1).solve_block(template.block);

        service
            .p2p_mine_local_block()
            .expect("competing local block");
        assert_eq!(service.node_ref().height(), 1);

        let response = service.handle_mut(RpcRequest::SubmitBlock(stale_block));
        let RpcResponse::Error(error) = response else {
            panic!("expected stale block submit error, got {response:?}");
        };
        assert_eq!(error.code, atho_errors::BLK_INVALID_HEIGHT.code.as_str());
        assert!(error
            .details
            .unwrap_or_default()
            .contains("stale mining template"));
        assert_eq!(service.node_ref().height(), 1);
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
    fn execute_command_geterrorcodes_can_explain_one_code() {
        let root = temp_data_dir("command-errorcodes");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let service = NodeService::new(NodeConfig::new(Network::Regnet));
        let response = service.handle(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "geterrorcodes",
            vec![String::from("ATHO-RPC-002")],
        )));
        let RpcResponse::Command(command) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(command.command, "geterrorcodes");
        assert_eq!(command.data["code"], "ATHO-RPC-002");
        assert_eq!(command.data["category"], "rpc");
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
    fn execute_command_setminingrewardaddress_updates_runtime_config() {
        let root = temp_data_dir("command-set-mining-reward-address");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Testnet));
        let address = encode_base56_address(Network::Testnet, &[8u8; 32]);
        let response = service.handle_mut(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "setminingrewardaddress",
            vec![address.clone()],
        )));
        let RpcResponse::Command(command) = response else {
            panic!("unexpected response: {response:?}");
        };

        assert_eq!(command.command, "setminingrewardaddress");
        assert_eq!(command.data["address"], address);
        assert_eq!(
            service
                .orchestrator
                .runtime
                .node
                .config
                .mining_reward_address,
            address
        );
    }

    #[test]
    fn format_atoms_decimal_keeps_exact_integer_scale() {
        assert_eq!(format_atoms_decimal(Network::Mainnet, 1), "0.00000001");
        assert_eq!(format_atoms_decimal(Network::Mainnet, 100), "0.00000100");
        assert_eq!(
            format_atoms_decimal(Network::Mainnet, 100_000_000),
            "1.00000000"
        );
        assert_eq!(
            format_atoms_decimal(Network::Mainnet, 1_234_567_890),
            "12.34567890"
        );
    }

    #[test]
    fn rendered_status_marks_peer_target_above_local_height_as_not_synced() {
        let status = NodeStatus {
            network: Network::Mainnet,
            block_count: 0,
            tip_hash: [0x11; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x22; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 128,
            network_diagnostics: NetworkDiagnostics {
                peer_count: 1,
                ..NetworkDiagnostics::default()
            },
        };

        let rendered = NodeService::render_status_value(&status);
        assert_eq!(rendered["local_height"], 0);
        assert_eq!(rendered["sync_target_height"], 128);
        assert_eq!(rendered["chain_synced"], false);
        assert_eq!(rendered["headers_synced"], true);
    }

    #[test]
    fn rendered_status_marks_validation_lag_as_not_synced_or_safe_to_mine() {
        let status = NodeStatus {
            network: Network::Mainnet,
            block_count: 128,
            tip_hash: [0x12; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x23; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 128,
            network_diagnostics: NetworkDiagnostics {
                peer_count: 1,
                best_header_height: 130,
                best_downloaded_body_height: 130,
                best_validated_height: 128,
                best_connected_height: 128,
                pending_validation_blocks: 2,
                untrusted_downloaded_blocks: 2,
                chain_validation_status: String::from("body_download_ahead"),
                sync_mode: String::from("checkpoint_anchored_downloading"),
                safe_to_mine: false,
                safe_to_serve: false,
                validation_lag_blocks: 2,
                ..NetworkDiagnostics::default()
            },
        };

        let rendered = NodeService::render_status_value(&status);

        assert_eq!(rendered["chain_synced"], false);
        assert_eq!(rendered["safe_to_mine"], false);
        assert_eq!(rendered["safe_to_serve"], false);
        assert_eq!(rendered["chain_validation_status"], "body_download_ahead");
        assert_eq!(rendered["validation_lag_blocks"], 2);
        assert!(NodeService::health_warnings(&status)
            .contains(&"block bodies are downloaded ahead of validation"));
    }

    #[test]
    fn transaction_submission_is_paused_until_chain_is_synced() {
        let mut status = NodeStatus {
            network: Network::Mainnet,
            block_count: 7,
            tip_hash: [0x21; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x55; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 9,
            network_diagnostics: NetworkDiagnostics {
                peer_count: 1,
                ..NetworkDiagnostics::default()
            },
        };

        let reason = NodeService::transaction_submission_sync_block_reason(&status)
            .expect("stale node should block tx submission");
        assert!(reason.contains("local_height=7"));
        assert!(reason.contains("sync_target_height=9"));

        status.block_count = 9;
        status.network_diagnostics.safe_to_mine = true;
        status.network_diagnostics.safe_to_serve = true;
        status.network_diagnostics.chain_validation_status.clear();
        assert!(NodeService::transaction_submission_sync_block_reason(&status).is_none());

        status.headers_synced = false;
        let reason = NodeService::transaction_submission_sync_block_reason(&status)
            .expect("header sync should block tx submission");
        assert!(reason.contains("headers are still synchronizing"));
    }

    #[test]
    fn mining_template_is_paused_when_peer_target_is_above_local_tip() {
        let status = NodeStatus {
            network: Network::Testnet,
            block_count: 7,
            tip_hash: [0x2a; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x58; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 9,
            network_diagnostics: NetworkDiagnostics {
                peer_count: 1,
                safe_to_mine: true,
                safe_to_serve: true,
                ..NetworkDiagnostics::default()
            },
        };

        let reason = NodeService::mining_template_sync_block_reason(&status)
            .expect("stale public node should not receive mining templates");
        assert!(reason.contains("local chain tip is behind"));
        assert!(reason.contains("local_height=7"));
        assert!(reason.contains("sync_target_height=9"));
    }

    #[test]
    fn transaction_submission_requires_ready_peer_on_public_networks() {
        let status = NodeStatus {
            network: Network::Testnet,
            block_count: 9,
            tip_hash: [0x25; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x57; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 9,
            network_diagnostics: NetworkDiagnostics::default(),
        };

        let reason = NodeService::transaction_submission_sync_block_reason(&status)
            .expect("public network without ready peers should block tx submission");
        assert!(reason.contains("no ready network peers"));
        assert!(reason.contains("peer_count=0"));

        let mut regnet = status;
        regnet.network = Network::Regnet;
        regnet.network_diagnostics.safe_to_mine = true;
        regnet.network_diagnostics.safe_to_serve = true;
        regnet.network_diagnostics.chain_validation_status.clear();
        assert!(NodeService::transaction_submission_sync_block_reason(&regnet).is_none());
    }

    #[test]
    fn sendrawtransaction_accepts_canonical_raw_transaction_and_computes_fee() {
        let root = temp_data_dir("sendrawtransaction-accept");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.start();
        let (seed_txid, seed_value, seed_script) = crate::dev::seed_utxo(Network::Regnet);
        service.sandbox_with_node_mut(|node| {
            node.dev_seed_chainstate(
                6,
                node.tip_hash(),
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
            .expect("seed spendable utxo");
        });

        let transaction = crate::dev::signed_spend_transaction(
            Network::Regnet,
            seed_txid,
            seed_value,
            seed_script,
        )
        .expect("signed raw transaction");
        let txid = transaction.txid();
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &transaction);
        let raw_tx_hex = hex::encode(transaction.full_bytes());
        let response = service.handle_mut(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "sendrawtransaction",
            vec![raw_tx_hex],
        )));

        let RpcResponse::Command(command) = response else {
            panic!("unexpected sendrawtransaction response: {response:?}");
        };
        assert_eq!(command.command, "sendrawtransaction");
        assert_eq!(command.data["accepted"], true);
        assert_eq!(command.data["txid"], hex::encode(txid));
        assert_eq!(command.data["fee_atoms"], fee_atoms);
        assert_eq!(service.node_ref().mempool_len(), 1);
        assert!(service.node_ref().mempool_contains(&txid));
    }

    #[test]
    fn parse_raw_transaction_hex_rejects_trailing_bytes_noncanonical_encoding() {
        let (seed_txid, seed_value, seed_script) = crate::dev::seed_utxo(Network::Regnet);
        let transaction = crate::dev::signed_spend_transaction(
            Network::Regnet,
            seed_txid,
            seed_value,
            seed_script,
        )
        .expect("signed transaction");
        let mut raw = transaction.full_bytes();
        raw.push(0);

        let error = parse_raw_transaction_hex(&hex::encode(raw)).unwrap_err();
        let rendered = error.to_string();
        assert!(
            rendered.contains("not valid canonical full transaction bytes")
                || rendered.contains("not canonical full transaction encoding"),
            "unexpected raw transaction parse error: {rendered}"
        );
    }

    #[test]
    fn sendrawtransaction_rejects_noncanonical_raw_transaction_bytes() {
        let root = temp_data_dir("sendrawtransaction-reject");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.start();
        let response = service.handle_mut(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "sendrawtransaction",
            vec![String::from("0001")],
        )));

        let RpcResponse::Error(error) = response else {
            panic!("unexpected noncanonical tx response: {response:?}");
        };
        assert!(error
            .details
            .as_deref()
            .unwrap_or_default()
            .contains("raw transaction"));
        assert_eq!(service.node_ref().mempool_len(), 0);
    }

    #[test]
    fn sendrawtransaction_rejects_legacy_lock_format() {
        let root = temp_data_dir("sendrawtransaction-legacy-lock");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.start();
        let (seed_txid, seed_value, seed_script) = crate::dev::seed_utxo(Network::Regnet);
        service.sandbox_with_node_mut(|node| {
            node.dev_seed_chainstate(
                6,
                node.tip_hash(),
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
            .expect("seed spendable utxo");
        });

        let transaction = sign_seed_spend_with_output_lock(
            Network::Regnet,
            seed_txid,
            seed_value,
            seed_script,
            vec![0x99],
        );
        let raw_tx_hex = hex::encode(transaction.full_bytes());
        let response = service.handle_mut(RpcRequest::ExecuteCommand(CommandInvocation::new(
            "sendrawtransaction",
            vec![raw_tx_hex],
        )));

        let RpcResponse::Error(error) = response else {
            panic!("unexpected legacy-lock response: {response:?}");
        };
        let details = error.details.as_deref().unwrap_or_default();
        assert!(
            details.contains("ATHO-TX-015")
                || details.contains("legacy lock format rejected")
                || details.contains("Legacy Lock Format Rejected"),
            "unexpected legacy-lock rejection details: {details}"
        );
        assert_eq!(service.node_ref().mempool_len(), 0);
    }

    #[test]
    fn gethealth_warns_when_local_tip_is_behind_sync_target() {
        let status = NodeStatus {
            network: Network::Mainnet,
            block_count: 0,
            tip_hash: [0x33; 48],
            tip_timestamp: 1_777_416_445,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0x44; 32],
            running: true,
            headers_synced: true,
            sync_best_height: 128,
            network_diagnostics: NetworkDiagnostics {
                peer_count: 1,
                ..NetworkDiagnostics::default()
            },
        };
        assert!(!NodeService::chain_synced(&status));
        let warnings = NodeService::health_warnings(&status);
        assert_eq!(
            warnings,
            vec!["local chain tip is behind the advertised network target"]
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
        assert_eq!(command.data["transactions"][0]["tx_pow_nonce"], 0);
        assert_eq!(command.data["transactions"][0]["tx_pow_bits"], 0);
        assert!(command.data["transactions"][0]["outputs"][0]["locking_script_hex"].is_string());
    }

    #[test]
    fn render_transaction_value_exposes_transaction_pow_fields() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 123,
            tx_pow_bits: 19,
        };

        let rendered = render_transaction_value(Network::Regnet, 0, &tx);

        assert_eq!(rendered["tx_pow_nonce"], 123);
        assert_eq!(rendered["tx_pow_bits"], 19);
    }

    #[test]
    fn execute_command_getblocktemplate_survives_invalid_mempool_entries() {
        let root = temp_data_dir("command-getblocktemplate-stale");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        service.start();
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
                    tx_pow_nonce: 0,
                    tx_pow_bits: 0,
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

fn explorer_api_snapshot_path(network: Network) -> PathBuf {
    database_dir(network).join(EXPLORER_API_SNAPSHOT_FILENAME)
}

fn write_explorer_api_snapshot(
    path: &Path,
    snapshot: &ExplorerApiSnapshot,
) -> Result<(), std::io::Error> {
    let bytes = bincode::serialize(snapshot)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "snapshot_encode"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&temp_path, path)?;
    Ok(())
}

fn load_recent_block_records(
    node: &crate::node::Node,
    tip_height: u64,
    limit: usize,
) -> Option<Vec<BlockArchiveRecord>> {
    let span = limit.max(1) as u64;
    let start_height = tip_height.saturating_add(1).saturating_sub(span);
    let mut records = Vec::with_capacity((tip_height.saturating_sub(start_height) + 1) as usize);
    for height in start_height..=tip_height {
        records.push(node.block_record_by_height(height)?);
    }
    Some(records)
}

fn full_transaction_count(node: &crate::node::Node, tip_height: u64) -> Option<u64> {
    let mut total = 0u64;
    for height in 0..=tip_height {
        total = total.saturating_add(node.block_record_by_height(height)?.tx_count as u64);
    }
    Some(total)
}

fn estimated_hashrate_from_records(records: &[BlockArchiveRecord], window_blocks: usize) -> u64 {
    if window_blocks == 0 || records.len() < 2 {
        return 0;
    }
    let start = records
        .len()
        .saturating_sub(window_blocks.saturating_add(1));
    let window = &records[start..];
    if window.len() < 2 {
        return 0;
    }
    let elapsed = window
        .last()
        .map(|record| record.timestamp)
        .unwrap_or_default()
        .saturating_sub(
            window
                .first()
                .map(|record| record.timestamp)
                .unwrap_or_default(),
        );
    if elapsed == 0 {
        return 0;
    }
    let blocks = window
        .iter()
        .skip(1)
        .map(|record| Block {
            header: record.header(),
            transactions: Vec::new(),
            witnesses: Default::default(),
            fees_total_atoms: record.fees_total_atoms,
            fees_miner_atoms: record.fees_miner_atoms,
        })
        .collect::<Vec<_>>();
    let work_per_second = pow::accumulated_chain_work(&blocks) / num_bigint::BigUint::from(elapsed);
    u64::try_from(work_per_second).unwrap_or(u64::MAX)
}

fn average_block_time_from_records(records: &[BlockArchiveRecord], window_blocks: usize) -> u64 {
    if window_blocks == 0 || records.len() < 2 {
        return 0;
    }
    let start = records
        .len()
        .saturating_sub(window_blocks.saturating_add(1));
    let window = &records[start..];
    if window.len() < 2 {
        return 0;
    }
    let elapsed = window
        .last()
        .map(|record| record.timestamp)
        .unwrap_or_default()
        .saturating_sub(
            window
                .first()
                .map(|record| record.timestamp)
                .unwrap_or_default(),
        );
    ((elapsed as u128 * 1_000) / (window_blocks as u128)) as u64
}

fn diagnostic_peer_group(remote_addr: &str, default_port: u16) -> String {
    let Some(address) = parse_remote_addr(remote_addr, default_port) else {
        return remote_addr.to_ascii_lowercase();
    };
    let host = address.host.to_ascii_lowercase();
    let Ok(ip) = host.parse::<IpAddr>() else {
        return host;
    };
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            format!("{}.{}.0.0/16", octets[0], octets[1])
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            format!("{:x}:{:x}::/32", segments[0], segments[1])
        }
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

fn parse_raw_transaction_hex(raw_tx_hex: &str) -> Result<Transaction, atho_rpc::error::RpcError> {
    let trimmed = raw_tx_hex.trim();
    let hex_value = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if hex_value.is_empty() {
        return Err(atho_rpc::error::RpcError::invalid_request(
            "raw transaction hex is empty",
        ));
    }
    if hex_value.len() % 2 != 0 {
        return Err(atho_rpc::error::RpcError::invalid_request(
            "raw transaction hex must contain whole bytes",
        ));
    }
    if hex_value.len() / 2 > MAX_TRANSACTION_RAW_BYTES {
        return Err(atho_rpc::error::RpcError::invalid_request(
            "raw transaction exceeds maximum standard transaction size",
        ));
    }
    if !hex_value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(atho_rpc::error::RpcError::invalid_request(
            "raw transaction contains non-hex characters",
        ));
    }

    let bytes = hex::decode(hex_value).map_err(|_| {
        atho_rpc::error::RpcError::invalid_request("raw transaction contains invalid hex")
    })?;
    let transaction = Transaction::from_full_bytes(&bytes).ok_or_else(|| {
        atho_rpc::error::RpcError::invalid_request(
            "raw transaction is not valid canonical full transaction bytes",
        )
    })?;
    if transaction.full_bytes() != bytes {
        return Err(atho_rpc::error::RpcError::invalid_request(
            "raw transaction is not canonical full transaction encoding",
        ));
    }
    Ok(transaction)
}
