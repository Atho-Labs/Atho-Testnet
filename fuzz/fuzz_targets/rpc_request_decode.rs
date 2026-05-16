#![no_main]
// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors


use atho_rpc::request::RpcRequest;
use atho_rpc::transport::read_message;
use libfuzzer_sys::fuzz_target;
use std::io::BufReader;

fuzz_target!(|data: &[u8]| {
    let mut framed = Vec::with_capacity(data.len() + 1);
    framed.extend_from_slice(data);
    framed.push(b'\n');
    let mut reader = BufReader::new(framed.as_slice());
    let _ = read_message::<_, RpcRequest>(&mut reader);
});
