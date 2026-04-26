use crate::protocol::{network_from_byte, network_to_byte, Message, MessageType};
use thiserror::Error;

const MAGIC: &[u8; 4] = b"ATHO";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("message too short")]
    MessageTooShort,
    #[error("invalid magic")]
    InvalidMagic,
    #[error("unknown message type")]
    UnknownMessageType,
    #[error("unsupported network")]
    UnsupportedNetwork,
    #[error("payload too large")]
    PayloadTooLarge,
}

#[derive(Debug, Default)]
pub struct WireCodec;

impl WireCodec {
    pub fn encode(message: &Message) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(message.message_type() as u8);
        out.push(network_to_byte(message.network()));
        let payload = message.payload();
        let len = u32::try_from(payload.len()).map_err(|_| CodecError::PayloadTooLarge)?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(payload);
        crate::audit::append_log(
            "p2p",
            &format!(
                "encoded message type={} network={}",
                u8::from(message.message_type()),
                message.network().id()
            ),
        );
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Message, CodecError> {
        if bytes.len() < 10 {
            return Err(CodecError::MessageTooShort);
        }
        if &bytes[..4] != MAGIC {
            return Err(CodecError::InvalidMagic);
        }
        let message_type =
            MessageType::try_from(bytes[4]).map_err(|_| CodecError::UnknownMessageType)?;
        let network = network_from_byte(bytes[5]).map_err(|_| CodecError::UnsupportedNetwork)?;
        let payload_len =
            u32::from_le_bytes(bytes[6..10].try_into().expect("slice length")) as usize;
        if bytes.len() < 10 + payload_len {
            return Err(CodecError::MessageTooShort);
        }
        let payload = bytes[10..10 + payload_len].to_vec();
        crate::audit::append_log(
            "p2p",
            &format!(
                "decoded message type={} network={}",
                u8::from(message_type),
                network.id()
            ),
        );
        Ok(Message::new(network, message_type, payload))
    }
}
