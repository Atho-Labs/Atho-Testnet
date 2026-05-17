// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Minimal local RPC server helpers and response shaping utilities.
use crate::error::RpcError;
use crate::request::RpcRequest;
use crate::response::{NetworkDiagnostics, NodeStatus, RpcResponse};
use atho_core::network::Network;

#[derive(Debug, Clone)]
pub struct RpcServer {
    pub network: Network,
    pub block_count: u64,
    pub tip_hash: [u8; 48],
    pub tip_timestamp: u64,
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub running: bool,
    pub headers_synced: bool,
    pub sync_best_height: u64,
}

impl RpcServer {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            block_count: 0,
            tip_hash: [0; 48],
            tip_timestamp: 0,
            mempool_count: 0,
            mempool_total_fee_atoms: 0,
            running: false,
            headers_synced: false,
            sync_best_height: 0,
        }
    }

    pub fn node_status(&self) -> NodeStatus {
        NodeStatus {
            network: self.network,
            block_count: self.block_count,
            tip_hash: self.tip_hash,
            tip_timestamp: self.tip_timestamp,
            estimated_hashrate_hps: 0,
            mempool_count: self.mempool_count,
            mempool_total_fee_atoms: self.mempool_total_fee_atoms,
            mempool_fingerprint: [0; 32],
            running: self.running,
            headers_synced: self.headers_synced,
            sync_best_height: self.sync_best_height,
            network_diagnostics: NetworkDiagnostics::default(),
        }
    }

    pub fn handle(&self, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::Authenticated { request, .. } => self.handle(*request),
            RpcRequest::GetBlockCount => RpcResponse::BlockCount(self.block_count),
            RpcRequest::GetNetwork => RpcResponse::Network(self.network.id().to_string()),
            RpcRequest::GetNodeStatus => RpcResponse::NodeStatus(self.node_status()),
            RpcRequest::GetBlockTemplate
            | RpcRequest::SubmitBlock(_)
            | RpcRequest::SubmitTransaction { .. }
            | RpcRequest::ListUtxos
            | RpcRequest::GetWalletActivity { .. }
            | RpcRequest::GetMempoolInfo
            | RpcRequest::GetMempoolSpentInputs
            | RpcRequest::ExecuteCommand(_) => RpcResponse::Error(RpcError::invalid_request(
                "method must be handled by the node runtime",
            )),
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
        assert!(matches!(
            server.handle(RpcRequest::GetNodeStatus),
            RpcResponse::NodeStatus(_)
        ));
    }
}
