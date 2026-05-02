//! Peer address discovery, scoring, and outbound candidate selection.
use crate::config::network_params;
use crate::peer::{PeerBook, PeerState};
use crate::protocol::{PeerAddress, ProtocolError, VersionMessage};
use atho_core::network::Network;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct AddressManager {
    network: Network,
    peers: PeerBook,
    peers_per_ip: BTreeMap<IpAddr, usize>,
    peers_per_subnet: BTreeMap<String, usize>,
}

impl AddressManager {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            peers: PeerBook::new(network),
            peers_per_ip: BTreeMap::new(),
            peers_per_subnet: BTreeMap::new(),
        }
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn add_manual_peer(&mut self, remote_addr: impl Into<String>) {
        self.peers.add_manual_peer(remote_addr);
    }

    pub fn manual_peers(&self) -> Vec<String> {
        self.peers.manual_peers()
    }

    pub fn accept_version(
        &mut self,
        remote_addr: impl Into<String>,
        version: &VersionMessage,
    ) -> Result<(), ProtocolError> {
        self.peers.accept_version(remote_addr, version)
    }

    pub fn note_gossip_addresses(
        &mut self,
        addresses: &[PeerAddress],
        public_source: bool,
    ) -> Result<Vec<PeerAddress>, ProtocolError> {
        let mut accepted = Vec::new();
        let mut seen = BTreeSet::new();
        for address in addresses
            .iter()
            .take(network_params(self.network).limits.max_addr_per_message)
        {
            let key = format_remote_addr(address);
            if !seen.insert(key) {
                continue;
            }
            if public_source && !is_publicly_routable_host(&address.host) {
                continue;
            }
            if let Some(ip) = parse_ip(&address.host) {
                if self.ip_count(ip) >= network_params(self.network).limits.max_peers_per_ip {
                    continue;
                }
                let subnet = subnet_key(ip);
                if self.subnet_count(&subnet)
                    >= network_params(self.network).limits.max_peers_per_subnet
                {
                    continue;
                }
            }
            match self.peers.note_address(address.clone()) {
                Ok(is_new) => {
                    if is_new {
                        if let Some(ip) = parse_ip(&address.host) {
                            *self.peers_per_ip.entry(ip).or_default() += 1;
                            *self.peers_per_subnet.entry(subnet_key(ip)).or_default() += 1;
                        }
                    }
                    accepted.push(address.clone());
                }
                Err(ProtocolError::PeerBookFull) => break,
                Err(err) => return Err(err),
            }
        }
        Ok(accepted)
    }

    pub fn advertisable_addresses(&self, max: usize) -> Vec<PeerAddress> {
        let mut seen = BTreeSet::new();
        let mut addresses = Vec::new();

        let mut peers = self.peers.peers();
        peers.sort_by(|left, right| {
            right
                .connected
                .cmp(&left.connected)
                .then(
                    right
                        .address
                        .last_seen_unix
                        .cmp(&left.address.last_seen_unix),
                )
                .then(
                    right
                        .version
                        .as_ref()
                        .map(|version| version.best_height)
                        .cmp(&left.version.as_ref().map(|version| version.best_height)),
                )
                .then(left.address.host.cmp(&right.address.host))
                .then(left.address.port.cmp(&right.address.port))
        });

        for peer in peers {
            if seen.insert((peer.address.host.clone(), peer.address.port)) {
                addresses.push(peer.address);
            }
            if addresses.len() >= max {
                return addresses;
            }
        }

        for manual in self.peers.manual_peers() {
            if let Some(address) = parse_remote_addr(&manual, self.network.p2p_port()) {
                if seen.insert((address.host.clone(), address.port)) {
                    addresses.push(address);
                }
            }
            if addresses.len() >= max {
                break;
            }
        }
        addresses
    }

    pub fn peers(&self) -> Vec<PeerState> {
        self.peers.peers()
    }

    pub fn peer_count(&self) -> usize {
        self.peers.peer_count()
    }

    pub fn best_height(&self) -> u64 {
        self.peers.best_height()
    }

    fn ip_count(&self, ip: IpAddr) -> usize {
        self.peers_per_ip.get(&ip).copied().unwrap_or(0)
    }

    fn subnet_count(&self, subnet: &str) -> usize {
        self.peers_per_subnet.get(subnet).copied().unwrap_or(0)
    }
}

pub fn parse_remote_addr(remote_addr: &str, default_port: u16) -> Option<PeerAddress> {
    if let Ok(socket_addr) = SocketAddr::from_str(remote_addr) {
        return Some(PeerAddress {
            host: socket_addr.ip().to_string(),
            port: socket_addr.port(),
            services: 0,
            last_seen_unix: 0,
        });
    }
    if let Some((host, port)) = remote_addr.rsplit_once(':') {
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        if let Ok(port) = port.parse::<u16>() {
            return Some(PeerAddress {
                host: host.to_string(),
                port,
                services: 0,
                last_seen_unix: 0,
            });
        }
    }
    Some(PeerAddress {
        host: remote_addr.to_string(),
        port: default_port,
        services: 0,
        last_seen_unix: 0,
    })
}

pub fn format_remote_addr(address: &PeerAddress) -> String {
    if address
        .host
        .parse::<IpAddr>()
        .is_ok_and(|ip| matches!(ip, IpAddr::V6(_)))
    {
        format!("[{}]:{}", address.host, address.port)
    } else {
        format!("{}:{}", address.host, address.port)
    }
}

fn parse_ip(host: &str) -> Option<IpAddr> {
    host.parse().ok()
}

fn is_publicly_routable_host(host: &str) -> bool {
    let Some(ip) = parse_ip(host) else {
        return !matches!(host, "localhost");
    };
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && ip != Ipv4Addr::UNSPECIFIED
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && ip != Ipv6Addr::LOCALHOST
        }
    }
}

fn subnet_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            format!("{}.{}.{}", octets[0], octets[1], octets[2])
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            format!(
                "{:x}:{:x}:{:x}:{:x}",
                segments[0], segments[1], segments[2], segments[3]
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MIN_SUPPORTED_PROTOCOL_VERSION;
    use crate::protocol::{Hash48, LOCAL_NODE_SERVICES};
    use atho_core::consensus::rules;
    use atho_core::genesis;

    #[test]
    fn public_gossip_rejects_private_addresses() {
        let mut manager = AddressManager::new(Network::Mainnet);
        manager
            .note_gossip_addresses(
                &[
                    PeerAddress {
                        host: String::from("127.0.0.1"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: 0,
                    },
                    PeerAddress {
                        host: String::from("8.8.8.8"),
                        port: 56001,
                        services: 0,
                        last_seen_unix: 0,
                    },
                ],
                true,
            )
            .expect("gossip");
        let addresses = manager.advertisable_addresses(8);
        assert_eq!(addresses.len(), 1);
        assert_eq!(addresses[0].host, "8.8.8.8");
    }

    #[test]
    fn version_acceptance_updates_best_height() {
        let mut manager = AddressManager::new(Network::Mainnet);
        let version = VersionMessage {
            protocol_version: rules::PROTOCOL_VERSION,
            min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            services: LOCAL_NODE_SERVICES,
            timestamp_unix: 1_700_000_000,
            network: Network::Mainnet,
            user_agent: String::from("/Atho:0.1.0/"),
            best_height: 77,
            ruleset_version: rules::RULESET_VERSION_V1,
            relay: true,
            genesis_hash: Hash48::from(genesis::genesis_hash(Network::Mainnet)),
            tip_hash: Hash48::ZERO,
            chainwork: Hash48::ZERO,
        };
        manager
            .accept_version("8.8.8.8:56000", &version)
            .expect("accept");
        assert_eq!(manager.best_height(), 77);
    }

    #[test]
    fn remote_addr_helpers_round_trip_ipv6_literals() {
        let parsed = parse_remote_addr("[2001:db8::1]:56000", 56000).expect("parse");
        assert_eq!(parsed.host, "2001:db8::1");
        assert_eq!(parsed.port, 56000);
        assert_eq!(format_remote_addr(&parsed), "[2001:db8::1]:56000");
    }
}
