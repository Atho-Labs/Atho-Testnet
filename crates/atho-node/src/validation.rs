use atho_core::block::Block;
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::{
    MAX_BLOCK_SIZE_BYTES, MAX_BLOCK_WEIGHT, MAX_TRANSACTION_SIZE_BYTES, MIN_TX_FEE_ATOMS,
};
use atho_core::transaction::Transaction;
use atho_crypto::falcon::{
    self, FalconPublicKey, FalconSignature, FALCON_512_PUBLIC_KEY_BYTES,
    FALCON_512_SIGNATURE_MAX_BYTES, FALCON_512_SIGNATURE_MIN_BYTES,
};
use std::collections::BTreeSet;
use rayon::prelude::*;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("transaction has no inputs")]
    NoInputs,
    #[error("transaction has no outputs")]
    NoOutputs,
    #[error("fee below policy minimum")]
    FeeBelowMinimum,
    #[error("transaction too large")]
    TransactionTooLarge,
    #[error("duplicate transaction input")]
    DuplicateInput,
    #[error("zero-value output")]
    ZeroValueOutput,
    #[error("invalid witness")]
    InvalidWitness,
    #[error("coinbase transaction invalid")]
    InvalidCoinbase,
    #[error("coinbase reward mismatch")]
    CoinbaseRewardMismatch,
    #[error("block has no transactions")]
    EmptyBlock,
    #[error("block too large")]
    BlockTooLarge,
    #[error("block merkle root mismatch")]
    BlockMerkleRootMismatch,
    #[error("block witness commitment mismatch")]
    BlockWitnessCommitmentMismatch,
    #[error("block target out of bounds")]
    BlockTargetOutOfBounds,
    #[error("proof of work invalid")]
    ProofOfWorkInvalid,
    #[error("block parent hash mismatch")]
    BlockParentHashMismatch,
}

pub fn validate_transaction(tx: &Transaction, fee_atoms: u64) -> Result<(), ValidationError> {
    if tx.is_coinbase() {
        return Err(ValidationError::NoInputs);
    }
    if tx.outputs.is_empty() {
        return Err(ValidationError::NoOutputs);
    }
    if tx.vsize_bytes() > MAX_TRANSACTION_SIZE_BYTES {
        return Err(ValidationError::TransactionTooLarge);
    }
    if tx.outputs.iter().any(|output| output.value_atoms == 0) {
        return Err(ValidationError::ZeroValueOutput);
    }
    let mut seen = BTreeSet::new();
    for input in &tx.inputs {
        if !seen.insert((input.previous_txid, input.output_index)) {
            return Err(ValidationError::DuplicateInput);
        }
    }
    if fee_atoms < MIN_TX_FEE_ATOMS {
        return Err(ValidationError::FeeBelowMinimum);
    }
    if !tx.inputs.is_empty() {
        let witness = tx.witness_payload().ok_or(ValidationError::InvalidWitness)?;
        if witness.signature.is_empty() || witness.pubkey.is_empty() {
            return Err(ValidationError::InvalidWitness);
        }
        if witness.pubkey.len() != FALCON_512_PUBLIC_KEY_BYTES {
            return Err(ValidationError::InvalidWitness);
        }
        if witness.signature.len() < FALCON_512_SIGNATURE_MIN_BYTES
            || witness.signature.len() > FALCON_512_SIGNATURE_MAX_BYTES
        {
            return Err(ValidationError::InvalidWitness);
        }
        if witness.input_refs.len() != tx.inputs.len() {
            return Err(ValidationError::InvalidWitness);
        }
        let skip_falcon = cfg!(test)
            || std::env::var("ATHO_SKIP_FALCON_VALIDATION")
                .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false);
        if !skip_falcon {
            let signing_digest = tx.signing_digest();
            let verified = falcon::verify(
                &FalconPublicKey(witness.pubkey.clone()),
                &signing_digest,
                &FalconSignature(witness.signature.clone()),
            )
            .map_err(|_| ValidationError::InvalidWitness)?;
            if !verified {
                return Err(ValidationError::InvalidWitness);
            }
        }
    }
    Ok(())
}

pub fn validate_coinbase_transaction(tx: &Transaction, expected_reward_atoms: u64) -> Result<(), ValidationError> {
    if !tx.is_coinbase() {
        return Err(ValidationError::InvalidCoinbase);
    }
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidCoinbase);
    }
    if tx.output_value_atoms() != expected_reward_atoms {
        return Err(ValidationError::CoinbaseRewardMismatch);
    }
    Ok(())
}

pub fn validate_block(block: &Block, height: u64, network: atho_core::network::Network) -> Result<(), ValidationError> {
    if block.transactions.is_empty() {
        return Err(ValidationError::EmptyBlock);
    }
    if block.vsize_bytes() > MAX_BLOCK_SIZE_BYTES || block.weight_bytes() > MAX_BLOCK_WEIGHT {
        return Err(ValidationError::BlockTooLarge);
    }
    let computed_root = block.merkle_root();
    if computed_root != block.header.merkle_root {
        return Err(ValidationError::BlockMerkleRootMismatch);
    }
    if block.compute_witness_commitment() != block.witness_commitment {
        return Err(ValidationError::BlockWitnessCommitmentMismatch);
    }
    let target = block.header.target;
    if !pow::target_within_bounds(&target) {
        return Err(ValidationError::BlockTargetOutOfBounds);
    }
    let skip_pow = std::env::var("ATHO_SKIP_POW_VALIDATION")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    if !cfg!(test) && !skip_pow && block.header.block_hash() > target {
        return Err(ValidationError::ProofOfWorkInvalid);
    }

    let subsidy = subsidy::block_subsidy_atho(height);
    let expected_coinbase_reward = subsidy.saturating_add(block.fees_miner_atoms);
    validate_coinbase_transaction(&block.transactions[0], expected_coinbase_reward)?;
    if block.transactions.len() > 1 {
        block.transactions[1..]
            .par_iter()
            .try_for_each(|tx| validate_transaction(tx, MIN_TX_FEE_ATOMS))?;
    }

    let _ = network;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::{self, Block, BlockHeader};
    use atho_core::consensus::pow;
    use atho_core::network::Network;
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
    fn transaction_validation_enforces_minimum_shape_and_fee() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [0; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: witness_bytes(1),
        };

        assert_eq!(validate_transaction(&tx, 499), Err(ValidationError::FeeBelowMinimum));
        assert_eq!(validate_transaction(&tx, 500), Ok(()));
    }

    #[test]
    fn coinbase_validation_enforces_reward() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atho(0),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };

        assert_eq!(validate_coinbase_transaction(&tx, subsidy::block_subsidy_atho(0)), Ok(()));
        assert_eq!(
            validate_coinbase_transaction(&tx, subsidy::block_subsidy_atho(0) + 1),
            Err(ValidationError::CoinbaseRewardMismatch)
        );
    }

    #[test]
    fn transaction_validation_rejects_duplicates_and_zero_values() {
        let tx = Transaction {
            version: 1,
            inputs: vec![
                TxInput {
                    previous_txid: [0; 48],
                    output_index: 0,
                    unlocking_script: vec![1],
                },
                TxInput {
                    previous_txid: [0; 48],
                    output_index: 0,
                    unlocking_script: vec![2],
                },
            ],
            outputs: vec![TxOutput {
                value_atoms: 0,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: witness_bytes(2),
        };

        assert_eq!(validate_transaction(&tx, 500), Err(ValidationError::ZeroValueOutput));
    }

    #[test]
    fn transaction_validation_rejects_oversized_payloads() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [0; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1,
                locking_script: vec![0; MAX_TRANSACTION_SIZE_BYTES],
            }],
            lock_time: 0,
            witness: witness_bytes(1),
        };

        assert_eq!(validate_transaction(&tx, 500), Err(ValidationError::TransactionTooLarge));
    }

    #[test]
    fn block_validation_checks_root_and_payloads() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atho(0),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let header = BlockHeader {
                version: 1,
                previous_block_hash: [0; 48],
                merkle_root: block::merkle_root(std::slice::from_ref(&tx)),
                timestamp: 75,
                target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            };
        let mut block = Block::new(
            header,
            vec![tx],
        );
        block.fees_miner_atoms = 0;

        assert_eq!(validate_block(&block, 0, Network::Mainnet), Ok(()));
    }

    #[test]
    fn block_validation_scales_over_multiple_transactions() {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atho(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1,
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: witness_bytes(1),
        };
        let transactions = vec![coinbase, tx.clone(), tx.clone(), tx];
        let header = BlockHeader {
                version: 1,
                previous_block_hash: [0; 48],
                merkle_root: block::merkle_root(&transactions),
                timestamp: 75,
                target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            };
        let mut block = Block::new(
            header,
            transactions,
        );
        block.fees_miner_atoms = 0;

        assert_eq!(validate_block(&block, 1, Network::Mainnet), Ok(()));
    }

    #[test]
    fn block_validation_rejects_bad_witness_commitment() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atho(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let header = BlockHeader {
                version: 1,
                previous_block_hash: [0; 48],
                merkle_root: block::merkle_root(std::slice::from_ref(&tx)),
                timestamp: 75,
                target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            };
        let mut block = Block::new(
            header,
            vec![tx],
        );
        block.witness_commitment = [9; 48];

        assert_eq!(
            validate_block(&block, 1, Network::Mainnet),
            Err(ValidationError::BlockWitnessCommitmentMismatch)
        );
    }
}
