# Atho Production Readiness Test Plan

## Purpose

This document defines the production-readiness test program for Atho.

It is intentionally split into:

- what is implemented and exercised now
- what is partially implemented and needs broader coverage
- what remains a release gate before public production claims

This plan does not change consensus behavior. It defines how the codebase is verified.

## Core Rules

- CPU validation remains canonical.
- No test harness may weaken consensus rules.
- Mempool policy tests must stay distinct from consensus tests.
- Cross-network contamination must fail closed.
- Randomized tests must be reproducible or log their seed.
- Every critical failure should map to an `ATHO-*` error code.

## Test Modes

### 1. Unit

Purpose:

- deterministic fast validation on every change

Coverage:

- hashes
- Falcon signatures
- canonical encoding
- addresses
- PoW math
- consensus constants
- storage primitives
- RPC error serialization

Primary commands:

```bash
cargo test -p atho-errors
cargo test -p atho-core
cargo test -p atho-crypto
cargo test -p atho-storage
cargo test -p atho-rpc
cargo test -p atho-wallet
```

### 2. Integration

Purpose:

- exercise real node/runtime/storage/RPC flows locally

Coverage:

- node startup
- block template creation
- mining
- restart/reload
- reorg selection
- snapshot import/export
- real TCP relay

Primary commands:

```bash
cargo test -p atho-node
cargo test -p atho-p2p
```

### 3. Prune-Test Sandbox

Purpose:

- isolate pruning, snapshot, recovery, and rollback tests from normal disposable regnet work

Network:

- network id: `Network::Prunetest`
- consensus id: `4`
- network tag: `atho-prunetest`
- p2p port: `9300`
- rpc port: `9310`
- visible address prefix: `P`
- internal HPK prefix: `ATHP`
- storage root suffix: `prunetest`
- p2p magic: `a7 54 48 04`

Difficulty:

- uses the easiest allowed consensus target within the current PoW bounds
- this is intentionally disposable and test-only
- it is isolated by network identity, not by silently relaxing validation

Current regression coverage:

```bash
cargo test -p atho-core genesis::tests::genesis_state_is_network_scoped -- --exact
cargo test -p atho-storage path::tests::network_database_dirs_are_unique_and_include_prunetest -- --exact
cargo test -p atho-node runtime::tests::config_loader_accepts_prunetest_network_from_env -- --exact
cargo test -p atho-node node::tests::prunetest_node_mines_and_restarts_in_an_isolated_database_root -- --exact
```

### 4. Fuzz and Parser Hardening

Purpose:

- reject malformed input without panic, uncontrolled allocation, or deadlock

Current command:

```bash
cargo check --manifest-path fuzz/Cargo.toml --all-targets
```

Status:

- active
- still incomplete relative to the long-term target set in the readiness program

### 5. Nightly Hardening

Purpose:

- rerun slower multi-node and pruning-sensitive suites on a schedule

Current scope:

- node package integration tests
- TCP mesh tests
- prune-test restart flow
- pruning snapshot/export paths
- fuzz-target build verification

## Current Implemented Coverage

### Consensus and Validation

- canonical block and transaction validation
- PoW target checks
- witness length hardening
- Falcon length and verification regression tests
- network and genesis separation

### Storage and Recovery

- LMDB schema migration from v2 to current schema
- persisted-tip mismatch detection
- incomplete-history quarantine
- malformed snapshot fail-closed behavior
- snapshot export/import round-trips
- rollback and candidate-commit failure regression coverage

### Networking

- version/verack handshake
- address gossip controls
- block/tx relay
- real TCP sync across multiple nodes
- compact block reconstruction path
- wrong-genesis and wrong-magic rejection

### Mining

- node-owned block template creation
- CPU miner reference path
- GPU probe/fallback path
- canonical header bytes for miners
- strict GPU-only vs auto-fallback behavior

## Highest-Priority Remaining Gaps

1. full prune lifecycle tooling
2. explicit repair and reindex CLI coverage
3. snapshot sync over P2P
4. long-run hostile mesh soak
5. broader RPC malformed-input coverage
6. continuous fuzz execution instead of build-only validation
7. cross-OS client smoke automation beyond core node builds

## Release Gates

Atho should not be called production-ready until all of the following are true:

- workspace unit and integration tests are green
- pruning lifecycle tests are green on `prunetest`
- cross-network contamination tests are green
- parser fuzz targets produce no crashing regression input
- hostile peer mesh coverage is green
- storage recovery and quarantine behavior is green
- no wallet secrets appear in logs or user-facing errors
- public-network soak coverage is green for the target release window

## Operating Rule

When a bug is found:

1. reproduce it with a failing test
2. fix it without weakening validation
3. add regression coverage
4. assign or reuse a clear `ATHO-*` error code
5. document whether the issue was consensus-critical, policy-only, or operational
