use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum RpcError {
    #[error("rpc method not found")]
    MethodNotFound,
    #[error("invalid rpc request")]
    InvalidRequest,
    #[error("internal rpc error")]
    Internal,
}
