use atho_core::network::Network;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Handshake = 1,
    Ping = 2,
    Pong = 3,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("unknown message type")]
    UnknownMessageType,
    #[error("unsupported network")]
    UnsupportedNetwork,
}

impl TryFrom<u8> for MessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Handshake),
            2 => Ok(Self::Ping),
            3 => Ok(Self::Pong),
            _ => Err(ProtocolError::UnknownMessageType),
        }
    }
}

impl From<MessageType> for u8 {
    fn from(value: MessageType) -> Self {
        value as u8
    }
}

pub fn network_to_byte(network: Network) -> u8 {
    match network {
        Network::Mainnet => 0,
        Network::Testnet => 1,
        Network::Regnet => 2,
    }
}

pub fn network_from_byte(value: u8) -> Result<Network, ProtocolError> {
    match value {
        0 => Ok(Network::Mainnet),
        1 => Ok(Network::Testnet),
        2 => Ok(Network::Regnet),
        _ => Err(ProtocolError::UnsupportedNetwork),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub network: Network,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    network: Network,
    message_type: MessageType,
    payload: Vec<u8>,
}

impl Message {
    pub fn new(network: Network, message_type: MessageType, payload: Vec<u8>) -> Self {
        Self {
            network,
            message_type,
            payload,
        }
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn message_type(&self) -> MessageType {
        self.message_type
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

impl Handshake {
    pub fn to_message(&self) -> Message {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&self.protocol_version.to_le_bytes());
        Message::new(self.network, MessageType::Handshake, payload)
    }
}

pub fn validate_handshake(handshake: &Handshake) -> Result<(), ProtocolError> {
    match handshake.network {
        Network::Mainnet | Network::Testnet | Network::Regnet => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_round_trips_as_message() {
        let handshake = Handshake {
            network: Network::Mainnet,
            protocol_version: 1,
        };
        let message = handshake.to_message();
        assert_eq!(message.network(), Network::Mainnet);
        assert_eq!(message.message_type(), MessageType::Handshake);
    }
}
