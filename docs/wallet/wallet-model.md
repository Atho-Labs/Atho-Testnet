# Wallet Model

## Purpose

The Atho wallet owns seed material, deterministic address derivation, keypool state, and encrypted local persistence.

It does not own consensus.

## Core Model

Implemented in:

- `crates/atho-wallet/src/wallet.rs`
- `crates/atho-wallet/src/hd.rs`
- `crates/atho-wallet/src/keypool.rs`
- `crates/atho-wallet/src/address_book.rs`
- `crates/atho-wallet/src/snapshot.rs`

The wallet includes:

- network binding
- optional mnemonic
- HD wallet state
- keypool
- address book
- wallet snapshot counters
- restore gap limit

## Deterministic Derivation

Receive and change paths are separate.

Current derivation metadata:

- account: `0`
- kind: `Receive` or `Change`
- index: monotonically increasing per kind

Why:

- separate receive/change tracks keep output ownership and address reuse easier to reason about

## Keypool

Current target sizes:

- receive pool: up to `5,000`
- change pool: up to `5,000`

Why:

- the GUI can request fresh addresses without blocking on expensive key-generation work every time

## Restore Behavior

Current restore gap limit:

- `1,000`

Why:

- Atho needs a deterministic discovery boundary for wallet rehydration without scanning infinitely

## Datafile Persistence

Current wallet datafile properties:

- binary envelope
- versioned header
- AES-256-GCM encryption
- PBKDF2-HMAC-SHA256 password derivation
- `.datafile` filename convention

Why:

- wallet-at-rest encryption is an operational minimum even in a local-first architecture

## Address And Balance Ownership

The wallet tracks:

- derived addresses
- receive and change counts
- locally known UTXO ownership through backend scanning

Current limitation:

- canonical activity history is not yet sourced from a dedicated node history API
- the Qt client still reconstructs history from exported ledger views

That is one of the main remaining product-quality gaps.

## Related Documentation

- [Addresses and Keys](../protocol/addresses-and-keys.md)
- [Qt Client](../gui-client/qt-client.md)
- [Current Production Status](../production-readiness/current-status.md)
