// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Node-side re-export of the canonical storage validation layer.
pub use atho_storage::validation::{
    derive_sig_ref_short, derive_witness_commit_ref, finalize_witness_commit_refs,
    transaction_contains_dust_outputs, validate_block, validate_block_with_context,
    validate_block_with_context_and_schedule, validate_coinbase_transaction,
    validate_coinbase_transaction_with_schedule, validate_transaction,
    validate_transaction_for_height, validate_transaction_for_height_with_schedule,
    validate_transaction_standard_policy, validate_transaction_structure_for_height_with_schedule,
    validate_transaction_with_context, validate_transaction_with_context_and_schedule,
    validate_transaction_with_context_for_mempool,
    validate_transaction_with_context_minimum_fee_and_schedule,
    validate_transaction_with_context_structure_and_schedule, verify_transaction_signature,
    BlockValidationContext, ValidationError,
};

#[cfg(any(test, feature = "devtools"))]
pub use atho_storage::validation::validate_block_without_pow;
