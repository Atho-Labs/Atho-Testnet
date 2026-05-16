// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Secret-bearing byte containers.
//!
//! This module keeps raw secret material behind a small wrapper that zeroizes
//! on drop so higher-level wallet and signing code does not accidentally retain
//! sensitive data in memory longer than necessary.
use std::fmt;
use zeroize::Zeroize;

/// Heap-allocated secret bytes that are wiped on drop.
///
/// WALLET SECURITY: Use this wrapper for mnemonic seeds, private keys, and any
/// other value that should not survive ordinary object teardown.
#[derive(PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct SecretBytes(pub Vec<u8>);

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBytes(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::SecretBytes;

    #[test]
    fn secret_bytes_debug_is_redacted() {
        let secret = SecretBytes(vec![0x41, 0x42, 0x43, 0x44]);
        let rendered = format!("{secret:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("SecretBytes(["));
    }
}
