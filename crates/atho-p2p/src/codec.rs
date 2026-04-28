use crate::config::{network_from_magic, network_params};
use crate::protocol::{MessageCommand, NetworkMessage, ProtocolError};
use atho_core::crypto::hash::sha3_256;
use thiserror::Error;

const FRAME_HEADER_BYTES: usize = 24;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("message too short")]
    MessageTooShort,
    #[error("invalid network magic")]
    InvalidMagic,
    #[error("checksum mismatch")]
    ChecksumMismatch,
    #[error("payload too large")]
    PayloadTooLarge,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

#[derive(Debug, Default)]
pub struct WireCodec;

impl WireCodec {
    pub fn encode(message: &NetworkMessage) -> Result<Vec<u8>, CodecError> {
        let params = network_params(message.network);
        let payload = message.encode_payload()?;
        let payload_len = u32::try_from(payload.len()).map_err(|_| CodecError::PayloadTooLarge)?;
        if payload_len > params.limits.max_message_size {
            return Err(CodecError::PayloadTooLarge);
        }

        let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
        out.extend_from_slice(&params.magic);
        out.extend_from_slice(&message.command().as_padded_bytes());
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&payload_checksum(&payload));
        out.extend_from_slice(&payload);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<NetworkMessage, CodecError> {
        if bytes.len() < FRAME_HEADER_BYTES {
            return Err(CodecError::MessageTooShort);
        }
        let magic: [u8; 4] = bytes[..4].try_into().expect("slice length");
        let network = network_from_magic(magic).ok_or(CodecError::InvalidMagic)?;
        let command = MessageCommand::from_bytes(bytes[4..16].try_into().expect("slice length"))?;
        let payload_len =
            u32::from_le_bytes(bytes[16..20].try_into().expect("slice length")) as usize;
        if payload_len > network_params(network).limits.max_message_size as usize {
            return Err(CodecError::PayloadTooLarge);
        }
        if bytes.len() < FRAME_HEADER_BYTES + payload_len {
            return Err(CodecError::MessageTooShort);
        }
        let expected_checksum: [u8; 4] = bytes[20..24].try_into().expect("slice length");
        let payload = &bytes[24..24 + payload_len];
        if payload_checksum(payload) != expected_checksum {
            return Err(CodecError::ChecksumMismatch);
        }
        NetworkMessage::decode(network, command, payload).map_err(CodecError::Protocol)
    }
}

fn payload_checksum(payload: &[u8]) -> [u8; 4] {
    let digest = sha3_256(payload);
    [digest[0], digest[1], digest[2], digest[3]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Hash48, MessagePayload, VersionMessage, LOCAL_NODE_SERVICES};
    use atho_core::consensus::rules;
    use atho_core::genesis;
    use atho_core::network::Network;

    #[test]
    fn bitcoin_style_frame_round_trips_version_messages() {
        let message = NetworkMessage::new(
            Network::Mainnet,
            MessagePayload::Version(VersionMessage {
                protocol_version: rules::PROTOCOL_VERSION,
                min_protocol_version: crate::config::MIN_SUPPORTED_PROTOCOL_VERSION,
                services: LOCAL_NODE_SERVICES,
                timestamp_unix: 1_700_000_000,
                network: Network::Mainnet,
                user_agent: String::from("/Atho:0.1.0/"),
                best_height: 0,
                ruleset_version: rules::RULESET_VERSION_V1,
                relay: true,
                genesis_hash: Hash48::from(genesis::genesis_hash(Network::Mainnet)),
                tip_hash: Hash48::ZERO,
                chainwork: Hash48::ZERO,
            }),
        );
        let bytes = WireCodec::encode(&message).expect("encode");
        let decoded = WireCodec::decode(&bytes).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let message = NetworkMessage::new(Network::Regnet, MessagePayload::Ping { nonce: 42 });
        let mut bytes = WireCodec::encode(&message).expect("encode");
        *bytes.last_mut().expect("payload byte") ^= 0xff;
        assert_eq!(WireCodec::decode(&bytes), Err(CodecError::ChecksumMismatch));
    }
}
