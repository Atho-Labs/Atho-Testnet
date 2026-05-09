use sha3::{Digest, Sha3_256, Sha3_384};

pub fn sha3_256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn sha3_384(data: &[u8]) -> [u8; 48] {
    let mut hasher = Sha3_384::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn sha3_256_hex(data: &[u8]) -> String {
    hex::encode(sha3_256(data))
}
