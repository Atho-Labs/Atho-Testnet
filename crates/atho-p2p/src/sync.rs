#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncState {
    pub best_height: u64,
    pub headers_synced: bool,
}

impl SyncState {
    pub fn advance(&mut self, height: u64) {
        self.best_height = self.best_height.max(height);
        crate::audit::append_log("p2p", &format!("sync advanced best_height={}", self.best_height));
    }

    pub fn mark_headers_synced(&mut self) {
        self.headers_synced = true;
        crate::audit::append_log("p2p", "headers synced");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_state_advances_and_marks_synced() {
        let mut state = SyncState::default();
        state.advance(10);
        state.advance(7);
        state.mark_headers_synced();
        assert_eq!(state.best_height, 10);
        assert!(state.headers_synced);
    }
}
