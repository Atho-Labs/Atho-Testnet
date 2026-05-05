//! Canonical Atho transaction and witness encoding.
//!
//! This module defines the transaction body, the attached witness payload, and
//! the byte serialization routines used for txids, wtxids, relay, and wallet
//! signing.
//!
//! CONSENSUS: Transaction hashing must only use canonical byte layouts.
use crate::constants::{
    FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES, MAX_WITNESS_INPUT_REFS,
    TX_POW_BITS_BYTES, TX_POW_NONCE_BYTES,
};
use crate::crypto::hash::sha3_384;
use crate::encoding::{compact_size_len, write_compact_size};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_384};

/// Compact witness reference that binds an input to the shared witness payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessInputRef {
    #[serde(with = "serde_big_array::BigArray")]
    pub sig_ref_short: [u8; 2],
    #[serde(with = "serde_big_array::BigArray")]
    pub witness_commit_ref: [u8; 16],
}

/// Shared witness payload carried by an Atho transaction.
///
/// Atho groups the Falcon signature, Falcon public key, and per-input witness
/// references into one canonical witness blob to keep transaction hashing and
/// relay deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TxWitness {
    pub signature: Vec<u8>,
    pub pubkey: Vec<u8>,
    pub input_refs: Vec<WitnessInputRef>,
}

impl TxWitness {
    /// Returns `true` when no witness fields are populated.
    pub fn is_empty(&self) -> bool {
        self.signature.is_empty() && self.pubkey.is_empty() && self.input_refs.is_empty()
    }

    /// Serializes the witness in the canonical full-transaction form.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.signature.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.signature);
        out.extend_from_slice(&(self.pubkey.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.pubkey);
        out.extend_from_slice(&(self.input_refs.len() as u32).to_le_bytes());
        for item in &self.input_refs {
            out.extend_from_slice(&item.sig_ref_short);
            out.extend_from_slice(&item.witness_commit_ref);
        }
        out
    }

    /// Serializes the witness using compact-size prefixes for relay paths.
    pub fn compact_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            compact_size_len(self.signature.len())
                + self.signature.len()
                + compact_size_len(self.pubkey.len())
                + self.pubkey.len()
                + compact_size_len(self.input_refs.len())
                + self.input_refs.len() * 18,
        );
        write_compact_size(&mut out, self.signature.len());
        out.extend_from_slice(&self.signature);
        write_compact_size(&mut out, self.pubkey.len());
        out.extend_from_slice(&self.pubkey);
        write_compact_size(&mut out, self.input_refs.len());
        for item in &self.input_refs {
            out.extend_from_slice(&item.sig_ref_short);
            out.extend_from_slice(&item.witness_commit_ref);
        }
        out
    }

    /// Returns the subset of witness bytes committed by the witness tree.
    pub fn commitment_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.signature.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.signature);
        out.extend_from_slice(&(self.pubkey.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.pubkey);
        out
    }

    /// Parses a canonical witness blob.
    ///
    /// SECURITY: Length checks reject malformed Falcon material before higher
    /// layers attempt signature verification.
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
        if sig_len != FALCON_512_SIGNATURE_BYTES {
            return None;
        }
        let signature = read_vec(bytes, &mut offset, sig_len)?;
        let pubkey_len = read_u32(bytes, &mut offset)? as usize;
        if pubkey_len != FALCON_512_PUBLIC_KEY_BYTES {
            return None;
        }
        let pubkey = read_vec(bytes, &mut offset, pubkey_len)?;
        let ref_count = read_u32(bytes, &mut offset)? as usize;
        if ref_count > MAX_WITNESS_INPUT_REFS {
            return None;
        }
        let input_ref_bytes = ref_count.checked_mul(18)?;
        if bytes.len() != offset.checked_add(input_ref_bytes)? {
            return None;
        }
        let mut input_refs = Vec::with_capacity(ref_count);
        for _ in 0..ref_count {
            let sig_ref_short = {
                let bytes = bytes.get(offset..offset.checked_add(2)?)?;
                let mut out = [0u8; 2];
                out.copy_from_slice(bytes);
                offset += 2;
                out
            };
            let witness_commit_ref = {
                let bytes = bytes.get(offset..offset.checked_add(16)?)?;
                let mut out = [0u8; 16];
                out.copy_from_slice(bytes);
                offset += 16;
                out
            };
            input_refs.push(WitnessInputRef {
                sig_ref_short,
                witness_commit_ref,
            });
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

/// Reference to one previously created output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxInput {
    #[serde(with = "serde_big_array::BigArray")]
    pub previous_txid: [u8; 48],
    pub output_index: u32,
    pub unlocking_script: Vec<u8>,
}

/// Spendable output created by a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxOutput {
    pub value_atoms: u64,
    pub locking_script: Vec<u8>,
}

/// Canonical Atho transaction body.
///
/// The `witness` field stores the canonical serialized [`TxWitness`] bytes so
/// the transaction can move through storage and the network as a single object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub version: u16,
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    pub lock_time: u32,
    pub witness: Vec<u8>,
    pub tx_pow_nonce: u64,
    pub tx_pow_bits: u8,
}

impl Transaction {
    /// Returns `true` when the transaction is a coinbase.
    pub fn is_coinbase(&self) -> bool {
        self.inputs.is_empty()
    }

    /// Returns the sum of all output values in atoms.
    pub fn checked_output_value_atoms(&self) -> Option<u64> {
        self.outputs
            .iter()
            .try_fold(0u64, |total, output| total.checked_add(output.value_atoms))
    }

    /// Returns the canonical bytes used for txid calculation.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.base_bytes()
    }

    pub fn write_base_bytes(&self, out: &mut Vec<u8>) {
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
    }

    pub fn update_base_hasher<D: Digest>(&self, hasher: &mut D) {
        hasher.update(self.version.to_le_bytes());
        hasher.update((self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            hasher.update(input.previous_txid);
            hasher.update(input.output_index.to_le_bytes());
            hasher.update((input.unlocking_script.len() as u32).to_le_bytes());
            hasher.update(&input.unlocking_script);
        }
        hasher.update((self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            hasher.update(output.value_atoms.to_le_bytes());
            hasher.update((output.locking_script.len() as u32).to_le_bytes());
            hasher.update(&output.locking_script);
        }
        hasher.update(self.lock_time.to_le_bytes());
    }

    pub fn base_size_bytes(&self) -> usize {
        2 + 4
            + self
                .inputs
                .iter()
                .map(|input| 48 + 4 + 4 + input.unlocking_script.len())
                .sum::<usize>()
            + 4
            + self
                .outputs
                .iter()
                .map(|output| 8 + 4 + output.locking_script.len())
                .sum::<usize>()
            + 4
    }

    pub fn full_size_bytes(&self) -> usize {
        2 + 2
            + 4
            + self
                .inputs
                .iter()
                .map(|input| 48 + 4 + 4 + input.unlocking_script.len())
                .sum::<usize>()
            + 4
            + self
                .outputs
                .iter()
                .map(|output| 8 + 4 + output.locking_script.len())
                .sum::<usize>()
            + 4
            + self.witness.len()
            + 4
            + TX_POW_NONCE_BYTES
            + TX_POW_BITS_BYTES
    }

    pub fn compact_size_bytes(&self) -> usize {
        2 + compact_size_len(self.inputs.len())
            + self
                .inputs
                .iter()
                .map(|input| {
                    48 + 4
                        + compact_size_len(input.unlocking_script.len())
                        + input.unlocking_script.len()
                })
                .sum::<usize>()
            + compact_size_len(self.outputs.len())
            + self
                .outputs
                .iter()
                .map(|output| {
                    8 + compact_size_len(output.locking_script.len()) + output.locking_script.len()
                })
                .sum::<usize>()
            + 4
            + compact_size_len(self.witness.len())
            + self.witness.len()
            + TX_POW_NONCE_BYTES
            + TX_POW_BITS_BYTES
    }

    /// Serializes the full transaction including witness bytes.
    pub fn full_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.full_size_bytes());
        out.extend_from_slice(&self.full_bytes_without_pow_fields());
        out.extend_from_slice(&self.tx_pow_nonce.to_le_bytes());
        out.push(self.tx_pow_bits);
        out
    }

    /// Serializes the full transaction including witness bytes but excluding
    /// wallet transaction proof-of-work fields.
    pub fn full_bytes_without_pow_fields(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            self.full_size_bytes()
                .saturating_sub(TX_POW_NONCE_BYTES)
                .saturating_sub(TX_POW_BITS_BYTES),
        );
        out.extend_from_slice(&self.version.to_le_bytes());
        out.push(0x00);
        out.push(0x01);
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
        out.extend_from_slice(&(self.witness.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.witness);
        out.extend_from_slice(&self.lock_time.to_le_bytes());
        out
    }

    /// Serializes the transaction using compact-size prefixes for relay paths.
    pub fn compact_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.compact_size_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        write_compact_size(&mut out, self.inputs.len());
        for input in &self.inputs {
            out.extend_from_slice(&input.previous_txid);
            out.extend_from_slice(&input.output_index.to_le_bytes());
            write_compact_size(&mut out, input.unlocking_script.len());
            out.extend_from_slice(&input.unlocking_script);
        }
        write_compact_size(&mut out, self.outputs.len());
        for output in &self.outputs {
            out.extend_from_slice(&output.value_atoms.to_le_bytes());
            write_compact_size(&mut out, output.locking_script.len());
            out.extend_from_slice(&output.locking_script);
        }
        out.extend_from_slice(&self.lock_time.to_le_bytes());
        write_compact_size(&mut out, self.witness.len());
        out.extend_from_slice(&self.witness);
        out.extend_from_slice(&self.tx_pow_nonce.to_le_bytes());
        out.push(self.tx_pow_bits);
        out
    }

    /// Serializes the base transaction form used for txid hashing.
    pub fn base_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.base_size_bytes());
        self.write_base_bytes(&mut out);
        out
    }

    /// Returns the witness byte count carried by the transaction.
    pub fn witness_bytes(&self) -> usize {
        self.witness.len()
    }

    pub fn weight_bytes(&self) -> usize {
        let base = self.base_size_bytes();
        let total = self.full_size_bytes();
        base.saturating_mul(3).saturating_add(total)
    }

    pub fn vsize_bytes(&self) -> usize {
        (self.weight_bytes().saturating_add(3)) / 4
    }

    /// Returns the effective feerate for the provided absolute fee.
    pub fn feerate_atoms_per_vbyte(&self, fee_atoms: u64) -> (u64, usize) {
        let vsize = self.vsize_bytes().max(1);
        (fee_atoms / vsize as u64, vsize)
    }

    /// Computes the canonical transaction identifier.
    ///
    /// CONSENSUS: Atho txids exclude witness bytes, matching the canonical base
    /// serialization used in UTXO references and Merkle roots.
    pub fn txid(&self) -> [u8; 48] {
        let mut hasher = Sha3_384::new();
        self.update_base_hasher(&mut hasher);
        hasher.finalize().into()
    }

    pub fn wtxid(&self) -> [u8; 48] {
        sha3_384(&self.full_bytes())
    }

    pub fn witness_commitment_hash(&self) -> [u8; 48] {
        let mut out = self.base_bytes();
        if let Some(witness) = self.witness_payload() {
            out.extend_from_slice(&witness.commitment_bytes());
        }
        sha3_384(&out)
    }

    /// Canonical prehash for Atho transaction signatures.
    ///
    /// This is the exact message digest signed under the
    /// `ATHO_TX_SIGN_V1` domain: `SHA3-384(base_bytes())`, where
    /// `base_bytes()` excludes witness data.
    pub fn signing_digest(&self) -> [u8; 48] {
        self.txid()
    }

    pub fn witness_payload(&self) -> Option<TxWitness> {
        if self.witness.is_empty() {
            return None;
        }
        TxWitness::from_bytes(&self.witness)
    }

    /// Parses the canonical full-transaction byte layout emitted by
    /// [`Transaction::full_bytes`].
    ///
    /// CONSENSUS: Disk and recovery code must decode exactly the same bytes
    /// that hashing and relay code produce. Diverging decoders would create
    /// storage-only transaction shapes that the rest of the node cannot verify.
    pub fn from_full_bytes(bytes: &[u8]) -> Option<Self> {
        fn read_u16(bytes: &[u8], offset: &mut usize) -> Option<u16> {
            let end = offset.checked_add(2)?;
            let slice = bytes.get(*offset..end)?;
            let mut buf = [0u8; 2];
            buf.copy_from_slice(slice);
            *offset = end;
            Some(u16::from_le_bytes(buf))
        }

        fn read_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
            let end = offset.checked_add(4)?;
            let slice = bytes.get(*offset..end)?;
            let mut buf = [0u8; 4];
            buf.copy_from_slice(slice);
            *offset = end;
            Some(u32::from_le_bytes(buf))
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

        fn read_vec(bytes: &[u8], offset: &mut usize, len: usize) -> Option<Vec<u8>> {
            let end = offset.checked_add(len)?;
            let slice = bytes.get(*offset..end)?;
            *offset = end;
            Some(slice.to_vec())
        }

        let mut offset = 0usize;
        let version = read_u16(bytes, &mut offset)?;
        if bytes.get(offset..offset.checked_add(2)?)? != [0x00, 0x01] {
            return None;
        }
        offset += 2;

        let input_count = read_u32(bytes, &mut offset)? as usize;
        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            let previous_txid = read_array::<48>(bytes, &mut offset)?;
            let output_index = read_u32(bytes, &mut offset)?;
            let unlocking_len = read_u32(bytes, &mut offset)? as usize;
            let unlocking_script = read_vec(bytes, &mut offset, unlocking_len)?;
            inputs.push(TxInput {
                previous_txid,
                output_index,
                unlocking_script,
            });
        }

        let output_count = read_u32(bytes, &mut offset)? as usize;
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            let value_atoms = read_u64(bytes, &mut offset)?;
            let locking_len = read_u32(bytes, &mut offset)? as usize;
            let locking_script = read_vec(bytes, &mut offset, locking_len)?;
            outputs.push(TxOutput {
                value_atoms,
                locking_script,
            });
        }

        let witness_len = read_u32(bytes, &mut offset)? as usize;
        let witness = read_vec(bytes, &mut offset, witness_len)?;
        if !witness.is_empty() && TxWitness::from_bytes(&witness).is_none() {
            return None;
        }
        let lock_time = read_u32(bytes, &mut offset)?;
        let (tx_pow_nonce, tx_pow_bits) = if offset == bytes.len() {
            (0, 0)
        } else {
            let tx_pow_nonce = read_u64(bytes, &mut offset)?;
            let tx_pow_bits = *bytes.get(offset)?;
            offset += 1;
            if offset != bytes.len() {
                return None;
            }
            (tx_pow_nonce, tx_pow_bits)
        };
        Some(Self {
            version,
            inputs,
            outputs,
            lock_time,
            witness,
            tx_pow_nonce,
            tx_pow_bits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{
        FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES, MAX_WITNESS_INPUT_REFS,
    };

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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let mut with_witness = base.clone();
        with_witness.witness = vec![9, 8, 7, 6];

        assert_eq!(base.txid(), with_witness.txid());
        assert_eq!(base.signing_digest(), with_witness.signing_digest());
        assert_ne!(base.wtxid(), with_witness.wtxid());
    }

    #[test]
    fn pow_fields_do_not_change_txid_or_signing_digest() {
        let base = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![4, 5],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let mut with_pow = base.clone();
        with_pow.tx_pow_nonce = 42;
        with_pow.tx_pow_bits = 19;

        assert_eq!(base.txid(), with_pow.txid());
        assert_eq!(base.signing_digest(), with_pow.signing_digest());
        assert_eq!(
            base.full_bytes_without_pow_fields(),
            with_pow.full_bytes_without_pow_fields()
        );
    }

    #[test]
    fn pow_fields_change_full_bytes_and_wtxid() {
        let base = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![4, 5],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let mut with_pow = base.clone();
        with_pow.tx_pow_nonce = 42;
        with_pow.tx_pow_bits = 19;

        assert_ne!(base.full_bytes(), with_pow.full_bytes());
        assert_ne!(base.wtxid(), with_pow.wtxid());
    }

    #[test]
    fn compact_bytes_are_not_larger_than_full_bytes() {
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
            witness: vec![9, 9, 9],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };

        assert!(tx.compact_bytes().len() <= tx.full_bytes().len());
    }

    #[test]
    fn witness_payload_round_trips() {
        let payload = TxWitness {
            signature: vec![1; FALCON_512_SIGNATURE_BYTES],
            pubkey: vec![4; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: vec![
                WitnessInputRef {
                    sig_ref_short: [6, 7],
                    witness_commit_ref: [8; 16],
                },
                WitnessInputRef {
                    sig_ref_short: [9, 10],
                    witness_commit_ref: [11; 16],
                },
            ],
        };
        let encoded = payload.canonical_bytes();
        let decoded = TxWitness::from_bytes(&encoded).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn witness_payload_rejects_oversized_signature_length() {
        let payload = TxWitness {
            signature: vec![1; FALCON_512_SIGNATURE_BYTES + 1],
            pubkey: vec![4; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: vec![WitnessInputRef {
                sig_ref_short: [6, 7],
                witness_commit_ref: [8; 16],
            }],
        };
        let encoded = payload.canonical_bytes();
        assert!(TxWitness::from_bytes(&encoded).is_none());
    }

    #[test]
    fn witness_payload_rejects_oversized_pubkey_length() {
        let payload = TxWitness {
            signature: vec![1; FALCON_512_SIGNATURE_BYTES],
            pubkey: vec![4; FALCON_512_PUBLIC_KEY_BYTES + 1],
            input_refs: vec![WitnessInputRef {
                sig_ref_short: [6, 7],
                witness_commit_ref: [8; 16],
            }],
        };
        let encoded = payload.canonical_bytes();
        assert!(TxWitness::from_bytes(&encoded).is_none());
    }

    #[test]
    fn witness_payload_rejects_truncated_payload() {
        let payload = TxWitness {
            signature: vec![1; FALCON_512_SIGNATURE_BYTES],
            pubkey: vec![4; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: vec![WitnessInputRef {
                sig_ref_short: [6, 7],
                witness_commit_ref: [8; 16],
            }],
        };
        let mut encoded = payload.canonical_bytes();
        encoded.pop();
        assert!(TxWitness::from_bytes(&encoded).is_none());
    }

    #[test]
    fn witness_payload_rejects_ref_count_above_budget() {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&(FALCON_512_SIGNATURE_BYTES as u32).to_le_bytes());
        encoded.extend_from_slice(&vec![1u8; FALCON_512_SIGNATURE_BYTES]);
        encoded.extend_from_slice(&(FALCON_512_PUBLIC_KEY_BYTES as u32).to_le_bytes());
        encoded.extend_from_slice(&vec![4u8; FALCON_512_PUBLIC_KEY_BYTES]);
        encoded.extend_from_slice(&((MAX_WITNESS_INPUT_REFS + 1) as u32).to_le_bytes());
        assert!(TxWitness::from_bytes(&encoded).is_none());
    }

    #[test]
    fn full_bytes_round_trip() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
                output_index: 3,
                unlocking_script: vec![1, 2, 3, 4],
            }],
            outputs: vec![
                TxOutput {
                    value_atoms: 500,
                    locking_script: vec![5, 6],
                },
                TxOutput {
                    value_atoms: 700,
                    locking_script: vec![7, 8, 9],
                },
            ],
            lock_time: 44,
            witness: TxWitness {
                signature: vec![1; FALCON_512_SIGNATURE_BYTES],
                pubkey: vec![2; FALCON_512_PUBLIC_KEY_BYTES],
                input_refs: vec![WitnessInputRef {
                    sig_ref_short: [3, 4],
                    witness_commit_ref: [5; 16],
                }],
            }
            .canonical_bytes(),
            tx_pow_nonce: 7,
            tx_pow_bits: 18,
        };
        let encoded = tx.full_bytes();
        let decoded = Transaction::from_full_bytes(&encoded).expect("decode tx");
        assert_eq!(decoded, tx);
    }

    #[test]
    fn checked_output_value_atoms_rejects_overflow() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
                output_index: 3,
                unlocking_script: vec![1, 2, 3, 4],
            }],
            outputs: vec![
                TxOutput {
                    value_atoms: u64::MAX,
                    locking_script: vec![5, 6],
                },
                TxOutput {
                    value_atoms: 1,
                    locking_script: vec![7, 8, 9],
                },
            ],
            lock_time: 44,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };

        assert_eq!(tx.checked_output_value_atoms(), None);
    }
}
