use crate::crypto::hash::sha3_384;
use crate::encoding::{compact_size_len, write_compact_size};
use crate::transaction::{Transaction, TxWitness};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u16,
    pub network_id: crate::network::Network,
    pub height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub previous_block_hash: [u8; 48],
    #[serde(with = "serde_big_array::BigArray")]
    pub merkle_root: [u8; 48],
    #[serde(with = "serde_big_array::BigArray")]
    pub witness_root: [u8; 48],
    pub timestamp: u64,
    #[serde(with = "serde_big_array::BigArray")]
    pub difficulty_target_or_bits: [u8; 48],
    pub nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    #[serde(skip, default)]
    pub witnesses: BTreeMap<[u8; 48], TxWitness>,
    pub fees_total_atoms: u64,
    pub fees_miner_atoms: u64,
    pub fees_burned_atoms: u64,
    pub fees_pool_atoms: u64,
    pub cumulative_burned_atoms: u64,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            header: BlockHeader {
                version: 0,
                network_id: crate::network::Network::Mainnet,
                height: 0,
                previous_block_hash: [0; 48],
                merkle_root: [0; 48],
                witness_root: [0; 48],
                timestamp: 0,
                difficulty_target_or_bits: [0; 48],
                nonce: 0,
            },
            transactions: Vec::new(),
            witnesses: BTreeMap::new(),
            fees_total_atoms: 0,
            fees_miner_atoms: 0,
            fees_burned_atoms: 0,
            fees_pool_atoms: 0,
            cumulative_burned_atoms: 0,
        }
    }
}

impl BlockHeader {
    pub fn canonical_size_bytes_without_nonce(&self) -> usize {
        2 + 1 + 8 + 48 + 48 + 48 + 8 + 48
    }

    pub fn canonical_size_bytes(&self) -> usize {
        self.canonical_size_bytes_without_nonce() + 8
    }

    pub fn canonical_bytes_without_nonce(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.canonical_size_bytes_without_nonce());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.push(self.network_id.consensus_id());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.previous_block_hash);
        out.extend_from_slice(&self.merkle_root);
        out.extend_from_slice(&self.witness_root);
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out.extend_from_slice(&self.difficulty_target_or_bits);
        out
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.canonical_size_bytes());
        out.extend_from_slice(&self.canonical_bytes_without_nonce());
        out.extend_from_slice(&self.nonce.to_le_bytes());
        out
    }

    pub fn block_hash(&self) -> [u8; 48] {
        sha3_384(&self.canonical_bytes())
    }
}

impl Block {
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        let mut block = Self {
            header,
            ..Self::default()
        };
        block.transactions = transactions;
        block.witnesses = block
            .transactions
            .iter()
            .filter_map(|tx| tx.witness_payload().map(|witness| (tx.txid(), witness)))
            .collect();
        block
    }

    pub fn base_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.base_size_bytes());
        out.extend_from_slice(&self.header.canonical_bytes());
        out.extend_from_slice(&(self.transactions.len() as u32).to_le_bytes());
        for tx in &self.transactions {
            let tx_bytes = tx.base_bytes();
            out.extend_from_slice(&(tx_bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&tx_bytes);
        }
        out
    }

    pub fn full_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.full_size_bytes());
        out.extend_from_slice(&self.header.canonical_bytes());
        out.extend_from_slice(&(self.transactions.len() as u32).to_le_bytes());
        for tx in &self.transactions {
            let tx_bytes = tx.full_bytes();
            out.extend_from_slice(&(tx_bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&tx_bytes);
        }
        out
    }

    pub fn compact_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.compact_size_bytes());
        out.extend_from_slice(&self.header.canonical_bytes());
        write_compact_size(&mut out, self.transactions.len());
        for tx in &self.transactions {
            let tx_bytes = tx.compact_bytes();
            write_compact_size(&mut out, tx_bytes.len());
            out.extend_from_slice(&tx_bytes);
        }
        out
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.full_bytes()
    }

    pub fn base_size_bytes(&self) -> usize {
        self.header.canonical_size_bytes()
            + 4
            + self
                .transactions
                .iter()
                .map(|tx| 4 + tx.base_size_bytes())
                .sum::<usize>()
    }

    pub fn full_size_bytes(&self) -> usize {
        self.header.canonical_size_bytes()
            + 4
            + self
                .transactions
                .iter()
                .map(|tx| 4 + tx.full_size_bytes())
                .sum::<usize>()
    }

    pub fn size_bytes(&self) -> usize {
        self.full_size_bytes()
    }

    pub fn witness_bytes(&self) -> usize {
        self.witnesses
            .values()
            .map(|witness| witness.canonical_bytes().len())
            .sum()
    }

    pub fn compute_witness_root(&self) -> [u8; 48] {
        witness_root(&self.transactions)
    }

    pub fn compute_witness_commitment(&self) -> [u8; 48] {
        self.compute_witness_root()
    }

    pub fn weight_bytes(&self) -> usize {
        let base = self.base_size_bytes();
        let total = self.full_size_bytes();
        base.saturating_mul(3).saturating_add(total)
    }

    pub fn vsize_bytes(&self) -> usize {
        (self.weight_bytes().saturating_add(3)) / 4
    }

    pub fn compact_size_bytes(&self) -> usize {
        let mut total =
            self.header.canonical_size_bytes() + compact_size_len(self.transactions.len());
        for tx in &self.transactions {
            let tx_size = tx.compact_size_bytes();
            total = total.saturating_add(compact_size_len(tx_size));
            total = total.saturating_add(tx_size);
        }
        total
    }

    pub fn merkle_root(&self) -> [u8; 48] {
        merkle_root(&self.transactions)
    }
}

pub fn merkle_root(transactions: &[Transaction]) -> [u8; 48] {
    if transactions.is_empty() {
        return [0; 48];
    }

    let mut layer: Vec<[u8; 48]> = transactions.iter().map(Transaction::txid).collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity((layer.len() + 1) / 2);
        for chunk in layer.chunks(2) {
            let mut bytes = [0u8; 96];
            bytes[..48].copy_from_slice(&chunk[0]);
            bytes[48..].copy_from_slice(chunk.get(1).unwrap_or(&chunk[0]));
            next.push(sha3_384(&bytes));
        }
        layer = next;
    }
    layer[0]
}

pub fn witness_root(transactions: &[Transaction]) -> [u8; 48] {
    if transactions.is_empty() {
        return [0; 48];
    }

    let mut layer: Vec<[u8; 48]> = transactions
        .iter()
        .map(Transaction::witness_commitment_hash)
        .collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity((layer.len() + 1) / 2);
        for chunk in layer.chunks(2) {
            let mut bytes = [0u8; 96];
            bytes[..48].copy_from_slice(&chunk[0]);
            bytes[48..].copy_from_slice(chunk.get(1).unwrap_or(&chunk[0]));
            next.push(sha3_384(&bytes));
        }
        layer = next;
    }
    layer[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_hash_is_stable_for_canonical_header() {
        let header = BlockHeader {
            version: 1,
            network_id: crate::network::Network::Mainnet,
            height: 1,
            previous_block_hash: [2; 48],
            merkle_root: [3; 48],
            witness_root: [4; 48],
            timestamp: 75,
            difficulty_target_or_bits: [5; 48],
            nonce: 42,
        };

        assert_eq!(header.block_hash(), header.block_hash());
        assert_eq!(
            header.canonical_bytes().len(),
            2 + 1 + 8 + 48 + 48 + 48 + 8 + 48 + 8
        );
    }

    #[test]
    fn merkle_root_is_deterministic() {
        let tx = Transaction {
            version: 1,
            inputs: vec![crate::transaction::TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![crate::transaction::TxOutput {
                value_atoms: 500,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };

        let root = merkle_root(&[tx.clone(), tx]);
        assert_ne!(root, [0; 48]);
    }

    #[test]
    fn compact_block_bytes_are_not_larger_than_full_bytes() {
        let tx = Transaction {
            version: 1,
            inputs: vec![crate::transaction::TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![crate::transaction::TxOutput {
                value_atoms: 500,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![9, 9, 9],
        };
        let header = BlockHeader {
            version: 1,
            network_id: crate::network::Network::Mainnet,
            height: 1,
            previous_block_hash: [2; 48],
            merkle_root: merkle_root(&[tx.clone()]),
            witness_root: witness_root(&[tx.clone()]),
            timestamp: 75,
            difficulty_target_or_bits: [5; 48],
            nonce: 42,
        };
        let block = Block::new(header, vec![tx]);

        assert!(block.compact_bytes().len() <= block.full_bytes().len());
    }
}
