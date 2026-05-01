use crate::command::CommandResponse;
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

impl BlockTemplate {
    pub fn header_bytes_without_nonce(&self) -> Vec<u8> {
        self.block.header.canonical_bytes_without_nonce()
    }

    pub fn nonce_offset_bytes(&self) -> usize {
        self.block.header.canonical_size_bytes_without_nonce()
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WalletActivityKind {
    Mined,
    Received,
    Sent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletActivityEntry {
    pub height: u64,
    pub kind: WalletActivityKind,
    pub label: String,
    pub amount_atoms: i128,
    #[serde(with = "serde_big_array::BigArray")]
    pub txid: [u8; 48],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkPeerDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPeerDiagnostics {
    pub remote_addr: String,
    pub direction: NetworkPeerDirection,
    pub handshake_ready: bool,
    pub best_height: Option<u64>,
    pub protocol_version: Option<u32>,
    pub services: Option<u64>,
    pub user_agent: Option<String>,
    pub ruleset_version: Option<u32>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub last_send_unix: Option<u64>,
    pub last_receive_unix: Option<u64>,
    pub quality_score: Option<u32>,
    pub consecutive_failures: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NetworkDiagnostics {
    pub peer_count: usize,
    pub inbound_peer_count: usize,
    pub outbound_peer_count: usize,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub peers: Vec<NetworkPeerDiagnostics>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub network: Network,
    pub block_count: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub tip_hash: [u8; 48],
    pub mempool_count: usize,
    pub mempool_total_fee_atoms: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub mempool_fingerprint: [u8; 32],
    pub running: bool,
    pub headers_synced: bool,
    pub sync_best_height: u64,
    pub network_diagnostics: NetworkDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    WalletActivity(Vec<WalletActivityEntry>),
    MempoolInfo(MempoolInfo),
    MempoolSpentInputs(Vec<MempoolSpentInput>),
    Command(CommandResponse),
    Error(RpcError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_error_response_is_stable() {
        assert_eq!(
            RpcResponse::Error(RpcError::invalid_request("bad request")),
            RpcResponse::Error(RpcError::invalid_request("bad request"))
        );
    }

    #[test]
    fn block_template_exposes_canonical_header_bytes() {
        let template = BlockTemplate {
            network: Network::Mainnet,
            height: 0,
            previous_block_hash: [0; 48],
            target: [0; 48],
            transaction_count: 0,
            fees_atoms: 0,
            block: Block::default(),
        };

        assert_eq!(
            template.header_bytes_without_nonce(),
            template.block.header.canonical_bytes_without_nonce()
        );
        assert_eq!(
            template.nonce_offset_bytes(),
            template.block.header.canonical_size_bytes_without_nonce()
        );
    }
}
