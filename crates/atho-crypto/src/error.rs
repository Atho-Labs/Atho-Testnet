use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("crypto backend unavailable")]
    BackendUnavailable,
    #[error("invalid key length")]
    InvalidKeyLength,
    #[error("crypto operation failed")]
    OperationFailed,
}
