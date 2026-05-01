use crate::state::UiState;
use atho_rpc::response::{NetworkPeerDiagnostics, RpcResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewModel {
    pub network_label: String,
    pub block_count: u64,
    pub tip_hash: [u8; 48],
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub mempool_fingerprint: [u8; 32],
    pub peer_count: usize,
    pub inbound_peer_count: usize,
    pub outbound_peer_count: usize,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub peers: Vec<NetworkPeerDiagnostics>,
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
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            mempool_fingerprint: [0; 32],
            peer_count: 0,
            inbound_peer_count: 0,
            outbound_peer_count: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            sync_best_height: 0,
            running: false,
            headers_synced: false,
            ui_state: UiState::default(),
            sync_stage: String::new(),
        }
    }
}

impl ViewModel {
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
}
