// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_core::network::Network;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageConfig {
    pub network: Network,
}
