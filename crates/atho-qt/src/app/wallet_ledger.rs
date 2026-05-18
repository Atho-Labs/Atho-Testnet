// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Wallet balance summarization helpers for UI views.

use super::models::WalletBalanceSummary;
use atho_storage::utxo::UtxoEntry;

/// Splits wallet UTXOs into currently spendable and still-pending totals.
pub(crate) fn summarize_wallet_utxos(
    utxos: &[UtxoEntry],
    current_height: u64,
) -> WalletBalanceSummary {
    let mut available_atoms = 0u64;
    let mut pending_atoms = 0u64;

    // The UI treats maturity as the only gating factor here, so the summary can
    // stay cheap and deterministic while deeper spend-policy checks happen later.
    for utxo in utxos {
        if utxo.is_spendable_at(current_height) {
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
