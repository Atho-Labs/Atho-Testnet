use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error")]
    Io(#[from] std::io::Error),
    #[error("database path unavailable")]
    PathUnavailable,
    #[error("corrupt storage data")]
    CorruptData,
    #[error("missing utxo")]
    MissingUtxo,
    #[error("utxo already exists")]
    DuplicateUtxo,
    #[error("cross-network replay detected")]
    CrossNetworkReplay,
    #[error("no block to disconnect")]
    NoBlockToDisconnect,
}
