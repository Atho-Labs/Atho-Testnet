//! Inventory, block, and transaction relay helpers.
use crate::config::{network_params, NetworkParams};
use crate::peer::PeerBook;
use crate::protocol::{
    GetHeadersMessage, Hash48, InventoryKind, InventoryVector, MessagePayload, NetworkMessage,
    PeerAddress, ProtocolError, VersionMessage, LOCAL_NODE_SERVICES,
};
use crate::sync::{block_locator, SyncState};
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::rules;
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
        let _ = self.peers.note_address(address)?;
        Ok(())
    }

    pub fn build_version_message(
        &self,
        best_height: u64,
        tip_hash: [u8; 48],
        chainwork: [u8; 48],
    ) -> NetworkMessage {
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
                tip_hash: Hash48::from(tip_hash),
                chainwork: Hash48::from(chainwork),
            }),
        )
    }

    pub fn prime_with_locator(
        &mut self,
        best_height: u64,
        best_tip: Option<Hash48>,
        locator_hashes: Vec<Hash48>,
    ) {
        self.sync
            .prime_with_locator(best_height, best_tip, locator_hashes);
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

    pub fn accept_version(
        &mut self,
        remote_addr: impl Into<String>,
        version: &VersionMessage,
    ) -> Result<(), ProtocolError> {
        let remote_addr = remote_addr.into();
        self.peers.accept_version(remote_addr.clone(), version)?;
        let local_best_height = self.sync.best_height;
        self.sync.best_height = self.sync.best_height.max(version.best_height);
        self.sync.headers_synced = version.best_height <= local_best_height;
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

    pub fn reseed_locator_from_local_tip(&mut self, blocks: &[Block]) {
        self.sync.locator_hashes = block_locator(blocks);
        self.sync.inflight_headers_peer = None;
    }

    pub fn reseed_locator_hashes(&mut self, locator_hashes: Vec<Hash48>) {
        self.sync.locator_hashes = locator_hashes;
        self.sync.inflight_headers_peer = None;
    }

    pub fn mark_headers_unsynced(&mut self) {
        self.sync.headers_synced = false;
    }

    pub fn accept_headers(&mut self, headers: &[BlockHeader]) -> Result<(), ProtocolError> {
        self.sync.accept_headers(self.network, headers)
    }

    pub fn accept_headers_from_peer(
        &mut self,
        peer: &str,
        headers: &[BlockHeader],
    ) -> Result<(), ProtocolError> {
        self.sync
            .accept_headers_from_peer(peer, self.network, headers)
    }

    pub fn note_local_chain_progress(&mut self, blocks: &[Block], peer_best_height: Option<u64>) {
        let local_best_height = blocks.last().map(|block| block.header.height).unwrap_or(0);
        let local_tip = blocks
            .last()
            .map(|block| Hash48::from(block.header.block_hash()));
        self.note_local_chain_progress_at(local_best_height, local_tip, peer_best_height);
    }

    pub fn note_local_chain_progress_at(
        &mut self,
        local_best_height: u64,
        local_tip: Option<Hash48>,
        peer_best_height: Option<u64>,
    ) {
        self.sync.best_height = self
            .sync
            .best_height
            .max(peer_best_height.unwrap_or(local_best_height))
            .max(local_best_height);
        if let Some(local_tip) = local_tip {
            if local_best_height >= self.sync.best_height || self.sync.locator_hashes.is_empty() {
                self.sync.locator_hashes.retain(|hash| *hash != local_tip);
                self.sync.locator_hashes.insert(0, local_tip);
                self.sync.locator_hashes.truncate(32);
            }
            if local_best_height >= self.sync.best_height {
                self.sync.best_tip = Some(local_tip);
            }
        }
        if local_best_height >= self.sync.best_height {
            self.sync.headers_synced = true;
            self.sync.clear_requested_header_locators();
        }
    }

    pub fn note_observed_tip(
        &mut self,
        blocks: &[Block],
        observed_height: u64,
        observed_tip: [u8; 48],
    ) {
        let local_best_height = blocks.last().map(|block| block.header.height).unwrap_or(0);
        let local_tip = blocks
            .last()
            .map(|block| Hash48::from(block.header.block_hash()));
        self.note_observed_tip_at(local_best_height, local_tip, observed_height, observed_tip);
    }

    pub fn note_observed_tip_at(
        &mut self,
        local_best_height: u64,
        local_tip: Option<Hash48>,
        observed_height: u64,
        observed_tip: [u8; 48],
    ) {
        let advanced_target = observed_height > self.sync.best_height;
        if observed_height >= self.sync.best_height {
            self.sync.best_height = observed_height;
            self.sync.best_tip = Some(Hash48::from(observed_tip));
        }
        self.sync.best_height = self.sync.best_height.max(local_best_height);
        if local_best_height >= self.sync.best_height {
            self.sync.headers_synced = true;
            self.sync.clear_requested_header_locators();
            self.sync.best_tip = local_tip;
        } else if advanced_target {
            self.sync.headers_synced = false;
        }
    }

    pub fn refresh_sync_target(&mut self, blocks: &[Block], peer_best_height: Option<u64>) {
        let local_best_height = blocks.last().map(|block| block.header.height).unwrap_or(0);
        let local_tip = blocks
            .last()
            .map(|block| Hash48::from(block.header.block_hash()));
        self.refresh_sync_target_at(local_best_height, local_tip, peer_best_height);
    }

    pub fn refresh_sync_target_at(
        &mut self,
        local_best_height: u64,
        local_tip: Option<Hash48>,
        peer_best_height: Option<u64>,
    ) {
        let previous_best_height = self.sync.best_height;
        let target_height = peer_best_height
            .unwrap_or(local_best_height)
            .max(local_best_height)
            .max(previous_best_height);
        let target_advanced = target_height > previous_best_height;
        self.sync.best_height = target_height;

        if local_best_height >= self.sync.best_height {
            self.sync.headers_synced = true;
        } else if target_advanced {
            self.sync.headers_synced = false;
        }
        if self.sync.headers_synced {
            if local_best_height >= self.sync.best_height {
                self.sync.clear_requested_header_locators();
                self.sync.best_tip = local_tip;
            }
        }
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
    fn relay_loop_tracks_configured_dns_seeds_and_builds_versions() {
        let relay = RelayLoop::new(Network::Mainnet);
        let genesis = genesis::genesis_block(Network::Mainnet);
        let version = relay.build_version_message(
            genesis.header.height,
            genesis.header.block_hash(),
            [0; 48],
        );
        assert_eq!(
            relay.dns_seed_count(),
            crate::config::MAINNET_DNS_SEEDS.len()
        );
        match version.payload {
            MessagePayload::Version(version) => {
                assert_eq!(version.network, Network::Mainnet);
                assert_eq!(version.protocol_version, rules::PROTOCOL_VERSION);
            }
            _ => panic!("expected version message"),
        }
    }

    #[test]
    fn refresh_sync_target_keeps_observed_remote_height_when_local_is_still_behind() {
        let mut relay = RelayLoop::new(Network::Regnet);
        let local_blocks = vec![genesis::genesis_block(Network::Regnet)];

        relay.sync.best_height = 128;
        relay.sync.headers_synced = false;
        relay.refresh_sync_target(&local_blocks, None);

        assert_eq!(relay.sync.best_height, 128);
        assert!(!relay.sync.headers_synced);
    }

    #[test]
    fn refresh_sync_target_preserves_inflight_header_locators_while_behind() {
        let mut relay = RelayLoop::new(Network::Regnet);
        let local_blocks = vec![genesis::genesis_block(Network::Regnet)];

        relay.sync.best_height = 128;
        relay.sync.headers_synced = false;
        relay.sync.locator_hashes = vec![Hash48::from([9; 48])];
        let _ = relay.build_getheaders("peer");

        relay.refresh_sync_target(&local_blocks, Some(128));

        assert!(!relay.sync.headers_synced);
        assert!(relay
            .sync
            .requested_locator_hashes_by_peer
            .contains_key("peer"));
    }

    #[test]
    fn refresh_sync_target_does_not_forget_completed_header_sync_while_bodies_lag() {
        let mut relay = RelayLoop::new(Network::Regnet);
        let local_blocks = vec![genesis::genesis_block(Network::Regnet)];

        relay.sync.best_height = 128;
        relay.sync.headers_synced = true;
        relay.sync.best_tip = Some(Hash48::from([8; 48]));
        relay.sync.locator_hashes = vec![Hash48::from([8; 48])];

        relay.refresh_sync_target(&local_blocks, Some(128));

        assert_eq!(relay.sync.best_height, 128);
        assert!(relay.sync.headers_synced);
        assert_eq!(relay.sync.best_tip, Some(Hash48::from([8; 48])));
        assert_eq!(relay.sync.locator_hashes, vec![Hash48::from([8; 48])]);
    }

    #[test]
    fn refresh_sync_target_drops_stale_remote_height_once_local_tip_catches_up() {
        let mut relay = RelayLoop::new(Network::Regnet);
        let local_blocks = vec![Block {
            header: BlockHeader {
                version: 1,
                network_id: Network::Regnet,
                height: 128,
                previous_block_hash: [7; 48],
                merkle_root: [1; 48],
                witness_root: [2; 48],
                timestamp: 1_700_000_128,
                difficulty_target_or_bits: [3; 48],
                nonce: 128,
            },
            ..Block::default()
        }];

        relay.sync.best_height = 128;
        relay.sync.headers_synced = false;
        relay.refresh_sync_target(&local_blocks, None);

        assert_eq!(relay.sync.best_height, 128);
        assert!(relay.sync.headers_synced);
        assert_eq!(
            relay.sync.best_tip,
            Some(Hash48::from(local_blocks[0].header.block_hash()))
        );
    }

    #[test]
    fn local_progress_preserves_remote_sync_target_until_local_height_catches_up() {
        let mut relay = RelayLoop::new(Network::Regnet);
        let local_blocks = vec![genesis::genesis_block(Network::Regnet)];

        relay.sync.best_height = 128;
        relay.sync.headers_synced = false;
        relay.note_local_chain_progress(&local_blocks, Some(128));

        assert_eq!(relay.sync.best_height, 128);
        assert!(!relay.sync.headers_synced);
        assert_eq!(
            relay.sync.locator_hashes.first().copied(),
            Some(Hash48::from(local_blocks[0].header.block_hash()))
        );
    }

    #[test]
    fn local_progress_does_not_replace_advanced_header_locator_while_behind() {
        let mut relay = RelayLoop::new(Network::Regnet);
        let local_blocks = vec![genesis::genesis_block(Network::Regnet)];

        relay.sync.best_height = 128;
        relay.sync.headers_synced = true;
        relay.sync.locator_hashes = vec![Hash48::from([9; 48])];

        relay.note_local_chain_progress(&local_blocks, Some(128));

        assert_eq!(relay.sync.best_height, 128);
        assert!(relay.sync.headers_synced);
        assert_eq!(relay.sync.locator_hashes, vec![Hash48::from([9; 48])]);
    }

    #[test]
    fn observed_future_tip_advances_target_without_claiming_header_sync() {
        let mut relay = RelayLoop::new(Network::Regnet);
        let local_blocks = vec![genesis::genesis_block(Network::Regnet)];
        relay.prime(&local_blocks);

        relay.note_observed_tip(&local_blocks, 7, [7; 48]);

        assert_eq!(relay.sync.best_height, 7);
        assert_eq!(relay.sync.best_tip, Some(Hash48::from([7; 48])));
        assert!(!relay.sync.headers_synced);
    }
}
