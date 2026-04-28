use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
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
            runtime: NodeRuntime::load_or_new(config),
            sync_state: SyncState::default(),
            relay: RelayLoop::new(network),
            rpc_server: RpcServer::new(network),
        }
    }

    pub fn try_new(config: NodeConfig) -> Result<Self, NodeError> {
        let network = config.network;
        Ok(Self {
            runtime: NodeRuntime::try_load_or_recover(config)?,
            sync_state: SyncState::default(),
            relay: RelayLoop::new(network),
            rpc_server: RpcServer::new(network),
        })
    }

    pub fn start(&mut self) {
        self.runtime.start();
        self.relay.prime(self.runtime.node.blocks());
        self.sync_state = self.relay.sync_state().clone();
        self.rpc_server.block_count = self.runtime.node.height();
        self.rpc_server.mempool_count = self.runtime.node.mempool_len();
        self.rpc_server.mempool_total_fee_atoms = self.runtime.node.mempool_total_fee_atoms();
        self.rpc_server.running = self.runtime.running;
        self.rpc_server.headers_synced = self.sync_state.headers_synced;
        self.rpc_server.sync_best_height = self.sync_state.best_height;
        let _ = dev::append_log(
            "p2p",
            &format!(
                "orchestrator started network={} height={} mempool={} best_height={} synced={}",
                self.runtime.node.config.network.id(),
                self.runtime.node.height(),
                self.runtime.node.mempool_len(),
                self.sync_state.best_height,
                self.sync_state.headers_synced
            ),
        );
    }

    pub fn stop(&mut self) {
        self.runtime.stop();
        self.rpc_server.running = false;
        let _ = dev::append_log(
            "p2p",
            &format!(
                "orchestrator stopped network={} height={}",
                self.runtime.node.config.network.id(),
                self.runtime.node.height()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::network::Network;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct CurrentDirGuard(std::path::PathBuf);

    impl CurrentDirGuard {
        fn switch_to(path: &std::path::Path) -> Self {
            let previous = std::env::current_dir().expect("cwd");
            std::env::set_current_dir(path).expect("set cwd");
            Self(previous)
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    #[test]
    fn orchestrator_starts_runtime_and_marks_sync_state() {
        let root = std::env::temp_dir().join(format!(
            "atho-orchestrator-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp root");
        let _guard = CurrentDirGuard::switch_to(&root);
        let mut orchestrator = NodeOrchestrator::new(NodeConfig::new(Network::Mainnet));
        orchestrator.start();
        assert!(orchestrator.runtime.running);
        assert_eq!(
            orchestrator.sync_state.best_height,
            orchestrator.runtime.node.height()
        );
        orchestrator.stop();
        assert!(!orchestrator.runtime.running);
    }
}
