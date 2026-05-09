//! Secret-bearing byte containers.
//!
//! This module keeps raw secret material behind a small wrapper that zeroizes
//! on drop so higher-level wallet and signing code does not accidentally retain
//! sensitive data in memory longer than necessary.
use zeroize::Zeroize;

/// Heap-allocated secret bytes that are wiped on drop.
///
/// WALLET SECURITY: Use this wrapper for mnemonic seeds, private keys, and any
/// other value that should not survive ordinary object teardown.
#[derive(Debug, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct SecretBytes(pub Vec<u8>);
