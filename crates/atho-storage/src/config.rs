// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Minimal storage-layer configuration shared by the persistence backends.

use atho_core::network::Network;

/// Selects which network namespace the storage backend should serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageConfig {
    pub network: Network,
}
