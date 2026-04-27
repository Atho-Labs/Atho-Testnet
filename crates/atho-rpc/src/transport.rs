use crate::request::RpcRequest;
use crate::response::RpcResponse;
use serde::{de::DeserializeOwned, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use thiserror::Error;

const MAX_RPC_MESSAGE_BYTES: usize = 1 << 20;
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum RpcTransportError {
    #[error("rpc transport io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("rpc transport serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("rpc transport returned an empty response")]
    EmptyResponse,
    #[error("rpc transport message exceeded the allowed size")]
    MessageTooLarge,
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
    let encoded = serde_json::to_string(message)?;
    writer.write_all(encoded.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn read_message<R, T>(reader: &mut R) -> Result<T, RpcTransportError>
where
    R: BufRead,
    T: DeserializeOwned,
{
    let mut line = Vec::new();

    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }

        if let Some(newline_index) = buffer.iter().position(|byte| *byte == b'\n') {
            line.extend_from_slice(&buffer[..newline_index]);
            reader.consume(newline_index + 1);
            break;
        }

        line.extend_from_slice(buffer);
        if line.len() > MAX_RPC_MESSAGE_BYTES {
            return Err(RpcTransportError::MessageTooLarge);
        }
        let consumed = buffer.len();
        reader.consume(consumed);
    }

    if line.is_empty() {
        return Err(RpcTransportError::EmptyResponse);
    }

    if line.len() > MAX_RPC_MESSAGE_BYTES {
        return Err(RpcTransportError::MessageTooLarge);
    }

    if line.last() == Some(&b'\r') {
        line.pop();
    }

    Ok(serde_json::from_slice(&line)?)
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

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct TestMessage {
        value: String,
    }

    #[test]
    fn read_message_parses_json_lines() {
        let payload = br#"{"value":"atho"}"#;
        let mut reader = BufReader::new(&payload[..]);
        let message: TestMessage = read_message(&mut reader).unwrap();
        assert_eq!(
            message,
            TestMessage {
                value: String::from("atho")
            }
        );
    }

    #[test]
    fn read_message_rejects_oversized_lines() {
        let payload = vec![b'a'; MAX_RPC_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(&payload[..]);
        let err = read_message::<_, TestMessage>(&mut reader).unwrap_err();
        assert!(matches!(err, RpcTransportError::MessageTooLarge));
    }
}
