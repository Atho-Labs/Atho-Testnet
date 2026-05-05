# Crypto Migration Report

## Summary

Atho’s active cryptographic path is now Falcon-512 through the vendored Rust `fn-dsa` implementation.

The migration removed older, parallel cryptographic directions from the active runtime path so the repository would have one clear signature authority.

## What Changed

- Falcon-512 became the only active signature scheme
- the Rust vendor tree became the active implementation source
- Atho transaction signing was frozen to the `ATHO_TX_SIGN_V1` domain
- fixed-size Falcon signature and key expectations were wired into validation

## Why The Migration Happened

The project needed:

- one active signature path
- one active parameter set
- one explicit domain-separated transaction-signing rule

Without that cleanup, wallet, validator, and test code would continue to carry unnecessary branching and ambiguity.

## Resulting Active Rules

- signature scheme: Falcon-512
- public key size: `897 bytes`
- secret key size: `1,281 bytes`
- signature size: `666 bytes`
- transaction signing digest: `SHA3-384(Transaction::base_bytes())`
- transaction domain label: `ATHO_TX_SIGN_V1`

Reserved but inactive labels:

- `ATHO_BLOCK_SIG_V1`
- `ATHO_WALLET_LOCAL_SIG_V1`
- `ATHO_PACKAGE_SIG_V1`
- `ATHO_TEST_DEV_SIG_V1`

## Files And Areas Affected

Primary areas:

- `crates/atho-core/src/consensus/signatures.rs`
- `crates/atho-core/src/transaction.rs`
- `crates/atho-crypto/src/falcon.rs`
- `crates/atho-storage/src/validation.rs`
- `crates/atho-wallet/src/wallet/datafile.rs`
- test and audit harnesses that depended on signing

## Current State

The migration is complete for the active path.

Open follow-on work is no longer about “which signature scheme is active.” It is about:

- network/runtime hardening
- wallet history model cleanup
- production-readiness work outside the signature boundary

## Related Documentation

- [Cryptography](cryptography.md)
- [Transactions](../protocol/transactions.md)
