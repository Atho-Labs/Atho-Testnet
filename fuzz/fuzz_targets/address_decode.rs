#![no_main]

use atho_core::address::{decode_base56_address, encode_base56_address};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(address) = std::str::from_utf8(data) else {
        return;
    };

    let decoded = decode_base56_address(address);
    if let Ok((digest, network)) = decoded {
        let reencoded = encode_base56_address(network, &digest);
        let reparsed = decode_base56_address(&reencoded).expect("re-encoded address decodes");
        assert_eq!(reparsed, (digest, network));
    }
});
