use crate::crypto::hash::sha3_384;
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
    #[serde(with = "serde_big_array::BigArray")]
    pub witness_root: [u8; 48],
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
            witness_root: [0; 48],
            fees_total_atoms: 0,
            fees_miner_atoms: 0,
            fees_burned_atoms: 0,
            fees_pool_atoms: 0,
            cumulative_burned_atoms: 0,
        }
    }
}

impl BlockHeader {
    pub fn canonical_bytes_without_nonce(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + 1 + 8 + 48 + 48 + 48 + 8 + 48);
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
        let mut out = self.canonical_bytes_without_nonce();
        out.extend_from_slice(&self.nonce.to_le_bytes());
        out
    }

    pub fn block_hash(&self) -> [u8; 48] {
        sha3_384(&self.canonical_bytes())
    }
}

impl Block {
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        let mut block = Self::default();
        block.header = header;
        block.transactions = transactions;
        block.witnesses = block
            .transactions
            .iter()
            .filter_map(|tx| tx.witness_payload().map(|witness| (tx.txid(), witness)))
            .collect();
        block.witness_root = block.compute_witness_root();
        block.header.witness_root = block.witness_root;
        block
    }

    pub fn base_bytes(&self) -> Vec<u8> {
        let mut out = self.header.canonical_bytes();
        out.extend_from_slice(&(self.transactions.len() as u32).to_le_bytes());
        for tx in &self.transactions {
            let tx_bytes = tx.base_bytes();
            out.extend_from_slice(&(tx_bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&tx_bytes);
        }
        out
    }

    pub fn full_bytes(&self) -> Vec<u8> {
        let mut out = self.header.canonical_bytes();
        out.extend_from_slice(&(self.transactions.len() as u32).to_le_bytes());
        for tx in &self.transactions {
            let tx_bytes = tx.full_bytes();
            out.extend_from_slice(&(tx_bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&tx_bytes);
        }
        out
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.full_bytes()
    }

    pub fn size_bytes(&self) -> usize {
        self.full_bytes().len()
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
        let base = self.base_bytes().len();
        let total = self.full_bytes().len();
        base.saturating_mul(3).saturating_add(total)
    }

    pub fn vsize_bytes(&self) -> usize {
        (self.weight_bytes().saturating_add(3)) / 4
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
            let mut bytes = Vec::with_capacity(96);
            bytes.extend_from_slice(&chunk[0]);
            if let Some(right) = chunk.get(1) {
                bytes.extend_from_slice(right);
            } else {
                bytes.extend_from_slice(&chunk[0]);
            }
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

    let mut layer: Vec<[u8; 48]> = transactions.iter().map(Transaction::wtxid).collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity((layer.len() + 1) / 2);
        for chunk in layer.chunks(2) {
            let mut bytes = Vec::with_capacity(96);
            bytes.extend_from_slice(&chunk[0]);
            if let Some(right) = chunk.get(1) {
                bytes.extend_from_slice(right);
            } else {
                bytes.extend_from_slice(&chunk[0]);
            }
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
}
