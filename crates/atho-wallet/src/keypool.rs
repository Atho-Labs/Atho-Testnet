use crate::hd::{AddressKind, DerivationPath, HdWallet};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

// Keep the default pool large enough to feel instant during normal receive/change use without
// turning first-run wallet creation and datafile serialization into a heavy startup spike.
pub const KEYPOOL_TARGET_SIZE: usize = 1_024;
const PREFILL_PROGRESS_BATCH: usize = 64;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keypool {
    receive: VecDeque<DerivationPath>,
    change: VecDeque<DerivationPath>,
}

impl Keypool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn refill(&mut self, wallet: &mut HdWallet, receive_target: usize, change_target: usize) {
        while self.receive.len() < receive_target {
            self.receive
                .push_back(wallet.next_path(AddressKind::Receive));
        }
        while self.change.len() < change_target {
            self.change.push_back(wallet.next_path(AddressKind::Change));
        }
    }

    pub fn take_receive(&mut self) -> Option<DerivationPath> {
        self.receive.pop_front()
    }

    pub fn take_change(&mut self) -> Option<DerivationPath> {
        self.change.pop_front()
    }

    pub fn snapshot(&self) -> (Vec<DerivationPath>, Vec<DerivationPath>) {
        (
            self.receive.iter().copied().collect(),
            self.change.iter().copied().collect(),
        )
    }

    pub fn receive_len(&self) -> usize {
        self.receive.len()
    }

    pub fn change_len(&self) -> usize {
        self.change.len()
    }

    pub fn highest_receive_index(&self) -> Option<u32> {
        self.receive.back().map(|path| path.index)
    }

    pub fn highest_change_index(&self) -> Option<u32> {
        self.change.back().map(|path| path.index)
    }

    pub fn from_snapshot(receive: Vec<DerivationPath>, change: Vec<DerivationPath>) -> Self {
        Self {
            receive: receive.into_iter().collect(),
            change: change.into_iter().collect(),
        }
    }

    pub fn refill_to_target(&mut self, wallet: &mut HdWallet) {
        self.refill_to_target_with_progress(wallet, |_, _| {});
    }

    pub fn refill_to_target_with_progress<F>(&mut self, wallet: &mut HdWallet, mut progress: F)
    where
        F: FnMut(usize, usize),
    {
        let receive_needed = KEYPOOL_TARGET_SIZE.saturating_sub(self.receive.len());
        let change_needed = KEYPOOL_TARGET_SIZE.saturating_sub(self.change.len());
        let total = receive_needed.saturating_add(change_needed);
        if total == 0 {
            progress(0, 0);
            return;
        }

        let mut completed = 0usize;
        progress(0, total);

        while self.receive.len() < KEYPOOL_TARGET_SIZE {
            self.receive
                .push_back(wallet.next_path(AddressKind::Receive));
            completed = completed.saturating_add(1);
            if completed % PREFILL_PROGRESS_BATCH == 0 || completed == total {
                progress(completed, total);
            }
        }
        while self.change.len() < KEYPOOL_TARGET_SIZE {
            self.change.push_back(wallet.next_path(AddressKind::Change));
            completed = completed.saturating_add(1);
            if completed % PREFILL_PROGRESS_BATCH == 0 || completed == total {
                progress(completed, total);
            }
        }

        progress(total, total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hd::{HdWallet, WalletSeed};

    #[test]
    fn keypool_refills_receive_and_change_slots() {
        let mut wallet = HdWallet::new(WalletSeed([9; 32]));
        let mut pool = Keypool::new();

        pool.refill(&mut wallet, 2, 1);

        assert_eq!(pool.take_receive().map(|p| p.index), Some(0));
        assert_eq!(pool.take_receive().map(|p| p.index), Some(1));
        assert_eq!(pool.take_change().map(|p| p.index), Some(0));
        assert!(pool.take_receive().is_none());
    }

    #[test]
    fn keypool_refill_to_target_reports_completion() {
        let mut wallet = HdWallet::new(WalletSeed([11; 32]));
        let mut pool = Keypool::new();
        let mut updates = Vec::new();

        pool.refill_to_target_with_progress(&mut wallet, |completed, total| {
            updates.push((completed, total));
        });

        assert!(!updates.is_empty());
        assert_eq!(
            updates.last().copied(),
            Some((KEYPOOL_TARGET_SIZE * 2, KEYPOOL_TARGET_SIZE * 2))
        );
    }
}
