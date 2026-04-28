use crate::config::NodeConfig;
use crate::dev;
use crate::error::rpc_error_from_node;
use crate::error::NodeError;
use crate::mempool::MempoolEntry;
use crate::miner::Miner;
use crate::orchestrator::NodeOrchestrator;
use atho_core::block::Block;
use atho_core::network::Network;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::{BlockTemplate, MempoolInfo, MempoolSpentInput, NodeStatus, RpcResponse};
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
                    Ok(txid) => RpcResponse::TransactionSubmitted(txid),
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
                    Ok(()) => RpcResponse::BlockSubmitted {
                        accepted: true,
                        block_hash,
                    },
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

    pub fn status(&self) -> SystemStatus {
        SystemStatus {
            network: self.network(),
            block_count: self.orchestrator.runtime.node.height(),
            mempool_count: self.orchestrator.runtime.node.mempool_len(),
            mempool_total_fee_atoms: self.orchestrator.runtime.node.mempool_total_fee_atoms(),
            wallet_snapshot: self.wallet_snapshot.clone(),
            running: self.orchestrator.runtime.running,
            headers_synced: self.orchestrator.sync_state.headers_synced,
            sync_best_height: self.orchestrator.sync_state.best_height,
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
}

impl Drop for NodeService {
    fn drop(&mut self) {
        self.stop();
    }
}
