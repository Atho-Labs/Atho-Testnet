use crate::config::NodeConfig;
use crate::dev;
use crate::runtime::NodeRuntime;
use atho_p2p::relay::RelayLoop;
use atho_p2p::sync::SyncState;
use atho_rpc::server::RpcServer;

#[derive(Debug)]
pub struct NodeOrchestrator {
    pub runtime: NodeRuntime,
    pub sync_state: SyncState,
    pub relay: RelayLoop,
    pub rpc_server: RpcServer,
}

impl NodeOrchestrator {
    pub fn new(config: NodeConfig) -> Self {
        let network = config.network;
        Self {
            runtime: NodeRuntime::new(config),
            sync_state: SyncState::default(),
            relay: RelayLoop::new(network),
            rpc_server: RpcServer::new(network),
        }
    }

    pub fn start(&mut self) {
        self.runtime.start();
        self.sync_state.advance(self.runtime.node.chainstate.height);
        self.sync_state.mark_headers_synced();
        self.relay.prime();
        self.relay.sync_headers(self.sync_state.best_height);
        let _ = dev::append_log(
            "p2p",
            &format!(
                "orchestrator started network={} height={} synced={}",
                self.runtime.node.config.network.id(),
                self.sync_state.best_height,
                self.sync_state.headers_synced
            ),
        );
    }

    pub fn stop(&mut self) {
        self.runtime.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::network::Network;

    #[test]
    fn orchestrator_starts_runtime_and_marks_sync_state() {
        let mut orchestrator = NodeOrchestrator::new(NodeConfig::new(Network::Mainnet));
        orchestrator.start();
        assert!(orchestrator.runtime.running);
        assert!(orchestrator.sync_state.headers_synced);
        orchestrator.stop();
        assert!(!orchestrator.runtime.running);
    }
}
