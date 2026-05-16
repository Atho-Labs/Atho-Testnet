// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Length-delimited RPC transport framing.
use crate::request::RpcRequest;
use crate::response::RpcResponse;
use atho_core::constants::MAX_BLOCK_SERIALIZED_BYTES;
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, RPC_EMPTY_RESPONSE, RPC_MESSAGE_TOO_LARGE,
    RPC_SERIALIZATION, RPC_TRANSPORT_IO,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use thiserror::Error;

// Internal RPC must be able to move full block templates and submitted blocks
// without tripping a transport limit under heavy mempool load. Keep the cap
// bounded but above the maximum serialized block size with headroom for framing.
const MAX_RPC_MESSAGE_BYTES: usize = MAX_BLOCK_SERIALIZED_BYTES + (4 << 20);
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum RpcTransportError {
    #[error("rpc transport io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("rpc transport serialization error: {0}")]
    Serialization(String),
    #[error("rpc transport returned an empty response")]
    EmptyResponse,
    #[error("rpc transport message exceeded the allowed size")]
    MessageTooLarge,
}

impl AthoErrorMeta for RpcTransportError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Io(_) => &RPC_TRANSPORT_IO,
            Self::Serialization(_) => &RPC_SERIALIZATION,
            Self::EmptyResponse => &RPC_EMPTY_RESPONSE,
            Self::MessageTooLarge => &RPC_MESSAGE_TOO_LARGE,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-rpc::transport"
    }

    fn safe_details(&self) -> Option<String> {
        match self {
            Self::Io(error) => Some(error.to_string()),
            Self::Serialization(error) => Some(error.clone()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RpcClient {
    address: String,
}

impl RpcClient {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
        }
    }

    pub fn call(&self, request: &RpcRequest) -> Result<RpcResponse, RpcTransportError> {
        let mut stream = connect_stream(&self.address)?;
        write_message(&mut stream, request)?;
        let mut reader = BufReader::new(stream);
        let response: RpcResponse = read_message(&mut reader)?;
        Ok(response)
    }
}

pub fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), RpcTransportError>
where
    W: Write,
    T: Serialize,
{
    let encoded = serde_json::to_vec(message)
        .map_err(|err| RpcTransportError::Serialization(err.to_string()))?;
    if encoded.len() > MAX_RPC_MESSAGE_BYTES {
        return Err(RpcTransportError::MessageTooLarge);
    }
    let payload_len =
        u32::try_from(encoded.len()).map_err(|_| RpcTransportError::MessageTooLarge)?;
    writer.write_all(&payload_len.to_le_bytes())?;
    writer.write_all(&encoded)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message<R, T>(reader: &mut R) -> Result<T, RpcTransportError>
where
    R: Read,
    T: DeserializeOwned,
{
    let mut length_prefix = [0u8; 4];
    match reader.read_exact(&mut length_prefix) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
            return Err(RpcTransportError::EmptyResponse)
        }
        Err(err) => return Err(err.into()),
    }
    let payload_len = u32::from_le_bytes(length_prefix) as usize;
    if payload_len == 0 {
        return Err(RpcTransportError::EmptyResponse);
    }
    if payload_len > MAX_RPC_MESSAGE_BYTES {
        return Err(RpcTransportError::MessageTooLarge);
    }
    let mut payload = vec![0u8; payload_len];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map_err(|err| RpcTransportError::Serialization(err.to_string()))
}

fn connect_stream(address: &str) -> Result<TcpStream, RpcTransportError> {
    let mut last_error = None;
    for socket_addr in address.to_socket_addrs()? {
        match TcpStream::connect_timeout(&socket_addr, RPC_CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(RPC_IO_TIMEOUT))?;
                stream.set_write_timeout(Some(RPC_IO_TIMEOUT))?;
                return Ok(stream);
            }
            Err(err) => last_error = Some(err),
        }
    }

    match last_error {
        Some(err) => Err(err.into()),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rpc address did not resolve to any socket address",
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde::Serialize;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct TestMessage {
        value: String,
    }

    #[test]
    fn read_message_parses_length_prefixed_json_frames() {
        let mut payload = Vec::new();
        write_message(
            &mut payload,
            &TestMessage {
                value: String::from("atho"),
            },
        )
        .expect("encode");
        let mut reader = BufReader::new(&payload[..]);
        let message: TestMessage = read_message(&mut reader).expect("decode");
        assert_eq!(
            message,
            TestMessage {
                value: String::from("atho")
            }
        );
    }

    #[test]
    fn read_message_rejects_oversized_frames() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&((MAX_RPC_MESSAGE_BYTES + 1) as u32).to_le_bytes());
        let mut reader = BufReader::new(&payload[..]);
        let err = read_message::<_, TestMessage>(&mut reader).unwrap_err();
        assert!(matches!(err, RpcTransportError::MessageTooLarge));
    }

    #[test]
    fn read_message_rejects_invalid_json_payloads() {
        let payload = [4u8, 0, 0, 0, 1, 2, 3, 4];
        let mut reader = BufReader::new(&payload[..]);
        let err = read_message::<_, TestMessage>(&mut reader).unwrap_err();
        assert!(matches!(err, RpcTransportError::Serialization(_)));
    }

    #[test]
    fn transport_accepts_messages_larger_than_one_mebibyte() {
        let message = TestMessage {
            value: "a".repeat((1 << 20) + 128),
        };
        let mut payload = Vec::new();
        write_message(&mut payload, &message).expect("encode large payload");
        let mut reader = BufReader::new(&payload[..]);
        let decoded: TestMessage = read_message(&mut reader).expect("decode large payload");
        assert_eq!(decoded, message);
    }
}
