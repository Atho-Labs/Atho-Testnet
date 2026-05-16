// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Node-layer errors that wrap runtime, storage, validation, and RPC failures.
use crate::runtime::RuntimeError;
use crate::validation::ValidationError;
use atho_errors::{AthoErrorDescriptor, AthoErrorMeta};
use atho_p2p::connection::ConnectionError;
use atho_p2p::protocol::ProtocolError;
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
    #[error(transparent)]
    P2pConnection(#[from] ConnectionError),
    #[error(transparent)]
    P2pProtocol(#[from] ProtocolError),
}

impl AthoErrorMeta for NodeError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Runtime(error) => error.descriptor(),
            Self::Validation(error) => error.descriptor(),
            Self::Storage(error) => error.descriptor(),
            Self::P2pConnection(error) => error.descriptor(),
            Self::P2pProtocol(error) => error.descriptor(),
        }
    }

    fn source_module(&self) -> &'static str {
        match self {
            Self::Runtime(error) => error.source_module(),
            Self::Validation(error) => error.source_module(),
            Self::Storage(error) => error.source_module(),
            Self::P2pConnection(error) => error.source_module(),
            Self::P2pProtocol(error) => error.source_module(),
        }
    }

    fn safe_details(&self) -> Option<String> {
        match self {
            Self::Runtime(error) => error.safe_details(),
            Self::Validation(error) => error.safe_details(),
            Self::Storage(error) => error.safe_details(),
            Self::P2pConnection(error) => error.safe_details(),
            Self::P2pProtocol(error) => error.safe_details(),
        }
    }
}

pub fn rpc_error_from_node(error: NodeError) -> RpcError {
    RpcError::from_atho_error(error.to_atho_error())
}
