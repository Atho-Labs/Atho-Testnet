use thiserror::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum RpcError {
    #[error("rpc method not found")]
    MethodNotFound,
    #[error("invalid rpc request")]
    InvalidRequest,
    #[error("internal rpc error")]
    Internal,
}
