// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Lightweight UI state shared across desktop app panels.

use atho_node::mining_backend::MiningBackendKind;
use atho_wallet::snapshot::WalletSnapshot;

/// Mutable UI-facing state that mirrors node and wallet status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiState {
    pub wallet_snapshot: WalletSnapshot,
    pub connected: bool,
    pub generate_coins: bool,
    pub mining_cores: u32,
    pub mining_backend: MiningBackendKind,
    pub rotate_coinbase_address: bool,
}

impl UiState {
    /// Updates whether the UI considers the node connection live.
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    /// Updates whether the continuous mining toggle is enabled.
    pub fn set_generate_coins(&mut self, enabled: bool) {
        self.generate_coins = enabled;
    }

    /// Updates the preferred CPU mining core count shown in the UI.
    pub fn set_mining_cores(&mut self, cores: u32) {
        self.mining_cores = cores;
    }

    /// Updates the selected mining backend in the UI model.
    pub fn set_mining_backend(&mut self, backend: MiningBackendKind) {
        self.mining_backend = backend;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_state_tracks_connection_status() {
        let mut state = UiState::default();
        state.set_connected(true);
        assert!(state.connected);
    }
}
