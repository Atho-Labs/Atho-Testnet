use crate::error::StorageError;
use crate::utxo::{BlockUndo, UtxoEntry, UtxoSet};
use atho_core::address::internal_hpk_bytes;
use atho_core::block::{Block, BlockHeader};
use atho_core::constants::GENESIS_COINBASE_ATOMS;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};

    fn witness_bytes(inputs: usize) -> Vec<u8> {
        TxWitness {
            signature: vec![9, 9, 9],
            pubkey: vec![8, 8, 8],
            input_refs: (0..inputs).map(|_| vec![7, 7]).collect(),
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
            witness: witness_bytes(1),
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
}
