use crate::crypto::hash::sha3_384;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TxWitness {
    pub signature: Vec<u8>,
    pub pubkey: Vec<u8>,
    pub input_refs: Vec<Vec<u8>>,
}

impl TxWitness {
    pub fn is_empty(&self) -> bool {
        self.signature.is_empty() && self.pubkey.is_empty() && self.input_refs.is_empty()
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.signature.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.signature);
        out.extend_from_slice(&(self.pubkey.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.pubkey);
        out.extend_from_slice(&(self.input_refs.len() as u32).to_le_bytes());
        for item in &self.input_refs {
            out.extend_from_slice(&(item.len() as u32).to_le_bytes());
            out.extend_from_slice(item);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut offset = 0usize;
        let read_u32 = |bytes: &[u8], offset: &mut usize| -> Option<u32> {
            let end = offset.checked_add(4)?;
            let slice = bytes.get(*offset..end)?;
            let mut buf = [0u8; 4];
            buf.copy_from_slice(slice);
            *offset = end;
            Some(u32::from_le_bytes(buf))
        };
        let read_vec = |bytes: &[u8], offset: &mut usize, len: usize| -> Option<Vec<u8>> {
            let end = offset.checked_add(len)?;
            let slice = bytes.get(*offset..end)?;
            *offset = end;
            Some(slice.to_vec())
        };

        let sig_len = read_u32(bytes, &mut offset)? as usize;
        let signature = read_vec(bytes, &mut offset, sig_len)?;
        let pubkey_len = read_u32(bytes, &mut offset)? as usize;
        let pubkey = read_vec(bytes, &mut offset, pubkey_len)?;
        let ref_count = read_u32(bytes, &mut offset)? as usize;
        let mut input_refs = Vec::with_capacity(ref_count);
        for _ in 0..ref_count {
            let len = read_u32(bytes, &mut offset)? as usize;
            input_refs.push(read_vec(bytes, &mut offset, len)?);
        }
        if offset != bytes.len() {
            return None;
        }
        Some(Self {
            signature,
            pubkey,
            input_refs,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxInput {
    pub previous_txid: [u8; 48],
    pub output_index: u32,
    pub unlocking_script: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxOutput {
    pub value_atoms: u64,
    pub locking_script: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub version: u16,
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    pub lock_time: u32,
    pub witness: Vec<u8>,
}

impl Transaction {
    pub fn is_coinbase(&self) -> bool {
        self.inputs.is_empty()
    }

    pub fn output_value_atoms(&self) -> u64 {
        self.outputs.iter().map(|output| output.value_atoms).sum()
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.base_bytes()
    }

    pub fn base_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            out.extend_from_slice(&input.previous_txid);
            out.extend_from_slice(&input.output_index.to_le_bytes());
            out.extend_from_slice(&(input.unlocking_script.len() as u32).to_le_bytes());
            out.extend_from_slice(&input.unlocking_script);
        }
        out.extend_from_slice(&(self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            out.extend_from_slice(&output.value_atoms.to_le_bytes());
            out.extend_from_slice(&(output.locking_script.len() as u32).to_le_bytes());
            out.extend_from_slice(&output.locking_script);
        }
        out.extend_from_slice(&self.lock_time.to_le_bytes());
        out
    }

    pub fn witness_bytes(&self) -> usize {
        self.witness.len()
    }

    pub fn weight_bytes(&self) -> usize {
        let base = self.base_bytes().len();
        let total = base.saturating_add(self.witness_bytes());
        base.saturating_mul(3).saturating_add(total)
    }

    pub fn vsize_bytes(&self) -> usize {
        (self.weight_bytes().saturating_add(3)) / 4
    }

    pub fn txid(&self) -> [u8; 48] {
        sha3_384(&self.base_bytes())
    }

    pub fn wtxid(&self) -> [u8; 48] {
        let mut out = self.base_bytes();
        out.extend_from_slice(&(self.witness.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.witness);
        sha3_384(&out)
    }

    pub fn signing_digest(&self) -> [u8; 48] {
        sha3_384(&self.base_bytes())
    }

    pub fn witness_payload(&self) -> Option<TxWitness> {
        if self.witness.is_empty() {
            return None;
        }
        TxWitness::from_bytes(&self.witness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txid_is_stable_for_canonical_encoding() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 500,
                locking_script: vec![4, 5],
            }],
            lock_time: 0,
            witness: vec![],
        };

        assert_eq!(tx.txid(), tx.signing_digest());
        assert!(!tx.canonical_bytes().is_empty());
        assert_eq!(tx.vsize_bytes(), tx.weight_bytes().div_ceil(4));
    }

    #[test]
    fn witness_changes_wtxid_but_not_txid() {
        let base = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 500,
                locking_script: vec![4, 5],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let mut with_witness = base.clone();
        with_witness.witness = vec![9, 8, 7, 6];

        assert_eq!(base.txid(), with_witness.txid());
        assert_eq!(base.signing_digest(), with_witness.signing_digest());
        assert_ne!(base.wtxid(), with_witness.wtxid());
    }

    #[test]
    fn witness_payload_round_trips() {
        let payload = TxWitness {
            signature: vec![1, 2, 3],
            pubkey: vec![4, 5],
            input_refs: vec![vec![6], vec![7, 8]],
        };
        let encoded = payload.canonical_bytes();
        let decoded = TxWitness::from_bytes(&encoded).unwrap();
        assert_eq!(decoded, payload);
    }
}
