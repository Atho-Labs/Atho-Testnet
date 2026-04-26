use crate::config::NodeConfig;
use crate::orchestrator::NodeOrchestrator;
use atho_core::network::Network;
use atho_wallet::snapshot::WalletSnapshot;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;

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
        }
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
