# Atho Storage Code Guide

## Scope

Storage logic mainly lives in:

- `crates/atho-storage/src/db.rs`
- `crates/atho-storage/src/chainstate.rs`
- `crates/atho-storage/src/utxo.rs`
- `crates/atho-storage/src/path.rs`

## Storage Boundaries

The storage layer owns:

- chain tip snapshots
- block archive records
- transaction archive records
- UTXO entries
- peer and peer-health records
- schema versioning

## Dangerous Areas

Mark these with `STORAGE` or `WARNING(storage)` comments:

- multi-record commits
- schema migrations
- legacy-layout rejection
- reindex and repair flows
- pruning and snapshot support

## Atomicity Rule

The chain tip snapshot and UTXO set must move together.

If a tip is persisted without the matching UTXO state, the node can restart into a corrupt view of the chain.

## Network Isolation Rule

Mainnet, testnet, regnet, and prunetest must never share a database root.

This protects against accidental replay or reuse of the wrong chainstate.
