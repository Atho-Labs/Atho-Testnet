use crate::config::{network_params, MIN_SUPPORTED_PROTOCOL_VERSION};
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::rules;
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const NODE_NETWORK: u64 = 1 << 0;
pub const NODE_WITNESS: u64 = 1 << 3;
pub const LOCAL_NODE_SERVICES: u64 = NODE_NETWORK | NODE_WITNESS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Hash48(#[serde(with = "serde_big_array::BigArray")] pub [u8; 48]);

impl Hash48 {
    pub const ZERO: Self = Self([0; 48]);

    pub fn into_inner(self) -> [u8; 48] {
        self.0
    }
}

impl From<[u8; 48]> for Hash48 {
    fn from(value: [u8; 48]) -> Self {
        Self(value)
    }
}

impl From<Hash48> for [u8; 48] {
    fn from(value: Hash48) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageCommand {
    Version,
    Verack,
    Ping,
    Pong,
    GetAddr,
    Addr,
    Inv,
    GetData,
    NotFound,
    GetHeaders,
    Headers,
    Block,
    Tx,
    MemPool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InventoryKind {
    Transaction,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryVector {
    pub kind: InventoryKind,
    pub hash: Hash48,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAddress {
    pub host: String,
    pub port: u16,
    pub services: u64,
    pub last_seen_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionMessage {
    pub protocol_version: u32,
    pub min_protocol_version: u32,
    pub services: u64,
    pub timestamp_unix: i64,
    pub network: Network,
    pub user_agent: String,
    pub best_height: u64,
    pub ruleset_version: u32,
    pub relay: bool,
    pub genesis_hash: Hash48,
    pub tip_hash: Hash48,
    pub chainwork: Hash48,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetHeadersMessage {
    pub locator_hashes: Vec<Hash48>,
    pub stop_hash: Hash48,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePayload {
    Version(VersionMessage),
    Verack,
    Ping { nonce: u64 },
    Pong { nonce: u64 },
    GetAddr,
    Addr { addresses: Vec<PeerAddress> },
    Inv { inventory: Vec<InventoryVector> },
    GetData { inventory: Vec<InventoryVector> },
    NotFound { inventory: Vec<InventoryVector> },
    GetHeaders(GetHeadersMessage),
    Headers { headers: Vec<BlockHeader> },
    Block(Block),
    Tx(Transaction),
    MemPool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkMessage {
    pub network: Network,
    pub payload: MessagePayload,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("unsupported network")]
    UnsupportedNetwork,
    #[error("unknown message command")]
    UnknownMessageCommand,
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("malformed payload")]
    MalformedPayload,
    #[error("unexpected payload for message command")]
    UnexpectedPayload,
    #[error("unsupported protocol version")]
    UnsupportedProtocolVersion,
    #[error("genesis mismatch")]
    GenesisMismatch,
    #[error("ruleset mismatch")]
    RulesetMismatch,
    #[error("too many peer addresses")]
    TooManyPeerAddresses,
    #[error("too many inventory entries")]
    TooManyInventoryEntries,
    #[error("too many headers")]
    TooManyHeaders,
    #[error("invalid headers sequence")]
    InvalidHeadersSequence,
    #[error("peer book full")]
    PeerBookFull,
}

impl MessageCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Verack => "verack",
            Self::Ping => "ping",
            Self::Pong => "pong",
            Self::GetAddr => "getaddr",
            Self::Addr => "addr",
            Self::Inv => "inv",
            Self::GetData => "getdata",
            Self::NotFound => "notfound",
            Self::GetHeaders => "getheaders",
            Self::Headers => "headers",
            Self::Block => "block",
            Self::Tx => "tx",
            Self::MemPool => "mempool",
        }
    }

    pub fn from_bytes(raw: [u8; 12]) -> Result<Self, ProtocolError> {
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        let name =
            std::str::from_utf8(&raw[..end]).map_err(|_| ProtocolError::UnknownMessageCommand)?;
        match name {
            "version" => Ok(Self::Version),
            "verack" => Ok(Self::Verack),
            "ping" => Ok(Self::Ping),
            "pong" => Ok(Self::Pong),
            "getaddr" => Ok(Self::GetAddr),
            "addr" => Ok(Self::Addr),
            "inv" => Ok(Self::Inv),
            "getdata" => Ok(Self::GetData),
            "notfound" => Ok(Self::NotFound),
            "getheaders" => Ok(Self::GetHeaders),
            "headers" => Ok(Self::Headers),
            "block" => Ok(Self::Block),
            "tx" => Ok(Self::Tx),
            "mempool" => Ok(Self::MemPool),
            _ => Err(ProtocolError::UnknownMessageCommand),
        }
    }

    pub fn as_padded_bytes(self) -> [u8; 12] {
        let mut out = [0u8; 12];
        let name = self.as_str().as_bytes();
        out[..name.len()].copy_from_slice(name);
        out
    }
}

impl MessagePayload {
    pub fn command(&self) -> MessageCommand {
        match self {
            Self::Version(_) => MessageCommand::Version,
            Self::Verack => MessageCommand::Verack,
            Self::Ping { .. } => MessageCommand::Ping,
            Self::Pong { .. } => MessageCommand::Pong,
            Self::GetAddr => MessageCommand::GetAddr,
            Self::Addr { .. } => MessageCommand::Addr,
            Self::Inv { .. } => MessageCommand::Inv,
            Self::GetData { .. } => MessageCommand::GetData,
            Self::NotFound { .. } => MessageCommand::NotFound,
            Self::GetHeaders(_) => MessageCommand::GetHeaders,
            Self::Headers { .. } => MessageCommand::Headers,
            Self::Block(_) => MessageCommand::Block,
            Self::Tx(_) => MessageCommand::Tx,
            Self::MemPool => MessageCommand::MemPool,
        }
    }
}

impl NetworkMessage {
    pub fn new(network: Network, payload: MessagePayload) -> Self {
        Self { network, payload }
    }

    pub fn command(&self) -> MessageCommand {
        self.payload.command()
    }

    pub fn encode_payload(&self) -> Result<Vec<u8>, ProtocolError> {
        match &self.payload {
            MessagePayload::Version(message) => serialize(message),
            MessagePayload::Verack | MessagePayload::GetAddr | MessagePayload::MemPool => {
                Ok(Vec::new())
            }
            MessagePayload::Ping { nonce } | MessagePayload::Pong { nonce } => serialize(nonce),
            MessagePayload::Addr { addresses } => {
                if addresses.len() > network_params(self.network).limits.max_addr_per_message {
                    return Err(ProtocolError::TooManyPeerAddresses);
                }
                serialize(addresses)
            }
            MessagePayload::Inv { inventory }
            | MessagePayload::GetData { inventory }
            | MessagePayload::NotFound { inventory } => {
                if inventory.len() > network_params(self.network).limits.max_inv_per_message {
                    return Err(ProtocolError::TooManyInventoryEntries);
                }
                serialize(inventory)
            }
            MessagePayload::GetHeaders(message) => serialize(message),
            MessagePayload::Headers { headers } => {
                if headers.len() > network_params(self.network).limits.max_headers_per_message {
                    return Err(ProtocolError::TooManyHeaders);
                }
                serialize(headers)
            }
            MessagePayload::Block(block) => serialize(block),
            MessagePayload::Tx(transaction) => serialize(transaction),
        }
    }

    pub fn decode(
        network: Network,
        command: MessageCommand,
        payload: &[u8],
    ) -> Result<Self, ProtocolError> {
        let payload = match command {
            MessageCommand::Version => MessagePayload::Version(deserialize(payload)?),
            MessageCommand::Verack => {
                expect_empty(payload)?;
                MessagePayload::Verack
            }
            MessageCommand::Ping => MessagePayload::Ping {
                nonce: deserialize(payload)?,
            },
            MessageCommand::Pong => MessagePayload::Pong {
                nonce: deserialize(payload)?,
            },
            MessageCommand::GetAddr => {
                expect_empty(payload)?;
                MessagePayload::GetAddr
            }
            MessageCommand::Addr => {
                let addresses: Vec<PeerAddress> = deserialize(payload)?;
                if addresses.len() > network_params(network).limits.max_addr_per_message {
                    return Err(ProtocolError::TooManyPeerAddresses);
                }
                MessagePayload::Addr { addresses }
            }
            MessageCommand::Inv => {
                let inventory: Vec<InventoryVector> = deserialize(payload)?;
                if inventory.len() > network_params(network).limits.max_inv_per_message {
                    return Err(ProtocolError::TooManyInventoryEntries);
                }
                MessagePayload::Inv { inventory }
            }
            MessageCommand::GetData => {
                let inventory: Vec<InventoryVector> = deserialize(payload)?;
                if inventory.len() > network_params(network).limits.max_inv_per_message {
                    return Err(ProtocolError::TooManyInventoryEntries);
                }
                MessagePayload::GetData { inventory }
            }
            MessageCommand::NotFound => {
                let inventory: Vec<InventoryVector> = deserialize(payload)?;
                if inventory.len() > network_params(network).limits.max_inv_per_message {
                    return Err(ProtocolError::TooManyInventoryEntries);
                }
                MessagePayload::NotFound { inventory }
            }
            MessageCommand::GetHeaders => MessagePayload::GetHeaders(deserialize(payload)?),
            MessageCommand::Headers => {
                let headers: Vec<BlockHeader> = deserialize(payload)?;
                if headers.len() > network_params(network).limits.max_headers_per_message {
                    return Err(ProtocolError::TooManyHeaders);
                }
                MessagePayload::Headers { headers }
            }
            MessageCommand::Block => MessagePayload::Block(deserialize(payload)?),
            MessageCommand::Tx => MessagePayload::Tx(deserialize(payload)?),
            MessageCommand::MemPool => {
                expect_empty(payload)?;
                MessagePayload::MemPool
            }
        };
        Ok(Self::new(network, payload))
    }
}

pub fn validate_version_message(
    message: &VersionMessage,
    expected_network: Network,
) -> Result<(), ProtocolError> {
    if message.network != expected_network {
        return Err(ProtocolError::UnsupportedNetwork);
    }
    let params = network_params(expected_network);
    if message.protocol_version < MIN_SUPPORTED_PROTOCOL_VERSION
        || message.protocol_version < params.min_supported_protocol_version
    {
        return Err(ProtocolError::UnsupportedProtocolVersion);
    }
    if message.min_protocol_version > params.protocol_version {
        return Err(ProtocolError::UnsupportedProtocolVersion);
    }
    if message.genesis_hash != Hash48::from(genesis::genesis_hash(expected_network)) {
        return Err(ProtocolError::GenesisMismatch);
    }
    if message.ruleset_version != rules::ruleset_version_at_height(message.best_height) {
        return Err(ProtocolError::RulesetMismatch);
    }
    Ok(())
}

fn expect_empty(payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(ProtocolError::UnexpectedPayload)
    }
}

fn serialize<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    bincode::serialize(value).map_err(|_| ProtocolError::MalformedPayload)
}

fn deserialize<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> Result<T, ProtocolError> {
    bincode::deserialize(payload).map_err(|_| ProtocolError::MalformedPayload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_round_trip_to_padded_bytes() {
        for command in [
            MessageCommand::Version,
            MessageCommand::Verack,
            MessageCommand::Ping,
            MessageCommand::Pong,
            MessageCommand::GetAddr,
            MessageCommand::Addr,
            MessageCommand::Inv,
            MessageCommand::GetData,
            MessageCommand::NotFound,
            MessageCommand::GetHeaders,
            MessageCommand::Headers,
            MessageCommand::Block,
            MessageCommand::Tx,
            MessageCommand::MemPool,
        ] {
            assert_eq!(
                MessageCommand::from_bytes(command.as_padded_bytes()).expect("decode"),
                command
            );
        }
    }

    #[test]
    fn version_message_validation_rejects_wrong_genesis() {
        let message = VersionMessage {
            protocol_version: rules::PROTOCOL_VERSION,
            min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            services: LOCAL_NODE_SERVICES,
            timestamp_unix: 1_700_000_000,
            network: Network::Mainnet,
            user_agent: String::from("/Atho:0.1.0/"),
            best_height: 0,
            ruleset_version: rules::RULESET_VERSION_V1,
            relay: true,
            genesis_hash: Hash48([9; 48]),
            tip_hash: Hash48::ZERO,
            chainwork: Hash48::ZERO,
        };
        assert_eq!(
            validate_version_message(&message, Network::Mainnet),
            Err(ProtocolError::GenesisMismatch)
        );
    }
}
