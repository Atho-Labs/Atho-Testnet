#![no_main]
// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors


use atho_core::transaction::TxWitness;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(witness) = TxWitness::from_bytes(data) {
        let canonical = witness.canonical_bytes();
        let reparsed = TxWitness::from_bytes(&canonical).expect("canonical witness reparses");
        assert_eq!(reparsed, witness);

        let compact = witness.compact_bytes();
        assert!(!compact.is_empty() || witness.is_empty());
    }
});
