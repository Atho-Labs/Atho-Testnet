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
    let Ok(block) = options.deserialize::<atho_core::block::Block>(data) else {
        return;
    };

    let encoded = options.serialize(&block).expect("block bincode serialize");
    let reparsed = options
        .deserialize::<atho_core::block::Block>(&encoded)
        .expect("block bincode reparses");
    assert_eq!(reparsed.header, block.header);
    assert_eq!(reparsed.transactions, block.transactions);

    let _ = block.header.block_hash();
    let _ = block.full_bytes();
    let _ = block.compact_bytes();
    let _ = block.vsize_bytes();
});
