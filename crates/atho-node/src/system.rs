use crate::config::NodeConfig;
use crate::error::rpc_error_from_node;
use crate::mempool::MempoolEntry;
use crate::miner::Miner;
use crate::orchestrator::NodeOrchestrator;
use atho_core::block::Block;
use atho_core::network::Network;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::{BlockTemplate, MempoolInfo, RpcResponse};
use atho_wallet::snapshot::WalletSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStatus {
    pub network: Network,
    pub block_count: u64,
    pub mempool_count: usize,
    pub wallet_snapshot: WalletSnapshot,
    pub running: bool,
    pub headers_synced: bool,
    pub sync_best_height: u64,
}

#[derive(Debug)]
pub struct AthoSystem {
    orchestrator: NodeOrchestrator,
    wallet_snapshot: WalletSnapshot,
}

impl AthoSystem {
    pub fn new(config: NodeConfig) -> Self {
        Self {
            orchestrator: NodeOrchestrator::new(config),
            wallet_snapshot: WalletSnapshot::default(),
        }
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
                RpcResponse::BlockCount(self.orchestrator.runtime.node.chainstate.height)
            }
            RpcRequest::GetMempoolInfo => RpcResponse::MempoolInfo(MempoolInfo {
                transaction_count: self.orchestrator.runtime.node.mempool.len(),
                total_fee_atoms: self.orchestrator.runtime.node.mempool.total_fee_atoms(),
            }),
            RpcRequest::ListUtxos => RpcResponse::Utxos(self.list_utxos()),
            RpcRequest::GetBlockTemplate => {
                let miner = Miner::new(1);
                match self.orchestrator.runtime.node.build_candidate_block(&miner) {
                    Ok(block) => RpcResponse::BlockTemplate(self.block_template(block)),
                    Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
                }
            }
            RpcRequest::SubmitBlock(_) | RpcRequest::SubmitTransaction { .. } => {
                RpcResponse::Error(atho_rpc::error::RpcError::InvalidRequest)
            }
        }
    }

    pub fn handle_mut(&mut self, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::SubmitTransaction {
                transaction,
                fee_atoms,
            } => {
                match self
                    .orchestrator
                    .runtime
                    .node
                    .submit_transaction(MempoolEntry {
                        transaction,
                        fee_atoms,
                    }) {
                    Ok(txid) => RpcResponse::TransactionSubmitted(txid),
                    Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
                }
            }
            RpcRequest::SubmitBlock(block) => {
                let block_hash = block.header.block_hash();
                match self.orchestrator.runtime.node.submit_block(&block) {
                    Ok(()) => RpcResponse::BlockSubmitted {
                        accepted: true,
                        block_hash,
                    },
                    Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
                }
            }
            RpcRequest::GetBlockTemplate => {
                let miner = Miner::new(1);
                match self.orchestrator.runtime.node.build_candidate_block(&miner) {
                    Ok(block) => RpcResponse::BlockTemplate(self.block_template(block)),
                    Err(err) => RpcResponse::Error(rpc_error_from_node(err)),
                }
            }
            RpcRequest::ListUtxos => RpcResponse::Utxos(self.list_utxos()),
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
            .chainstate
            .utxo_snapshot()
            .entries()
            .cloned()
            .collect()
    }

    pub fn status(&self) -> SystemStatus {
        SystemStatus {
            network: self.network(),
            block_count: self.orchestrator.runtime.node.chainstate.height,
            mempool_count: self.orchestrator.runtime.node.mempool.len(),
            wallet_snapshot: self.wallet_snapshot.clone(),
            running: self.orchestrator.runtime.running,
            headers_synced: self.orchestrator.sync_state.headers_synced,
            sync_best_height: self.orchestrator.sync_state.best_height,
        }
    }

    pub fn wallet_snapshot(&self) -> &WalletSnapshot {
        &self.wallet_snapshot
    }

    pub fn is_running(&self) -> bool {
        self.orchestrator.runtime.running
    }
}

impl Drop for AthoSystem {
    fn drop(&mut self) {
        self.stop();
    }
}
