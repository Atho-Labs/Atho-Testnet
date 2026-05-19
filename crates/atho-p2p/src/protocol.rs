// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Atho P2P message definitions and protocol validation.
use crate::config::{network_params, MIN_SUPPORTED_PROTOCOL_VERSION};
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::rules;
use atho_core::constants::MAX_BLOCK_SIZE_BYTES;
use atho_core::crypto::hash::sha3_256;
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, NET_GENESIS_MISMATCH, NET_RULESET_MISMATCH,
    NET_UNSUPPORTED_NETWORK, P2P_HANDSHAKE_INCOMPLETE, P2P_INVALID_COMPACT_BLOCK,
    P2P_INVALID_HEADERS_SEQUENCE, P2P_MALFORMED_PAYLOAD, P2P_PAYLOAD_TOO_LARGE, P2P_PEER_BOOK_FULL,
    P2P_TOO_MANY_HEADERS, P2P_TOO_MANY_INVENTORY, P2P_TOO_MANY_LOCATORS,
    P2P_TOO_MANY_PEER_ADDRESSES, P2P_UNEXPECTED_PAYLOAD, P2P_UNKNOWN_COMMAND,
    P2P_UNSUPPORTED_PROTOCOL, P2P_USER_AGENT_TOO_LONG,
};
use bincode::Options;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const NODE_NETWORK: u64 = 1 << 0;
pub const NODE_WITNESS: u64 = 1 << 3;
pub const LOCAL_NODE_SERVICES: u64 = NODE_NETWORK | NODE_WITNESS;
// version + marker + input_count + output_count + witness_len + lock_time +
// tx_pow_nonce + tx_pow_bits for the strict full-transaction format.
const MIN_SERIALIZED_TRANSACTION_BYTES: usize = 29;

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
    CompactBlock,
    GetBlockTxn,
    BlockTxn,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefilledTransaction {
    pub index: u32,
    pub transaction: Transaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactBlockMessage {
    pub header: BlockHeader,
    pub tx_count: usize,
    pub short_ids: Vec<u64>,
    pub prefilled_transactions: Vec<PrefilledTransaction>,
    pub fees_total_atoms: u64,
    pub fees_miner_atoms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetBlockTxnMessage {
    pub block_hash: Hash48,
    pub indexes: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockTxnMessage {
    pub block_hash: Hash48,
    pub indexes: Vec<u32>,
    pub transactions: Vec<Transaction>,
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
    CompactBlock(CompactBlockMessage),
    GetBlockTxn(GetBlockTxnMessage),
    BlockTxn(BlockTxnMessage),
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
    #[error("user agent too long")]
    UserAgentTooLong,
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
    #[error("handshake incomplete")]
    HandshakeIncomplete,
    #[error("too many locator hashes")]
    TooManyLocatorHashes,
    #[error("invalid compact block")]
    InvalidCompactBlock,
}

impl AthoErrorMeta for ProtocolError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::UnsupportedNetwork => &NET_UNSUPPORTED_NETWORK,
            Self::UnknownMessageCommand => &P2P_UNKNOWN_COMMAND,
            Self::PayloadTooLarge => &P2P_PAYLOAD_TOO_LARGE,
            Self::MalformedPayload => &P2P_MALFORMED_PAYLOAD,
            Self::UnexpectedPayload => &P2P_UNEXPECTED_PAYLOAD,
            Self::UnsupportedProtocolVersion => &P2P_UNSUPPORTED_PROTOCOL,
            Self::UserAgentTooLong => &P2P_USER_AGENT_TOO_LONG,
            Self::GenesisMismatch => &NET_GENESIS_MISMATCH,
            Self::RulesetMismatch => &NET_RULESET_MISMATCH,
            Self::TooManyPeerAddresses => &P2P_TOO_MANY_PEER_ADDRESSES,
            Self::TooManyInventoryEntries => &P2P_TOO_MANY_INVENTORY,
            Self::TooManyHeaders => &P2P_TOO_MANY_HEADERS,
            Self::InvalidHeadersSequence => &P2P_INVALID_HEADERS_SEQUENCE,
            Self::PeerBookFull => &P2P_PEER_BOOK_FULL,
            Self::HandshakeIncomplete => &P2P_HANDSHAKE_INCOMPLETE,
            Self::TooManyLocatorHashes => &P2P_TOO_MANY_LOCATORS,
            Self::InvalidCompactBlock => &P2P_INVALID_COMPACT_BLOCK,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-p2p::protocol"
    }
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
            Self::CompactBlock => "cmpctblock",
            Self::GetBlockTxn => "getblocktxn",
            Self::BlockTxn => "blocktxn",
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
            "cmpctblock" => Ok(Self::CompactBlock),
            "getblocktxn" => Ok(Self::GetBlockTxn),
            "blocktxn" => Ok(Self::BlockTxn),
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
            Self::CompactBlock(_) => MessageCommand::CompactBlock,
            Self::GetBlockTxn(_) => MessageCommand::GetBlockTxn,
            Self::BlockTxn(_) => MessageCommand::BlockTxn,
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
            MessagePayload::GetHeaders(message) => {
                if message.locator_hashes.len() > 32 {
                    return Err(ProtocolError::TooManyLocatorHashes);
                }
                serialize(message)
            }
            MessagePayload::Headers { headers } => {
                if headers.len() > network_params(self.network).limits.max_headers_per_message {
                    return Err(ProtocolError::TooManyHeaders);
                }
                serialize(headers)
            }
            MessagePayload::Block(block) => serialize(block),
            MessagePayload::Tx(transaction) => serialize(transaction),
            MessagePayload::CompactBlock(message) => {
                validate_compact_block_message(message)?;
                serialize(message)
            }
            MessagePayload::GetBlockTxn(message) => {
                validate_getblocktxn_message(message)?;
                serialize(message)
            }
            MessagePayload::BlockTxn(message) => {
                validate_blocktxn_message(message)?;
                serialize(message)
            }
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
            MessageCommand::GetHeaders => {
                let message: GetHeadersMessage = deserialize(payload)?;
                if message.locator_hashes.len() > 32 {
                    return Err(ProtocolError::TooManyLocatorHashes);
                }
                MessagePayload::GetHeaders(message)
            }
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
            MessageCommand::CompactBlock => {
                let message: CompactBlockMessage = deserialize(payload)?;
                validate_compact_block_message(&message)?;
                MessagePayload::CompactBlock(message)
            }
            MessageCommand::GetBlockTxn => {
                let message: GetBlockTxnMessage = deserialize(payload)?;
                validate_getblocktxn_message(&message)?;
                MessagePayload::GetBlockTxn(message)
            }
            MessageCommand::BlockTxn => {
                let message: BlockTxnMessage = deserialize(payload)?;
                validate_blocktxn_message(&message)?;
                MessagePayload::BlockTxn(message)
            }
        };
        Ok(Self::new(network, payload))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactBlockReconstruction {
    Complete(Box<Block>),
    Missing {
        block_hash: [u8; 48],
        indexes: Vec<u32>,
    },
}

pub fn compact_short_id(transaction_identity: [u8; 48]) -> u64 {
    let mut preimage =
        Vec::with_capacity(b"ATHO_COMPACT_SHORTID_V1".len() + transaction_identity.len());
    preimage.extend_from_slice(b"ATHO_COMPACT_SHORTID_V1");
    preimage.extend_from_slice(&transaction_identity);
    let digest = sha3_256(&preimage);
    u64::from_le_bytes(digest[..8].try_into().expect("compact short id"))
}

pub fn compact_block_from_block(block: &Block) -> CompactBlockMessage {
    CompactBlockMessage {
        header: block.header.clone(),
        tx_count: block.transactions.len(),
        short_ids: block
            .transactions
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 0)
            .map(|(_, tx)| compact_short_id(tx.witness_commitment_hash()))
            .collect(),
        prefilled_transactions: block
            .transactions
            .first()
            .cloned()
            .map(|transaction| {
                vec![PrefilledTransaction {
                    index: 0,
                    transaction,
                }]
            })
            .unwrap_or_default(),
        fees_total_atoms: block.fees_total_atoms,
        fees_miner_atoms: block.fees_miner_atoms,
    }
}

pub fn reconstruct_compact_block<F>(
    message: &CompactBlockMessage,
    mut lookup_short_id: F,
    overrides: &BTreeMap<u32, Transaction>,
) -> Result<CompactBlockReconstruction, ProtocolError>
where
    F: FnMut(u64) -> Option<Transaction>,
{
    validate_compact_block_message(message)?;

    let mut slots = vec![None; message.tx_count];
    for prefilled in &message.prefilled_transactions {
        let index = prefilled.index as usize;
        if index >= slots.len() || slots[index].is_some() {
            return Err(ProtocolError::InvalidCompactBlock);
        }
        slots[index] = Some(prefilled.transaction.clone());
    }
    let non_prefilled_indexes = slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| slot.is_none().then_some(index))
        .collect::<Vec<_>>();
    if non_prefilled_indexes.len() != message.short_ids.len() {
        return Err(ProtocolError::InvalidCompactBlock);
    }

    let mut missing = Vec::new();
    for (index, short_id) in non_prefilled_indexes
        .into_iter()
        .zip(message.short_ids.iter())
    {
        if let Some(transaction) = overrides.get(&(index as u32)).cloned() {
            slots[index] = Some(transaction);
        } else if let Some(transaction) = lookup_short_id(*short_id) {
            slots[index] = Some(transaction);
        } else {
            missing.push(index as u32);
        }
    }

    if !missing.is_empty() {
        return Ok(CompactBlockReconstruction::Missing {
            block_hash: message.header.block_hash(),
            indexes: missing,
        });
    }

    let transactions = slots
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(ProtocolError::InvalidCompactBlock)?;
    Ok(CompactBlockReconstruction::Complete(Box::new(Block {
        header: message.header.clone(),
        transactions,
        witnesses: Default::default(),
        fees_total_atoms: message.fees_total_atoms,
        fees_miner_atoms: message.fees_miner_atoms,
    })))
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
    if message.user_agent.len() > params.limits.max_user_agent_bytes {
        return Err(ProtocolError::UserAgentTooLong);
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
    bincode_options()
        .serialize(value)
        .map_err(|_| ProtocolError::MalformedPayload)
}

fn deserialize<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> Result<T, ProtocolError> {
    bincode_options()
        .with_limit(payload.len() as u64)
        .reject_trailing_bytes()
        .deserialize(payload)
        .map_err(|_| ProtocolError::MalformedPayload)
}

fn bincode_options() -> impl Options {
    bincode::DefaultOptions::new()
}

fn max_block_transaction_count() -> usize {
    MAX_BLOCK_SIZE_BYTES / MIN_SERIALIZED_TRANSACTION_BYTES
}

fn validate_compact_block_message(message: &CompactBlockMessage) -> Result<(), ProtocolError> {
    if message.tx_count == 0 || message.tx_count > max_block_transaction_count() {
        return Err(ProtocolError::InvalidCompactBlock);
    }
    if message.prefilled_transactions.len() > message.tx_count
        || message.short_ids.len() > message.tx_count
    {
        return Err(ProtocolError::InvalidCompactBlock);
    }
    if message.prefilled_transactions.len() + message.short_ids.len() != message.tx_count {
        return Err(ProtocolError::InvalidCompactBlock);
    }
    Ok(())
}

fn validate_getblocktxn_message(message: &GetBlockTxnMessage) -> Result<(), ProtocolError> {
    if message.indexes.len() > max_block_transaction_count() {
        return Err(ProtocolError::InvalidCompactBlock);
    }
    Ok(())
}

fn validate_blocktxn_message(message: &BlockTxnMessage) -> Result<(), ProtocolError> {
    if message.indexes.len() != message.transactions.len()
        || message.indexes.len() > max_block_transaction_count()
    {
        return Err(ProtocolError::InvalidCompactBlock);
    }
    Ok(())
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
            MessageCommand::CompactBlock,
            MessageCommand::GetBlockTxn,
            MessageCommand::BlockTxn,
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

    #[test]
    fn version_message_validation_rejects_oversized_user_agent() {
        let message = VersionMessage {
            protocol_version: rules::PROTOCOL_VERSION,
            min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            services: LOCAL_NODE_SERVICES,
            timestamp_unix: 1_700_000_000,
            network: Network::Mainnet,
            user_agent: "A".repeat(
                network_params(Network::Mainnet)
                    .limits
                    .max_user_agent_bytes
                    .saturating_add(1),
            ),
            best_height: 0,
            ruleset_version: rules::RULESET_VERSION_V1,
            relay: true,
            genesis_hash: Hash48::from(genesis::genesis_hash(Network::Mainnet)),
            tip_hash: Hash48::ZERO,
            chainwork: Hash48::ZERO,
        };
        assert_eq!(
            validate_version_message(&message, Network::Mainnet),
            Err(ProtocolError::UserAgentTooLong)
        );
    }

    #[test]
    fn getheaders_rejects_locator_above_cap() {
        let message = NetworkMessage::new(
            Network::Mainnet,
            MessagePayload::GetHeaders(GetHeadersMessage {
                locator_hashes: vec![Hash48::ZERO; 33],
                stop_hash: Hash48::ZERO,
            }),
        );
        assert_eq!(
            message.encode_payload(),
            Err(ProtocolError::TooManyLocatorHashes)
        );
    }

    #[test]
    fn malformed_and_oversized_peer_discovery_payloads_are_rejected() {
        let limit = network_params(Network::Mainnet).limits.max_addr_per_message;
        let addresses = (0..=limit)
            .map(|index| PeerAddress {
                host: format!("203.0.113.{}", index % 255),
                port: 56000,
                services: 0,
                last_seen_unix: 1_700_000_000,
            })
            .collect::<Vec<_>>();
        let payload = serialize(&addresses).expect("serialize oversized addr payload");

        assert_eq!(
            NetworkMessage::decode(Network::Mainnet, MessageCommand::Addr, &payload),
            Err(ProtocolError::TooManyPeerAddresses)
        );
        assert_eq!(
            NetworkMessage::decode(Network::Mainnet, MessageCommand::GetAddr, &[1]),
            Err(ProtocolError::UnexpectedPayload)
        );
        assert_eq!(
            NetworkMessage::decode(Network::Mainnet, MessageCommand::Ping, &[]),
            Err(ProtocolError::MalformedPayload)
        );
    }

    #[test]
    fn compact_block_round_trips_through_prefill_and_short_ids() {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![atho_core::transaction::TxOutput {
                value_atoms: 1,
                locking_script: vec![0],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![atho_core::transaction::TxOutput {
                value_atoms: 1,
                locking_script: vec![1],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let block = Block {
            header: BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: [1; 48],
                witness_root: [2; 48],
                founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
                founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
                timestamp: 1,
                difficulty_target_or_bits: [3; 48],
                nonce: 4,
            },
            transactions: vec![coinbase, tx.clone()],
            witnesses: Default::default(),
            fees_total_atoms: 5,
            fees_miner_atoms: 5,
        };
        let compact = compact_block_from_block(&block);
        let reconstructed = reconstruct_compact_block(
            &compact,
            |short_id| {
                (short_id == compact_short_id(tx.witness_commitment_hash())).then_some(tx.clone())
            },
            &BTreeMap::new(),
        )
        .expect("reconstruct");
        assert_eq!(
            reconstructed,
            CompactBlockReconstruction::Complete(Box::new(block))
        );
    }

    #[test]
    fn compact_short_ids_differentiate_tx_pow_variants_with_same_txid() {
        let base = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![atho_core::transaction::TxOutput {
                value_atoms: 1,
                locking_script: vec![1],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let variant = Transaction {
            tx_pow_nonce: 77,
            tx_pow_bits: 4,
            ..base.clone()
        };

        assert_eq!(base.txid(), variant.txid());
        assert_ne!(
            base.witness_commitment_hash(),
            variant.witness_commitment_hash()
        );
        assert_ne!(
            compact_short_id(base.witness_commitment_hash()),
            compact_short_id(variant.witness_commitment_hash())
        );
    }

    #[test]
    fn compact_block_rejects_excessive_tx_count_before_slot_allocation() {
        let message = CompactBlockMessage {
            header: BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: [1; 48],
                witness_root: [2; 48],
                founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
                founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
                timestamp: 1,
                difficulty_target_or_bits: [3; 48],
                nonce: 4,
            },
            tx_count: max_block_transaction_count() + 1,
            short_ids: Vec::new(),
            prefilled_transactions: vec![PrefilledTransaction {
                index: 0,
                transaction: Transaction {
                    version: 1,
                    inputs: vec![],
                    outputs: vec![],
                    lock_time: 0,
                    witness: vec![],
                    tx_pow_nonce: 0,
                    tx_pow_bits: 0,
                },
            }],
            fees_total_atoms: 0,
            fees_miner_atoms: 0,
        };
        assert_eq!(
            reconstruct_compact_block(&message, |_| None, &BTreeMap::new()),
            Err(ProtocolError::InvalidCompactBlock)
        );
    }

    #[test]
    fn getblocktxn_rejects_index_count_above_reasonable_block_bound() {
        let message = NetworkMessage::new(
            Network::Mainnet,
            MessagePayload::GetBlockTxn(GetBlockTxnMessage {
                block_hash: Hash48::ZERO,
                indexes: vec![0; max_block_transaction_count() + 1],
            }),
        );
        assert_eq!(
            message.encode_payload(),
            Err(ProtocolError::InvalidCompactBlock)
        );
    }
}
