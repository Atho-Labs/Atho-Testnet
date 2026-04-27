use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum RpcError {
    #[error("rpc method not found")]
    MethodNotFound,
    #[error("invalid rpc request: {0}")]
    InvalidRequest(String),
    #[error("internal rpc error")]
    Internal,
    #[error("validation error: {0}")]
    Validation(String),
}
