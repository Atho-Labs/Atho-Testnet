use crate::error::RpcError;
use crate::request::RpcRequest;
use crate::response::RpcResponse;
use atho_core::network::Network;

#[derive(Debug, Clone)]
pub struct RpcServer {
    pub network: Network,
    pub block_count: u64,
}

impl RpcServer {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            block_count: 0,
        }
    }

    pub fn handle(&self, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::GetBlockCount => RpcResponse::BlockCount(self.block_count),
            RpcRequest::GetNetwork => RpcResponse::Network(self.network.id().to_string()),
            RpcRequest::GetBlockTemplate
            | RpcRequest::SubmitBlock(_)
            | RpcRequest::SubmitTransaction { .. }
            | RpcRequest::ListUtxos
            | RpcRequest::GetMempoolInfo => RpcResponse::Error(RpcError::InvalidRequest),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::RpcRequest;

    #[test]
    fn server_reports_block_count_and_network() {
        let server = RpcServer::new(Network::Mainnet);
        assert_eq!(
            server.handle(RpcRequest::GetNetwork),
            RpcResponse::Network("atho-mainnet".into())
        );
        assert_eq!(
            server.handle(RpcRequest::GetBlockCount),
            RpcResponse::BlockCount(0)
        );
    }
}
