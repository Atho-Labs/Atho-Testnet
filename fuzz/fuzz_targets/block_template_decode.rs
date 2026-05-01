#![no_main]

use bincode::Options;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let options = bincode::DefaultOptions::new()
        .with_limit(data.len() as u64)
        .reject_trailing_bytes();
    let Ok(template) = options.deserialize::<atho_rpc::response::BlockTemplate>(data) else {
        return;
    };

    let encoded = options
        .serialize(&template)
        .expect("block template bincode serialize");
    let reparsed = options
        .deserialize::<atho_rpc::response::BlockTemplate>(&encoded)
        .expect("block template bincode reparses");
    assert_eq!(reparsed, template);
    assert_eq!(
        template.header_bytes_without_nonce(),
        template.block.header.canonical_bytes_without_nonce()
    );
    assert_eq!(
        template.nonce_offset_bytes(),
        template.block.header.canonical_size_bytes_without_nonce()
    );
});
