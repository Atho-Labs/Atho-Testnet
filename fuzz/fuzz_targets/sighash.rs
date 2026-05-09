#![no_main]

mod common;

use bincode::Options;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let options = bincode::DefaultOptions::new()
        .with_limit(data.len() as u64)
        .reject_trailing_bytes();
    let Ok(tx) = options.deserialize::<atho_core::transaction::Transaction>(data) else {
        return;
    };

    let digest = tx.signing_digest();
    let _ = atho_core::consensus::signatures::transaction_signing_digest(
        atho_core::network::Network::Regnet,
        &tx,
    );

    if let Some(first_output) = tx.outputs.first() {
        let mut mutated = tx.clone();
        mutated.outputs[0].value_atoms = first_output.value_atoms.wrapping_add(1);
        assert_ne!(digest, mutated.signing_digest());
    } else if let Some(first_input) = tx.inputs.first() {
        let mut mutated = tx.clone();
        mutated.inputs[0].output_index = first_input.output_index.wrapping_add(1);
        assert_ne!(digest, mutated.signing_digest());
    }
});
