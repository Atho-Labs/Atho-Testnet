//! Derived Qt view state computed from node status snapshots.
use crate::state::UiState;
use atho_rpc::response::{NetworkPeerDiagnostics, RpcResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewModel {
    pub network_label: String,
    pub block_count: u64,
    pub tip_hash: [u8; 48],
    pub tip_timestamp: u64,
    pub estimated_hashrate_hps: u64,
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub mempool_fingerprint: [u8; 32],
    pub peer_count: usize,
    pub inbound_peer_count: usize,
    pub outbound_peer_count: usize,
    pub connecting_peer_count: usize,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub peers: Vec<NetworkPeerDiagnostics>,
    pub connecting_peers: Vec<NetworkPeerDiagnostics>,
    pub sync_best_height: u64,
    pub running: bool,
    pub headers_synced: bool,
    pub ui_state: UiState,
    pub sync_stage: String,
}

impl Default for ViewModel {
    fn default() -> Self {
        Self {
            network_label: String::new(),
            block_count: 0,
            tip_hash: [0; 48],
            tip_timestamp: 0,
            estimated_hashrate_hps: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0; 32],
            peer_count: 0,
            inbound_peer_count: 0,
            outbound_peer_count: 0,
            connecting_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            connecting_peers: Vec::new(),
            sync_best_height: 0,
            running: false,
            headers_synced: false,
            ui_state: UiState::default(),
            sync_stage: String::new(),
        }
    }
}

impl ViewModel {
    pub fn sync_target_height(&self) -> u64 {
        self.sync_best_height.max(self.block_count)
    }

    pub fn chain_synced(&self) -> bool {
        self.running && self.headers_synced && self.block_count >= self.sync_target_height()
    }

    pub fn sync_progress(&self) -> f32 {
        if self.chain_synced() {
            return 1.0;
        }
        let target = self.sync_target_height();
        if target == 0 {
            return 0.0;
        }
        (self.block_count as f32 / target as f32).clamp(0.0, 1.0)
    }

    pub fn sync_progress_display(&self) -> f32 {
        if self.chain_synced() {
            return 1.0;
        }
        let progress = self.sync_progress();
        progress.min(0.999)
    }

    pub fn update_from_network(&mut self, response: RpcResponse) {
        match response {
            RpcResponse::BlockCount(count) => {
                self.block_count = count;
                self.sync_stage = String::from("Synced");
            }
            RpcResponse::Network(label) => {
                self.network_label = label;
                self.sync_stage = String::from("Online");
            }
            RpcResponse::BlockTemplate(_)
            | RpcResponse::NodeStatus(_)
            | RpcResponse::BlockSubmitted { .. }
            | RpcResponse::Command(_)
            | RpcResponse::TransactionSubmitted(_)
            | RpcResponse::Utxos(_)
            | RpcResponse::WalletActivity(_)
            | RpcResponse::MempoolInfo(_)
            | RpcResponse::MempoolSpentInputs(_) => {}
            RpcResponse::Error(_) => {
                self.sync_stage = String::from("Disconnected");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_rpc::response::RpcResponse;

    #[test]
    fn view_model_updates_from_rpc_responses() {
        let mut view = ViewModel::default();
        view.update_from_network(RpcResponse::Network(String::from("atho-mainnet")));
        view.update_from_network(RpcResponse::BlockCount(7));
        assert_eq!(view.network_label, "atho-mainnet");
        assert_eq!(view.block_count, 7);
        assert_eq!(view.sync_stage, "Synced");
    }

    #[test]
    fn chain_synced_requires_local_height_to_reach_sync_target() {
        let mut view = ViewModel::default();
        view.running = true;
        view.headers_synced = true;
        view.block_count = 0;
        view.sync_best_height = 128;
        assert!(!view.chain_synced());
        assert_eq!(view.sync_target_height(), 128);
        assert_eq!(view.sync_progress(), 0.0);

        view.block_count = 128;
        assert!(view.chain_synced());
        assert_eq!(view.sync_progress(), 1.0);
    }

    #[test]
    fn sync_progress_display_stays_below_full_while_headers_are_unsynced() {
        let mut view = ViewModel::default();
        view.running = true;
        view.headers_synced = false;
        view.block_count = 128;
        view.sync_best_height = 128;

        assert!(!view.chain_synced());
        assert_eq!(view.sync_progress(), 1.0);
        assert!(view.sync_progress_display() < 1.0);
    }
}
