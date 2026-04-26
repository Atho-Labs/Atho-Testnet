use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WalletSnapshot {
    pub receive_count: u32,
    pub change_count: u32,
}

impl WalletSnapshot {
    pub fn record_receive(&mut self) {
        self.receive_count = self.receive_count.saturating_add(1);
    }

    pub fn record_change(&mut self) {
        self.change_count = self.change_count.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_tracks_counts() {
        let mut snapshot = WalletSnapshot::default();
        snapshot.record_receive();
        snapshot.record_change();
        assert_eq!(snapshot.receive_count, 1);
        assert_eq!(snapshot.change_count, 1);
    }
}
