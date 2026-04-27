use crate::runtime::RuntimeError;
use crate::validation::ValidationError;
use atho_rpc::error::RpcError;
use atho_storage::error::StorageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

pub fn rpc_error_from_node(error: NodeError) -> RpcError {
    match error {
        NodeError::Validation(validation) => RpcError::Validation(validation.to_string()),
        NodeError::Runtime(_) | NodeError::Storage(_) => RpcError::Internal,
    }
}
