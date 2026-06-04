// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Wallet balance summarization helpers for UI views.

use super::models::WalletBalanceSummary;
use atho_storage::utxo::UtxoEntry;

/// Splits wallet UTXOs into policy-available and still-pending totals.
pub(crate) fn summarize_wallet_utxos(
    utxos: &[UtxoEntry],
    current_height: u64,
    min_confirmations: u64,
) -> WalletBalanceSummary {
    let mut available_atoms = 0u64;
    let mut pending_atoms = 0u64;

    for utxo in utxos {
        if utxo.is_spendable_at(current_height)
            && utxo.confirmation_count(current_height) >= min_confirmations
        {
            available_atoms = available_atoms.saturating_add(utxo.value_atoms);
        } else {
            pending_atoms = pending_atoms.saturating_add(utxo.value_atoms);
        }
    }

    WalletBalanceSummary {
        available_atoms,
        pending_atoms,
        total_atoms: available_atoms.saturating_add(pending_atoms),
    }
}
