//! RPC response payloads returned by the Atho node service.
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
    pub roles: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkDiagnostics {
    pub peer_count: usize,
    pub inbound_peer_count: usize,
    pub outbound_peer_count: usize,
    pub full_relay_peer_count: usize,
    pub block_relay_peer_count: usize,
    pub sync_peer_count: usize,
    pub tx_relay_peer_count: usize,
    pub addr_relay_peer_count: usize,
    pub connecting_peer_count: usize,
    pub known_peer_count: usize,
    pub healthy_peer_count: usize,
    pub stale_peer_count: usize,
    pub banned_peer_count: usize,
    pub dns_seed_status: String,
    pub bootstrap_status: String,
    pub peer_discovery_status: String,
    pub last_getaddr_time_unix: Option<u64>,
    pub last_addr_received_time_unix: Option<u64>,
    pub peer_db_path: String,
    pub topology_health_score: u8,
    pub topology_warnings: Vec<String>,
    pub best_header_height: u64,
    pub best_downloaded_body_height: u64,
    pub best_validated_height: u64,
    pub best_connected_height: u64,
    pub latest_finalized_height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub latest_finalized_hash: [u8; 48],
    pub pending_validation_blocks: usize,
    pub untrusted_downloaded_blocks: usize,
    pub untrusted_downloaded_bytes: usize,
    pub fast_download_enabled: bool,
    pub checkpoint_anchored_sync_enabled: bool,
    pub background_validation_enabled: bool,
    pub chain_validation_status: String,
    pub sync_mode: String,
    pub safe_to_mine: bool,
    pub safe_to_serve: bool,
    pub validation_lag_blocks: u64,
    pub max_fast_download_ahead: u64,
    pub max_untrusted_block_cache: usize,
    pub max_pending_validation_blocks: usize,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub peers: Vec<NetworkPeerDiagnostics>,
    pub connecting_peers: Vec<NetworkPeerDiagnostics>,
}

impl Default for NetworkDiagnostics {
    fn default() -> Self {
        Self {
            peer_count: 0,
            inbound_peer_count: 0,
            outbound_peer_count: 0,
            full_relay_peer_count: 0,
            block_relay_peer_count: 0,
            sync_peer_count: 0,
            tx_relay_peer_count: 0,
            addr_relay_peer_count: 0,
            connecting_peer_count: 0,
            known_peer_count: 0,
            healthy_peer_count: 0,
            stale_peer_count: 0,
            banned_peer_count: 0,
            dns_seed_status: String::new(),
            bootstrap_status: String::new(),
            peer_discovery_status: String::new(),
            last_getaddr_time_unix: None,
            last_addr_received_time_unix: None,
            peer_db_path: String::new(),
            topology_health_score: 0,
            topology_warnings: Vec::new(),
            best_header_height: 0,
            best_downloaded_body_height: 0,
            best_validated_height: 0,
            best_connected_height: 0,
            latest_finalized_height: 0,
            latest_finalized_hash: [0; 48],
            pending_validation_blocks: 0,
            untrusted_downloaded_blocks: 0,
            untrusted_downloaded_bytes: 0,
            fast_download_enabled: false,
            checkpoint_anchored_sync_enabled: false,
            background_validation_enabled: false,
            chain_validation_status: String::new(),
            sync_mode: String::new(),
            safe_to_mine: false,
            safe_to_serve: false,
            validation_lag_blocks: 0,
            max_fast_download_ahead: 0,
            max_untrusted_block_cache: 0,
            max_pending_validation_blocks: 0,
            bytes_sent: 0,
            bytes_received: 0,
            peers: Vec::new(),
            connecting_peers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub network: Network,
    pub block_count: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub tip_hash: [u8; 48],
    pub tip_timestamp: u64,
    pub estimated_hashrate_hps: u64,
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
