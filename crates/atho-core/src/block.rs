use crate::crypto::hash::sha3_384;
use crate::transaction::{Transaction, TxWitness};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    pub version: u16,
    pub previous_block_hash: [u8; 48],
    pub merkle_root: [u8; 48],
    pub timestamp: u64,
    pub target: [u8; 48],
    pub nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub witnesses: BTreeMap<[u8; 48], TxWitness>,
    pub witness_commitment: [u8; 48],
    pub state_root: [u8; 48],
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
                previous_block_hash: [0; 48],
                merkle_root: [0; 48],
                timestamp: 0,
                target: [0; 48],
                nonce: 0,
            },
            transactions: Vec::new(),
            witnesses: BTreeMap::new(),
            witness_commitment: [0; 48],
            state_root: [0; 48],
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
        let mut out = Vec::with_capacity(2 + 48 + 48 + 8 + 48);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.previous_block_hash);
        out.extend_from_slice(&self.merkle_root);
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out.extend_from_slice(&self.target);
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
        block.witness_commitment = block.compute_witness_commitment();
        block
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = self.header.canonical_bytes();
        out.extend_from_slice(&(self.transactions.len() as u32).to_le_bytes());
        for tx in &self.transactions {
            let tx_bytes = tx.canonical_bytes();
            out.extend_from_slice(&(tx_bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&tx_bytes);
        }
        out
    }

    pub fn size_bytes(&self) -> usize {
        self.canonical_bytes().len()
    }

    pub fn witness_bytes(&self) -> usize {
        self.witnesses
            .values()
            .map(|witness| witness.canonical_bytes().len())
            .sum()
    }

    pub fn compute_witness_commitment(&self) -> [u8; 48] {
        let mut layer: Vec<[u8; 48]> = self.transactions.iter().map(Transaction::wtxid).collect();
        if layer.is_empty() {
            return [0; 48];
        }
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

    pub fn weight_bytes(&self) -> usize {
        let base = self.canonical_bytes().len();
        let total = base.saturating_add(self.witness_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_hash_is_stable_for_canonical_header() {
        let header = BlockHeader {
            version: 1,
            previous_block_hash: [2; 48],
            merkle_root: [3; 48],
            timestamp: 75,
            target: [4; 48],
            nonce: 42,
        };

        assert_eq!(header.block_hash(), header.block_hash());
        assert_eq!(header.canonical_bytes().len(), 2 + 48 + 48 + 8 + 48 + 8);
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
