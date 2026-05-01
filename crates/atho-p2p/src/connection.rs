use crate::address_manager::{format_remote_addr, AddressManager};
use crate::banlist::BanList;
use crate::config::network_params;
use crate::handshake::{HandshakeAction, HandshakeState};
use crate::protocol::{MessagePayload, NetworkMessage, PeerAddress, ProtocolError, VersionMessage};
use atho_core::network::Network;
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, P2P_BANNED_PEER, P2P_INBOUND_LIMIT, P2P_OUTBOUND_LIMIT,
    P2P_PEER_ALREADY_CONNECTED, P2P_UNKNOWN_PEER,
};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone)]
struct PeerSession {
    direction: ConnectionDirection,
    handshake: HandshakeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSessionSnapshot {
    pub remote_addr: String,
    pub direction: ConnectionDirection,
    pub handshake_ready: bool,
    pub best_height: Option<u64>,
    pub protocol_version: Option<u32>,
    pub services: Option<u64>,
    pub user_agent: Option<String>,
    pub ruleset_version: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    Send {
        peer: String,
        message: NetworkMessage,
    },
    Ready {
        peer: String,
        best_height: u64,
    },
    Message {
        peer: String,
        message: NetworkMessage,
    },
    Disconnect {
        peer: String,
        reason: String,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectionError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("peer already connected")]
    PeerAlreadyConnected,
    #[error("unknown peer session")]
    UnknownPeer,
    #[error("peer is banned")]
    BannedPeer,
    #[error("inbound peer limit reached")]
    InboundLimitReached,
    #[error("outbound peer limit reached")]
    OutboundLimitReached,
}

impl AthoErrorMeta for ConnectionError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Protocol(error) => error.descriptor(),
            Self::PeerAlreadyConnected => &P2P_PEER_ALREADY_CONNECTED,
            Self::UnknownPeer => &P2P_UNKNOWN_PEER,
            Self::BannedPeer => &P2P_BANNED_PEER,
            Self::InboundLimitReached => &P2P_INBOUND_LIMIT,
            Self::OutboundLimitReached => &P2P_OUTBOUND_LIMIT,
        }
    }

    fn source_module(&self) -> &'static str {
        match self {
            Self::Protocol(error) => error.source_module(),
            _ => "atho-p2p::connection",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionManager {
    network: Network,
    address_manager: AddressManager,
    banlist: BanList,
    sessions: BTreeMap<String, PeerSession>,
}

impl ConnectionManager {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            address_manager: AddressManager::new(network),
            banlist: BanList::new(network),
            sessions: BTreeMap::new(),
        }
    }

    pub fn add_manual_peer(&mut self, remote_addr: impl Into<String>) {
        self.address_manager.add_manual_peer(remote_addr);
    }

    pub fn note_gossip_addresses(
        &mut self,
        addresses: &[PeerAddress],
        public_source: bool,
    ) -> Result<Vec<PeerAddress>, ConnectionError> {
        self.address_manager
            .note_gossip_addresses(addresses, public_source)
            .map_err(ConnectionError::Protocol)
    }

    pub fn accept_inbound(
        &mut self,
        remote_addr: impl Into<String>,
    ) -> Result<(), ConnectionError> {
        let remote_addr = remote_addr.into();
        if self.sessions.contains_key(&remote_addr) {
            return Err(ConnectionError::PeerAlreadyConnected);
        }
        if self.banlist.is_banned(&remote_addr, now_unix()) {
            return Err(ConnectionError::BannedPeer);
        }
        if self.inbound_count() >= network_params(self.network).limits.max_inbound_peers {
            return Err(ConnectionError::InboundLimitReached);
        }
        self.sessions.insert(
            remote_addr,
            PeerSession {
                direction: ConnectionDirection::Inbound,
                handshake: HandshakeState::inbound(self.network),
            },
        );
        Ok(())
    }

    pub fn open_outbound(
        &mut self,
        remote_addr: impl Into<String>,
        local_version: NetworkMessage,
    ) -> Result<Vec<ConnectionEvent>, ConnectionError> {
        let remote_addr = remote_addr.into();
        if self.sessions.contains_key(&remote_addr) {
            return Err(ConnectionError::PeerAlreadyConnected);
        }
        if self.banlist.is_banned(&remote_addr, now_unix()) {
            return Err(ConnectionError::BannedPeer);
        }
        if self.outbound_count() >= network_params(self.network).limits.max_outbound_peers {
            return Err(ConnectionError::OutboundLimitReached);
        }
        let (handshake, actions) = HandshakeState::outbound(self.network, local_version)
            .map_err(ConnectionError::Protocol)?;
        self.sessions.insert(
            remote_addr.clone(),
            PeerSession {
                direction: ConnectionDirection::Outbound,
                handshake,
            },
        );
        Ok(actions
            .into_iter()
            .map(|action| match action {
                HandshakeAction::Send(message) => ConnectionEvent::Send {
                    peer: remote_addr.clone(),
                    message,
                },
                HandshakeAction::Ready { best_height } => ConnectionEvent::Ready {
                    peer: remote_addr.clone(),
                    best_height,
                },
            })
            .collect())
    }

    pub fn receive(
        &mut self,
        remote_addr: &str,
        message: NetworkMessage,
        local_version: &NetworkMessage,
    ) -> Result<Vec<ConnectionEvent>, ConnectionError> {
        if self.banlist.is_banned(remote_addr, now_unix()) {
            return Err(ConnectionError::BannedPeer);
        }
        let Some(session) = self.sessions.get_mut(remote_addr) else {
            return Err(ConnectionError::UnknownPeer);
        };

        if !session.handshake.is_ready() {
            match session.handshake.receive(&message, local_version) {
                Ok(actions) => {
                    if let Some(version) = session.handshake.remote_version() {
                        self.address_manager
                            .accept_version(remote_addr.to_string(), version)
                            .map_err(ConnectionError::Protocol)?;
                    }
                    return Ok(actions
                        .into_iter()
                        .map(|action| match action {
                            HandshakeAction::Send(message) => ConnectionEvent::Send {
                                peer: remote_addr.to_string(),
                                message,
                            },
                            HandshakeAction::Ready { best_height } => ConnectionEvent::Ready {
                                peer: remote_addr.to_string(),
                                best_height,
                            },
                        })
                        .collect());
                }
                Err(err) => {
                    let banned = self.banlist.record(remote_addr.to_string(), 50);
                    let reason = err.to_string();
                    let _ = self.sessions.remove(remote_addr);
                    if banned {
                        return Ok(vec![ConnectionEvent::Disconnect {
                            peer: remote_addr.to_string(),
                            reason,
                        }]);
                    }
                    return Err(ConnectionError::Protocol(err));
                }
            }
        }

        match &message.payload {
            MessagePayload::Ping { nonce } => Ok(vec![ConnectionEvent::Send {
                peer: remote_addr.to_string(),
                message: NetworkMessage::new(self.network, MessagePayload::Pong { nonce: *nonce }),
            }]),
            MessagePayload::GetAddr => Ok(vec![ConnectionEvent::Send {
                peer: remote_addr.to_string(),
                message: NetworkMessage::new(
                    self.network,
                    MessagePayload::Addr {
                        addresses: {
                            let limits = network_params(self.network).limits;
                            let share_limit = limits
                                .max_outbound_peers
                                .saturating_mul(8)
                                .max(8)
                                .min(limits.max_addr_per_message);
                            self.address_manager
                                .advertisable_addresses(share_limit)
                                .into_iter()
                                .filter(|address| format_remote_addr(address) != remote_addr)
                                .collect()
                        },
                    },
                ),
            }]),
            _ => Ok(vec![ConnectionEvent::Message {
                peer: remote_addr.to_string(),
                message,
            }]),
        }
    }

    pub fn remote_best_height(&self, remote_addr: &str) -> Option<u64> {
        self.sessions.get(remote_addr).and_then(|session| {
            session
                .handshake
                .remote_version()
                .map(|version| version.best_height)
        })
    }

    pub fn remote_version(&self, remote_addr: &str) -> Option<&VersionMessage> {
        self.sessions
            .get(remote_addr)
            .and_then(|session| session.handshake.remote_version())
    }

    pub fn has_peer(&self, remote_addr: &str) -> bool {
        self.sessions.contains_key(remote_addr)
    }

    pub fn disconnect(&mut self, remote_addr: &str) -> bool {
        self.sessions.remove(remote_addr).is_some()
    }

    pub fn address_manager(&self) -> &AddressManager {
        &self.address_manager
    }

    pub fn peer_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn inbound_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.direction == ConnectionDirection::Inbound)
            .count()
    }

    pub fn outbound_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.direction == ConnectionDirection::Outbound)
            .count()
    }

    pub fn peer_snapshots(&self) -> Vec<PeerSessionSnapshot> {
        self.sessions
            .iter()
            .map(|(remote_addr, session)| {
                let remote_version = session.handshake.remote_version();
                PeerSessionSnapshot {
                    remote_addr: remote_addr.clone(),
                    direction: session.direction,
                    handshake_ready: session.handshake.is_ready(),
                    best_height: remote_version.map(|version| version.best_height),
                    protocol_version: remote_version.map(|version| version.protocol_version),
                    services: remote_version.map(|version| version.services),
                    user_agent: remote_version.map(|version| version.user_agent.clone()),
                    ruleset_version: remote_version.map(|version| version.ruleset_version),
                }
            })
            .collect()
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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

    fn version_message(network: Network, height: u64) -> NetworkMessage {
        NetworkMessage::new(
            network,
            MessagePayload::Version(VersionMessage {
                protocol_version: rules::PROTOCOL_VERSION,
                min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
                services: LOCAL_NODE_SERVICES,
                timestamp_unix: 1_700_000_000,
                network,
                user_agent: String::from("/Atho:0.1.0/"),
                best_height: height,
                ruleset_version: rules::RULESET_VERSION_V1,
                relay: true,
                genesis_hash: Hash48::from(genesis::genesis_hash(network)),
                tip_hash: Hash48::ZERO,
                chainwork: Hash48::ZERO,
            }),
        )
    }

    #[test]
    fn managers_complete_a_two_way_handshake() {
        let mut left = ConnectionManager::new(Network::Mainnet);
        let mut right = ConnectionManager::new(Network::Mainnet);
        right.accept_inbound("left").expect("inbound");
        let mut outbound = left
            .open_outbound("right", version_message(Network::Mainnet, 5))
            .expect("outbound");
        assert_eq!(outbound.len(), 1);

        let first = match outbound.remove(0) {
            ConnectionEvent::Send { message, .. } => message,
            other => panic!("unexpected action: {other:?}"),
        };
        let inbound_actions = right
            .receive("left", first, &version_message(Network::Mainnet, 8))
            .expect("receive version");
        assert!(inbound_actions
            .iter()
            .any(|event| matches!(event, ConnectionEvent::Send { message, .. } if matches!(message.payload, MessagePayload::Verack))));
    }

    #[test]
    fn getaddr_returns_known_addresses_after_handshake() {
        let mut manager = ConnectionManager::new(Network::Mainnet);
        manager.accept_inbound("left").expect("inbound");
        manager
            .address_manager
            .note_gossip_addresses(
                &[crate::protocol::PeerAddress {
                    host: String::from("8.8.8.8"),
                    port: 56000,
                    services: 0,
                    last_seen_unix: 0,
                }],
                false,
            )
            .expect("note");
        let _ = manager
            .receive(
                "left",
                version_message(Network::Mainnet, 1),
                &version_message(Network::Mainnet, 1),
            )
            .expect("version");
        let _ = manager
            .receive(
                "left",
                NetworkMessage::new(Network::Mainnet, MessagePayload::Verack),
                &version_message(Network::Mainnet, 1),
            )
            .expect("verack");
        let actions = manager
            .receive(
                "left",
                NetworkMessage::new(Network::Mainnet, MessagePayload::GetAddr),
                &version_message(Network::Mainnet, 1),
            )
            .expect("getaddr");
        assert!(actions.iter().any(|event| matches!(
            event,
            ConnectionEvent::Send {
                message: NetworkMessage {
                    payload: MessagePayload::Addr { .. },
                    ..
                },
                ..
            }
        )));
    }
}
