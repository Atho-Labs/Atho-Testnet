//! Wallet address-book records and label tracking.
use crate::hd::{AddressKind, DerivationPath};
use atho_core::network::Network;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressRecord {
    pub network: Network,
    pub path: DerivationPath,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AddressBook {
    entries: Vec<AddressRecord>,
}

impl AddressBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, network: Network, path: DerivationPath, label: Option<String>) {
        self.entries.push(AddressRecord {
            network,
            path,
            label,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn last(&self) -> Option<&AddressRecord> {
        self.entries.last()
    }

    pub fn count_kind(&self, kind: AddressKind) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.path.kind == kind)
            .count()
    }

    pub fn snapshot(&self) -> Vec<AddressRecord> {
        self.entries.clone()
    }

    pub fn from_records(entries: Vec<AddressRecord>) -> Self {
        Self { entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hd::{AddressKind, DerivationPath};

    #[test]
    fn address_book_records_paths_by_network() {
        let mut book = AddressBook::new();
        book.record(
            Network::Mainnet,
            DerivationPath {
                account: 0,
                kind: AddressKind::Receive,
                index: 0,
            },
            Some(String::from("primary")),
        );

        assert_eq!(book.len(), 1);
        assert_eq!(book.count_kind(AddressKind::Receive), 1);
        assert_eq!(
            book.last().and_then(|r| r.label.as_deref()),
            Some("primary")
        );
    }
}
