use crate::dev;
use crate::validation::{validate_transaction_with_context, ValidationError};
use atho_core::transaction::Transaction;
use atho_storage::utxo::UtxoEntry;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct MempoolEntry {
    pub transaction: Transaction,
    pub fee_atoms: u64,
}

impl MempoolEntry {
    pub fn feerate_atoms_per_vbyte(&self) -> u64 {
        let vsize = self.transaction.vsize_bytes().max(1) as u64;
        self.fee_atoms / vsize
    }
}

#[derive(Debug, Default)]
pub struct Mempool {
    entries: BTreeMap<[u8; 48], MempoolEntry>,
    spent_inputs: BTreeSet<([u8; 48], u32)>,
}

impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    fn input_keys(tx: &Transaction) -> impl Iterator<Item = ([u8; 48], u32)> + '_ {
        tx.inputs
            .iter()
            .map(|input| (input.previous_txid, input.output_index))
    }

    fn reserve_inputs(&mut self, tx: &Transaction) -> Result<(), ValidationError> {
        let mut inserted = Vec::new();
        for key in Self::input_keys(tx) {
            if !self.spent_inputs.insert(key) {
                for key in inserted.into_iter().rev() {
                    let _ = self.spent_inputs.remove(&key);
                }
                return Err(ValidationError::MempoolConflict);
            }
            inserted.push(key);
        }
        Ok(())
    }

    fn release_inputs(&mut self, tx: &Transaction) {
        for key in Self::input_keys(tx) {
            let _ = self.spent_inputs.remove(&key);
        }
    }

    fn validate_entry<F>(
        &self,
        entry: &MempoolEntry,
        spend_height: u64,
        mut lookup: F,
    ) -> Result<u64, ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let fee = validate_transaction_with_context(
            &entry.transaction,
            entry.fee_atoms,
            spend_height,
            |txid, output_index| lookup(txid, output_index),
        )?;
        if Self::input_keys(&entry.transaction).any(|key| self.spent_inputs.contains(&key)) {
            return Err(ValidationError::MempoolConflict);
        }
        Ok(fee)
    }

    pub fn admit<F>(
        &mut self,
        entry: MempoolEntry,
        spend_height: u64,
        lookup: F,
    ) -> Result<[u8; 48], ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let txid = entry.transaction.txid();
        if self.entries.contains_key(&txid) {
            return Err(ValidationError::MempoolConflict);
        }
        self.validate_entry(&entry, spend_height, lookup)?;
        self.reserve_inputs(&entry.transaction)?;
        self.entries.insert(txid, entry);
        let entry = self
            .entries
            .get(&txid)
            .expect("mempool entry just inserted");
        let summary = dev::summarize_transaction(&entry.transaction, Some(entry.fee_atoms));
        let _ = dev::append_log("mempool", &format!("admitted {summary}"));
        Ok(txid)
    }

    pub fn admit_many<F>(
        &mut self,
        entries: Vec<MempoolEntry>,
        spend_height: u64,
        mut lookup: F,
    ) -> Result<Vec<[u8; 48]>, ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let mut txids = Vec::with_capacity(entries.len());
        for entry in entries {
            let txid = self.admit(entry, spend_height, &mut lookup)?;
            txids.push(txid);
        }
        Ok(txids)
    }

    pub fn revalidate<F>(&mut self, spend_height: u64, mut lookup: F)
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let current = std::mem::take(&mut self.entries);
        self.spent_inputs.clear();
        let before = current.len();
        for (txid, entry) in current {
            if self
                .validate_entry(&entry, spend_height, &mut lookup)
                .is_ok()
                && self.reserve_inputs(&entry.transaction).is_ok()
            {
                self.entries.insert(txid, entry);
            }
        }
        let kept = self.entries.len();
        let _ = dev::append_log(
            "mempool",
            &format!(
                "revalidated kept={} dropped={}",
                kept,
                before.saturating_sub(kept)
            ),
        );
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

    pub fn valid_transactions<F>(
        &self,
        spend_height: u64,
        mut lookup: F,
    ) -> Result<(Vec<Transaction>, u64), ValidationError>
    where
        F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
    {
        let mut ordered: Vec<&MempoolEntry> = self.entries.values().collect();
        ordered.sort_by(|left, right| {
            right
                .feerate_atoms_per_vbyte()
                .cmp(&left.feerate_atoms_per_vbyte())
                .then_with(|| right.fee_atoms.cmp(&left.fee_atoms))
                .then_with(|| left.transaction.txid().cmp(&right.transaction.txid()))
        });

        let mut txs = Vec::with_capacity(ordered.len());
        let mut fees = 0u64;
        for entry in ordered {
            let fee = validate_transaction_with_context(
                &entry.transaction,
                entry.fee_atoms,
                spend_height,
                |txid, output_index| lookup(txid, output_index),
            )?;
            fees = fees.saturating_add(fee);
            txs.push(entry.transaction.clone());
        }
        Ok((txs, fees))
    }

    pub fn total_fee_atoms(&self) -> u64 {
        self.entries.values().map(|entry| entry.fee_atoms).sum()
    }

    pub fn spent_inputs_snapshot(&self) -> Vec<([u8; 48], u32)> {
        self.spent_inputs.iter().cloned().collect()
    }

    pub fn remove_block_transactions(&mut self, block: &atho_core::block::Block) {
        for tx in &block.transactions {
            let txid = tx.txid();
            if let Some(entry) = self.entries.remove(&txid) {
                self.release_inputs(&entry.transaction);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES};

    fn witness_bytes_for_tx(tx: &Transaction) -> Vec<u8> {
        let signature = vec![9; FALCON_512_SIGNATURE_BYTES];
        let pubkey = vec![8; FALCON_512_PUBLIC_KEY_BYTES];
        let txid = tx.txid();
        let staged = TxWitness {
            signature: signature.clone(),
            pubkey: pubkey.clone(),
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: crate::validation::derive_sig_ref_short(
                        &txid,
                        &signature,
                        index as u32,
                    ),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        };
        let staged_tx = Transaction {
            witness: staged.canonical_bytes(),
            ..tx.clone()
        };
        let witness_root = staged_tx.witness_commitment_hash();
        TxWitness {
            signature,
            pubkey,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: crate::validation::derive_sig_ref_short(
                        &txid,
                        &vec![9; FALCON_512_SIGNATURE_BYTES],
                        index as u32,
                    ),
                    witness_commit_ref: crate::validation::derive_witness_commit_ref(
                        &txid,
                        &witness_root,
                        index as u32,
                    ),
                })
                .collect(),
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
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };

        let txid = mempool
            .admit(
                MempoolEntry {
                    transaction: tx,
                    fee_atoms: 500,
                },
                0,
                |_, _| None,
            )
            .err()
            .expect("missing utxo should fail");

        assert_eq!(txid, ValidationError::MissingUtxo);
    }

    #[test]
    fn mempool_rejects_conflicts() {
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
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };

        let _ = mempool.spent_inputs.insert(([2; 48], 0));
        assert_eq!(
            mempool
                .reserve_inputs(&tx)
                .err()
                .expect("conflict should fail"),
            ValidationError::MempoolConflict
        );
    }

    #[test]
    fn valid_transactions_are_sorted_by_feerate() {
        let mut mempool = Mempool::new();
        let low = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [4; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_500,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let low = Transaction {
            witness: witness_bytes_for_tx(&low),
            ..low
        };
        let high = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [5; 48],
                output_index: 0,
                unlocking_script: vec![3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 7_000,
                locking_script: vec![4],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let high = Transaction {
            witness: witness_bytes_for_tx(&high),
            ..high
        };

        let low_txid = low.txid();
        let high_txid = high.txid();
        let _ = mempool.entries.insert(
            low_txid,
            MempoolEntry {
                transaction: low.clone(),
                fee_atoms: 2_500,
            },
        );
        let _ = mempool.entries.insert(
            high_txid,
            MempoolEntry {
                transaction: high.clone(),
                fee_atoms: 3_000,
            },
        );

        let mut utxos = std::collections::BTreeMap::new();
        utxos.insert(
            ([4; 48], 0),
            UtxoEntry::new(
                atho_core::network::Network::Mainnet,
                [4; 48],
                0,
                10_000,
                vec![1],
                0,
                false,
            ),
        );
        utxos.insert(
            ([5; 48], 0),
            UtxoEntry::new(
                atho_core::network::Network::Mainnet,
                [5; 48],
                0,
                10_000,
                vec![3],
                0,
                false,
            ),
        );

        let (txs, fees) = mempool
            .valid_transactions(7, |txid, output_index| {
                utxos.get(&(*txid, output_index)).cloned()
            })
            .expect("both transactions should validate");

        assert_eq!(fees, 5_500);
        assert_eq!(txs[0].txid(), high_txid);
        assert_eq!(txs[1].txid(), low_txid);
    }
}
