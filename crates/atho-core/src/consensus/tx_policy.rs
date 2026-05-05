//! Transaction anti-spam policy helpers shared across wallets, nodes, and tests.
//!
//! These helpers define the active relay/consensus transaction policy: fee
//! floors, dust rules, output caps, and wallet transaction proof-of-work.

use crate::constants::{
    DUST_RELAY_VALUE_ATOMS, MAX_STANDARD_OUTPUTS, MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE,
    MIN_TX_FEE_ATOMS, TX_POW_DOMAIN, TX_POW_MAX_BITS, TX_POW_MIN_BITS,
};
use crate::crypto::hash::sha3_256;
use crate::genesis::genesis_hash;
use crate::network::Network;
use crate::transaction::Transaction;
use getrandom::getrandom;
use sha3::{Digest, Sha3_256};

fn read_le_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let slice = bytes.get(*offset..end)?;
    let mut out = [0u8; 4];
    out.copy_from_slice(slice);
    *offset = end;
    Some(u32::from_le_bytes(out))
}

fn update_tx_pow_message_hasher(hasher: &mut Sha3_256, tx: &Transaction) {
    tx.update_base_hasher(hasher);

    if tx.witness.is_empty() {
        hasher.update(0u32.to_le_bytes());
        return;
    }

    let mut offset = 0usize;
    let Some(signature_len) = read_le_u32(&tx.witness, &mut offset).map(|len| len as usize) else {
        hasher.update(0u32.to_le_bytes());
        return;
    };
    let Some(signature) = tx.witness.get(offset..offset.saturating_add(signature_len)) else {
        hasher.update(0u32.to_le_bytes());
        return;
    };
    offset += signature_len;

    let Some(pubkey_len) = read_le_u32(&tx.witness, &mut offset).map(|len| len as usize) else {
        hasher.update(0u32.to_le_bytes());
        return;
    };
    let Some(pubkey) = tx.witness.get(offset..offset.saturating_add(pubkey_len)) else {
        hasher.update(0u32.to_le_bytes());
        return;
    };
    offset += pubkey_len;

    let Some(ref_count) = read_le_u32(&tx.witness, &mut offset) else {
        hasher.update(0u32.to_le_bytes());
        return;
    };
    let Some(ref_bytes_len) = (ref_count as usize).checked_mul(18) else {
        hasher.update(0u32.to_le_bytes());
        return;
    };
    let Some(expected_end) = offset.checked_add(ref_bytes_len) else {
        hasher.update(0u32.to_le_bytes());
        return;
    };
    if expected_end != tx.witness.len() {
        hasher.update(0u32.to_le_bytes());
        return;
    }

    // Wallet tx PoW must survive block assembly. The per-input
    // witness_commit_ref is block-specific and is rewritten when a miner
    // binds the transaction to a block witness root, so exclude it here
    // while still binding the PoW to the signed witness material.
    hasher.update((signature_len as u32).to_le_bytes());
    hasher.update(signature);
    hasher.update((pubkey_len as u32).to_le_bytes());
    hasher.update(pubkey);
    hasher.update(ref_count.to_le_bytes());

    for _ in 0..ref_count {
        let Some(sig_ref_short) = tx.witness.get(offset..offset.saturating_add(2)) else {
            hasher.update(0u32.to_le_bytes());
            return;
        };
        hasher.update(sig_ref_short);
        offset = offset.saturating_add(18);
    }
}

pub fn minimum_required_fee_atoms(network: Network, tx: &Transaction) -> u64 {
    let _ = network;
    let vbytes = tx.vsize_bytes().max(1) as u64;
    MIN_TX_FEE_ATOMS.max(vbytes.saturating_mul(MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE))
}

pub fn minimum_output_amount_atoms(network: Network, tx: &Transaction) -> u64 {
    let _ = network;
    let _ = tx;
    DUST_RELAY_VALUE_ATOMS
}

pub fn maximum_standard_outputs(network: Network, tx: &Transaction) -> usize {
    let _ = network;
    let _ = tx;
    MAX_STANDARD_OUTPUTS
}

pub fn required_tx_pow_bits(network: Network, tx: &Transaction, fee_atoms: u64) -> u8 {
    let _ = network;
    if tx.is_coinbase() {
        return 0;
    }

    let tx_vbytes = tx.vsize_bytes().max(1) as u64;
    let fee_rate = fee_atoms / tx_vbytes;
    let output_count = tx.outputs.len();

    let mut bits = TX_POW_MIN_BITS as i16;

    if tx_vbytes > 500 {
        bits += 1;
    }
    if tx_vbytes > 1_000 {
        bits += 1;
    }
    if tx_vbytes > 2_000 {
        bits += 1;
    }

    if output_count > 2 {
        bits += 1;
    }
    if output_count > 8 {
        bits += 1;
    }
    if output_count > 32 {
        bits += 1;
    }

    if fee_rate >= 100 {
        bits -= 1;
    } else if fee_rate >= 10 {
    } else if fee_rate >= 1 {
        bits += 2;
    } else {
        bits += 4;
    }

    if output_count > 32 && fee_rate <= 1 {
        bits += 2;
    }

    bits.clamp(TX_POW_MIN_BITS as i16, TX_POW_MAX_BITS as i16) as u8
}

pub fn transaction_pow_preimage(network: Network, tx: &Transaction) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(TX_POW_DOMAIN);
    hasher.update([network.consensus_id()]);
    hasher.update(genesis_hash(network));
    update_tx_pow_message_hasher(&mut hasher, tx);
    hasher.finalize().into()
}

pub fn leading_zero_bits(bytes: &[u8]) -> u8 {
    let mut count = 0u8;
    for byte in bytes {
        if *byte == 0 {
            count = count.saturating_add(8);
            continue;
        }
        for bit in (0..8).rev() {
            if byte & (1 << bit) != 0 {
                return count;
            }
            count = count.saturating_add(1);
        }
    }
    count
}

pub fn transaction_pow_hash(preimage: &[u8; 32], nonce: u64) -> [u8; 32] {
    let mut payload = [0u8; 40];
    payload[..32].copy_from_slice(preimage);
    payload[32..].copy_from_slice(&nonce.to_be_bytes());
    sha3_256(&payload)
}

fn transaction_pow_nonce_start(preimage: &[u8; 32]) -> u64 {
    let mut nonce = [0u8; 8];
    if getrandom(&mut nonce).is_ok() {
        return u64::from_be_bytes(nonce);
    }
    nonce.copy_from_slice(&preimage[..8]);
    u64::from_be_bytes(nonce)
}

pub fn transaction_pow_is_valid(network: Network, tx: &Transaction, fee_atoms: u64) -> bool {
    let required_bits = required_tx_pow_bits(network, tx, fee_atoms);
    if required_bits == 0 {
        return true;
    }
    if tx.tx_pow_bits != required_bits {
        return false;
    }
    let preimage = transaction_pow_preimage(network, tx);
    leading_zero_bits(&transaction_pow_hash(&preimage, tx.tx_pow_nonce)) >= required_bits
}

pub fn solve_transaction_pow(network: Network, tx: &mut Transaction, fee_atoms: u64) -> u64 {
    let required_bits = required_tx_pow_bits(network, tx, fee_atoms);
    tx.tx_pow_bits = required_bits;
    if required_bits == 0 {
        tx.tx_pow_nonce = 0;
        return 0;
    }
    let preimage = transaction_pow_preimage(network, tx);
    let mut nonce = transaction_pow_nonce_start(&preimage);
    loop {
        if leading_zero_bits(&transaction_pow_hash(&preimage, nonce)) >= required_bits {
            tx.tx_pow_nonce = nonce;
            return nonce;
        }
        nonce = nonce.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::TX_POW_DOMAIN;
    use crate::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};

    fn sample_tx(output_count: usize, output_value: u64) -> Transaction {
        Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![2; 32],
            }],
            outputs: (0..output_count)
                .map(|_| TxOutput {
                    value_atoms: output_value,
                    locking_script: vec![3; 32],
                })
                .collect(),
            lock_time: 0,
            witness: vec![4; 0],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        }
    }

    fn inflate_tx_to_min_vbytes(tx: &mut Transaction, minimum_vbytes: usize) {
        while tx.vsize_bytes() <= minimum_vbytes {
            tx.witness.push(0);
        }
    }

    #[test]
    fn normal_low_fee_payment_requires_nineteen_bits() {
        let mut tx = sample_tx(2, 2_000);
        inflate_tx_to_min_vbytes(&mut tx, 500);
        let fee = tx.vsize_bytes() as u64;
        assert!(tx.vsize_bytes() > 500);
        assert!(tx.vsize_bytes() <= 1_000);
        assert_eq!(required_tx_pow_bits(Network::Regnet, &tx, fee), 19);
    }

    #[test]
    fn many_output_low_fee_shape_hits_harder_pow() {
        let mut tx = sample_tx(64, 1_000);
        tx.inputs = vec![TxInput {
            previous_txid: [9; 48],
            output_index: 0,
            unlocking_script: vec![8; 32],
        }];
        inflate_tx_to_min_vbytes(&mut tx, 2_000);
        let fee = tx.vsize_bytes() as u64;
        assert!(tx.vsize_bytes() > 2_000);
        assert_eq!(required_tx_pow_bits(Network::Regnet, &tx, fee), 26);
    }

    #[test]
    fn required_fee_examples_match_policy_floor() {
        for (vbytes, expected) in [(250usize, 500u64), (500, 500), (650, 650), (2_500, 2_500)] {
            let mut tx = sample_tx(2, 2_000);
            while tx.vsize_bytes() < vbytes {
                tx.witness.push(0);
            }
            assert_eq!(minimum_required_fee_atoms(Network::Regnet, &tx), expected);
        }
    }

    #[test]
    fn pow_bit_examples_match_final_table() {
        let mut low_fee = sample_tx(2, 2_000);
        inflate_tx_to_min_vbytes(&mut low_fee, 500);
        assert_eq!(
            required_tx_pow_bits(Network::Regnet, &low_fee, low_fee.vsize_bytes() as u64),
            19
        );

        let small = sample_tx(2, 2_000);
        assert_eq!(required_tx_pow_bits(Network::Regnet, &small, 500), 18);

        let mut normal_fee = sample_tx(2, 2_000);
        inflate_tx_to_min_vbytes(&mut normal_fee, 500);
        assert_eq!(
            required_tx_pow_bits(Network::Regnet, &normal_fee, 6_500),
            17
        );
        assert_eq!(
            required_tx_pow_bits(Network::Regnet, &normal_fee, 65_000),
            16
        );
    }

    #[test]
    fn solver_finds_valid_nonce_for_regnet_v1() {
        let mut tx = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Regnet, &tx);
        let nonce = solve_transaction_pow(Network::Regnet, &mut tx, fee);
        assert_eq!(nonce, tx.tx_pow_nonce);
        assert!(transaction_pow_is_valid(Network::Regnet, &tx, fee));
    }

    #[test]
    fn all_networks_require_pow_for_normal_transactions() {
        let tx = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Mainnet, &tx);
        assert_eq!(required_tx_pow_bits(Network::Mainnet, &tx, fee), 18);
        assert_eq!(required_tx_pow_bits(Network::Testnet, &tx, fee), 18);
    }

    #[test]
    fn pow_domain_constant_is_frozen() {
        assert_eq!(TX_POW_DOMAIN, b"ATHO_TX_POW_V1");
    }

    #[test]
    fn changing_nonce_keeps_same_preimage_and_can_change_validity() {
        let mut tx = sample_tx(2, 2_000);
        let fee = minimum_required_fee_atoms(Network::Regnet, &tx);
        solve_transaction_pow(Network::Regnet, &mut tx, fee);
        let original_preimage = transaction_pow_preimage(Network::Regnet, &tx);
        let original_nonce = tx.tx_pow_nonce;
        tx.tx_pow_nonce = tx.tx_pow_nonce.wrapping_add(1);

        assert_eq!(
            transaction_pow_preimage(Network::Regnet, &tx),
            original_preimage
        );
        if tx.tx_pow_nonce != original_nonce {
            assert!(!transaction_pow_is_valid(Network::Regnet, &tx, fee));
        }
    }

    #[test]
    fn block_specific_witness_refs_do_not_invalidate_transaction_pow() {
        let mut tx = sample_tx(2, 2_000);
        tx.witness = TxWitness {
            signature: vec![7; crate::constants::FALCON_512_SIGNATURE_BYTES],
            pubkey: vec![8; crate::constants::FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: vec![WitnessInputRef {
                sig_ref_short: [9, 10],
                witness_commit_ref: [0; 16],
            }],
        }
        .canonical_bytes();
        inflate_tx_to_min_vbytes(&mut tx, 500);
        let fee = tx.vsize_bytes() as u64;
        solve_transaction_pow(Network::Regnet, &mut tx, fee);
        assert!(transaction_pow_is_valid(Network::Regnet, &tx, fee));

        let mut witness = tx.witness_payload().expect("witness");
        witness.input_refs[0].witness_commit_ref = [0xaa; 16];
        tx.witness = witness.canonical_bytes();

        assert!(transaction_pow_is_valid(Network::Regnet, &tx, fee));
    }
}
