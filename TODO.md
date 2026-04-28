# Atho Rust Rebuild TODO

Reference set:
- AthoOutline Rust Version
- Atho Rust Build Guide / Implementation Guide
- Document Reference From Prototype

Rules:
- build from the ground up
- keep modules small
- keep the trusted core small and auditable
- preserve confirmed protocol behavior and constants
- avoid prototype architecture carryover
- test each layer before moving up

## Phase 0: Workspace and Tooling
- [x] Pin Rust toolchain with `rust-toolchain.toml`
- [x] Configure VS Code for `rust-analyzer`
- [x] Create Rust workspace root
- [x] Verify `cargo check`
- [x] Verify `cargo test`

## Phase 1: Protocol Foundation
- [x] Create `atho-core` crate
- [x] Add network identity constants
- [x] Add protocol monetary constants
- [x] Add Base56/address checksum helpers
- [x] Add SHA3-256 helper
- [x] Add canonical consensus parameter module
- [x] Freeze subsidy schedule in code and tests
- [x] Define block header structure
- [x] Define transaction structure
- [x] Define canonical txid and signing digest rules

## Phase 2: Crypto Boundary
- [x] Create `atho-crypto` crate
- [x] Add Falcon-512 RS boundary
- [x] Freeze Atho Falcon-512 RS consensus rules and domain labels
- [x] Remove Kyber from the active crypto path
- [x] Add zeroization and secret handling rules
- [x] Add minimal crypto self-tests and linkage checks

## Phase 3: Storage and Chainstate
- [x] Create `atho-storage` crate
- [x] Define storage layout
- [x] Define chainstate data model
- [x] Define UTXO persistence path
- [x] Add reorg-safe state transition tests

## Phase 4: Wallet Core
- [x] Create `atho-wallet` crate
- [x] Define HD wallet primitives
- [x] Define keypool refill path
- [x] Define wallet snapshot model
- [x] Add mnemonic phrase support with 12/24/48-word options
- [x] Add address generation and restore tests
- [x] Add wallet datafile persistence with password AES-256 encryption

## Phase 5: Mempool and Validation
- [x] Create `atho-node` crate
- [x] Add mempool admission rules
- [x] Add fee policy validation
- [x] Add transaction validation pipeline
- [x] Add block validation pipeline

## Phase 6: Network and RPC
- [x] Create `atho-p2p` crate
- [x] Create `atho-rpc` crate
- [x] Add wire codec
- [x] Add peer handshake rules
- [x] Add sync state
- [x] Add RPC surface for thin client

## Phase 7: Node Runtime
- [x] Add `athod` binary entrypoint
- [x] Add runtime config loading
- [x] Add startup and shutdown flow
- [x] Add logging and error boundaries
- [x] Add node orchestration
- [x] Add miner candidate block assembly and connect path

## Phase 8: Thin Desktop Client
- [x] Create `atho-qt` crate
- [x] Add `atho-qt` binary entrypoint
- [x] Add read-only node connection
- [x] Add wallet snapshot display
- [x] Add minimal send/receive UI

## Phase 9: Security and Production Readiness
- [x] Add structured error types across crates
- [x] Add secret zeroization where required
- [x] Add integration tests for protocol fixtures
- [x] Add benchmark harnesses for hot paths
- [x] Add packaging and release notes

## System Integration
- [x] Compose node state behind `AthoSystem`
- [x] Feed Qt status from composed node state
- [x] Parallelize block validation and batch mempool admission
- [x] Wire wallet session state into the Qt dashboard
- [x] Replace placeholder Qt views with node-backed data

## Dev Workflow
- [x] Add local dev wipe/export/watch workflow
- [x] Add append-only chain and transaction audit files under `dev/`
- [x] Add live log tailing for local testing

## Current Working Rule
Only build the lowest unresolved layer first. Do not move up until the current layer compiles, tests pass, and the behavior is frozen.
r
