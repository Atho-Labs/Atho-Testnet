use crate::error::RpcError;
use atho_core::block::Block;
use atho_core::network::Network;
use atho_storage::utxo::UtxoEntry;
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
pub struct MempoolSpentInput {
    #[serde(with = "serde_big_array::BigArray")]
    pub txid: [u8; 48],
    pub output_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub network: Network,
    pub block_count: u64,
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    pub running: bool,
    pub headers_synced: bool,
    pub sync_best_height: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcResponse {
    BlockCount(u64),
    Network(String),
    NodeStatus(NodeStatus),
    BlockTemplate(BlockTemplate),
    BlockSubmitted {
        accepted: bool,
        #[serde(with = "serde_big_array::BigArray")]
        block_hash: [u8; 48],
    },
    TransactionSubmitted(#[serde(with = "serde_big_array::BigArray")] [u8; 48]),
    Utxos(Vec<UtxoEntry>),
    MempoolInfo(MempoolInfo),
    MempoolSpentInputs(Vec<MempoolSpentInput>),
    Error(RpcError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_error_response_is_stable() {
        assert_eq!(
            RpcResponse::Error(RpcError::InvalidRequest(String::from("bad request"))),
            RpcResponse::Error(RpcError::InvalidRequest(String::from("bad request")))
        );
    }
}
