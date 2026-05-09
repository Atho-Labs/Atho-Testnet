#![no_main]

use atho_p2p::codec::WireCodec;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = WireCodec::decode(data);
});
