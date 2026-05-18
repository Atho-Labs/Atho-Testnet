// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Error types returned by the Atho cryptography support crate.

use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, SIG_BACKEND_UNAVAILABLE, SIG_CRYPTO_OPERATION_FAILED,
    SIG_INVALID_KEY_LENGTH,
};
use thiserror::Error;

/// Stable high-level cryptography failures exposed to callers.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("crypto backend unavailable")]
    BackendUnavailable,
    #[error("invalid key length")]
    InvalidKeyLength,
    #[error("crypto operation failed")]
    OperationFailed,
}

impl AthoErrorMeta for CryptoError {
    /// Maps each crypto error to its canonical registry descriptor.
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::BackendUnavailable => &SIG_BACKEND_UNAVAILABLE,
            Self::InvalidKeyLength => &SIG_INVALID_KEY_LENGTH,
            Self::OperationFailed => &SIG_CRYPTO_OPERATION_FAILED,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-crypto"
    }
}
