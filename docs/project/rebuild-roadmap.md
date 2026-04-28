# Historical Rebuild Roadmap

This document preserves the original phased rebuild plan that guided the Rust implementation.

It is historical context, not the live production roadmap.

Current live status and blockers are documented in:

- [Current Production Status](../production-readiness/current-status.md)
- [Roadmap to Production](../production-readiness/roadmap.md)

## Source References

The rebuild was driven by these reference materials:

- `docs/reference/materials/atho-outline-rust-version.pdf`
- `docs/reference/materials/atho-rust-build-guide.pdf`
- `docs/reference/materials/document-reference-from-prototype.pdf`

## Original Working Rules

- build from the ground up
- keep modules small
- keep the trusted core small and auditable
- preserve confirmed protocol behavior and constants
- avoid prototype architecture carryover
- test each layer before moving up

## Completed Rebuild Phases

### Phase 0: Workspace and Tooling

Completed:

- Rust toolchain pin
- workspace creation
- baseline `cargo check` and `cargo test`

### Phase 1: Protocol Foundation

Completed:

- `atho-core`
- network identity constants
- monetary constants
- Base56 checksum helpers
- transaction and block structures
- canonical txid and signing-digest rules

### Phase 2: Crypto Boundary

Completed:

- `atho-crypto`
- Falcon boundary
- zeroization and secret handling
- active-path crypto simplification

### Phase 3: Storage and Chainstate

Completed:

- `atho-storage`
- chainstate model
- UTXO persistence
- reorg-safe state transition tests
- later atomic single-environment LMDB cleanup

### Phase 4: Wallet Core

Completed:

- `atho-wallet`
- HD primitives
- keypool
- mnemonic support
- encrypted wallet datafiles

### Phase 5: Validation and Mempool

Completed:

- `atho-node`
- mempool admission rules
- fee policy checks
- transaction validation
- block validation

### Phase 6: Network and RPC

Completed foundation:

- `atho-p2p`
- `atho-rpc`
- wire codec
- handshake rules
- sync scaffolding
- thin-client RPC surface

Not yet complete:

- live TCP peer runtime
- downloader and compact block paths

### Phase 7: Node Runtime

Completed:

- `athod`
- runtime lifecycle
- orchestration
- miner candidate assembly

### Phase 8: Desktop Client

Completed:

- `atho-qt`
- thin client shell
- read-only and RPC-backed status
- send/receive/mining controls

### Phase 9: Security and Hardening

Completed in substantial part:

- structured errors
- secret zeroization
- integration tests
- benchmarks
- packaging/release notes
- adversarial harnesses

Still open:

- full production network hardening
- schema migrations
- full GUI automation
