#![no_main]
// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors


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

    let reserialized = options
        .serialize(&tx)
        .expect("transaction roundtrip serialization");
    let reparsed = options
        .deserialize::<atho_core::transaction::Transaction>(&reserialized)
        .expect("transaction roundtrip deserialize");
    assert_eq!(reparsed, tx);

    if let Some(witness) = tx.witness_payload() {
        let witness_encoded = witness.canonical_bytes();
        let witness_reparsed = atho_core::transaction::TxWitness::from_bytes(&witness_encoded)
            .expect("witness reparses after canonical encode");
        assert_eq!(witness_reparsed, witness);
    }
});
