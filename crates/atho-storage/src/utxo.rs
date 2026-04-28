use crate::error::StorageError;
use atho_core::block::Block;
use atho_core::constants::{COINBASE_MATURITY_BLOCKS, STANDARD_TX_CONFIRMATIONS};
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtxoKey {
    pub txid: [u8; 48],
    pub output_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub network: Network,
    #[serde(with = "serde_big_array::BigArray")]
    pub txid: [u8; 48],
    pub output_index: u32,
    pub value_atoms: u64,
    pub locking_script: Vec<u8>,
    pub created_height: u64,
    pub is_coinbase: bool,
}

impl UtxoEntry {
    pub fn new(
        network: Network,
        txid: [u8; 48],
        output_index: u32,
        value_atoms: u64,
        locking_script: Vec<u8>,
        created_height: u64,
        is_coinbase: bool,
    ) -> Self {
        Self {
            network,
            txid,
            output_index,
            value_atoms,
            locking_script,
            created_height,
            is_coinbase,
        }
    }

    pub fn coinbase(
        network: Network,
        txid: [u8; 48],
        output_index: u32,
        value_atoms: u64,
        locking_script: Vec<u8>,
        created_height: u64,
    ) -> Self {
        Self::new(
            network,
            txid,
            output_index,
            value_atoms,
            locking_script,
            created_height,
            true,
        )
    }

    pub fn confirmation_count(&self, spend_height: u64) -> u64 {
        spend_height
            .saturating_sub(self.created_height)
            .saturating_add(1)
    }

    pub fn required_confirmations(&self) -> u64 {
        if self.is_coinbase {
            COINBASE_MATURITY_BLOCKS
        } else {
            STANDARD_TX_CONFIRMATIONS
        }
    }

    pub fn is_spendable_at(&self, spend_height: u64) -> bool {
        self.confirmation_count(spend_height) >= self.required_confirmations()
    }

    pub fn is_coinbase_mature(&self, spend_height: u64) -> bool {
        self.is_spendable_at(spend_height)
    }
}

#[derive(Debug, Clone)]
pub struct UtxoSet {
    network: Network,
    entries: BTreeMap<UtxoKey, UtxoEntry>,
}

impl UtxoSet {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            entries: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, entry: UtxoEntry) -> Result<(), StorageError> {
        if entry.network != self.network {
            return Err(StorageError::CrossNetworkReplay);
        }
        let key = UtxoKey {
            txid: entry.txid,
            output_index: entry.output_index,
        };
        if self.entries.insert(key, entry).is_some() {
            return Err(StorageError::DuplicateUtxo);
        }
        Ok(())
    }

    pub fn get(&self, txid: [u8; 48], output_index: u32) -> Option<&UtxoEntry> {
        let key = UtxoKey { txid, output_index };
        self.entries.get(&key)
    }

    pub fn remove(&mut self, txid: [u8; 48], output_index: u32) -> Result<UtxoEntry, StorageError> {
        let key = UtxoKey { txid, output_index };
        self.entries.remove(&key).ok_or(StorageError::MissingUtxo)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn entries(&self) -> impl Iterator<Item = &UtxoEntry> {
        self.entries.values()
    }

    pub fn apply_block(&mut self, block: &Block) -> Result<BlockUndo, StorageError> {
        let mut undo = BlockUndo {
            spent: Vec::new(),
            created: Vec::new(),
        };

        for tx in &block.transactions {
            let spent = match spend_inputs(self, tx) {
                Ok(spent) => spent,
                Err(err) => {
                    self.disconnect_block(undo);
                    return Err(err);
                }
            };
            undo.spent.extend(spent);

            let created = create_outputs(tx, self.network, block.header.height);
            for output in created {
                if let Err(err) = self.insert(output.clone()) {
                    self.disconnect_block(undo);
                    return Err(err);
                }
                undo.created.push(output);
            }
        }

        Ok(undo)
    }

    pub fn disconnect_block(&mut self, undo: BlockUndo) {
        for output in undo.created {
            let _ = self.remove(output.txid, output.output_index);
        }
        for entry in undo.spent {
            let _ = self.insert(entry);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockUndo {
    pub(crate) spent: Vec<UtxoEntry>,
    pub(crate) created: Vec<UtxoEntry>,
}

impl BlockUndo {
    #[allow(dead_code)]
    pub(crate) fn empty() -> Self {
        Self {
            spent: Vec::new(),
            created: Vec::new(),
        }
    }
}

fn spend_inputs(set: &mut UtxoSet, tx: &Transaction) -> Result<Vec<UtxoEntry>, StorageError> {
    let mut spent = Vec::with_capacity(tx.inputs.len());
    for input in &tx.inputs {
        match set.remove(input.previous_txid, input.output_index) {
            Ok(entry) => spent.push(entry),
            Err(err) => {
                for entry in spent.into_iter().rev() {
                    let _ = set.insert(entry);
                }
                return Err(err);
            }
        }
    }
    Ok(spent)
}

fn create_outputs(tx: &Transaction, network: Network, created_height: u64) -> Vec<UtxoEntry> {
    let txid = tx.txid();
    tx.outputs
        .iter()
        .enumerate()
        .map(|(output_index, output)| {
            UtxoEntry::new(
                network,
                txid,
                output_index as u32,
                output.value_atoms,
                output.locking_script.clone(),
                created_height,
                tx.is_coinbase(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::crypto::hash::sha3_256;
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};

    fn derive_sig_ref_short(txid: &[u8; 48], signature: &[u8], input_index: u32) -> [u8; 2] {
        let mut preimage = Vec::with_capacity(
            b"ATHO_SIG_REF_SHORT_V1".len()
                + txid.len()
                + signature.len()
                + core::mem::size_of::<u32>(),
        );
        preimage.extend_from_slice(b"ATHO_SIG_REF_SHORT_V1");
        preimage.extend_from_slice(txid);
        preimage.extend_from_slice(signature);
        preimage.extend_from_slice(&input_index.to_be_bytes());
        let digest = sha3_256(&preimage);
        [digest[0], digest[1]]
    }

    fn derive_witness_commit_ref(
        txid: &[u8; 48],
        witness_root: &[u8; 48],
        input_index: u32,
    ) -> [u8; 16] {
        let mut preimage = Vec::with_capacity(
            b"ATHO_WITNESS_COMMIT_REF_V1".len()
                + txid.len()
                + core::mem::size_of::<u32>()
                + witness_root.len(),
        );
        preimage.extend_from_slice(b"ATHO_WITNESS_COMMIT_REF_V1");
        preimage.extend_from_slice(txid);
        preimage.extend_from_slice(&input_index.to_be_bytes());
        preimage.extend_from_slice(witness_root);
        let digest = sha3_256(&preimage);
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest[..16]);
        out
    }

    fn witness_bytes_for_tx(tx: &Transaction) -> Vec<u8> {
        let signature = vec![9, 9, 9];
        let pubkey = vec![8, 8, 8];
        let txid = tx.txid();
        let staged = TxWitness {
            signature: signature.clone(),
            pubkey: pubkey.clone(),
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        };
        let staged_tx = Transaction {
            witness: staged.canonical_bytes(),
            ..tx.clone()
        };
        let witness_root = staged_tx.witness_commitment_hash();
        let sig_bytes = signature.clone();
        TxWitness {
            signature: sig_bytes.clone(),
            pubkey,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: derive_witness_commit_ref(
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
    fn utxo_set_accepts_entries_and_clears_for_reorg() {
        let mut set = UtxoSet::new(Network::Mainnet);
        let _ = set.insert(UtxoEntry::new(
            Network::Mainnet,
            [7; 48],
            0,
            500,
            vec![1],
            0,
            false,
        ));

        assert_eq!(set.len(), 1);
        set.clear();
        assert!(set.is_empty());
    }

    #[test]
    fn utxo_set_applies_and_disconnects_block() {
        let mut set = UtxoSet::new(Network::Mainnet);
        set.insert(UtxoEntry::new(
            Network::Mainnet,
            [1; 48],
            0,
            500,
            vec![1],
            0,
            false,
        ))
        .unwrap();

        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![2],
            }],
            outputs: vec![TxOutput {
                value_atoms: 400,
                locking_script: vec![3],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: 500,
                locking_script: vec![9],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let transactions = vec![coinbase, tx];
        let mut block = Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 75,
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            transactions,
        );
        block.fees_miner_atoms = 500;

        let undo = set.apply_block(&block).unwrap();
        assert_eq!(set.len(), 2);
        set.disconnect_block(undo);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn utxo_set_rolls_back_prior_transactions_when_later_input_fails() {
        let mut set = UtxoSet::new(Network::Mainnet);
        let funding = UtxoEntry::new(Network::Mainnet, [1; 48], 0, 500, vec![1], 0, false);
        set.insert(funding.clone()).unwrap();

        let spend = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: funding.txid,
                output_index: funding.output_index,
                unlocking_script: funding.locking_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 400,
                locking_script: vec![3],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let spend = Transaction {
            witness: witness_bytes_for_tx(&spend),
            ..spend
        };
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: 500,
                locking_script: vec![9],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let transactions = vec![coinbase, spend.clone(), spend];
        let block = Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 75,
                difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                    Network::Mainnet,
                ),
                nonce: 0,
            },
            transactions,
        );

        let err = set.apply_block(&block).unwrap_err();
        assert!(matches!(err, StorageError::MissingUtxo));
        assert_eq!(set.len(), 1);
        assert!(set.get(funding.txid, funding.output_index).is_some());
    }
}
