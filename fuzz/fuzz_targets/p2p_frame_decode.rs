#![no_main]
// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors


use atho_p2p::codec::WireCodec;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = WireCodec::decode(data);
});
