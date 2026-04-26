use atho_wallet::snapshot::WalletSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiState {
    pub wallet_snapshot: WalletSnapshot,
    pub connected: bool,
    pub generate_coins: bool,
    pub mining_cores: u32,
}

impl UiState {
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    pub fn set_generate_coins(&mut self, enabled: bool) {
        self.generate_coins = enabled;
    }

    pub fn set_mining_cores(&mut self, cores: u32) {
        self.mining_cores = cores;
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
