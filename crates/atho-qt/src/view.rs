use crate::state::UiState;
use atho_rpc::error::RpcError;
use atho_rpc::response::{NetworkPeerDiagnostics, RpcResponse};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewModel {
    pub network_label: String,
    pub block_count: u64,
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
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
            | RpcResponse::TransactionSubmitted(_)
            | RpcResponse::Utxos(_)
            | RpcResponse::WalletActivity(_)
            | RpcResponse::MempoolInfo(_)
            | RpcResponse::MempoolSpentInputs(_) => {}
            RpcResponse::Error(RpcError::MethodNotFound)
            | RpcResponse::Error(RpcError::InvalidRequest(_))
            | RpcResponse::Error(RpcError::Validation(_))
            | RpcResponse::Error(RpcError::Internal) => {
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
