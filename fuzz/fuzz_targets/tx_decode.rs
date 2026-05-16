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

    let encoded = options
        .serialize(&tx)
        .expect("transaction bincode reserialize");
    let reparsed = options
        .deserialize::<atho_core::transaction::Transaction>(&encoded)
        .expect("transaction bincode reparses");
    assert_eq!(reparsed, tx);

    let _ = tx.txid();
    let _ = tx.wtxid();
    let _ = tx.signing_digest();
    let _ = tx.canonical_bytes();
    let _ = tx.full_bytes();
});
