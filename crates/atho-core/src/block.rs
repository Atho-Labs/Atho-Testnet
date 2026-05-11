//! Canonical Atho block and block-header encoding.
//!
//! This module defines the consensus-visible block container, the exact header
//! byte layout used for hashing, and the Merkle/witness commitment trees used
//! during block validation.
//!
//! CONSENSUS: Header hashing and transaction commitment construction must remain
//! byte-for-byte deterministic across all nodes.
use crate::crypto::hash::sha3_384;
use crate::encoding::{compact_size_len, write_compact_size};
use crate::transaction::{Transaction, TxWitness};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const HEADER_CANONICAL_SIZE_WITHOUT_NONCE: usize = 2 + 1 + 8 + 48 + 48 + 48 + 8 + 48;
const HEADER_CANONICAL_SIZE: usize = HEADER_CANONICAL_SIZE_WITHOUT_NONCE + 8;

/// Canonical Atho block header.
///
/// The header contains the minimal data needed to identify a block, validate
/// its proof of work, and bind the block body through Merkle and witness roots.
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

/// Full Atho block payload as stored and relayed by full nodes.
///
/// The header is the consensus commitment. Transactions are the canonical block
/// body, while `witnesses` is a convenience cache reconstructed from the
/// transaction witness payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    #[serde(skip, default)]
    pub witnesses: BTreeMap<[u8; 48], TxWitness>,
    pub fees_total_atoms: u64,
    pub fees_miner_atoms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSizeMetrics {
    pub base_size_bytes: usize,
    pub raw_size_bytes: usize,
    pub weight_bytes: usize,
    pub vsize_bytes: usize,
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
        }
    }
}

impl BlockHeader {
    /// Returns the number of canonical header bytes before the nonce field.
    pub fn canonical_size_bytes_without_nonce(&self) -> usize {
        HEADER_CANONICAL_SIZE_WITHOUT_NONCE
    }

    /// Returns the exact canonical header size used for PoW hashing.
    pub fn canonical_size_bytes(&self) -> usize {
        HEADER_CANONICAL_SIZE
    }

    /// Serializes the header fields that are stable while miners search nonce space.
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

    /// Serializes the full canonical header in consensus order.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.canonical_size_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.push(self.network_id.consensus_id());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.previous_block_hash);
        out.extend_from_slice(&self.merkle_root);
        out.extend_from_slice(&self.witness_root);
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out.extend_from_slice(&self.difficulty_target_or_bits);
        out.extend_from_slice(&self.nonce.to_le_bytes());
        out
    }

    /// Computes the Atho block hash from canonical header bytes only.
    ///
    /// CONSENSUS: Any change to this byte layout invalidates historical block
    /// hashes and will split the network.
    pub fn block_hash(&self) -> [u8; 48] {
        let mut bytes = [0u8; HEADER_CANONICAL_SIZE];
        let mut offset = 0usize;
        bytes[offset..offset + 2].copy_from_slice(&self.version.to_le_bytes());
        offset += 2;
        bytes[offset] = self.network_id.consensus_id();
        offset += 1;
        bytes[offset..offset + 8].copy_from_slice(&self.height.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 48].copy_from_slice(&self.previous_block_hash);
        offset += 48;
        bytes[offset..offset + 48].copy_from_slice(&self.merkle_root);
        offset += 48;
        bytes[offset..offset + 48].copy_from_slice(&self.witness_root);
        offset += 48;
        bytes[offset..offset + 8].copy_from_slice(&self.timestamp.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 48].copy_from_slice(&self.difficulty_target_or_bits);
        offset += 48;
        bytes[offset..offset + 8].copy_from_slice(&self.nonce.to_le_bytes());
        sha3_384(&bytes)
    }

    /// Parses the exact canonical header encoding emitted by
    /// [`BlockHeader::canonical_bytes`].
    pub fn from_canonical_bytes(bytes: &[u8]) -> Option<Self> {
        fn read_u16(bytes: &[u8], offset: &mut usize) -> Option<u16> {
            let end = offset.checked_add(2)?;
            let slice = bytes.get(*offset..end)?;
            let mut buf = [0u8; 2];
            buf.copy_from_slice(slice);
            *offset = end;
            Some(u16::from_le_bytes(buf))
        }

        fn read_u64(bytes: &[u8], offset: &mut usize) -> Option<u64> {
            let end = offset.checked_add(8)?;
            let slice = bytes.get(*offset..end)?;
            let mut buf = [0u8; 8];
            buf.copy_from_slice(slice);
            *offset = end;
            Some(u64::from_le_bytes(buf))
        }

        fn read_array<const N: usize>(bytes: &[u8], offset: &mut usize) -> Option<[u8; N]> {
            let end = offset.checked_add(N)?;
            let slice = bytes.get(*offset..end)?;
            let mut out = [0u8; N];
            out.copy_from_slice(slice);
            *offset = end;
            Some(out)
        }

        let mut offset = 0usize;
        let version = read_u16(bytes, &mut offset)?;
        let network_id = crate::network::Network::from_consensus_id(*bytes.get(offset)?)?;
        offset += 1;
        let height = read_u64(bytes, &mut offset)?;
        let previous_block_hash = read_array::<48>(bytes, &mut offset)?;
        let merkle_root = read_array::<48>(bytes, &mut offset)?;
        let witness_root = read_array::<48>(bytes, &mut offset)?;
        let timestamp = read_u64(bytes, &mut offset)?;
        let difficulty_target_or_bits = read_array::<48>(bytes, &mut offset)?;
        let nonce = read_u64(bytes, &mut offset)?;
        if offset != bytes.len() {
            return None;
        }
        Some(Self {
            version,
            network_id,
            height,
            previous_block_hash,
            merkle_root,
            witness_root,
            timestamp,
            difficulty_target_or_bits,
            nonce,
        })
    }
}

impl Block {
    /// Builds a block and repopulates the witness cache from the transaction list.
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

    /// Serializes the block without witness bytes.
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

    /// Serializes the full block including embedded transaction witness data.
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

    /// Serializes the block using compact-size prefixes for relay-oriented uses.
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

    /// Returns the canonical full-block bytes used by disk and wire encoders.
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

    pub fn size_metrics(&self) -> BlockSizeMetrics {
        let header_and_count = self.header.canonical_size_bytes() + 4;
        let mut base_size_bytes = header_and_count;
        let mut raw_size_bytes = header_and_count;
        for tx in &self.transactions {
            base_size_bytes = base_size_bytes.saturating_add(4 + tx.base_size_bytes());
            raw_size_bytes = raw_size_bytes.saturating_add(4 + tx.full_size_bytes());
        }
        let weight_bytes = base_size_bytes
            .saturating_mul(3)
            .saturating_add(raw_size_bytes);
        let vsize_bytes = (weight_bytes.saturating_add(3)) / 4;
        BlockSizeMetrics {
            base_size_bytes,
            raw_size_bytes,
            weight_bytes,
            vsize_bytes,
        }
    }

    pub fn size_bytes(&self) -> usize {
        self.full_size_bytes()
    }

    /// Returns the total witness bytes cached for the block.
    pub fn witness_bytes(&self) -> usize {
        self.witnesses
            .values()
            .map(|witness| witness.canonical_bytes().len())
            .sum()
    }

    /// Recomputes the witness commitment root from the block transactions.
    pub fn compute_witness_root(&self) -> [u8; 48] {
        witness_root(&self.transactions)
    }

    /// Returns the commitment written into the block header for witness data.
    pub fn compute_witness_commitment(&self) -> [u8; 48] {
        self.compute_witness_root()
    }

    pub fn weight_bytes(&self) -> usize {
        self.size_metrics().weight_bytes
    }

    pub fn vsize_bytes(&self) -> usize {
        self.size_metrics().vsize_bytes
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

    /// Returns the block's transaction Merkle root.
    pub fn merkle_root(&self) -> [u8; 48] {
        merkle_root(&self.transactions)
    }

    /// Parses the canonical block byte layout emitted by [`Block::full_bytes`].
    pub fn from_canonical_bytes(bytes: &[u8]) -> Option<Self> {
        fn read_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
            let end = offset.checked_add(4)?;
            let slice = bytes.get(*offset..end)?;
            let mut buf = [0u8; 4];
            buf.copy_from_slice(slice);
            *offset = end;
            Some(u32::from_le_bytes(buf))
        }

        fn read_vec(bytes: &[u8], offset: &mut usize, len: usize) -> Option<Vec<u8>> {
            let end = offset.checked_add(len)?;
            let slice = bytes.get(*offset..end)?;
            *offset = end;
            Some(slice.to_vec())
        }

        let header_len = BlockHeader {
            version: 0,
            network_id: crate::network::Network::Mainnet,
            height: 0,
            previous_block_hash: [0; 48],
            merkle_root: [0; 48],
            witness_root: [0; 48],
            timestamp: 0,
            difficulty_target_or_bits: [0; 48],
            nonce: 0,
        }
        .canonical_size_bytes();
        let header = BlockHeader::from_canonical_bytes(bytes.get(..header_len)?)?;
        let mut offset = header_len;
        let tx_count = read_u32(bytes, &mut offset)? as usize;
        let mut transactions = Vec::with_capacity(tx_count);
        for _ in 0..tx_count {
            let tx_len = read_u32(bytes, &mut offset)? as usize;
            let tx_bytes = read_vec(bytes, &mut offset, tx_len)?;
            let tx = Transaction::from_full_bytes(&tx_bytes)?;
            transactions.push(tx);
        }
        if offset != bytes.len() {
            return None;
        }
        Some(Self::new(header, transactions))
    }
}

/// Computes the transaction Merkle root for a block body.
///
/// CONSENSUS: The last hash in an odd-length layer is duplicated to match the
/// consensus tree construction used everywhere else in Atho.
pub fn merkle_root(transactions: &[Transaction]) -> [u8; 48] {
    if transactions.is_empty() {
        return [0; 48];
    }

    let mut layer: Vec<[u8; 48]> = transactions.iter().map(Transaction::txid).collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
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

/// Computes the witness commitment tree root for the block body.
pub fn witness_root(transactions: &[Transaction]) -> [u8; 48] {
    if transactions.is_empty() {
        return [0; 48];
    }

    let mut layer: Vec<[u8; 48]> = transactions
        .iter()
        .map(Transaction::witness_commitment_hash)
        .collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let header = BlockHeader {
            version: 1,
            network_id: crate::network::Network::Mainnet,
            height: 1,
            previous_block_hash: [2; 48],
            merkle_root: merkle_root(std::slice::from_ref(&tx)),
            witness_root: witness_root(std::slice::from_ref(&tx)),
            timestamp: 75,
            difficulty_target_or_bits: [5; 48],
            nonce: 42,
        };
        let block = Block::new(header, vec![tx]);

        assert!(block.compact_bytes().len() <= block.full_bytes().len());
    }

    #[test]
    fn canonical_block_bytes_round_trip() {
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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let header = BlockHeader {
            version: 1,
            network_id: crate::network::Network::Mainnet,
            height: 7,
            previous_block_hash: [2; 48],
            merkle_root: merkle_root(std::slice::from_ref(&tx)),
            witness_root: witness_root(std::slice::from_ref(&tx)),
            timestamp: 75,
            difficulty_target_or_bits: [5; 48],
            nonce: 42,
        };
        let mut block = Block::new(header, vec![tx]);
        block.fees_total_atoms = 0;
        block.fees_miner_atoms = 0;

        let decoded = Block::from_canonical_bytes(&block.canonical_bytes()).expect("decode block");
        assert_eq!(decoded.header, block.header);
        assert_eq!(decoded.transactions, block.transactions);
    }
}
