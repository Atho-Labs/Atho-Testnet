use crate::state::UiState;
use atho_rpc::error::RpcError;
use atho_rpc::response::RpcResponse;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewModel {
    pub network_label: String,
    pub block_count: u64,
    pub mempool_count: usize,
    pub sync_best_height: u64,
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
            | RpcResponse::BlockSubmitted { .. }
            | RpcResponse::TransactionSubmitted(_)
            | RpcResponse::MempoolInfo(_) => {}
            RpcResponse::Error(RpcError::MethodNotFound)
            | RpcResponse::Error(RpcError::InvalidRequest)
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
