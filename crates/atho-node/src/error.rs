use crate::runtime::RuntimeError;
use crate::validation::ValidationError;
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
