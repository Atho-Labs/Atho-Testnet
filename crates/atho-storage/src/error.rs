// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Storage-layer errors and their `ATHO-*` registry mappings.
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, DB_BRANCH_NOT_PREFERRED, DB_CORRUPT_DATA,
    DB_CROSS_NETWORK_REPLAY, DB_EMPTY_BRANCH, DB_FORK_POINT_UNAVAILABLE,
    DB_INCOMPLETE_BLOCK_HISTORY, DB_INVALID_BRANCH_SEQUENCE, DB_IO, DB_LEGACY_STORAGE_LAYOUT,
    DB_LMDB, DB_NO_BLOCK_TO_DISCONNECT, DB_PATH_UNAVAILABLE, DB_PERSISTED_GENESIS_MISMATCH,
    DB_PERSISTED_TIP_MISMATCH, DB_ROLLBACK_FAILURE, DB_SCHEMA_VERSION_MISMATCH, UTXO_DUPLICATE,
    UTXO_MISSING,
};
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
    #[error("storage metadata mismatch: {field}")]
    StorageMetadataMismatch { field: &'static str },
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
    #[error("reorg depth {depth} exceeds max reorg depth {max_depth}")]
    ReorgTooDeep {
        depth: u64,
        max_depth: u64,
        fork_height: u64,
        current_height: u64,
    },
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
                | StorageError::StorageMetadataMismatch { .. }
        ) || matches!(
            self,
            StorageError::Io(error)
                if error.kind() == std::io::ErrorKind::UnexpectedEof
        )
    }
}

impl AthoErrorMeta for StorageError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Validation(error) => error.descriptor(),
            Self::Io(_) => &DB_IO,
            Self::Lmdb(_) => &DB_LMDB,
            Self::PathUnavailable => &DB_PATH_UNAVAILABLE,
            Self::CorruptData => &DB_CORRUPT_DATA,
            Self::PersistedGenesisMismatch => &DB_PERSISTED_GENESIS_MISMATCH,
            Self::PersistedTipMismatch => &DB_PERSISTED_TIP_MISMATCH,
            Self::IncompleteBlockHistory => &DB_INCOMPLETE_BLOCK_HISTORY,
            Self::MissingUtxo => &UTXO_MISSING,
            Self::DuplicateUtxo => &UTXO_DUPLICATE,
            Self::CrossNetworkReplay => &DB_CROSS_NETWORK_REPLAY,
            Self::LegacyStorageLayout => &DB_LEGACY_STORAGE_LAYOUT,
            Self::SchemaVersionMismatch { .. } => &DB_SCHEMA_VERSION_MISMATCH,
            Self::StorageMetadataMismatch { .. } => &DB_CORRUPT_DATA,
            Self::NoBlockToDisconnect => &DB_NO_BLOCK_TO_DISCONNECT,
            Self::EmptyBranch => &DB_EMPTY_BRANCH,
            Self::ForkPointUnavailable => &DB_FORK_POINT_UNAVAILABLE,
            Self::InvalidBranchSequence => &DB_INVALID_BRANCH_SEQUENCE,
            Self::BranchNotPreferred => &DB_BRANCH_NOT_PREFERRED,
            Self::ReorgTooDeep { .. } => &DB_BRANCH_NOT_PREFERRED,
            Self::RollbackFailure => &DB_ROLLBACK_FAILURE,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-storage::db"
    }

    fn safe_details(&self) -> Option<String> {
        match self {
            Self::SchemaVersionMismatch { expected, found } => {
                Some(format!("expected schema version {expected}, found {found}"))
            }
            Self::StorageMetadataMismatch { field } => {
                Some(format!("storage metadata field mismatch: {field}"))
            }
            Self::Io(error) => Some(error.to_string()),
            Self::Lmdb(error) => Some(error.to_string()),
            Self::ReorgTooDeep {
                depth,
                max_depth,
                fork_height,
                current_height,
            } => Some(format!(
                "reorg depth {depth} exceeds max {max_depth} at fork height {fork_height} from current height {current_height}"
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_errors::AthoErrorMeta;

    #[test]
    fn persisted_genesis_mismatch_has_fatal_database_code() {
        let error = StorageError::PersistedGenesisMismatch.to_atho_error();
        assert_eq!(error.code().as_str(), "ATHO-DB-005");
        assert_eq!(error.severity().as_str(), "fatal");
    }
}
