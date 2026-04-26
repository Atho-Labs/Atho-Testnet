use crate::error::RpcError;
use atho_core::block::Block;
use atho_core::network::Network;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockTemplate {
    pub network: Network,
    pub height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub previous_block_hash: [u8; 48],
    #[serde(with = "serde_big_array::BigArray")]
    pub target: [u8; 48],
    pub transaction_count: usize,
    pub fees_atoms: u64,
    pub block: Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MempoolInfo {
    pub transaction_count: usize,
    pub total_fee_atoms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcResponse {
    BlockCount(u64),
    Network(String),
    BlockTemplate(BlockTemplate),
    BlockSubmitted {
        accepted: bool,
        #[serde(with = "serde_big_array::BigArray")]
        block_hash: [u8; 48],
    },
    TransactionSubmitted(#[serde(with = "serde_big_array::BigArray")] [u8; 48]),
    MempoolInfo(MempoolInfo),
    Error(RpcError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_error_response_is_stable() {
        assert_eq!(
            RpcResponse::Error(RpcError::InvalidRequest),
            RpcResponse::Error(RpcError::InvalidRequest)
        );
    }
}
