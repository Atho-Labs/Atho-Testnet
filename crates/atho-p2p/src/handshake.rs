use crate::protocol::{MessagePayload, NetworkMessage, ProtocolError, VersionMessage};
use atho_core::network::Network;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeAction {
    Send(NetworkMessage),
    Ready { best_height: u64 },
}

#[derive(Debug, Clone)]
pub struct HandshakeState {
    network: Network,
    _direction: HandshakeDirection,
    local_version_sent: bool,
    local_verack_sent: bool,
    remote_version: Option<VersionMessage>,
    remote_verack_seen: bool,
    ready: bool,
}

impl HandshakeState {
    pub fn inbound(network: Network) -> Self {
        Self {
            network,
            _direction: HandshakeDirection::Inbound,
            local_version_sent: false,
            local_verack_sent: false,
            remote_version: None,
            remote_verack_seen: false,
            ready: false,
        }
    }

    pub fn outbound(
        network: Network,
        local_version: NetworkMessage,
    ) -> Result<(Self, Vec<HandshakeAction>), ProtocolError> {
        if !matches!(local_version.payload, MessagePayload::Version(_)) {
            return Err(ProtocolError::UnexpectedPayload);
        }
        Ok((
            Self {
                network,
                _direction: HandshakeDirection::Outbound,
                local_version_sent: true,
                local_verack_sent: false,
                remote_version: None,
                remote_verack_seen: false,
                ready: false,
            },
            vec![HandshakeAction::Send(local_version)],
        ))
    }

    pub fn remote_version(&self) -> Option<&VersionMessage> {
        self.remote_version.as_ref()
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn receive(
        &mut self,
        message: &NetworkMessage,
        local_version: &NetworkMessage,
    ) -> Result<Vec<HandshakeAction>, ProtocolError> {
        if message.network != self.network || local_version.network != self.network {
            return Err(ProtocolError::UnsupportedNetwork);
        }

        let mut actions = Vec::new();
        match &message.payload {
            MessagePayload::Version(version) => {
                if self.remote_version.is_some() {
                    return Err(ProtocolError::UnexpectedPayload);
                }
                crate::protocol::validate_version_message(version, self.network)?;
                self.remote_version = Some(version.clone());
                if !self.local_version_sent {
                    actions.push(HandshakeAction::Send(local_version.clone()));
                    self.local_version_sent = true;
                }
                if !self.local_verack_sent {
                    actions.push(HandshakeAction::Send(NetworkMessage::new(
                        self.network,
                        MessagePayload::Verack,
                    )));
                    self.local_verack_sent = true;
                }
            }
            MessagePayload::Verack => {
                if self.remote_version.is_none() {
                    return Err(ProtocolError::HandshakeIncomplete);
                }
                self.remote_verack_seen = true;
            }
            _ => return Err(ProtocolError::HandshakeIncomplete),
        }

        if !self.ready
            && self.remote_version.is_some()
            && self.local_verack_sent
            && self.remote_verack_seen
        {
            self.ready = true;
            let best_height = self
                .remote_version
                .as_ref()
                .map(|version| version.best_height)
                .unwrap_or_default();
            actions.push(HandshakeAction::Ready { best_height });
        }

        Ok(actions)
    }
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
    fn outbound_and_inbound_handshake_reaches_ready() {
        let local = version_message(Network::Mainnet, 10);
        let remote = version_message(Network::Mainnet, 11);
        let (mut outbound, init) =
            HandshakeState::outbound(Network::Mainnet, local.clone()).expect("outbound");
        assert_eq!(init.len(), 1);
        let mut inbound = HandshakeState::inbound(Network::Mainnet);

        let inbound_actions = inbound
            .receive(&init[0].clone().into_send(), &remote)
            .unwrap();
        assert!(inbound_actions
            .iter()
            .any(|action| matches!(action, HandshakeAction::Send(_))));
        let outbound_actions = outbound.receive(&remote, &local).unwrap();
        assert!(outbound_actions
            .iter()
            .any(|action| matches!(action, HandshakeAction::Send(message) if matches!(message.payload, MessagePayload::Verack))));
        let ready = outbound
            .receive(
                &NetworkMessage::new(Network::Mainnet, MessagePayload::Verack),
                &local,
            )
            .unwrap();
        assert!(ready
            .iter()
            .any(|action| matches!(action, HandshakeAction::Ready { best_height: 11 })));
    }

    trait IntoSendMessage {
        fn into_send(self) -> NetworkMessage;
    }

    impl IntoSendMessage for HandshakeAction {
        fn into_send(self) -> NetworkMessage {
            match self {
                HandshakeAction::Send(message) => message,
                HandshakeAction::Ready { .. } => panic!("expected send action"),
            }
        }
    }
}
