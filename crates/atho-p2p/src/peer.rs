use crate::address_manager::{format_remote_addr, parse_remote_addr};
use crate::config::network_params;
use crate::protocol::{validate_version_message, PeerAddress, ProtocolError, VersionMessage};
use atho_core::network::Network;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerState {
    pub address: PeerAddress,
    pub version: Option<VersionMessage>,
    pub connected: bool,
    pub ban_score: u32,
}

#[derive(Debug, Clone)]
pub struct PeerBook {
    network: Network,
    manual_peers: BTreeSet<String>,
    known_peers: BTreeMap<String, PeerState>,
}

impl PeerBook {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            manual_peers: BTreeSet::new(),
            known_peers: BTreeMap::new(),
        }
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn dns_seeds(&self) -> &'static [&'static str] {
        network_params(self.network).dns_seeds
    }

    pub fn add_manual_peer(&mut self, remote_addr: impl Into<String>) {
        self.manual_peers.insert(remote_addr.into());
    }

    pub fn manual_peers(&self) -> Vec<String> {
        self.manual_peers.iter().cloned().collect()
    }

    pub fn note_address(&mut self, address: PeerAddress) -> Result<bool, ProtocolError> {
        let key = format_remote_addr(&address);
        if let Some(existing) = self.known_peers.get_mut(&key) {
            existing.address.host = address.host;
            existing.address.port = address.port;
            existing.address.services |= address.services;
            existing.address.last_seen_unix =
                existing.address.last_seen_unix.max(address.last_seen_unix);
            return Ok(false);
        }
        if self.known_peers.len() >= network_params(self.network).limits.max_known_peers {
            return Err(ProtocolError::PeerBookFull);
        }
        self.known_peers.insert(
            key,
            PeerState {
                address,
                version: None,
                connected: false,
                ban_score: 0,
            },
        );
        Ok(true)
    }

    pub fn accept_version(
        &mut self,
        remote_addr: impl Into<String>,
        version: &VersionMessage,
    ) -> Result<(), ProtocolError> {
        validate_version_message(version, self.network)?;
        let remote_addr = remote_addr.into();
        let default_port = network_params(self.network).default_port;
        let mut address = parse_remote_addr(&remote_addr, default_port).unwrap_or(PeerAddress {
            host: remote_addr.clone(),
            port: default_port,
            services: 0,
            last_seen_unix: 0,
        });
        address.last_seen_unix = now_unix();
        if !self.known_peers.contains_key(&remote_addr)
            && self.known_peers.len() >= network_params(self.network).limits.max_known_peers
        {
            return Err(ProtocolError::PeerBookFull);
        }
        self.known_peers.insert(
            remote_addr,
            PeerState {
                address,
                version: Some(version.clone()),
                connected: true,
                ban_score: 0,
            },
        );
        Ok(())
    }

    pub fn peer_count(&self) -> usize {
        self.known_peers.len()
    }

    pub fn peers(&self) -> Vec<PeerState> {
        self.known_peers.values().cloned().collect()
    }

    pub fn best_height(&self) -> u64 {
        self.known_peers
            .values()
            .filter_map(|peer| peer.version.as_ref().map(|version| version.best_height))
            .max()
            .unwrap_or_default()
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MIN_SUPPORTED_PROTOCOL_VERSION;
    use crate::protocol::{Hash48, LOCAL_NODE_SERVICES};
    use atho_core::consensus::rules;
    use atho_core::genesis;

    #[test]
    fn peer_book_accepts_versions_on_the_local_network() {
        let mut peers = PeerBook::new(Network::Mainnet);
        let version = VersionMessage {
            protocol_version: rules::PROTOCOL_VERSION,
            min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            services: LOCAL_NODE_SERVICES,
            timestamp_unix: 1_700_000_000,
            network: Network::Mainnet,
            user_agent: String::from("/Atho:0.1.0/"),
            best_height: 12,
            ruleset_version: rules::RULESET_VERSION_V1,
            relay: true,
            genesis_hash: Hash48::from(genesis::genesis_hash(Network::Mainnet)),
            tip_hash: Hash48::ZERO,
            chainwork: Hash48::ZERO,
        };
        peers
            .accept_version("127.0.0.1:56000", &version)
            .expect("accept");
        assert_eq!(peers.peer_count(), 1);
        assert_eq!(peers.best_height(), 12);
    }

    #[test]
    fn peer_book_keeps_dns_seeds_blank() {
        let peers = PeerBook::new(Network::Mainnet);
        assert!(peers.dns_seeds().is_empty());
    }
}
