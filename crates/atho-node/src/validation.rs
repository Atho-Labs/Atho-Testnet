use atho_core::block::Block;
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::{
    BLOCK_TIME_SECONDS, MAX_BLOCK_SIZE_BYTES, MAX_BLOCK_WEIGHT, MAX_TRANSACTION_SIZE_BYTES,
    MIN_TX_FEE_PER_VBYTE_ATOMS,
};
use atho_core::crypto::hash::sha3_256;
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use atho_crypto::falcon::{
    self, FalconPublicKey, FalconSignature, FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES,
};
use atho_storage::utxo::{UtxoEntry, UtxoSet};
use std::collections::BTreeSet;
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
    #[error("block witness root mismatch")]
    BlockWitnessRootMismatch,
    #[error("block target out of bounds")]
    BlockTargetOutOfBounds,
    #[error("proof of work invalid")]
    ProofOfWorkInvalid,
    #[error("block parent hash mismatch")]
    BlockParentHashMismatch,
    #[error("missing utxo")]
    MissingUtxo,
    #[error("input ownership mismatch")]
    InputOwnershipMismatch,
    #[error("input has insufficient confirmations")]
    InsufficientConfirmations,
    #[error("monetary supply exceeded")]
    MonetarySupplyExceeded,
    #[error("witness input reference mismatch")]
    WitnessInputReferenceMismatch,
    #[error("fee mismatch")]
    FeeMismatch,
    #[error("mempool conflict")]
    MempoolConflict,
    #[error("invalid block height")]
    InvalidBlockHeight,
    #[error("invalid block version")]
    InvalidBlockVersion,
    #[error("invalid block timestamp")]
    InvalidBlockTimestamp,
    #[error("block network mismatch")]
    BlockNetworkMismatch,
    #[error("multiple coinbase transactions")]
    MultipleCoinbaseTransactions,
}

pub fn derive_sig_ref_short(txid: &[u8; 48], signature: &[u8], input_index: u32) -> [u8; 2] {
    let mut preimage = Vec::with_capacity(
        b"ATHO_SIG_REF_SHORT_V1".len() + txid.len() + signature.len() + core::mem::size_of::<u32>(),
    );
    preimage.extend_from_slice(b"ATHO_SIG_REF_SHORT_V1");
    preimage.extend_from_slice(txid);
    preimage.extend_from_slice(signature);
    preimage.extend_from_slice(&input_index.to_be_bytes());
    let digest = sha3_256(&preimage);
    [digest[0], digest[1]]
}

pub fn derive_witness_commit_ref(
    txid: &[u8; 48],
    block_witness_root: &[u8; 48],
    input_index: u32,
) -> [u8; 16] {
    let mut preimage = Vec::with_capacity(
        b"ATHO_WITNESS_COMMIT_REF_V1".len()
            + txid.len()
            + core::mem::size_of::<u32>()
            + block_witness_root.len(),
    );
    preimage.extend_from_slice(b"ATHO_WITNESS_COMMIT_REF_V1");
    preimage.extend_from_slice(txid);
    preimage.extend_from_slice(&input_index.to_be_bytes());
    preimage.extend_from_slice(block_witness_root);
    let digest = sha3_256(&preimage);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

pub fn finalize_witness_commit_refs(tx: &Transaction, block_witness_root: [u8; 48]) -> Transaction {
    let Some(witness) = tx.witness_payload() else {
        return tx.clone();
    };
    let txid = tx.txid();
    let input_refs = witness
        .input_refs
        .iter()
        .enumerate()
        .map(
            |(index, input_ref)| atho_core::transaction::WitnessInputRef {
                sig_ref_short: input_ref.sig_ref_short,
                witness_commit_ref: derive_witness_commit_ref(
                    &txid,
                    &block_witness_root,
                    index as u32,
                ),
            },
        )
        .collect();
    Transaction {
        witness: atho_core::transaction::TxWitness {
            signature: witness.signature,
            pubkey: witness.pubkey,
            input_refs,
        }
        .canonical_bytes(),
        ..tx.clone()
    }
}

pub fn verify_transaction_signature(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.is_coinbase() {
        return Err(ValidationError::NoInputs);
    }
    let witness = tx
        .witness_payload()
        .ok_or(ValidationError::InvalidWitness)?;
    if witness.pubkey.len() != FALCON_512_PUBLIC_KEY_BYTES {
        return Err(ValidationError::InvalidWitness);
    }
    if witness.signature.len() != FALCON_512_SIGNATURE_BYTES {
        return Err(ValidationError::InvalidWitness);
    }
    let signing_digest = transaction_signing_digest(tx);
    let verified = falcon::verify(
        AthoSignatureDomain::Transaction,
        &FalconPublicKey(witness.pubkey),
        &signing_digest,
        &FalconSignature(witness.signature),
    )
    .map_err(|_| ValidationError::InvalidWitness)?;
    if !verified {
        return Err(ValidationError::InvalidWitness);
    }
    Ok(())
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
    let minimum_fee = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
    if fee_atoms < minimum_fee {
        return Err(ValidationError::FeeBelowMinimum);
    }
    if !tx.inputs.is_empty() {
        let witness = tx
            .witness_payload()
            .ok_or(ValidationError::InvalidWitness)?;
        if witness.input_refs.len() != tx.inputs.len() {
            return Err(ValidationError::InvalidWitness);
        }
        if witness.signature.is_empty() || witness.pubkey.is_empty() {
            return Err(ValidationError::InvalidWitness);
        }
        let txid = tx.txid();
        for (index, input_ref) in witness.input_refs.iter().enumerate() {
            let expected_short = derive_sig_ref_short(&txid, &witness.signature, index as u32);
            if input_ref.sig_ref_short != expected_short {
                return Err(ValidationError::WitnessInputReferenceMismatch);
            }
        }
        if !cfg!(test) {
            verify_transaction_signature(tx)?;
        }
    }
    Ok(())
}

pub fn validate_transaction_with_context<F>(
    tx: &Transaction,
    fee_atoms: u64,
    spend_height: u64,
    mut lookup: F,
) -> Result<u64, ValidationError>
where
    F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
{
    validate_transaction(tx, fee_atoms)?;
    let witness = tx
        .witness_payload()
        .ok_or(ValidationError::InvalidWitness)?;
    let txid = tx.txid();
    let mut input_total = 0u64;
    let mut seen = BTreeSet::new();

    for (index, input) in tx.inputs.iter().enumerate() {
        if !seen.insert((input.previous_txid, input.output_index)) {
            return Err(ValidationError::DuplicateInput);
        }
        let utxo =
            lookup(&input.previous_txid, input.output_index).ok_or(ValidationError::MissingUtxo)?;
        if utxo.locking_script != input.unlocking_script {
            return Err(ValidationError::InputOwnershipMismatch);
        }
        if !utxo.is_spendable_at(spend_height) {
            return Err(ValidationError::InsufficientConfirmations);
        }
        let expected_ref = derive_sig_ref_short(&txid, &witness.signature, index as u32);
        if witness.input_refs.get(index).map(|item| item.sig_ref_short) != Some(expected_ref) {
            return Err(ValidationError::WitnessInputReferenceMismatch);
        }
        input_total = input_total
            .checked_add(utxo.value_atoms)
            .ok_or(ValidationError::FeeMismatch)?;
    }

    let output_total = tx.output_value_atoms();
    let actual_fee = input_total
        .checked_sub(output_total)
        .ok_or(ValidationError::FeeMismatch)?;
    if actual_fee != fee_atoms {
        return Err(ValidationError::FeeMismatch);
    }
    Ok(actual_fee)
}

pub fn validate_coinbase_transaction(
    tx: &Transaction,
    expected_reward_atoms: u64,
) -> Result<(), ValidationError> {
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

fn validate_block_impl(
    block: &Block,
    height: u64,
    network: Network,
    skip_pow: bool,
) -> Result<(), ValidationError> {
    if block.transactions.is_empty() {
        return Err(ValidationError::EmptyBlock);
    }
    if block.header.version == 0 {
        return Err(ValidationError::InvalidBlockVersion);
    }
    if block.header.network_id != network {
        return Err(ValidationError::BlockNetworkMismatch);
    }
    if block.header.height != height {
        return Err(ValidationError::InvalidBlockHeight);
    }
    let expected_timestamp = height.saturating_mul(BLOCK_TIME_SECONDS);
    if height == 0 {
        if block.header.timestamp == 0 {
            return Err(ValidationError::InvalidBlockTimestamp);
        }
    } else if block.header.timestamp != expected_timestamp {
        return Err(ValidationError::InvalidBlockTimestamp);
    }
    if block.vsize_bytes() > MAX_BLOCK_SIZE_BYTES || block.weight_bytes() > MAX_BLOCK_WEIGHT {
        return Err(ValidationError::BlockTooLarge);
    }
    let computed_root = block.merkle_root();
    if computed_root != block.header.merkle_root {
        return Err(ValidationError::BlockMerkleRootMismatch);
    }
    if block.compute_witness_root() != block.witness_root {
        return Err(ValidationError::BlockWitnessRootMismatch);
    }
    let target = block.header.difficulty_target_or_bits;
    if !pow::target_within_bounds(&target) {
        return Err(ValidationError::BlockTargetOutOfBounds);
    }
    if !skip_pow && !pow::meets_target(&block.header.block_hash(), &target) {
        return Err(ValidationError::ProofOfWorkInvalid);
    }

    let subsidy = subsidy::block_subsidy_atoms(height);
    if subsidy::cumulative_subsidy_atoms(height) > subsidy::max_supply_atoms() {
        return Err(ValidationError::MonetarySupplyExceeded);
    }
    let expected_coinbase_reward = subsidy.saturating_add(block.fees_miner_atoms);
    validate_coinbase_transaction(&block.transactions[0], expected_coinbase_reward)?;
    if block.transactions.len() > 1 {
        for tx in &block.transactions[1..] {
            validate_transaction(tx, tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS)?;
        }
    }

    let _ = network;
    Ok(())
}

pub fn validate_block(block: &Block, height: u64, network: Network) -> Result<(), ValidationError> {
    validate_block_impl(block, height, network, cfg!(test))
}

pub fn validate_block_without_pow(
    block: &Block,
    height: u64,
    network: Network,
) -> Result<(), ValidationError> {
    validate_block_impl(block, height, network, true)
}

pub fn validate_block_with_context(
    block: &Block,
    height: u64,
    network: Network,
    expected_previous_hash: [u8; 48],
    mut utxos: UtxoSet,
) -> Result<(), ValidationError> {
    validate_block(block, height, network)?;
    if block.header.previous_block_hash != expected_previous_hash {
        return Err(ValidationError::BlockParentHashMismatch);
    }

    let expected_target = pow::target_for_height(network, height);
    if !cfg!(test) {
        if block.header.difficulty_target_or_bits != expected_target {
            return Err(ValidationError::BlockTargetOutOfBounds);
        }
        if !pow::meets_target(&block.header.block_hash(), &expected_target) {
            return Err(ValidationError::ProofOfWorkInvalid);
        }
    }

    let subsidy = subsidy::block_subsidy_atoms(height);
    if block.transactions.is_empty() {
        return Err(ValidationError::EmptyBlock);
    }
    validate_coinbase_transaction(
        &block.transactions[0],
        subsidy.saturating_add(block.fees_miner_atoms),
    )?;

    if block.transactions.len() == 1 {
        return Ok(());
    }

    let block_witness_root = block.witness_root;
    let mut seen_inputs = BTreeSet::new();
    let mut sum_fees = 0u64;

    for tx in &block.transactions[1..] {
        let fee_rate = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let fee = validate_transaction_with_context(tx, fee_rate, height, |txid, output_index| {
            utxos.get(*txid, output_index).cloned()
        })?;

        for input in &tx.inputs {
            if !seen_inputs.insert((input.previous_txid, input.output_index)) {
                return Err(ValidationError::MempoolConflict);
            }
            utxos
                .remove(input.previous_txid, input.output_index)
                .map_err(|_| ValidationError::MissingUtxo)?;
        }

        let witness = tx
            .witness_payload()
            .ok_or(ValidationError::InvalidWitness)?;
        for (index, input_ref) in witness.input_refs.iter().enumerate() {
            let expected_commit =
                derive_witness_commit_ref(&tx.txid(), &block_witness_root, index as u32);
            if input_ref.witness_commit_ref != expected_commit {
                return Err(ValidationError::WitnessInputReferenceMismatch);
            }
        }

        sum_fees = sum_fees.saturating_add(fee);
        for (output_index, output) in tx.outputs.iter().enumerate() {
            utxos
                .insert(UtxoEntry::new(
                    network,
                    tx.txid(),
                    output_index as u32,
                    output.value_atoms,
                    output.locking_script.clone(),
                    height,
                    tx.is_coinbase(),
                ))
                .map_err(|_| ValidationError::MempoolConflict)?;
        }
    }

    if sum_fees != block.fees_total_atoms {
        return Err(ValidationError::FeeMismatch);
    }
    if block.fees_total_atoms
        != block.fees_miner_atoms + block.fees_burned_atoms + block.fees_pool_atoms
    {
        return Err(ValidationError::FeeMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::{self, witness_root, Block, BlockHeader};
    use atho_core::consensus::pow;
    use atho_core::constants::{
        COINBASE_MATURITY_BLOCKS, MIN_TX_FEE_PER_VBYTE_ATOMS, STANDARD_TX_CONFIRMATIONS,
    };
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{generate_from_seed, sign};
    use atho_storage::utxo::UtxoEntry;

    fn witness_bytes_for_tx(tx: &Transaction) -> Vec<u8> {
        let signature = vec![9; FALCON_512_SIGNATURE_BYTES];
        let pubkey = vec![8; FALCON_512_PUBLIC_KEY_BYTES];
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
        TxWitness {
            signature: signature.clone(),
            pubkey,
            input_refs: (0..tx.inputs.len())
                .map(|index| WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
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
    fn transaction_signature_verifies_against_signed_digest() {
        let keypair = generate_from_seed(b"atho-signature-test").expect("falcon keypair");
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [3; 48],
                output_index: 0,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![4, 5, 6],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &transaction_signing_digest(&tx),
        )
        .expect("signature");
        let witness = TxWitness {
            signature: signature.0.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: vec![WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature.0, 0),
                witness_commit_ref: [0; 16],
            }],
        };
        let signed = Transaction {
            witness: witness.canonical_bytes(),
            ..tx
        };

        assert_eq!(verify_transaction_signature(&signed), Ok(()));
    }

    #[test]
    fn input_reference_has_fixed_collision_resistant_size() {
        let signature = vec![9; FALCON_512_SIGNATURE_BYTES];
        let first = derive_sig_ref_short(&[3; 48], &signature, 7);
        let same = derive_sig_ref_short(&[3; 48], &signature, 7);
        let different = derive_sig_ref_short(&[3; 48], &signature, 8);
        assert_eq!(first.len(), 2);
        assert_eq!(first, same);
        assert_ne!(first, different);
        assert_ne!(first, [0; 2]);
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
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let minimum_fee = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;

        assert_eq!(
            validate_transaction(&tx, minimum_fee - 1),
            Err(ValidationError::FeeBelowMinimum)
        );
        assert_eq!(validate_transaction(&tx, minimum_fee), Ok(()));
    }

    #[test]
    fn coinbase_validation_enforces_reward() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(0),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };

        assert_eq!(
            validate_coinbase_transaction(&tx, subsidy::block_subsidy_atoms(0)),
            Ok(())
        );
        assert_eq!(
            validate_coinbase_transaction(&tx, subsidy::block_subsidy_atoms(0) + 1),
            Err(ValidationError::CoinbaseRewardMismatch)
        );
    }

    #[test]
    fn block_validation_rejects_immature_coinbase_spends() {
        let immature = UtxoEntry::coinbase(
            Network::Mainnet,
            [7; 48],
            0,
            subsidy::block_subsidy_atoms(0),
            vec![1],
            0,
        );
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: immature.txid,
                output_index: immature.output_index,
                unlocking_script: immature.locking_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: immature.value_atoms.saturating_sub(1_000_000),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let fee_atoms = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let tx = Transaction {
            outputs: vec![TxOutput {
                value_atoms: immature.value_atoms.saturating_sub(fee_atoms),
                locking_script: vec![2],
            }],
            ..tx
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };

        let err = validate_transaction_with_context(
            &tx,
            fee_atoms,
            COINBASE_MATURITY_BLOCKS - 2,
            |_, _| Some(immature.clone()),
        )
        .unwrap_err();
        assert_eq!(err, ValidationError::InsufficientConfirmations);
    }

    #[test]
    fn transaction_validation_rejects_immature_standard_spends() {
        let mature_height = 10;
        let utxo = UtxoEntry::new(
            Network::Mainnet,
            [9; 48],
            0,
            25_000_000,
            vec![1],
            mature_height,
            false,
        );
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: utxo.txid,
                output_index: utxo.output_index,
                unlocking_script: utxo.locking_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: utxo.value_atoms.saturating_sub(1_000),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let fee_atoms = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;

        let spend_height = mature_height + STANDARD_TX_CONFIRMATIONS - 2;
        let err = validate_transaction_with_context(&tx, fee_atoms, spend_height, |_, _| {
            Some(utxo.clone())
        })
        .unwrap_err();
        assert_eq!(err, ValidationError::InsufficientConfirmations);
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
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };

        assert_eq!(
            validate_transaction(&tx, tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS),
            Err(ValidationError::ZeroValueOutput)
        );
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
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };

        assert_eq!(
            validate_transaction(&tx, tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS),
            Err(ValidationError::TransactionTooLarge)
        );
    }

    #[test]
    fn block_validation_checks_root_and_payloads() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(0),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let header = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 0,
            previous_block_hash: [0; 48],
            merkle_root: block::merkle_root(std::slice::from_ref(&tx)),
            witness_root: witness_root(std::slice::from_ref(&tx)),
            timestamp: 75,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        let mut block = Block::new(header, vec![tx]);
        block.fees_miner_atoms = 0;

        assert_eq!(validate_block(&block, 0, Network::Mainnet), Ok(()));
    }

    #[test]
    fn block_validation_rejects_timestamp_warp() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let header = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [0; 48],
            merkle_root: block::merkle_root(std::slice::from_ref(&tx)),
            witness_root: witness_root(std::slice::from_ref(&tx)),
            timestamp: 76,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        let mut block = Block::new(header, vec![tx]);
        block.fees_miner_atoms = 0;

        assert_eq!(
            validate_block(&block, 1, Network::Mainnet),
            Err(ValidationError::InvalidBlockTimestamp)
        );
    }

    #[test]
    fn block_validation_scales_over_multiple_transactions() {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
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
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let transactions = vec![coinbase, tx.clone(), tx.clone(), tx];
        let header = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [0; 48],
            merkle_root: block::merkle_root(&transactions),
            witness_root: witness_root(&transactions),
            timestamp: 75,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        let mut block = Block::new(header, transactions);
        block.fees_miner_atoms = 0;

        assert_eq!(validate_block(&block, 1, Network::Mainnet), Ok(()));
    }

    #[test]
    fn block_validation_rejects_bad_witness_commitment() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![2],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let header = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [0; 48],
            merkle_root: block::merkle_root(std::slice::from_ref(&tx)),
            witness_root: witness_root(std::slice::from_ref(&tx)),
            timestamp: 75,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 0,
        };
        let mut block = Block::new(header, vec![tx]);
        block.witness_root = [9; 48];

        assert_eq!(
            validate_block(&block, 1, Network::Mainnet),
            Err(ValidationError::BlockWitnessRootMismatch)
        );
    }

    #[test]
    fn compact_witness_refs_are_context_bound_not_global() {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(0),
                locking_script: vec![9],
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
            witness: vec![],
        };
        let tx = Transaction {
            witness: witness_bytes_for_tx(&tx),
            ..tx
        };
        let fee_atoms = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let mut coinbase = coinbase;
        coinbase.outputs[0].value_atoms = subsidy::block_subsidy_atoms(1).saturating_add(fee_atoms);

        let mut block_a = Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [1; 48],
                merkle_root: block::merkle_root(&[coinbase.clone(), tx.clone()]),
                witness_root: witness_root(&[coinbase.clone(), tx.clone()]),
                timestamp: 75,
                difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            },
            vec![coinbase.clone(), tx.clone()],
        );
        let mut block_b = Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [2; 48],
                merkle_root: block::merkle_root(&[coinbase.clone(), tx.clone()]),
                witness_root: witness_root(&[coinbase.clone(), tx.clone()]),
                timestamp: 75,
                difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
                nonce: 0,
            },
            vec![coinbase, tx],
        );

        block_a.fees_miner_atoms = fee_atoms;
        block_b.fees_miner_atoms = fee_atoms;
        assert_eq!(
            validate_block_without_pow(&block_a, 1, Network::Mainnet),
            Ok(())
        );
        assert_eq!(
            validate_block_without_pow(&block_b, 1, Network::Mainnet),
            Ok(())
        );

        block_b.witness_root = block_a.witness_root;
        assert_eq!(
            validate_block_without_pow(&block_b, 1, Network::Mainnet),
            Ok(())
        );
    }
}
