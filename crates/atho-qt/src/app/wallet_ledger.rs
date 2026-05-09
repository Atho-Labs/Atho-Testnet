use super::models::WalletBalanceSummary;
use atho_storage::utxo::UtxoEntry;

pub(crate) fn summarize_wallet_utxos(
    utxos: &[UtxoEntry],
    current_height: u64,
) -> WalletBalanceSummary {
    let mut available_atoms = 0u64;
    let mut pending_atoms = 0u64;

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
