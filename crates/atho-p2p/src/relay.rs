use crate::config::{network_params, NetworkParams};
use crate::peer::PeerBook;
use crate::protocol::{
    GetHeadersMessage, Hash48, InventoryKind, InventoryVector, MessagePayload, NetworkMessage,
    PeerAddress, ProtocolError, VersionMessage, LOCAL_NODE_SERVICES,
};
use crate::sync::SyncState;
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::{pow, rules};
use atho_core::genesis;
use atho_core::network::Network;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct RelayLoop {
    network: Network,
    params: NetworkParams,
    peers: PeerBook,
    sync: SyncState,
}

impl RelayLoop {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            params: network_params(network),
            peers: PeerBook::new(network),
            sync: SyncState::default(),
        }
    }

    pub fn prime(&mut self, blocks: &[Block]) {
        self.sync.prime(blocks);
        crate::audit::append_log(
            "p2p",
            &format!(
                "relay primed network={} height={} dns_seeds={}",
                self.network.id(),
                self.sync.best_height,
                self.params.dns_seeds.len()
            ),
        );
    }

    pub fn add_manual_peer(&mut self, remote_addr: impl Into<String>) {
        self.peers.add_manual_peer(remote_addr);
    }

    pub fn note_address(&mut self, address: PeerAddress) -> Result<(), ProtocolError> {
        self.peers.note_address(address)
    }

    pub fn build_version_message(&self, blocks: &[Block]) -> NetworkMessage {
        let best_height = blocks.last().map(|block| block.header.height).unwrap_or(0);
        let tip_hash = blocks
            .last()
            .map(|block| Hash48::from(block.header.block_hash()))
            .unwrap_or(Hash48::ZERO);
        let chainwork = Hash48::from(chainwork_bytes(blocks));
        NetworkMessage::new(
            self.network,
            MessagePayload::Version(VersionMessage {
                protocol_version: self.params.protocol_version,
                min_protocol_version: self.params.min_supported_protocol_version,
                services: LOCAL_NODE_SERVICES,
                timestamp_unix: unix_timestamp(),
                network: self.network,
                user_agent: String::from("/Atho:0.1.0/"),
                best_height,
                ruleset_version: rules::ruleset_version_at_height(best_height),
                relay: true,
                genesis_hash: Hash48::from(genesis::genesis_hash(self.network)),
                tip_hash,
                chainwork,
            }),
        )
    }

    pub fn accept_version(
        &mut self,
        remote_addr: impl Into<String>,
        version: &VersionMessage,
    ) -> Result<(), ProtocolError> {
        let remote_addr = remote_addr.into();
        self.peers.accept_version(remote_addr.clone(), version)?;
        self.sync.best_height = self.sync.best_height.max(version.best_height);
        crate::audit::append_log(
            "p2p",
            &format!(
                "accepted version peer={} protocol={} height={} ruleset={}",
                remote_addr, version.protocol_version, version.best_height, version.ruleset_version
            ),
        );
        Ok(())
    }

    pub fn build_getheaders(&mut self, peer: impl Into<String>) -> NetworkMessage {
        let message: GetHeadersMessage = self.sync.request_headers(peer, [0; 48]);
        NetworkMessage::new(self.network, MessagePayload::GetHeaders(message))
    }

    pub fn accept_headers(&mut self, headers: &[BlockHeader]) -> Result<(), ProtocolError> {
        self.sync.accept_headers(self.network, headers)
    }

    pub fn relay_block(&self, block_hash: &[u8; 48], tx_count: usize) -> NetworkMessage {
        crate::audit::append_log(
            "p2p",
            &format!(
                "relay block network={} block={} txs={}",
                self.network.id(),
                hex::encode(block_hash),
                tx_count
            ),
        );
        NetworkMessage::new(
            self.network,
            MessagePayload::Inv {
                inventory: vec![InventoryVector {
                    kind: InventoryKind::Block,
                    hash: Hash48::from(*block_hash),
                }],
            },
        )
    }

    pub fn relay_transaction(&self, txid: &[u8; 48]) -> NetworkMessage {
        crate::audit::append_log(
            "p2p",
            &format!(
                "relay tx network={} txid={}",
                self.network.id(),
                hex::encode(txid)
            ),
        );
        NetworkMessage::new(
            self.network,
            MessagePayload::Inv {
                inventory: vec![InventoryVector {
                    kind: InventoryKind::Transaction,
                    hash: Hash48::from(*txid),
                }],
            },
        )
    }

    pub fn sync_state(&self) -> &SyncState {
        &self.sync
    }

    pub fn peer_count(&self) -> usize {
        self.peers.peer_count()
    }

    pub fn dns_seed_count(&self) -> usize {
        self.params.dns_seeds.len()
    }

    pub fn params(&self) -> NetworkParams {
        self.params
    }
}

fn chainwork_bytes(blocks: &[Block]) -> [u8; 48] {
    let work = pow::accumulated_chain_work(blocks).to_bytes_be();
    let mut out = [0u8; 48];
    let copy_len = work.len().min(out.len());
    out[48 - copy_len..].copy_from_slice(&work[work.len() - copy_len..]);
    out
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_loop_keeps_dns_seeds_blank_and_builds_versions() {
        let relay = RelayLoop::new(Network::Mainnet);
        let version = relay.build_version_message(&[genesis::genesis_block(Network::Mainnet)]);
        assert_eq!(relay.dns_seed_count(), 0);
        match version.payload {
            MessagePayload::Version(version) => {
                assert_eq!(version.network, Network::Mainnet);
                assert_eq!(version.protocol_version, rules::PROTOCOL_VERSION);
            }
            _ => panic!("expected version message"),
        }
    }
}
