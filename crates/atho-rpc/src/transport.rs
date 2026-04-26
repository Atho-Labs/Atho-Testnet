use crate::request::RpcRequest;
use crate::response::RpcResponse;
use serde::{de::DeserializeOwned, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RpcTransportError {
    #[error("rpc transport io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("rpc transport serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("rpc transport returned an empty response")]
    EmptyResponse,
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
        let mut stream = TcpStream::connect(&self.address)?;
        stream.set_nodelay(true)?;
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
    let mut line = String::new();
    let bytes = reader.read_line(&mut line)?;
    if bytes == 0 {
        return Err(RpcTransportError::EmptyResponse);
    }
    Ok(serde_json::from_str(line.trim_end())?)
}
