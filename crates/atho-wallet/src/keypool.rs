use crate::hd::{AddressKind, DerivationPath, HdWallet};
use std::collections::VecDeque;
use serde::{Deserialize, Serialize};

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
            self.receive.push_back(wallet.next_path(AddressKind::Receive));
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

    pub fn from_snapshot(receive: Vec<DerivationPath>, change: Vec<DerivationPath>) -> Self {
        Self {
            receive: receive.into_iter().collect(),
            change: change.into_iter().collect(),
        }
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
}
