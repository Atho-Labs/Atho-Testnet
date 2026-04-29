use crate::config::NodeConfig;
use crate::dev;
use crate::error::NodeError;
use crate::runtime::NodeRuntime;
use crate::sync::NodeSync;
use atho_rpc::server::RpcServer;

#[derive(Debug)]
pub struct NodeOrchestrator {
    pub runtime: NodeRuntime,
    pub sync: NodeSync,
    pub rpc_server: RpcServer,
}

impl NodeOrchestrator {
    pub fn new(config: NodeConfig) -> Self {
        let network = config.network;
        Self {
            runtime: NodeRuntime::load_or_new(config),
            sync: NodeSync::new(network),
            rpc_server: RpcServer::new(network),
        }
    }

    pub fn try_new(config: NodeConfig) -> Result<Self, NodeError> {
        let network = config.network;
        Ok(Self {
            runtime: NodeRuntime::try_load_or_recover(config)?,
            sync: NodeSync::new(network),
            rpc_server: RpcServer::new(network),
        })
    }

    pub fn start(&mut self) {
        self.runtime.start();
        self.sync.prime(&self.runtime.node);
        self.rpc_server.block_count = self.runtime.node.height();
        self.rpc_server.mempool_count = self.runtime.node.mempool_len();
        self.rpc_server.mempool_total_fee_atoms = self.runtime.node.mempool_total_fee_atoms();
        self.rpc_server.running = self.runtime.running;
        self.rpc_server.headers_synced = self.sync.sync_state().headers_synced;
        self.rpc_server.sync_best_height = self.sync.sync_state().best_height;
        let _ = dev::append_log(
            "p2p",
            &format!(
                "orchestrator started network={} height={} mempool={} best_height={} synced={}",
                self.runtime.node.config.network.id(),
                self.runtime.node.height(),
                self.runtime.node.mempool_len(),
                self.sync.sync_state().best_height,
                self.sync.sync_state().headers_synced
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
    use crate::test_support::acquire_global_test_lock;
    use atho_core::network::Network;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct CurrentDirGuard {
        previous: std::path::PathBuf,
        _lock: crate::test_support::TestLockGuard,
    }

    impl CurrentDirGuard {
        fn switch_to(path: &std::path::Path) -> Self {
            let lock = acquire_global_test_lock();
            let previous = std::env::current_dir().expect("cwd");
            std::env::set_current_dir(path).expect("set cwd");
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
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
            orchestrator.sync.sync_state().best_height,
            orchestrator.runtime.node.height()
        );
        orchestrator.stop();
        assert!(!orchestrator.runtime.running);
    }
}
