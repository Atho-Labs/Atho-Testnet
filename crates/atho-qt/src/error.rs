// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! GUI-layer errors surfaced to desktop users.
use atho_errors::{AthoError, AthoErrorDescriptor, AthoErrorMeta, RPC_QT_UNEXPECTED};
use atho_rpc::error::RpcError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QtError {
    #[error("rpc error")]
    Rpc(#[from] RpcError),
    #[error("unexpected rpc error response")]
    UnexpectedRpcError,
}

impl AthoErrorMeta for QtError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Rpc(_) => &RPC_QT_UNEXPECTED,
            Self::UnexpectedRpcError => &RPC_QT_UNEXPECTED,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-qt"
    }

    fn to_atho_error(&self) -> AthoError {
        match self {
            Self::Rpc(error) => {
                let descriptor =
                    atho_errors::registry_descriptor(&error.code).unwrap_or(&RPC_QT_UNEXPECTED);
                let mut built = AthoError::new(descriptor, "atho-qt", error.message.clone());
                if let Some(details) = &error.details {
                    built = built.with_safe_details(details.clone());
                }
                built
            }
            Self::UnexpectedRpcError => AthoErrorMeta::to_atho_error(self),
        }
    }
}
