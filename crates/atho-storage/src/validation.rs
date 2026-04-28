use crate::utxo::{UtxoEntry, UtxoSet};
use atho_core::block::Block;
use atho_core::consensus::rules;
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::{
    MAX_BLOCK_SIZE_BYTES, MAX_BLOCK_WEIGHT, MAX_TRANSACTION_SIZE_BYTES, MIN_TX_FEE_PER_VBYTE_ATOMS,
};
use atho_core::crypto::hash::sha3_256;
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use atho_crypto::falcon::{
    self, FalconPublicKey, FalconSignature, FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES,
};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ValidationError {
    #[error("transaction has no inputs")]
    NoInputs,
    #[error("transaction has no outputs")]
    NoOutputs,
    #[error("fee below policy minimum")]
    FeeBelowMinimum,
    #[error("transaction too large")]
    TransactionTooLarge,
    #[error("invalid transaction version")]
    InvalidTransactionVersion,
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
    validate_transaction_for_height(tx, fee_atoms, 0)
}

pub fn validate_transaction_for_height(
    tx: &Transaction,
    fee_atoms: u64,
    height: u64,
) -> Result<(), ValidationError> {
    if tx.is_coinbase() {
        return Err(ValidationError::NoInputs);
    }
    if !rules::is_supported_transaction_version(tx.version, height) {
        return Err(ValidationError::InvalidTransactionVersion);
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
    verify_transaction_signature(tx)?;
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
    validate_transaction_for_height(tx, fee_atoms, spend_height)?;
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
    height: u64,
) -> Result<(), ValidationError> {
    if !tx.is_coinbase() {
        return Err(ValidationError::InvalidCoinbase);
    }
    if !rules::is_supported_transaction_version(tx.version, height) {
        return Err(ValidationError::InvalidTransactionVersion);
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
    if !rules::is_supported_block_version(block.header.version, height) {
        return Err(ValidationError::InvalidBlockVersion);
    }
    if block.header.network_id != network {
        return Err(ValidationError::BlockNetworkMismatch);
    }
    if block.header.height != height {
        return Err(ValidationError::InvalidBlockHeight);
    }
    if block.header.timestamp == 0 {
        return Err(ValidationError::InvalidBlockTimestamp);
    }
    if block.vsize_bytes() > MAX_BLOCK_SIZE_BYTES || block.weight_bytes() > MAX_BLOCK_WEIGHT {
        return Err(ValidationError::BlockTooLarge);
    }
    if block.merkle_root() != block.header.merkle_root {
        return Err(ValidationError::BlockMerkleRootMismatch);
    }
    let computed_witness_root = block.compute_witness_root();
    if computed_witness_root != block.header.witness_root {
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
    validate_coinbase_transaction(
        &block.transactions[0],
        subsidy.saturating_add(block.fees_miner_atoms),
        height,
    )?;
    if block.transactions.len() > 1 {
        for tx in &block.transactions[1..] {
            validate_transaction_for_height(
                tx,
                tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS,
                height,
            )?;
        }
    }
    Ok(())
}

pub fn validate_block(block: &Block, height: u64, network: Network) -> Result<(), ValidationError> {
    validate_block_impl(block, height, network, false)
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
    expected_target: [u8; 48],
    previous_blocks: &[Block],
    mut utxos: UtxoSet,
) -> Result<(), ValidationError> {
    validate_block(block, height, network)?;
    if block.header.previous_block_hash != expected_previous_hash {
        return Err(ValidationError::BlockParentHashMismatch);
    }

    if let Some(minimum_timestamp) = pow::minimum_next_block_timestamp(previous_blocks) {
        if block.header.timestamp < minimum_timestamp {
            return Err(ValidationError::InvalidBlockTimestamp);
        }
    }

    if block.header.difficulty_target_or_bits != expected_target {
        return Err(ValidationError::BlockTargetOutOfBounds);
    }
    if !pow::meets_target(&block.header.block_hash(), &expected_target) {
        return Err(ValidationError::ProofOfWorkInvalid);
    }

    validate_coinbase_transaction(
        &block.transactions[0],
        subsidy::block_subsidy_atoms(height).saturating_add(block.fees_miner_atoms),
        height,
    )?;

    let block_witness_root = block.header.witness_root;
    let mut seen_inputs = BTreeSet::new();
    let mut sum_fees = 0u64;

    for (tx_index, tx) in block.transactions.iter().enumerate() {
        if tx_index == 0 {
            continue;
        }

        let txid = tx.txid();
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
                derive_witness_commit_ref(&txid, &block_witness_root, index as u32);
            if input_ref.witness_commit_ref != expected_commit {
                return Err(ValidationError::WitnessInputReferenceMismatch);
            }
        }

        sum_fees = sum_fees.saturating_add(fee);
        for (output_index, output) in tx.outputs.iter().enumerate() {
            utxos
                .insert(UtxoEntry::new(
                    network,
                    txid,
                    output_index as u32,
                    output.value_atoms,
                    output.locking_script.clone(),
                    height,
                    false,
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
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::rules::BLOCK_VERSION_V2_PLACEHOLDER;
    use atho_core::crypto::hash::sha3_384;
    use atho_core::transaction::{Transaction, TxOutput};

    fn solve_block(mut block: Block) -> Block {
        let prefix = block.header.canonical_bytes_without_nonce();
        let target = block.header.difficulty_target_or_bits;
        for nonce in 0u64..=u32::MAX as u64 {
            let mut bytes = Vec::with_capacity(prefix.len() + 8);
            bytes.extend_from_slice(&prefix);
            bytes.extend_from_slice(&nonce.to_le_bytes());
            if pow::meets_target(&sha3_384(&bytes), &target) {
                block.header.nonce = nonce;
                return block;
            }
        }
        panic!("failed to solve test block");
    }

    #[test]
    fn header_witness_root_must_match_body_commitment() {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![1],
            }],
            lock_time: 1,
            witness: vec![],
        };
        let transactions = vec![coinbase];
        let mut block = Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 1,
                difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
                nonce: 0,
            },
            transactions,
        );
        block.header.witness_root[0] ^= 0xff;

        assert_eq!(
            validate_block_without_pow(&block, 1, Network::Mainnet),
            Err(ValidationError::BlockWitnessRootMismatch)
        );
    }

    #[test]
    fn future_block_version_is_rejected_before_activation() {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![1],
            }],
            lock_time: 1,
            witness: vec![],
        };
        let transactions = vec![coinbase];
        let block = Block::new(
            BlockHeader {
                version: BLOCK_VERSION_V2_PLACEHOLDER,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 1,
                difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
                nonce: 0,
            },
            transactions,
        );

        assert_eq!(
            validate_block_without_pow(&block, 1, Network::Mainnet),
            Err(ValidationError::InvalidBlockVersion)
        );
    }

    #[test]
    fn contextual_validation_rejects_unexpected_target() {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![1],
            }],
            lock_time: 1,
            witness: vec![],
        };
        let transactions = vec![coinbase];
        let initial_target = pow::initial_target_for_network(Network::Mainnet);
        let block = solve_block(Block::new(
            BlockHeader {
                version: 1,
                network_id: Network::Mainnet,
                height: 1,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 1,
                difficulty_target_or_bits: initial_target,
                nonce: 0,
            },
            transactions,
        ));
        let mut wrong_target = initial_target;
        wrong_target[0] ^= 0xff;

        assert_eq!(
            validate_block_with_context(
                &block,
                1,
                Network::Mainnet,
                [0; 48],
                wrong_target,
                &[],
                UtxoSet::new(Network::Mainnet),
            ),
            Err(ValidationError::BlockTargetOutOfBounds)
        );
    }
}
