// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, SIG_BACKEND_UNAVAILABLE, SIG_CRYPTO_OPERATION_FAILED,
    SIG_INVALID_KEY_LENGTH,
};
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

impl AthoErrorMeta for CryptoError {
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
