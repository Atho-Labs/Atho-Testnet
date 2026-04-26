use atho_rpc::error::RpcError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QtError {
    #[error("rpc error")]
    Rpc(#[from] RpcError),
    #[error("unexpected rpc error response")]
    UnexpectedRpcError,
}
