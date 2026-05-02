//! Deterministic HD derivation for Atho wallet seeds.
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressKind {
    Receive,
    Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationPath {
    pub account: u32,
    pub kind: AddressKind,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct WalletSeed(pub [u8; 32]);

#[derive(Debug, Clone)]
pub struct HdWallet {
    seed: WalletSeed,
    next_receive_index: u32,
    next_change_index: u32,
}

impl HdWallet {
    pub fn new(seed: WalletSeed) -> Self {
        Self {
            seed,
            next_receive_index: 0,
            next_change_index: 0,
        }
    }

    pub fn seed(&self) -> &[u8; 32] {
        &self.seed.0
    }

    pub fn counters(&self) -> (u32, u32) {
        (self.next_receive_index, self.next_change_index)
    }

    pub fn with_counters(
        seed: WalletSeed,
        next_receive_index: u32,
        next_change_index: u32,
    ) -> Self {
        Self {
            seed,
            next_receive_index,
            next_change_index,
        }
    }

    pub fn next_path(&mut self, kind: AddressKind) -> DerivationPath {
        match kind {
            AddressKind::Receive => {
                let index = self.next_receive_index;
                self.next_receive_index = self.next_receive_index.saturating_add(1);
                DerivationPath {
                    account: 0,
                    kind,
                    index,
                }
            }
            AddressKind::Change => {
                let index = self.next_change_index;
                self.next_change_index = self.next_change_index.saturating_add(1);
                DerivationPath {
                    account: 0,
                    kind,
                    index,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hd_wallet_tracks_independent_paths() {
        let seed = WalletSeed([7; 32]);
        let mut wallet = HdWallet::new(seed);

        let receive0 = wallet.next_path(AddressKind::Receive);
        let receive1 = wallet.next_path(AddressKind::Receive);
        let change0 = wallet.next_path(AddressKind::Change);

        assert_eq!(receive0.index, 0);
        assert_eq!(receive1.index, 1);
        assert_eq!(change0.index, 0);
        assert_eq!(wallet.seed(), &[7; 32]);
    }
}
