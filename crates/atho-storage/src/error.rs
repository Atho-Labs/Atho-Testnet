use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Validation(#[from] crate::validation::ValidationError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("lmdb error: {0}")]
    Lmdb(#[from] lmdb::Error),
    #[error("database path unavailable")]
    PathUnavailable,
    #[error("corrupt storage data")]
    CorruptData,
    #[error("persisted genesis state is inconsistent")]
    PersistedGenesisMismatch,
    #[error("persisted tip header does not match stored tip state")]
    PersistedTipMismatch,
    #[error("persisted block history is incomplete")]
    IncompleteBlockHistory,
    #[error("missing utxo")]
    MissingUtxo,
    #[error("utxo already exists")]
    DuplicateUtxo,
    #[error("cross-network replay detected")]
    CrossNetworkReplay,
    #[error("legacy multi-environment storage layout detected")]
    LegacyStorageLayout,
    #[error("storage schema version mismatch: expected {expected}, found {found}")]
    SchemaVersionMismatch { expected: u32, found: u32 },
    #[error("no block to disconnect")]
    NoBlockToDisconnect,
    #[error("branch has no blocks")]
    EmptyBranch,
    #[error("branch fork point is unavailable in retained history")]
    ForkPointUnavailable,
    #[error("invalid branch sequence")]
    InvalidBranchSequence,
    #[error("candidate branch is not preferred over the current chain")]
    BranchNotPreferred,
    #[error("failed to restore canonical state during rollback")]
    RollbackFailure,
}

impl StorageError {
    pub fn is_recoverable_local_state(&self) -> bool {
        matches!(
            self,
            StorageError::CorruptData
                | StorageError::PersistedGenesisMismatch
                | StorageError::PersistedTipMismatch
                | StorageError::IncompleteBlockHistory
                | StorageError::LegacyStorageLayout
                | StorageError::SchemaVersionMismatch { .. }
        )
    }
}
