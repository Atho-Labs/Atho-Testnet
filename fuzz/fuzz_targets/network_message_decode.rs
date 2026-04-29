#![no_main]

mod common;

use atho_p2p::codec::WireCodec;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let decoded = WireCodec::decode(data);
    if let Ok(message) = decoded {
        let encoded = WireCodec::encode(&message).expect("network message re-encodes");
        let reparsed = WireCodec::decode(&encoded).expect("re-encoded message decodes");
        assert_eq!(reparsed, message);
    }
});
