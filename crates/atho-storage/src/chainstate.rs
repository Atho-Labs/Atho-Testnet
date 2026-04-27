use crate::error::StorageError;
use crate::utxo::{BlockUndo, UtxoEntry, UtxoSet};
use atho_core::address::internal_hpk_bytes;
use atho_core::block::{Block, BlockHeader};
use atho_core::constants::{GENESIS_COINBASE_ATOMS, PRUNE_DEPTH_BLOCKS};
use atho_core::genesis;
use atho_core::network::Network;

#[derive(Debug, Clone)]
struct ChainUndo {
    previous_tip: Option<BlockHeader>,
    previous_tip_hash: [u8; 48],
    block_undo: BlockUndo,
}

#[derive(Debug)]
pub struct Chainstate {
    pub network: Network,
    pub tip: Option<BlockHeader>,
    pub tip_hash: [u8; 48],
    pub height: u64,
    blocks: Vec<Block>,
    utxos: UtxoSet,
    undo_stack: Vec<ChainUndo>,
}

impl Chainstate {
    pub fn new(network: Network) -> Self {
        let genesis = genesis::genesis_state(network);
        let genesis_block = genesis.block;
        let genesis_header = genesis_block.header.clone();
        let locking_script = internal_hpk_bytes(network, &genesis.reward_address)
            .unwrap_or_else(|| genesis.reward_address.as_bytes().to_vec());
        let mut utxos = UtxoSet::new(network);
        utxos
            .insert(UtxoEntry {
                network,
                txid: genesis.coinbase_txid,
                output_index: 0,
                value_atoms: GENESIS_COINBASE_ATOMS,
                locking_script,
            })
            .expect("genesis utxo is network-local and unique");
        Self {
            network,
            tip: Some(genesis_header),
            tip_hash: genesis.block_hash,
            height: 0,
            blocks: vec![genesis_block],
            utxos,
            undo_stack: Vec::new(),
        }
    }

    pub fn connect_header(&mut self, header: BlockHeader) {
        self.tip_hash = header.block_hash();
        self.tip = Some(header);
        self.height = self.tip.as_ref().map(|header| header.height).unwrap_or(0);
    }

    pub fn connect_block(&mut self, block: &Block) -> Result<(), StorageError> {
        let undo = self.utxos.apply_block(block)?;
        let previous_tip = self.tip.clone();
        let previous_tip_hash = self.tip_hash;
        self.tip = Some(block.header.clone());
        self.tip_hash = block.header.block_hash();
        self.height = block.header.height;
        self.blocks.push(block.clone());
        self.undo_stack.push(ChainUndo {
            previous_tip,
            previous_tip_hash,
            block_undo: undo,
        });
        self.prune_history();
        Ok(())
    }

    pub fn utxo_snapshot(&self) -> UtxoSet {
        self.utxos.clone()
    }

    pub fn utxo_entry(&self, txid: [u8; 48], output_index: u32) -> Option<UtxoEntry> {
        self.utxos.get(txid, output_index).cloned()
    }

    pub fn disconnect_last_block(&mut self) -> Result<(), StorageError> {
        let undo = self
            .undo_stack
            .pop()
            .ok_or(StorageError::NoBlockToDisconnect)?;
        self.utxos.disconnect_block(undo.block_undo);
        let _ = self.blocks.pop();
        self.tip = undo.previous_tip;
        self.tip_hash = undo.previous_tip_hash;
        self.height = self.tip.as_ref().map(|header| header.height).unwrap_or(0);
        Ok(())
    }

    pub fn utxo_count(&self) -> usize {
        self.utxos.len()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn insert_utxo(&mut self, entry: UtxoEntry) -> Result<(), StorageError> {
        self.utxos.insert(entry)
    }

    fn prune_history(&mut self) {
        self.prune_history_to_retain(PRUNE_DEPTH_BLOCKS as usize + 1);
    }

    fn prune_history_to_retain(&mut self, retain: usize) {
        if self.blocks.len() <= retain || retain == 0 {
            return;
        }
        let prune_count = self.blocks.len().saturating_sub(retain);
        if prune_count == 0 {
            return;
        }
        self.blocks.drain(1..1 + prune_count);
        self.undo_stack.drain(0..prune_count);
    }
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
    fn chainstate_tracks_tip_and_height() {
        let mut state = Chainstate::new(Network::Mainnet);
        assert_eq!(state.height, 0);
        assert!(state.tip.is_some());
        assert_ne!(state.tip_hash, [0; 48]);
        assert_eq!(state.blocks().len(), 1);

        state.connect_header(BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [0; 48],
            merkle_root: [0; 48],
            witness_root: [0; 48],
            timestamp: 75,
            difficulty_target_or_bits: atho_core::consensus::pow::initial_target_for_network(
                Network::Mainnet,
            ),
            nonce: 0,
        });

        assert_eq!(state.height, 1);
        assert!(state.tip.is_some());
    }

    #[test]
    fn chainstate_connects_and_disconnects_blocks() {
        let mut state = Chainstate::new(Network::Mainnet);
        state
            .utxos
            .insert(crate::utxo::UtxoEntry {
                network: Network::Mainnet,
                txid: [9; 48],
                output_index: 0,
                value_atoms: 500,
                locking_script: vec![1],
            })
            .unwrap();

        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
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

        state.connect_block(&block).unwrap();
        assert_eq!(state.height, 1);
        assert_eq!(state.utxo_count(), 3);
        assert_eq!(state.blocks().len(), 2);

        state.disconnect_last_block().unwrap();
        assert_eq!(state.height, 0);
        assert_eq!(state.utxo_count(), 2);
        assert_eq!(state.blocks().len(), 1);
    }

    #[test]
    fn chainstate_prunes_old_history_after_retention_window() {
        let mut state = Chainstate::new(Network::Mainnet);
        state.blocks = vec![
            state.blocks[0].clone(),
            state.blocks[0].clone(),
            state.blocks[0].clone(),
        ];
        state.undo_stack = vec![
            ChainUndo {
                previous_tip: None,
                previous_tip_hash: [0; 48],
                block_undo: BlockUndo::empty(),
            },
            ChainUndo {
                previous_tip: None,
                previous_tip_hash: [0; 48],
                block_undo: BlockUndo::empty(),
            },
        ];
        state.prune_history_to_retain(2);
        assert_eq!(state.blocks.len(), 2);
        assert_eq!(state.undo_stack.len(), 1);
    }
}
