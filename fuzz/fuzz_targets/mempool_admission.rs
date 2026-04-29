#![no_main]

mod common;

use atho_node::mempool::MempoolEntry;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let fixture = common::validation_fixture();
    let mut node = fixture.seed_node();
    let mut tx = fixture.transactions[0].clone();
    let fee_atoms = fixture.fees[0];
    let selector = data.first().copied().unwrap_or(0) % 6;

    match selector {
        0 => {}
        1 => {
            if let Some(witness) = tx.witness_payload() {
                let mut witness = witness;
                witness.signature[0] ^= 1;
                tx.witness = witness.canonical_bytes();
            }
        }
        2 => {
            tx.inputs[0].unlocking_script[0] ^= 1;
        }
        3 => {
            if let Some(witness) = tx.witness_payload() {
                let mut witness = witness;
                witness.input_refs.clear();
                tx.witness = witness.canonical_bytes();
            }
        }
        4 => {
            tx.outputs[0].value_atoms = tx.outputs[0].value_atoms.wrapping_add(1);
        }
        _ => {
            tx.inputs[0].previous_txid[0] ^= 1;
        }
    }

    let admitted = node.admit_transaction(MempoolEntry::new(tx.clone(), fee_atoms));
    if admitted.is_ok() {
        assert!(node.mempool_contains(&tx.txid()));
    }
});
