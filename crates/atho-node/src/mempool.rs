use crate::validation::{validate_transaction, ValidationError};
use crate::dev;
use atho_core::transaction::Transaction;
use rayon::prelude::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct MempoolEntry {
    pub transaction: Transaction,
    pub fee_atoms: u64,
}

#[derive(Debug, Default)]
pub struct Mempool {
    entries: BTreeMap<[u8; 48], MempoolEntry>,
}

impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn admit(&mut self, entry: MempoolEntry) -> Result<[u8; 48], ValidationError> {
        validate_transaction(&entry.transaction, entry.fee_atoms)?;
        let txid = entry.transaction.txid();
        self.entries.insert(txid, entry);
        let _ = dev::append_log("mempool", &format!("admitted tx {}", hex::encode(txid)));
        Ok(txid)
    }

    pub fn admit_many(
        &mut self,
        entries: Vec<MempoolEntry>,
    ) -> Result<Vec<[u8; 48]>, ValidationError> {
        let txids = entries
            .par_iter()
            .map(|entry| {
                validate_transaction(&entry.transaction, entry.fee_atoms)?;
                Ok(entry.transaction.txid())
            })
            .collect::<Result<Vec<_>, ValidationError>>()?;

        for (txid, entry) in txids.iter().copied().zip(entries.into_iter()) {
            self.entries.insert(txid, entry);
            let _ = dev::append_log("mempool", &format!("batch admitted tx {}", hex::encode(txid)));
        }

        Ok(txids)
    }

    pub fn contains(&self, txid: &[u8; 48]) -> bool {
        self.entries.contains_key(txid)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn transactions(&self) -> Vec<Transaction> {
        self.entries
            .values()
            .map(|entry| entry.transaction.clone())
            .collect()
    }

    pub fn total_fee_atoms(&self) -> u64 {
        self.entries.values().map(|entry| entry.fee_atoms).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_crypto::falcon::{FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_MIN_BYTES};
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};

    fn witness_bytes(inputs: usize) -> Vec<u8> {
        TxWitness {
            signature: vec![9; FALCON_512_SIGNATURE_MIN_BYTES],
            pubkey: vec![8; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: (0..inputs).map(|_| vec![7, 7]).collect(),
        }
        .canonical_bytes()
    }

    #[test]
    fn mempool_admits_valid_transactions() {
        let mut mempool = Mempool::new();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [2; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: witness_bytes(1),
        };

        let txid = mempool
            .admit(MempoolEntry {
                transaction: tx,
                fee_atoms: 500,
            })
            .unwrap();

        assert!(mempool.contains(&txid));
        assert_eq!(mempool.len(), 1);
    }

    #[test]
    fn mempool_admits_many_transactions_in_parallel() {
        let mut mempool = Mempool::new();
        let entries = vec![
            MempoolEntry {
                transaction: Transaction {
                    version: 1,
                    inputs: vec![TxInput {
                        previous_txid: [2; 48],
                        output_index: 0,
                        unlocking_script: vec![1],
                    }],
                    outputs: vec![TxOutput {
                        value_atoms: 1_000,
                        locking_script: vec![2],
                    }],
                    lock_time: 0,
                    witness: witness_bytes(1),
                },
                fee_atoms: 500,
            },
            MempoolEntry {
                transaction: Transaction {
                    version: 1,
                    inputs: vec![TxInput {
                        previous_txid: [3; 48],
                        output_index: 1,
                        unlocking_script: vec![3],
                    }],
                    outputs: vec![TxOutput {
                        value_atoms: 2_000,
                        locking_script: vec![4],
                    }],
                    lock_time: 0,
                    witness: witness_bytes(1),
                },
                fee_atoms: 500,
            },
        ];

        let txids = mempool.admit_many(entries).unwrap();
        assert_eq!(txids.len(), 2);
        assert_eq!(mempool.len(), 2);
    }
}
