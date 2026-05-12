//! Peer address discovery, scoring, and outbound candidate selection.
use crate::config::network_params;
use crate::peer::{PeerBook, PeerState};
use crate::protocol::{PeerAddress, ProtocolError, VersionMessage};
use atho_core::network::Network;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct AddressManager {
    network: Network,
    peers: PeerBook,
    peers_per_ip: BTreeMap<IpAddr, usize>,
    peers_per_subnet: BTreeMap<String, usize>,
    peers_per_source: BTreeMap<String, usize>,
}

impl AddressManager {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            peers: PeerBook::new(network),
            peers_per_ip: BTreeMap::new(),
            peers_per_subnet: BTreeMap::new(),
            peers_per_source: BTreeMap::new(),
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
        self.note_gossip_addresses_from_source(None, addresses, public_source)
    }

    pub fn note_gossip_addresses_from_source(
        &mut self,
        source_peer: Option<&str>,
        addresses: &[PeerAddress],
        public_source: bool,
    ) -> Result<Vec<PeerAddress>, ProtocolError> {
        let mut accepted = Vec::new();
        let mut seen = BTreeSet::new();
        let now = now_unix();
        let source_key = source_peer.map(str::to_owned);
        let source_count = source_key
            .as_ref()
            .and_then(|source| self.peers_per_source.get(source))
            .copied()
            .unwrap_or_default();
        let mut new_from_source = 0usize;
        let max_from_source = network_params(self.network).limits.max_addr_per_source;
        for address in addresses
            .iter()
            .take(network_params(self.network).limits.max_addr_per_message)
        {
            let key = format_remote_addr(address);
            if !seen.insert(key) {
                continue;
            }
            if source_key.is_some()
                && source_count.saturating_add(new_from_source) >= max_from_source
            {
                continue;
            }
            if !address_is_acceptable(self.network, address, public_source, now) {
                continue;
            }
            let mut address = address.clone();
            if address.last_seen_unix == 0 {
                address.last_seen_unix = now;
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
                        if let Some(source) = source_key.as_ref() {
                            *self.peers_per_source.entry(source.clone()).or_default() += 1;
                            new_from_source = new_from_source.saturating_add(1);
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
        self.advertisable_addresses_inner(max, false)
    }

    pub fn relay_addresses(&self, max: usize) -> Vec<PeerAddress> {
        self.advertisable_addresses_inner(max, true)
    }

    fn advertisable_addresses_inner(&self, max: usize, relay_safe: bool) -> Vec<PeerAddress> {
        let mut seen = BTreeSet::new();
        let mut addresses = Vec::new();
        let now = now_unix();
        let public_network = !matches!(self.network, Network::Regnet | Network::Prunetest);

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
            if !address_is_structurally_valid(&peer.address) {
                continue;
            }
            if relay_safe {
                if public_network && !is_publicly_routable_host(&peer.address.host) {
                    continue;
                }
                if public_network && peer.address.port != self.network.p2p_port() {
                    continue;
                }
                if is_stale_address(self.network, peer.address.last_seen_unix, now) {
                    continue;
                }
            }
            if seen.insert((peer.address.host.clone(), peer.address.port)) {
                addresses.push(peer.address);
            }
            if addresses.len() >= max {
                return addresses;
            }
        }

        for manual in self.peers.manual_peers() {
            if let Some(address) = parse_remote_addr(&manual, self.network.p2p_port()) {
                if !address_is_structurally_valid(&address) {
                    continue;
                }
                if relay_safe {
                    let public_network =
                        !matches!(self.network, Network::Regnet | Network::Prunetest);
                    if public_network && !is_publicly_routable_host(&address.host) {
                        continue;
                    }
                    if is_stale_address(self.network, address.last_seen_unix, now) {
                        continue;
                    }
                }
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

    pub fn fresh_peer_count(&self) -> usize {
        let now = now_unix();
        self.peers
            .peers()
            .into_iter()
            .filter(|peer| !is_stale_address(self.network, peer.address.last_seen_unix, now))
            .count()
    }

    pub fn stale_peer_count(&self) -> usize {
        let now = now_unix();
        self.peers
            .peers()
            .into_iter()
            .filter(|peer| is_stale_address(self.network, peer.address.last_seen_unix, now))
            .count()
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
        return is_public_dns_name(host);
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

fn is_public_dns_name(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".lan")
        || lower.ends_with(".internal")
        || !host.contains('.')
    {
        return false;
    }
    dns_name_is_well_formed(host)
}

fn dns_name_is_well_formed(host: &str) -> bool {
    if host.is_empty()
        || host.len() > 253
        || host
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        || host.contains('/')
        || host.contains('\\')
    {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

fn address_is_acceptable(
    network: Network,
    address: &PeerAddress,
    public_source: bool,
    now_unix: u64,
) -> bool {
    if !address_is_structurally_valid(address) {
        return false;
    }
    if public_source && !is_publicly_routable_host(&address.host) {
        return false;
    }
    if public_source && address.port != network.p2p_port() {
        return false;
    }
    !(public_source && is_stale_address(network, address.last_seen_unix, now_unix))
}

fn address_is_structurally_valid(address: &PeerAddress) -> bool {
    address.port != 0
        && (parse_ip(&address.host).is_some() || dns_name_is_well_formed(&address.host))
}

fn is_stale_address(network: Network, last_seen_unix: u64, now_unix: u64) -> bool {
    if last_seen_unix == 0 {
        return false;
    }
    now_unix.saturating_sub(last_seen_unix)
        > network_params(network).limits.max_peer_address_age_secs
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
    fn public_gossip_rejects_private_addresses() {
        let mut manager = AddressManager::new(Network::Mainnet);
        let now = now_unix();
        manager
            .note_gossip_addresses(
                &[
                    PeerAddress {
                        host: String::from("127.0.0.1"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("8.8.8.8"),
                        port: Network::Mainnet.p2p_port(),
                        services: 0,
                        last_seen_unix: now,
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
    fn public_gossip_rejects_malformed_local_and_stale_addresses() {
        let mut manager = AddressManager::new(Network::Mainnet);
        let now = now_unix();
        manager
            .note_gossip_addresses(
                &[
                    PeerAddress {
                        host: String::from(""),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("bad host.example"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("Wallet.LOCAL"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("10.0.0.8"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("8.8.8.9"),
                        port: 0,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("8.8.8.10"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now
                            - network_params(Network::Mainnet)
                                .limits
                                .max_peer_address_age_secs
                            - 1,
                    },
                    PeerAddress {
                        host: String::from("seed.atho.io"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                ],
                true,
            )
            .expect("gossip");

        let addresses = manager.advertisable_addresses(8);
        assert_eq!(addresses.len(), 1);
        assert_eq!(addresses[0].host, "seed.atho.io");
    }

    #[test]
    fn regnet_gossip_allows_local_addresses() {
        let mut manager = AddressManager::new(Network::Regnet);
        let accepted = manager
            .note_gossip_addresses(
                &[PeerAddress {
                    host: String::from("127.0.0.1"),
                    port: 18445,
                    services: 0,
                    last_seen_unix: 0,
                }],
                false,
            )
            .expect("gossip");
        assert_eq!(accepted.len(), 1);
        assert_eq!(manager.advertisable_addresses(8)[0].host, "127.0.0.1");
    }

    #[test]
    fn one_source_cannot_fill_the_peer_table() {
        let mut manager = AddressManager::new(Network::Mainnet);
        let limit = network_params(Network::Mainnet).limits.max_addr_per_source;
        let now = now_unix();
        let addresses = (0..limit + 12)
            .map(|index| PeerAddress {
                host: format!("11.{}.{}.1", index / 256, index % 256),
                port: 56000,
                services: 0,
                last_seen_unix: now,
            })
            .collect::<Vec<_>>();

        let accepted = manager
            .note_gossip_addresses_from_source(Some("source-a"), &addresses, true)
            .expect("first gossip");
        assert_eq!(accepted.len(), limit);

        let more = manager
            .note_gossip_addresses_from_source(
                Some("source-a"),
                &[PeerAddress {
                    host: String::from("12.0.0.1"),
                    port: 56000,
                    services: 0,
                    last_seen_unix: now,
                }],
                true,
            )
            .expect("second gossip");
        assert!(more.is_empty());

        let other_source = manager
            .note_gossip_addresses_from_source(
                Some("source-b"),
                &[PeerAddress {
                    host: String::from("12.0.0.1"),
                    port: 56000,
                    services: 0,
                    last_seen_unix: now,
                }],
                true,
            )
            .expect("other source");
        assert_eq!(other_source.len(), 1);
    }

    #[test]
    fn one_subnet_cannot_dominate_public_peer_table() {
        let mut manager = AddressManager::new(Network::Mainnet);
        let limit = network_params(Network::Mainnet).limits.max_peers_per_subnet;
        let now = now_unix();
        let clustered = (0..limit + 8)
            .map(|index| PeerAddress {
                host: format!("11.7.7.{}", index + 1),
                port: 56000,
                services: 0,
                last_seen_unix: now,
            })
            .collect::<Vec<_>>();

        let accepted = manager
            .note_gossip_addresses_from_source(Some("source-a"), &clustered, true)
            .expect("cluster gossip");
        assert_eq!(accepted.len(), limit);

        let same_subnet_from_other_source = manager
            .note_gossip_addresses_from_source(
                Some("source-b"),
                &[PeerAddress {
                    host: String::from("11.7.7.250"),
                    port: 56000,
                    services: 0,
                    last_seen_unix: now,
                }],
                true,
            )
            .expect("same subnet gossip");
        assert!(same_subnet_from_other_source.is_empty());

        let diverse_subnet = manager
            .note_gossip_addresses_from_source(
                Some("source-b"),
                &[PeerAddress {
                    host: String::from("11.7.8.1"),
                    port: 56000,
                    services: 0,
                    last_seen_unix: now,
                }],
                true,
            )
            .expect("diverse subnet gossip");
        assert_eq!(diverse_subnet.len(), 1);
    }

    #[test]
    fn relay_addresses_skip_private_manual_and_stale_entries_on_public_networks() {
        let mut manager = AddressManager::new(Network::Mainnet);
        let now = now_unix();
        manager.add_manual_peer("127.0.0.9:56000");
        manager
            .note_gossip_addresses(
                &[
                    PeerAddress {
                        host: String::from("127.0.0.1"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("8.8.4.4"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now
                            - network_params(Network::Mainnet)
                                .limits
                                .max_peer_address_age_secs
                            - 1,
                    },
                    PeerAddress {
                        host: String::from("8.8.8.8"),
                        port: 56000,
                        services: 0,
                        last_seen_unix: now,
                    },
                ],
                false,
            )
            .expect("gossip");

        let relay = manager.relay_addresses(8);
        assert_eq!(relay.len(), 1);
        assert_eq!(relay[0].host, "8.8.8.8");
    }

    #[test]
    fn public_gossip_rejects_non_listening_source_ports() {
        let mut manager = AddressManager::new(Network::Testnet);
        let now = now_unix();
        let accepted = manager
            .note_gossip_addresses(
                &[
                    PeerAddress {
                        host: String::from("74.208.219.116"),
                        port: 33284,
                        services: 0,
                        last_seen_unix: now,
                    },
                    PeerAddress {
                        host: String::from("74.208.219.116"),
                        port: Network::Testnet.p2p_port(),
                        services: 0,
                        last_seen_unix: now,
                    },
                ],
                true,
            )
            .expect("gossip");

        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].port, Network::Testnet.p2p_port());
        assert_eq!(manager.advertisable_addresses(8).len(), 1);
    }

    #[test]
    fn malformed_manual_peers_are_not_advertised_or_relayed() {
        let mut manager = AddressManager::new(Network::Mainnet);
        manager.add_manual_peer("8.8.8.8:0");
        manager.add_manual_peer("bad host.example:56000");
        manager.add_manual_peer("seed.atho.io:56000");

        let local = manager.advertisable_addresses(8);
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].host, "seed.atho.io");

        let relay = manager.relay_addresses(8);
        assert_eq!(relay.len(), 1);
        assert_eq!(relay[0].host, "seed.atho.io");
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
