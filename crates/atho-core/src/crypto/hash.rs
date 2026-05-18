// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Canonical SHA-3 hashing helpers used throughout the protocol.
//!
//! Keeping these wrappers in one place helps the codebase avoid accidental
//! hash-width mismatches between addresses, transaction IDs, and witnesses.

use sha3::{Digest, Sha3_256, Sha3_384};

/// Hashes `data` with SHA3-256 and returns the raw digest bytes.
pub fn sha3_256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Hashes `data` with SHA3-384 and returns the raw digest bytes.
pub fn sha3_384(data: &[u8]) -> [u8; 48] {
    let mut hasher = Sha3_384::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Hashes multiple chunks as one logical SHA3-384 message without buffering.
pub fn sha3_384_chunks<'a, I>(chunks: I) -> [u8; 48]
where
    I: IntoIterator<Item = &'a [u8]>,
{
    let mut hasher = Sha3_384::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    hasher.finalize().into()
}

/// Convenience helper for human-readable SHA3-256 output.
pub fn sha3_256_hex(data: &[u8]) -> String {
    hex::encode(sha3_256(data))
}
