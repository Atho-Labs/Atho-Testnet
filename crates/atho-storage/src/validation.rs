//! Transaction and block validation against chainstate context.
//!
//! This module bridges the pure protocol types in `atho-core` with the live
//! UTXO set so the node can validate transactions for mempool admission and
//! blocks for chain acceptance.
//!
//! SECURITY: Duplicate inputs, missing UTXOs, immature coinbase spends, wrong
//! witness references, and wrong-network blocks are rejected here before the
//! chainstate mutates.
use crate::utxo::{UtxoEntry, UtxoSet};
use atho_core::address::public_key_digest;
use atho_core::block::Block;
use atho_core::consensus::rules;
use atho_core::consensus::signatures::{
    transaction_signing_digest, transaction_signing_digest_for_input_indexes, AthoSignatureDomain,
};
use atho_core::consensus::tx_policy::{
    maximum_standard_outputs, minimum_output_amount_atoms, minimum_required_fee_atoms,
    required_tx_pow_bits, transaction_pow_is_valid_for_bits,
};
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::{
    ADDRESS_DIGEST_BYTES, DUST_RELAY_VALUE_ATOMS, FALCON_512_PUBLIC_KEY_BYTES,
    FALCON_512_SIGNATURE_BYTES, MAX_BLOCK_RAW_BYTES, MAX_BLOCK_VBYTES, MAX_BLOCK_WEIGHT,
    MAX_TRANSACTION_RAW_BYTES, MAX_TRANSACTION_VBYTES,
};
use atho_core::crypto::hash::sha3_256;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxWitness, WitnessInputRef, WitnessSignerGroup};
use atho_crypto::falcon::{self, FalconPublicKey, FalconSignature};
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, BLK_BLOCK_TOO_LARGE, BLK_COINBASE_REWARD_MISMATCH,
    BLK_DUPLICATE_TRANSACTION_ID, BLK_EMPTY_BLOCK, BLK_INVALID_COINBASE, BLK_INVALID_HEIGHT,
    BLK_INVALID_TIMESTAMP, BLK_INVALID_VERSION, BLK_MERKLE_ROOT_MISMATCH, BLK_MULTIPLE_COINBASE,
    BLK_PARENT_HASH_MISMATCH, BLK_POW_INVALID, BLK_TARGET_OUT_OF_BOUNDS, BLK_WITNESS_ROOT_MISMATCH,
    MEM_DUST_OUTPUT, MEM_MEMPOOL_CONFLICT, NET_BLOCK_NETWORK_MISMATCH, SIG_INVALID_WITNESS,
    SIG_WITNESS_INPUT_REF_MISMATCH, TX_DUPLICATE_INPUT, TX_FEE_BELOW_MINIMUM, TX_FEE_MISMATCH,
    TX_INPUT_OWNERSHIP_MISMATCH, TX_INSUFFICIENT_CONFIRMATIONS, TX_INVALID_POW_NONCE,
    TX_INVALID_VERSION, TX_MISSING_UTXO, TX_NO_INPUTS, TX_NO_OUTPUTS, TX_TOO_LARGE,
    TX_TOO_MANY_OUTPUTS, TX_WRONG_POW_BITS, TX_ZERO_VALUE_OUTPUT,
};
use rayon::prelude::*;
use std::collections::BTreeSet;
use thiserror::Error;

/// Validation failures raised while checking transactions or blocks.
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
    #[error("dust output below relay policy minimum")]
    DustOutput,
    #[error("transaction has too many outputs")]
    TooManyOutputs,
    #[error("invalid witness")]
    InvalidWitness,
    #[error("wrong transaction pow bits")]
    WrongTransactionPowBits,
    #[error("invalid transaction pow nonce")]
    InvalidTransactionPowNonce,
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
    #[error("duplicate transaction id")]
    DuplicateTransactionId,
    #[error("missing utxo")]
    MissingUtxo,
    #[error("input ownership mismatch")]
    InputOwnershipMismatch,
    #[error("input has insufficient confirmations")]
    InsufficientConfirmations,
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

struct PreparedTransactionValidation {
    signer_groups: Vec<PreparedSignerGroup>,
    signer_group_by_input: Vec<usize>,
}

#[derive(Clone)]
struct PreparedSignerGroup {
    signature: Vec<u8>,
    pubkey: Vec<u8>,
    input_refs: Vec<WitnessInputRef>,
}

impl AthoErrorMeta for ValidationError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::NoInputs => &TX_NO_INPUTS,
            Self::NoOutputs => &TX_NO_OUTPUTS,
            Self::FeeBelowMinimum => &TX_FEE_BELOW_MINIMUM,
            Self::TransactionTooLarge => &TX_TOO_LARGE,
            Self::InvalidTransactionVersion => &TX_INVALID_VERSION,
            Self::DuplicateInput => &TX_DUPLICATE_INPUT,
            Self::ZeroValueOutput => &TX_ZERO_VALUE_OUTPUT,
            Self::DustOutput => &MEM_DUST_OUTPUT,
            Self::TooManyOutputs => &TX_TOO_MANY_OUTPUTS,
            Self::InvalidWitness => &SIG_INVALID_WITNESS,
            Self::WrongTransactionPowBits => &TX_WRONG_POW_BITS,
            Self::InvalidTransactionPowNonce => &TX_INVALID_POW_NONCE,
            Self::InvalidCoinbase => &BLK_INVALID_COINBASE,
            Self::CoinbaseRewardMismatch => &BLK_COINBASE_REWARD_MISMATCH,
            Self::EmptyBlock => &BLK_EMPTY_BLOCK,
            Self::BlockTooLarge => &BLK_BLOCK_TOO_LARGE,
            Self::BlockMerkleRootMismatch => &BLK_MERKLE_ROOT_MISMATCH,
            Self::BlockWitnessRootMismatch => &BLK_WITNESS_ROOT_MISMATCH,
            Self::BlockTargetOutOfBounds => &BLK_TARGET_OUT_OF_BOUNDS,
            Self::ProofOfWorkInvalid => &BLK_POW_INVALID,
            Self::BlockParentHashMismatch => &BLK_PARENT_HASH_MISMATCH,
            Self::DuplicateTransactionId => &BLK_DUPLICATE_TRANSACTION_ID,
            Self::MissingUtxo => &TX_MISSING_UTXO,
            Self::InputOwnershipMismatch => &TX_INPUT_OWNERSHIP_MISMATCH,
            Self::InsufficientConfirmations => &TX_INSUFFICIENT_CONFIRMATIONS,
            Self::WitnessInputReferenceMismatch => &SIG_WITNESS_INPUT_REF_MISMATCH,
            Self::FeeMismatch => &TX_FEE_MISMATCH,
            Self::MempoolConflict => &MEM_MEMPOOL_CONFLICT,
            Self::InvalidBlockHeight => &BLK_INVALID_HEIGHT,
            Self::InvalidBlockVersion => &BLK_INVALID_VERSION,
            Self::InvalidBlockTimestamp => &BLK_INVALID_TIMESTAMP,
            Self::BlockNetworkMismatch => &NET_BLOCK_NETWORK_MISMATCH,
            Self::MultipleCoinbaseTransactions => &BLK_MULTIPLE_COINBASE,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-storage::validation"
    }
}

/// Derives the short witness reference committed per input.
///
/// CONSENSUS: Both signers and validators must derive the same two-byte tag for
/// the same txid, signature, and input index.
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

/// Derives the witness-commitment reference that binds an input to a block.
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

/// Rebuilds the transaction witness references against the selected block root.
pub fn finalize_witness_commit_refs(tx: &Transaction, block_witness_root: [u8; 48]) -> Transaction {
    let Some(mut witness) = tx.witness_payload() else {
        return tx.clone();
    };
    let txid = tx.txid();
    witness.for_each_input_ref_mut(|input_ref| {
        input_ref.witness_commit_ref =
            derive_witness_commit_ref(&txid, &block_witness_root, input_ref.input_index);
    });
    Transaction {
        witness: witness.canonical_bytes(),
        ..tx.clone()
    }
}

/// Verifies the Falcon witness signature carried by a transaction.
///
/// CONSENSUS: This check validates the canonical Atho signing digest. If wallet
/// signing and validator verification ever disagree on the digest bytes, spends
/// will fail network-wide.
pub fn verify_transaction_signature(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.is_coinbase() {
        return Err(ValidationError::NoInputs);
    }
    let witness = tx
        .witness_payload()
        .ok_or(ValidationError::InvalidWitness)?;
    verify_witness_signer_groups(tx, &witness)?;
    Ok(())
}

fn verify_transaction_signature_prepared(
    tx: &Transaction,
    prepared: PreparedTransactionValidation,
) -> Result<(), ValidationError> {
    for signer_group in prepared.signer_groups {
        if signer_group.pubkey.len() != FALCON_512_PUBLIC_KEY_BYTES {
            return Err(ValidationError::InvalidWitness);
        }
        if signer_group.signature.len() != FALCON_512_SIGNATURE_BYTES {
            return Err(ValidationError::InvalidWitness);
        }
        let input_indexes = signer_group
            .input_refs
            .iter()
            .map(|input_ref| input_ref.input_index)
            .collect::<Vec<_>>();
        let signing_digest = if input_indexes.len() == tx.inputs.len() {
            transaction_signing_digest(tx)
        } else {
            transaction_signing_digest_for_input_indexes(tx, &input_indexes)
        };
        let verified = falcon::verify(
            AthoSignatureDomain::Transaction,
            &FalconPublicKey(signer_group.pubkey),
            &signing_digest,
            &FalconSignature(signer_group.signature),
        )
        .map_err(|_| ValidationError::InvalidWitness)?;
        if !verified {
            return Err(ValidationError::InvalidWitness);
        }
    }
    Ok(())
}

fn witness_signer_groups(witness: &TxWitness) -> Vec<WitnessSignerGroup> {
    if witness.is_empty() {
        return Vec::new();
    }
    let mut groups = Vec::with_capacity(1 + witness.additional_signers.len());
    groups.push(WitnessSignerGroup {
        signature: witness.signature.clone(),
        pubkey: witness.pubkey.clone(),
        input_refs: witness.input_refs.clone(),
    });
    groups.extend(witness.additional_signers.clone());
    groups
}

fn verify_witness_signer_groups(
    tx: &Transaction,
    witness: &TxWitness,
) -> Result<(), ValidationError> {
    for signer_group in witness_signer_groups(witness) {
        if signer_group.pubkey.len() != FALCON_512_PUBLIC_KEY_BYTES {
            return Err(ValidationError::InvalidWitness);
        }
        if signer_group.signature.len() != FALCON_512_SIGNATURE_BYTES {
            return Err(ValidationError::InvalidWitness);
        }
        let input_indexes = signer_group
            .input_refs
            .iter()
            .map(|input_ref| input_ref.input_index)
            .collect::<Vec<_>>();
        let signing_digest = if input_indexes.len() == tx.inputs.len() {
            transaction_signing_digest(tx)
        } else {
            transaction_signing_digest_for_input_indexes(tx, &input_indexes)
        };
        let verified = falcon::verify(
            AthoSignatureDomain::Transaction,
            &FalconPublicKey(signer_group.pubkey),
            &signing_digest,
            &FalconSignature(signer_group.signature),
        )
        .map_err(|_| ValidationError::InvalidWitness)?;
        if !verified {
            return Err(ValidationError::InvalidWitness);
        }
    }
    Ok(())
}

fn prepare_transaction_validation(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    height: u64,
    schedule: &[rules::ScheduledActivation],
) -> Result<PreparedTransactionValidation, ValidationError> {
    if tx.is_coinbase() {
        return Err(ValidationError::NoInputs);
    }
    let supported_version =
        rules::is_supported_transaction_version_with_schedule(tx.version, height, schedule);
    if !supported_version {
        return Err(ValidationError::InvalidTransactionVersion);
    }
    if tx.outputs.is_empty() {
        return Err(ValidationError::NoOutputs);
    }
    if tx.outputs.len() > maximum_standard_outputs(network, tx) {
        return Err(ValidationError::TooManyOutputs);
    }
    let raw_size_bytes = tx.full_size_bytes();
    let vsize_bytes = tx.vsize_bytes();
    if raw_size_bytes > MAX_TRANSACTION_RAW_BYTES || vsize_bytes > MAX_TRANSACTION_VBYTES {
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
    let minimum_fee = minimum_required_fee_atoms(network, tx);
    if fee_atoms < minimum_fee {
        return Err(ValidationError::FeeBelowMinimum);
    }
    let dust_floor = minimum_output_amount_atoms(network, tx);
    if tx
        .outputs
        .iter()
        .any(|output| output.value_atoms < dust_floor)
    {
        return Err(ValidationError::DustOutput);
    }
    let witness = tx
        .witness_payload()
        .ok_or(ValidationError::InvalidWitness)?;
    if witness.signer_group_count() == 0 {
        return Err(ValidationError::InvalidWitness);
    }
    if witness.total_input_refs() != tx.inputs.len() {
        return Err(ValidationError::InvalidWitness);
    }
    let txid = tx.txid();
    let signer_groups = witness_signer_groups(&witness);
    let mut signer_group_by_input = vec![usize::MAX; tx.inputs.len()];
    let mut previous_first_input = None;

    for (group_index, signer_group) in signer_groups.iter().enumerate() {
        if signer_group.signature.is_empty()
            || signer_group.pubkey.is_empty()
            || signer_group.input_refs.is_empty()
        {
            return Err(ValidationError::InvalidWitness);
        }
        let mut previous_input_index = None;
        let first_input_index = signer_group
            .input_refs
            .first()
            .map(|input_ref| input_ref.input_index)
            .ok_or(ValidationError::InvalidWitness)?;
        if previous_first_input.is_some_and(|previous| first_input_index <= previous) {
            return Err(ValidationError::InvalidWitness);
        }
        previous_first_input = Some(first_input_index);

        for input_ref in &signer_group.input_refs {
            let input_index = input_ref.input_index as usize;
            if input_index >= tx.inputs.len() {
                return Err(ValidationError::InvalidWitness);
            }
            if previous_input_index.is_some_and(|previous| input_ref.input_index <= previous) {
                return Err(ValidationError::InvalidWitness);
            }
            previous_input_index = Some(input_ref.input_index);
            if signer_group_by_input[input_index] != usize::MAX {
                return Err(ValidationError::WitnessInputReferenceMismatch);
            }
            let expected_short =
                derive_sig_ref_short(&txid, &signer_group.signature, input_ref.input_index);
            if input_ref.sig_ref_short != expected_short {
                return Err(ValidationError::WitnessInputReferenceMismatch);
            }
            signer_group_by_input[input_index] = group_index;
        }
    }
    if signer_group_by_input
        .iter()
        .any(|group_index| *group_index == usize::MAX)
    {
        return Err(ValidationError::WitnessInputReferenceMismatch);
    }
    Ok(PreparedTransactionValidation {
        signer_groups: signer_groups
            .into_iter()
            .map(|signer_group| PreparedSignerGroup {
                signature: signer_group.signature,
                pubkey: signer_group.pubkey,
                input_refs: signer_group.input_refs,
            })
            .collect(),
        signer_group_by_input,
    })
}

fn locking_script_matches_public_key(
    network: Network,
    locking_script: &[u8],
    public_key: &[u8],
) -> bool {
    // Standard payment outputs use a 32-byte address digest, so bind those
    // outputs to the witness public key. Legacy/test script forms keep the
    // previous exact-script behavior.
    if locking_script.len() == ADDRESS_DIGEST_BYTES {
        public_key_digest(network, public_key).as_slice() == locking_script
    } else {
        true
    }
}

/// Performs context-free transaction validation with a caller-supplied fee.
pub fn validate_transaction(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
) -> Result<(), ValidationError> {
    validate_transaction_for_height(tx, fee_atoms, network, 0)
}

/// Returns `true` when a transaction contains relay-policy dust outputs.
pub fn transaction_contains_dust_outputs(tx: &Transaction) -> bool {
    tx.outputs
        .iter()
        .any(|output| output.value_atoms > 0 && output.value_atoms < DUST_RELAY_VALUE_ATOMS)
}

/// Enforces standard relay policy that is stricter than bare consensus.
///
/// POLICY: Dust output rejection is applied for mempool admission and wallet
/// construction, not by itself as a historical block-validity rule.
pub fn validate_transaction_standard_policy(
    tx: &Transaction,
    network: Network,
) -> Result<(), ValidationError> {
    let dust_floor = minimum_output_amount_atoms(network, tx);
    if tx
        .outputs
        .iter()
        .any(|output| output.value_atoms > 0 && output.value_atoms < dust_floor)
    {
        return Err(ValidationError::DustOutput);
    }
    Ok(())
}

/// Validates transaction structure against the active rule schedule.
pub fn validate_transaction_for_height(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    height: u64,
) -> Result<(), ValidationError> {
    validate_transaction_for_height_with_schedule(
        tx,
        fee_atoms,
        network,
        height,
        &rules::SCHEDULED_ACTIVATIONS,
    )
}

/// Performs context-free structural checks before UTXO lookup.
pub fn validate_transaction_structure_for_height_with_schedule(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    height: u64,
    schedule: &[rules::ScheduledActivation],
) -> Result<(), ValidationError> {
    let _ = prepare_transaction_validation(tx, fee_atoms, network, height, schedule)?;
    Ok(())
}

/// Performs context-free validation and Falcon signature verification.
pub fn validate_transaction_for_height_with_schedule(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    height: u64,
    schedule: &[rules::ScheduledActivation],
) -> Result<(), ValidationError> {
    let prepared = prepare_transaction_validation(tx, fee_atoms, network, height, schedule)?;
    let required_bits = required_tx_pow_bits(network, tx, fee_atoms);
    if tx.tx_pow_bits != required_bits {
        return Err(ValidationError::WrongTransactionPowBits);
    }
    if !transaction_pow_is_valid_for_bits(network, tx, required_bits) {
        return Err(ValidationError::InvalidTransactionPowNonce);
    }
    verify_transaction_signature_prepared(tx, prepared)?;
    Ok(())
}

/// Validates a transaction against UTXO context and checks fee exactness.
pub fn validate_transaction_with_context_structure_and_schedule<F>(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    spend_height: u64,
    lookup: F,
    schedule: &[rules::ScheduledActivation],
) -> Result<u64, ValidationError>
where
    F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
{
    let prepared = prepare_transaction_validation(tx, fee_atoms, network, spend_height, schedule)?;
    let actual_fee = validate_transaction_with_context_common_and_schedule(
        tx,
        &prepared,
        network,
        spend_height,
        lookup,
        schedule,
    )?;
    if actual_fee != fee_atoms {
        return Err(ValidationError::FeeMismatch);
    }
    let required_bits = required_tx_pow_bits(network, tx, actual_fee);
    if tx.tx_pow_bits != required_bits {
        return Err(ValidationError::WrongTransactionPowBits);
    }
    if !transaction_pow_is_valid_for_bits(network, tx, required_bits) {
        return Err(ValidationError::InvalidTransactionPowNonce);
    }
    Ok(actual_fee)
}

/// Validates a transaction against UTXO context with a minimum fee floor.
pub fn validate_transaction_with_context_minimum_fee_and_schedule<F>(
    tx: &Transaction,
    minimum_fee_atoms: u64,
    network: Network,
    spend_height: u64,
    lookup: F,
    schedule: &[rules::ScheduledActivation],
) -> Result<u64, ValidationError>
where
    F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
{
    let prepared =
        prepare_transaction_validation(tx, minimum_fee_atoms, network, spend_height, schedule)?;
    let actual_fee = validate_transaction_with_context_common_and_schedule(
        tx,
        &prepared,
        network,
        spend_height,
        lookup,
        schedule,
    )?;
    if actual_fee < minimum_fee_atoms {
        return Err(ValidationError::FeeBelowMinimum);
    }
    let required_bits = required_tx_pow_bits(network, tx, actual_fee);
    if tx.tx_pow_bits != required_bits {
        return Err(ValidationError::WrongTransactionPowBits);
    }
    if !transaction_pow_is_valid_for_bits(network, tx, required_bits) {
        return Err(ValidationError::InvalidTransactionPowNonce);
    }
    Ok(actual_fee)
}

fn validate_transaction_with_context_common_and_schedule<F>(
    tx: &Transaction,
    prepared: &PreparedTransactionValidation,
    network: Network,
    spend_height: u64,
    mut lookup: F,
    _schedule: &[rules::ScheduledActivation],
) -> Result<u64, ValidationError>
where
    F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
{
    let mut input_total = 0u64;

    for (index, input) in tx.inputs.iter().enumerate() {
        // SECURITY: A missing UTXO or mismatched locking script indicates an
        // attempt to spend coins the witness does not control.
        let utxo =
            lookup(&input.previous_txid, input.output_index).ok_or(ValidationError::MissingUtxo)?;
        if utxo.locking_script != input.unlocking_script {
            return Err(ValidationError::InputOwnershipMismatch);
        }
        if utxo.network != network {
            return Err(ValidationError::InputOwnershipMismatch);
        }
        if !locking_script_matches_public_key(
            utxo.network,
            &utxo.locking_script,
            &prepared.signer_groups[prepared.signer_group_by_input[index]].pubkey,
        ) {
            return Err(ValidationError::InputOwnershipMismatch);
        }
        if !utxo.is_spendable_at(spend_height) {
            return Err(ValidationError::InsufficientConfirmations);
        }
        input_total = input_total
            .checked_add(utxo.value_atoms)
            .ok_or(ValidationError::FeeMismatch)?;
    }

    let output_total = tx
        .checked_output_value_atoms()
        .ok_or(ValidationError::FeeMismatch)?;
    // The fee is the remaining value after all outputs are funded. Checked
    // arithmetic prevents underflow from turning overspends into huge fees.
    let actual_fee = input_total
        .checked_sub(output_total)
        .ok_or(ValidationError::FeeMismatch)?;
    Ok(actual_fee)
}

/// Validates a transaction against chainstate and an exact fee value.
pub fn validate_transaction_with_context<F>(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    spend_height: u64,
    lookup: F,
) -> Result<u64, ValidationError>
where
    F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
{
    validate_transaction_with_context_and_schedule(
        tx,
        fee_atoms,
        network,
        spend_height,
        lookup,
        &rules::SCHEDULED_ACTIVATIONS,
    )
}

/// Applies mempool policy before full chainstate validation.
pub fn validate_transaction_with_context_for_mempool<F>(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    spend_height: u64,
    lookup: F,
) -> Result<u64, ValidationError>
where
    F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
{
    validate_transaction_standard_policy(tx, network)?;
    validate_transaction_with_context(tx, fee_atoms, network, spend_height, lookup)
}

/// Validates a transaction against chainstate and the active rule schedule.
pub fn validate_transaction_with_context_and_schedule<F>(
    tx: &Transaction,
    fee_atoms: u64,
    network: Network,
    spend_height: u64,
    lookup: F,
    schedule: &[rules::ScheduledActivation],
) -> Result<u64, ValidationError>
where
    F: FnMut(&[u8; 48], u32) -> Option<UtxoEntry>,
{
    let prepared = prepare_transaction_validation(tx, fee_atoms, network, spend_height, schedule)?;
    let actual_fee = validate_transaction_with_context_common_and_schedule(
        tx,
        &prepared,
        network,
        spend_height,
        lookup,
        schedule,
    )?;
    if actual_fee != fee_atoms {
        return Err(ValidationError::FeeMismatch);
    }
    let required_bits = required_tx_pow_bits(network, tx, actual_fee);
    if tx.tx_pow_bits != required_bits {
        return Err(ValidationError::WrongTransactionPowBits);
    }
    if !transaction_pow_is_valid_for_bits(network, tx, required_bits) {
        return Err(ValidationError::InvalidTransactionPowNonce);
    }
    verify_transaction_signature_prepared(tx, prepared)?;
    Ok(actual_fee)
}

/// Validates a coinbase transaction against the expected reward.
pub fn validate_coinbase_transaction(
    tx: &Transaction,
    expected_reward_atoms: u64,
    height: u64,
) -> Result<(), ValidationError> {
    validate_coinbase_transaction_with_schedule(
        tx,
        expected_reward_atoms,
        height,
        &rules::SCHEDULED_ACTIVATIONS,
    )
}

/// Validates a coinbase transaction under a caller-supplied rule schedule.
pub fn validate_coinbase_transaction_with_schedule(
    tx: &Transaction,
    expected_reward_atoms: u64,
    height: u64,
    schedule: &[rules::ScheduledActivation],
) -> Result<(), ValidationError> {
    if !tx.is_coinbase() {
        return Err(ValidationError::InvalidCoinbase);
    }
    if !rules::is_supported_transaction_version_with_schedule(tx.version, height, schedule) {
        return Err(ValidationError::InvalidTransactionVersion);
    }
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidCoinbase);
    }
    let output_total = tx
        .checked_output_value_atoms()
        .ok_or(ValidationError::CoinbaseRewardMismatch)?;
    if output_total != expected_reward_atoms {
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
    validate_block_impl_with_schedule(
        block,
        height,
        network,
        skip_pow,
        &rules::SCHEDULED_ACTIVATIONS,
    )
}

fn validate_block_impl_with_schedule(
    block: &Block,
    height: u64,
    network: Network,
    skip_pow: bool,
    schedule: &[rules::ScheduledActivation],
) -> Result<(), ValidationError> {
    if block.transactions.is_empty() {
        return Err(ValidationError::EmptyBlock);
    }
    if !rules::is_supported_block_version_with_schedule(block.header.version, height, schedule) {
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
    let raw_size_bytes = block.full_size_bytes();
    let vsize_bytes = block.vsize_bytes();
    let weight_bytes = block.weight_bytes();
    if raw_size_bytes > MAX_BLOCK_RAW_BYTES
        || vsize_bytes > MAX_BLOCK_VBYTES
        || weight_bytes > MAX_BLOCK_WEIGHT
    {
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

    let subsidy = subsidy::block_subsidy_atoms_for_network(network, height);
    let expected_coinbase_reward = subsidy
        .checked_add(block.fees_miner_atoms)
        .ok_or(ValidationError::CoinbaseRewardMismatch)?;
    validate_coinbase_transaction_with_schedule(
        &block.transactions[0],
        expected_coinbase_reward,
        height,
        schedule,
    )?;
    if !txids_are_unique(&block.transactions) {
        return Err(ValidationError::DuplicateTransactionId);
    }
    if !block_inputs_are_unique(&block.transactions) {
        return Err(ValidationError::MempoolConflict);
    }
    if block.transactions.len() > 1 {
        for tx in &block.transactions[1..] {
            validate_transaction_structure_for_height_with_schedule(
                tx,
                minimum_required_fee_atoms(network, tx),
                network,
                height,
                schedule,
            )?;
        }
    }
    if block.transactions.len() > 1 {
        verify_transaction_signatures_parallel(&block.transactions[1..])?;
    }
    Ok(())
}

fn verify_transaction_signatures_parallel(
    transactions: &[Transaction],
) -> Result<(), ValidationError> {
    let results: Vec<Result<(), ValidationError>> = transactions
        .par_iter()
        .map(verify_transaction_signature)
        .collect();
    for result in results {
        result?;
    }
    Ok(())
}

fn txids_are_unique(transactions: &[Transaction]) -> bool {
    let mut seen = BTreeSet::new();
    transactions
        .iter()
        .map(Transaction::txid)
        .all(|txid| seen.insert(txid))
}

fn block_inputs_are_unique(transactions: &[Transaction]) -> bool {
    let mut seen = BTreeSet::new();
    for tx in transactions {
        for input in &tx.inputs {
            if !seen.insert((input.previous_txid, input.output_index)) {
                return false;
            }
        }
    }
    true
}

/// Validates a block under the current rule schedule and performs PoW checks.
pub fn validate_block(block: &Block, height: u64, network: Network) -> Result<(), ValidationError> {
    validate_block_impl(block, height, network, false)
}

/// Validates a block while skipping PoW checks for internal tooling.
///
/// WARNING(consensus): This helper is for diagnostics and tests. Network
/// acceptance must always validate proof of work.
pub fn validate_block_without_pow(
    block: &Block,
    height: u64,
    network: Network,
) -> Result<(), ValidationError> {
    validate_block_impl(block, height, network, true)
}

/// Validates a block against the expected parent, target, and live UTXO set.
pub fn validate_block_with_context(
    block: &Block,
    height: u64,
    network: Network,
    expected_previous_hash: [u8; 48],
    expected_target: [u8; 48],
    previous_blocks: &[Block],
    utxos: UtxoSet,
) -> Result<(), ValidationError> {
    validate_block_with_context_and_schedule(
        block,
        height,
        network,
        expected_previous_hash,
        expected_target,
        previous_blocks,
        utxos,
        &rules::SCHEDULED_ACTIVATIONS,
    )
}

/// Validates a block against chain context and an explicit activation schedule.
pub fn validate_block_with_context_and_schedule(
    block: &Block,
    height: u64,
    network: Network,
    expected_previous_hash: [u8; 48],
    expected_target: [u8; 48],
    previous_blocks: &[Block],
    mut utxos: UtxoSet,
    schedule: &[rules::ScheduledActivation],
) -> Result<(), ValidationError> {
    validate_block_impl_with_schedule(block, height, network, false, schedule)?;
    // CONSENSUS: The parent hash check binds this block to one exact chain tip.
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

    let block_witness_root = block.header.witness_root;
    // INVARIANT: No input may be spent twice within one block.
    let mut seen_inputs = BTreeSet::new();
    let mut sum_fees = 0u64;

    for (tx_index, tx) in block.transactions.iter().enumerate() {
        if tx_index == 0 {
            continue;
        }

        let txid = tx.txid();
        let fee_rate = minimum_required_fee_atoms(network, tx);
        let fee = validate_transaction_with_context_minimum_fee_and_schedule(
            tx,
            fee_rate,
            network,
            height,
            |txid, output_index| utxos.get(*txid, output_index).cloned(),
            schedule,
        )?;

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
        for signer_group in witness_signer_groups(&witness) {
            for input_ref in signer_group.input_refs {
                let expected_commit =
                    derive_witness_commit_ref(&txid, &block_witness_root, input_ref.input_index);
                if input_ref.witness_commit_ref != expected_commit {
                    return Err(ValidationError::WitnessInputReferenceMismatch);
                }
            }
        }

        sum_fees = sum_fees
            .checked_add(fee)
            .ok_or(ValidationError::FeeMismatch)?;
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
    if block.fees_total_atoms != block.fees_miner_atoms {
        return Err(ValidationError::FeeMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::address::public_key_digest;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::consensus::rules::{
        ScheduledActivation, BLOCK_VERSION_V1, BLOCK_VERSION_V2_PLACEHOLDER, RULESET_VERSION_V1,
        RULESET_VERSION_V2_PLACEHOLDER, TRANSACTION_VERSION_V1, TRANSACTION_VERSION_V2_PLACEHOLDER,
    };
    use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
    use atho_core::consensus::tx_policy::{
        minimum_required_fee_atoms, solve_transaction_pow, transaction_pow_is_valid,
    };
    use atho_core::constants::{
        DUST_RELAY_VALUE_ATOMS, MAX_BLOCK_RAW_BYTES, MAX_TRANSACTION_RAW_BYTES,
    };
    use atho_core::crypto::hash::sha3_384;
    use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
    use atho_crypto::falcon::{
        generate_from_seed, sign, FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_BYTES,
    };

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

    fn regnet_part2_spend(
        output_count: usize,
        output_value_atoms: u64,
    ) -> (UtxoEntry, Transaction, u64) {
        let keypair = generate_from_seed(b"atho-validation-regnet-v2").expect("funding keypair");
        let locking_script = public_key_digest(Network::Regnet, &keypair.public_key.0).to_vec();
        let utxo = UtxoEntry::new(
            Network::Regnet,
            [0x44; 48],
            0,
            100_000,
            locking_script.clone(),
            10,
            false,
        );
        let mut tx = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![TxInput {
                previous_txid: utxo.txid,
                output_index: utxo.output_index,
                unlocking_script: locking_script,
            }],
            outputs: (0..output_count)
                .map(|_| TxOutput {
                    value_atoms: output_value_atoms,
                    locking_script: vec![7; ADDRESS_DIGEST_BYTES],
                })
                .collect(),
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &transaction_signing_digest(&tx),
        )
        .expect("signature");
        let signature_bytes = signature.0.clone();
        tx.witness = TxWitness {
            signature: signature_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature_bytes, 0),
                witness_commit_ref: [0; 16],
            }],
            additional_signers: vec![],
        }
        .canonical_bytes();
        let fee_atoms = utxo
            .value_atoms
            .checked_sub(
                tx.checked_output_value_atoms()
                    .expect("test transaction output total"),
            )
            .expect("test fee atoms");
        (utxo, tx, fee_atoms)
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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
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
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
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

    #[test]
    fn scheduled_v2_activation_changes_block_and_transaction_acceptance() {
        let schedule = [
            ScheduledActivation {
                name: "atho-ruleset-v1",
                ruleset_version: RULESET_VERSION_V1,
                block_version: BLOCK_VERSION_V1,
                transaction_version: TRANSACTION_VERSION_V1,
                activation_height: Some(0),
            },
            ScheduledActivation {
                name: "atho-ruleset-v2",
                ruleset_version: RULESET_VERSION_V2_PLACEHOLDER,
                block_version: BLOCK_VERSION_V2_PLACEHOLDER,
                transaction_version: TRANSACTION_VERSION_V2_PLACEHOLDER,
                activation_height: Some(12),
            },
        ];

        let coinbase_v1 = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(11),
                locking_script: vec![1],
            }],
            lock_time: 1,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let pre_activation_block = Block::new(
            BlockHeader {
                version: BLOCK_VERSION_V1,
                network_id: Network::Mainnet,
                height: 11,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(std::slice::from_ref(&coinbase_v1)),
                witness_root: witness_root(std::slice::from_ref(&coinbase_v1)),
                timestamp: 1,
                difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
                nonce: 0,
            },
            vec![coinbase_v1.clone()],
        );
        assert_eq!(
            validate_block_impl_with_schedule(
                &pre_activation_block,
                11,
                Network::Mainnet,
                true,
                &schedule
            ),
            Ok(())
        );

        let coinbase_v2 = Transaction {
            version: TRANSACTION_VERSION_V2_PLACEHOLDER,
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(12),
                locking_script: vec![1],
            }],
            ..coinbase_v1.clone()
        };
        let activation_block = Block::new(
            BlockHeader {
                version: BLOCK_VERSION_V2_PLACEHOLDER,
                network_id: Network::Mainnet,
                height: 12,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(std::slice::from_ref(&coinbase_v2)),
                witness_root: witness_root(std::slice::from_ref(&coinbase_v2)),
                timestamp: 1,
                difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
                nonce: 0,
            },
            vec![coinbase_v2.clone()],
        );
        assert_eq!(
            validate_block_impl_with_schedule(
                &activation_block,
                12,
                Network::Mainnet,
                true,
                &schedule
            ),
            Ok(())
        );

        let stale_v1_block = Block::new(
            BlockHeader {
                version: BLOCK_VERSION_V1,
                ..activation_block.header.clone()
            },
            vec![coinbase_v1],
        );
        assert_eq!(
            validate_block_impl_with_schedule(
                &stale_v1_block,
                12,
                Network::Mainnet,
                true,
                &schedule
            ),
            Err(ValidationError::InvalidBlockVersion)
        );
    }

    #[test]
    fn oversized_transaction_raw_bytes_are_rejected() {
        let mut tx = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1,
                locking_script: vec![0; MAX_TRANSACTION_RAW_BYTES + 1],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let txid = tx.txid();
        let signature = vec![1u8; FALCON_512_SIGNATURE_BYTES];
        let witness = TxWitness {
            signature: signature.clone(),
            pubkey: vec![2u8; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: derive_sig_ref_short(&txid, &signature, 0),
                witness_commit_ref: [0; 16],
            }],
            additional_signers: vec![],
        };
        tx.witness = witness.canonical_bytes();

        assert_eq!(
            validate_transaction_structure_for_height_with_schedule(
                &tx,
                1,
                Network::Mainnet,
                1,
                &rules::SCHEDULED_ACTIVATIONS,
            ),
            Err(ValidationError::TransactionTooLarge)
        );
    }

    #[test]
    fn standard_policy_rejects_sub_dust_outputs() {
        let tx = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![TxInput {
                previous_txid: [7; 48],
                output_index: 0,
                unlocking_script: vec![1],
            }],
            outputs: vec![TxOutput {
                value_atoms: DUST_RELAY_VALUE_ATOMS - 1,
                locking_script: vec![2; ADDRESS_DIGEST_BYTES],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };

        assert!(transaction_contains_dust_outputs(&tx));
        assert_eq!(
            validate_transaction_standard_policy(&tx, Network::Regnet),
            Err(ValidationError::DustOutput)
        );
    }

    #[test]
    fn regnet_v2_rejects_too_many_outputs() {
        let (_utxo, tx, fee_atoms) = regnet_part2_spend(65, 1_000);
        assert_eq!(
            validate_transaction(&tx, fee_atoms, Network::Regnet),
            Err(ValidationError::TooManyOutputs)
        );
    }

    #[test]
    fn regnet_v2_allows_exactly_sixty_four_outputs() {
        let keypair = generate_from_seed(b"atho-validation-regnet-v2-64").expect("funding keypair");
        let locking_script = public_key_digest(Network::Regnet, &keypair.public_key.0).to_vec();
        let utxo = UtxoEntry::new(
            Network::Regnet,
            [0x45; 48],
            0,
            1_000_000,
            locking_script.clone(),
            10,
            false,
        );
        let mut tx = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![TxInput {
                previous_txid: utxo.txid,
                output_index: utxo.output_index,
                unlocking_script: locking_script,
            }],
            outputs: (0..64)
                .map(|_| TxOutput {
                    value_atoms: 1_000,
                    locking_script: vec![7; ADDRESS_DIGEST_BYTES],
                })
                .collect(),
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &transaction_signing_digest(&tx),
        )
        .expect("signature");
        let signature_bytes = signature.0.clone();
        tx.witness = TxWitness {
            signature: signature_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature_bytes, 0),
                witness_commit_ref: [0; 16],
            }],
            additional_signers: vec![],
        }
        .canonical_bytes();
        let fee_atoms = utxo
            .value_atoms
            .checked_sub(
                tx.checked_output_value_atoms()
                    .expect("test transaction output total"),
            )
            .expect("test fee atoms");
        solve_transaction_pow(Network::Regnet, &mut tx, fee_atoms);
        let lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };
        assert_eq!(
            validate_transaction_with_context(&tx, fee_atoms, Network::Regnet, 20, lookup),
            Ok(fee_atoms)
        );
    }

    #[test]
    fn regnet_v2_allows_output_exactly_at_dust_floor() {
        let (utxo, mut tx, fee_atoms) = regnet_part2_spend(2, DUST_RELAY_VALUE_ATOMS);
        solve_transaction_pow(Network::Regnet, &mut tx, fee_atoms);
        let lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };
        assert_eq!(
            validate_transaction_with_context(&tx, fee_atoms, Network::Regnet, 20, lookup),
            Ok(fee_atoms)
        );
    }

    #[test]
    fn regnet_v2_rejects_wrong_transaction_pow_bits() {
        let (utxo, mut tx, fee_atoms) = regnet_part2_spend(2, 10_000);
        solve_transaction_pow(Network::Regnet, &mut tx, fee_atoms);
        tx.tx_pow_bits = tx.tx_pow_bits.saturating_add(1);
        let lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };
        assert_eq!(
            validate_transaction_with_context(&tx, fee_atoms, Network::Regnet, 20, lookup,),
            Err(ValidationError::WrongTransactionPowBits)
        );
    }

    #[test]
    fn regnet_v2_rejects_invalid_transaction_pow_nonce() {
        let (utxo, mut tx, fee_atoms) = regnet_part2_spend(2, 10_000);
        solve_transaction_pow(Network::Regnet, &mut tx, fee_atoms);
        let mut bad_nonce = tx.tx_pow_nonce.wrapping_add(1);
        while {
            tx.tx_pow_nonce = bad_nonce;
            transaction_pow_is_valid(Network::Regnet, &tx, fee_atoms)
        } {
            bad_nonce = bad_nonce.wrapping_add(1);
        }
        let lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };
        assert_eq!(
            validate_transaction_with_context(&tx, fee_atoms, Network::Regnet, 20, lookup,),
            Err(ValidationError::InvalidTransactionPowNonce)
        );
    }

    #[test]
    fn output_total_overflow_is_rejected_during_context_validation() {
        let keypair = generate_from_seed(b"atho-validation-output-overflow").expect("keypair");
        let locking_script = public_key_digest(Network::Mainnet, &keypair.public_key.0).to_vec();
        let utxo = UtxoEntry::new(
            Network::Mainnet,
            [0x51; 48],
            0,
            u64::MAX,
            locking_script.clone(),
            10,
            false,
        );
        let mut tx = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![TxInput {
                previous_txid: utxo.txid,
                output_index: utxo.output_index,
                unlocking_script: locking_script,
            }],
            outputs: vec![
                TxOutput {
                    value_atoms: u64::MAX,
                    locking_script: vec![7; ADDRESS_DIGEST_BYTES],
                },
                TxOutput {
                    value_atoms: DUST_RELAY_VALUE_ATOMS,
                    locking_script: vec![8; ADDRESS_DIGEST_BYTES],
                },
            ],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &transaction_signing_digest(&tx),
        )
        .expect("signature");
        let signature_bytes = signature.0.clone();
        tx.witness = TxWitness {
            signature: signature_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature_bytes, 0),
                witness_commit_ref: [0; 16],
            }],
            additional_signers: vec![],
        }
        .canonical_bytes();
        let fee_atoms = minimum_required_fee_atoms(Network::Mainnet, &tx);
        let lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };

        assert_eq!(
            validate_transaction_with_context(&tx, fee_atoms, Network::Mainnet, 20, lookup),
            Err(ValidationError::FeeMismatch)
        );
    }

    #[test]
    fn wrong_public_key_for_standard_output_is_rejected() {
        let funding = generate_from_seed(b"atho-validation-funding").expect("funding keypair");
        let wrong = generate_from_seed(b"atho-validation-wrong").expect("wrong keypair");
        let lock_script = public_key_digest(Network::Mainnet, &funding.public_key.0).to_vec();
        let utxo = UtxoEntry::new(
            Network::Mainnet,
            [9; 48],
            0,
            10_000,
            lock_script.clone(),
            1,
            false,
        );
        let mut tx = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![TxInput {
                previous_txid: utxo.txid,
                output_index: utxo.output_index,
                unlocking_script: lock_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 9_000,
                locking_script: vec![7; ADDRESS_DIGEST_BYTES],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &wrong.secret_key,
            &transaction_signing_digest(&tx),
        )
        .expect("signature");
        let sig_bytes = signature.0.clone();
        tx.witness = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: wrong.public_key.0.clone(),
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &sig_bytes, 0),
                witness_commit_ref: [0; 16],
            }],
            additional_signers: vec![],
        }
        .canonical_bytes();

        let lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };

        assert_eq!(
            validate_transaction_with_context(
                &tx,
                tx.vsize_bytes() as u64,
                Network::Mainnet,
                1,
                lookup,
            ),
            Err(ValidationError::InputOwnershipMismatch)
        );
    }

    #[test]
    fn oversized_block_raw_bytes_are_rejected() {
        let coinbase = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(1),
                locking_script: vec![0; MAX_BLOCK_RAW_BYTES + 1],
            }],
            lock_time: 1,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let transactions = vec![coinbase];
        let block = Block::new(
            BlockHeader {
                version: BLOCK_VERSION_V1,
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
            Err(ValidationError::BlockTooLarge)
        );
    }

    #[test]
    fn higher_fee_transactions_are_accepted_in_blocks() {
        let funding_keypair =
            generate_from_seed(b"atho-validation-high-fee").expect("funding keypair");
        let funding_script =
            public_key_digest(Network::Mainnet, &funding_keypair.public_key.0).to_vec();
        let funding = UtxoEntry::new(
            Network::Mainnet,
            [11; 48],
            0,
            10_000,
            funding_script.clone(),
            0,
            false,
        );

        let mut tx = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![TxInput {
                previous_txid: funding.txid,
                output_index: funding.output_index,
                unlocking_script: funding_script.clone(),
            }],
            outputs: vec![TxOutput {
                value_atoms: 9_000,
                locking_script: vec![7; ADDRESS_DIGEST_BYTES],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let coinbase = Transaction {
            version: TRANSACTION_VERSION_V1,
            inputs: vec![],
            outputs: vec![TxOutput {
                value_atoms: subsidy::block_subsidy_atoms(6).saturating_add(1_000),
                locking_script: vec![1],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &funding_keypair.secret_key,
            &transaction_signing_digest(&tx),
        )
        .expect("signature");
        let signature_bytes = signature.0.clone();
        let staged_witness = TxWitness {
            signature: signature_bytes.clone(),
            pubkey: funding_keypair.public_key.0.clone(),
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature_bytes, 0),
                witness_commit_ref: [0; 16],
            }],
            additional_signers: vec![],
        };
        let staged_tx = Transaction {
            witness: staged_witness.canonical_bytes(),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
            ..tx.clone()
        };
        let staged_transactions = vec![coinbase.clone(), staged_tx.clone()];
        let block_witness_root = witness_root(&staged_transactions);
        tx.witness = TxWitness {
            signature: signature_bytes.clone(),
            pubkey: funding_keypair.public_key.0.clone(),
            input_refs: vec![WitnessInputRef {
                input_index: 0,
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature_bytes, 0),
                witness_commit_ref: derive_witness_commit_ref(&tx.txid(), &block_witness_root, 0),
            }],
            additional_signers: vec![],
        }
        .canonical_bytes();
        solve_transaction_pow(Network::Mainnet, &mut tx, 1_000);

        let transactions = vec![coinbase, tx.clone()];
        let block = Block::new(
            BlockHeader {
                version: BLOCK_VERSION_V1,
                network_id: Network::Mainnet,
                height: 6,
                previous_block_hash: [0; 48],
                merkle_root: merkle_root(&transactions),
                witness_root: witness_root(&transactions),
                timestamp: 1,
                difficulty_target_or_bits: pow::initial_target_for_network(Network::Mainnet),
                nonce: 0,
            },
            transactions,
        );
        let mut block = block;
        block.fees_total_atoms = 1_000;
        block.fees_miner_atoms = 1_000;
        let solved = solve_block(block);
        let mut utxos = UtxoSet::new(Network::Mainnet);
        utxos.insert(funding).unwrap();

        assert_eq!(
            validate_block_with_context(
                &solved,
                6,
                Network::Mainnet,
                [0; 48],
                pow::initial_target_for_network(Network::Mainnet),
                &[],
                utxos,
            ),
            Ok(())
        );
    }

    #[test]
    fn validation_error_codes_are_stable_for_key_failures() {
        use atho_errors::AthoErrorMeta;

        assert_eq!(
            ValidationError::InvalidWitness
                .to_atho_error()
                .code()
                .as_str(),
            "ATHO-SIG-001"
        );
        assert_eq!(
            ValidationError::ProofOfWorkInvalid
                .to_atho_error()
                .code()
                .as_str(),
            "ATHO-BLK-005"
        );
        assert_eq!(
            ValidationError::MempoolConflict
                .to_atho_error()
                .code()
                .as_str(),
            "ATHO-MEM-001"
        );
        assert_eq!(
            ValidationError::BlockNetworkMismatch
                .to_atho_error()
                .code()
                .as_str(),
            "ATHO-NET-002"
        );
    }
}
