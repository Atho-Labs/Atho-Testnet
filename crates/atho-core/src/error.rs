// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Core protocol error types and their `ATHO-*` registry mappings.
//!
//! The errors in this module stay close to the protocol primitives so higher
//! layers can distinguish address parsing failures from transaction, block, and
//! consensus failures without relying on string matching.
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, ADDR_INVALID_ALPHABET, ADDR_INVALID_CHECKSUM,
    ADDR_INVALID_PREFIX, BLK_BLOCK_TOO_LARGE, BLK_EMPTY_BLOCK, BLK_MERKLE_ROOT_MISMATCH,
    CONS_INVALID_POW_TARGET, CONS_INVALID_SUBSIDY_SCHEDULE, TX_DUPLICATE_INPUT, TX_NO_INPUTS,
    TX_NO_OUTPUTS, TX_TOO_LARGE, TX_ZERO_VALUE_OUTPUT,
};
use thiserror::Error;

/// Address-decoding failures in the core protocol layer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AddressError {
    #[error("invalid base56 alphabet")]
    InvalidAlphabet,
    #[error("invalid visible prefix")]
    InvalidPrefix,
    #[error("invalid checksum")]
    InvalidChecksum,
}

/// Proof-of-work and subsidy schedule failures raised by the core layer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConsensusError {
    #[error("invalid subsidy schedule")]
    InvalidSubsidySchedule,
    #[error("invalid proof-of-work target")]
    InvalidProofOfWorkTarget,
}

/// Canonical transaction structure failures detected without chain context.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransactionError {
    #[error("transaction has no inputs")]
    NoInputs,
    #[error("transaction has no outputs")]
    NoOutputs,
    #[error("duplicate transaction input")]
    DuplicateInput,
    #[error("zero-value output")]
    ZeroValueOutput,
    #[error("transaction too large")]
    TooLarge,
}

/// Canonical block-structure failures detected without chain context.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlockError {
    #[error("block has no transactions")]
    EmptyBlock,
    #[error("block too large")]
    TooLarge,
    #[error("block merkle root mismatch")]
    MerkleRootMismatch,
}

/// Top-level wrapper for core-protocol failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error(transparent)]
    Address(#[from] AddressError),
    #[error(transparent)]
    Consensus(#[from] ConsensusError),
    #[error(transparent)]
    Transaction(#[from] TransactionError),
    #[error(transparent)]
    Block(#[from] BlockError),
}

impl AthoErrorMeta for AddressError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::InvalidAlphabet => &ADDR_INVALID_ALPHABET,
            Self::InvalidPrefix => &ADDR_INVALID_PREFIX,
            Self::InvalidChecksum => &ADDR_INVALID_CHECKSUM,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-core::address"
    }
}

impl AthoErrorMeta for ConsensusError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::InvalidSubsidySchedule => &CONS_INVALID_SUBSIDY_SCHEDULE,
            Self::InvalidProofOfWorkTarget => &CONS_INVALID_POW_TARGET,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-core::consensus"
    }
}

impl AthoErrorMeta for TransactionError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::NoInputs => &TX_NO_INPUTS,
            Self::NoOutputs => &TX_NO_OUTPUTS,
            Self::DuplicateInput => &TX_DUPLICATE_INPUT,
            Self::ZeroValueOutput => &TX_ZERO_VALUE_OUTPUT,
            Self::TooLarge => &TX_TOO_LARGE,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-core::transaction"
    }
}

impl AthoErrorMeta for BlockError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::EmptyBlock => &BLK_EMPTY_BLOCK,
            Self::TooLarge => &BLK_BLOCK_TOO_LARGE,
            Self::MerkleRootMismatch => &BLK_MERKLE_ROOT_MISMATCH,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-core::block"
    }
}

impl AthoErrorMeta for CoreError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Address(error) => error.descriptor(),
            Self::Consensus(error) => error.descriptor(),
            Self::Transaction(error) => error.descriptor(),
            Self::Block(error) => error.descriptor(),
        }
    }

    fn source_module(&self) -> &'static str {
        match self {
            Self::Address(error) => error.source_module(),
            Self::Consensus(error) => error.source_module(),
            Self::Transaction(error) => error.source_module(),
            Self::Block(error) => error.source_module(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_wraps_specific_protocol_errors() {
        let err = CoreError::from(AddressError::InvalidChecksum);
        assert!(matches!(
            err,
            CoreError::Address(AddressError::InvalidChecksum)
        ));
    }
}
