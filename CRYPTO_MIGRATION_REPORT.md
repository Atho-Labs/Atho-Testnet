# Atho Crypto Migration Report

## Summary

Atho now uses Falcon-512 RS as the only active post-quantum signature implementation.
The old C Falcon path and Kyber-512 path were removed from active use.

## Removed Legacy Paths

- Deleted the legacy Falcon C tree at `Falcon 512 `
- Deleted the Kyber tree at `Kyber512`
- Removed the old `atho-crypto` C FFI build script
- Removed the Kyber module from `atho-crypto`
- Removed Kyber wallet datafile mode support from the active path

## Enabled Rust Falcon Path

- Added a direct dependency from `atho-crypto` to the vendored Rust Falcon implementation
- Flattened the vendored Falcon manifests so they can be consumed as standalone packages
- Switched key generation, signing, and verification to the Rust `fn-dsa` implementation
- Locked Atho transaction signing to the `ATHO_TX_SIG_V1` domain
- Switched transaction signatures to fixed-size 666-byte Falcon-512 RS signatures

## Consensus / Domain Rules

- Falcon parameter set: Falcon-512 only
- Public key size: 897 bytes
- Secret key size: 1,281 bytes
- Signature size: 666 bytes
- Transaction signing prehash: `SHA3-384(Transaction::base_bytes())`
- Transaction domain label: `ATHO_TX_SIG_V1`
- Reserved domain labels:
  - `ATHO_BLOCK_SIG_V1`
  - `ATHO_WALLET_LOCAL_SIG_V1`
  - `ATHO_PACKAGE_SIG_V1`
  - `ATHO_TEST_DEV_SIG_V1`

## Files Changed

- `crates/atho-core/src/consensus/signatures.rs`
- `crates/atho-core/src/consensus.rs`
- `crates/atho-core/src/transaction.rs`
- `crates/atho-crypto/Cargo.toml`
- `crates/atho-crypto/src/falcon.rs`
- `crates/atho-crypto/src/lib.rs`
- `crates/atho-node/src/validation.rs`
- `crates/atho-node/src/dev.rs`
- `crates/atho-node/src/mempool.rs`
- `crates/atho-node/src/node.rs`
- `crates/atho-node/src/bin/atho-attack.rs`
- `crates/atho-node/tests/protocol_fixtures.rs`
- `crates/atho-qt/src/app.rs`
- `crates/atho-wallet/src/wallet/datafile.rs`
- `README.md`
- `TODO.md`
- `RELEASE_NOTES.md`

## Verification

- `cargo test --workspace` passed after the migration
- The active code path no longer references the legacy Falcon C tree or Kyber

## Remaining Notes

- The vendored Rust Falcon tree remains in the repository as the active implementation source.
- Unrelated local edits already existed in:
  - `crates/atho-qt/src/app/wallet_ledger.rs`
  - `crates/atho-qt/src/app/widgets/mod.rs`
  - `crates/atho-wallet/src/wallet.rs`
  These were left untouched.
