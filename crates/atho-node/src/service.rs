use crate::config::NodeConfig;
use crate::dev;
use crate::error::rpc_error_from_node;
use crate::error::NodeError;
use crate::mempool::MempoolEntry;
use crate::miner::Miner;
use crate::orchestrator::NodeOrchestrator;
use crate::sync::{NodeSyncError, SyncNotice};
use crate::wallet_history;
use atho_core::block::Block;
use atho_core::network::Network;
use atho_p2p::connection::ConnectionEvent;
use atho_p2p::protocol::NetworkMessage;
use atho_rpc::request::{RpcRequest, WalletHistoryAddress};
use atho_rpc::response::{
    BlockTemplate, MempoolInfo, MempoolSpentInput, NodeStatus, RpcResponse, WalletActivityEntry,
};
use atho_storage::db::PeerHealthRecord;
use atho_wallet::snapshot::WalletSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStatus {
    pub network: Network,
    pub block_count: u64,
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub wallet_snapshot: WalletSnapshot,
    pub running: bool,
    pub headers_synced: bool,
    pub sync_best_height: u64,
}

#[derive(Debug)]
pub struct NodeService {
    orchestrator: NodeOrchestrator,
    wallet_snapshot: WalletSnapshot,
}

impl NodeService {
    pub fn new(config: NodeConfig) -> Self {
        Self {
            orchestrator: NodeOrchestrator::new(config),
            wallet_snapshot: WalletSnapshot::default(),
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
        }
    }

    pub fn try_new(config: NodeConfig) -> Result<Self, NodeError> {
        Ok(Self {
            orchestrator: NodeOrchestrator::try_new(config)?,
            wallet_snapshot: WalletSnapshot::default(),
        })
    }

    pub fn start(&mut self) {
        self.orchestrator.start();
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
            mempool_count: status.mempool_count,
            mempool_total_fee_atoms: status.mempool_total_fee_atoms,
            running: status.running,
            headers_synced: status.headers_synced,
            sync_best_height: status.sync_best_height,
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

    pub fn p2p_peer_health(&self, remote_addr: &str) -> Option<PeerHealthRecord> {
        self.orchestrator
            .runtime
            .node
            .load_peer_health(remote_addr)
            .ok()
            .flatten()
    }

    pub fn p2p_save_peer_health(&self, record: &PeerHealthRecord) {
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

    pub fn p2p_relay_compact_tip_message(&self) -> Option<NetworkMessage> {
        let block = self
            .orchestrator
            .runtime
            .node
            .block_by_hash(self.orchestrator.runtime.node.tip_hash())?;
        Some(self.orchestrator.sync.relay_compact_block_message(&block))
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
        let notice = self.orchestrator.sync.disconnect_peer(remote_addr, reason);
        self.refresh_runtime_views();
        notice
    }

    pub fn p2p_prime(&mut self) {
        self.orchestrator
            .sync
            .prime(&self.orchestrator.runtime.node);
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
        self.orchestrator.rpc_server.mempool_count = self.orchestrator.runtime.node.mempool_len();
        self.orchestrator.rpc_server.mempool_total_fee_atoms =
            self.orchestrator.runtime.node.mempool_total_fee_atoms();
        self.orchestrator.rpc_server.running = self.orchestrator.runtime.running;
        self.orchestrator.rpc_server.headers_synced =
            self.orchestrator.sync.sync_state().headers_synced;
        self.orchestrator.rpc_server.sync_best_height =
            self.orchestrator.sync.sync_state().best_height;
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

impl Drop for NodeService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn sync_error_into_node(error: NodeSyncError) -> NodeError {
    match error {
        NodeSyncError::Node(error) => error,
        NodeSyncError::Protocol(error) => NodeError::P2pProtocol(error),
        NodeSyncError::Connection(error) => NodeError::P2pConnection(error),
    }
}
