# Atho

Atho is a from-scratch Rust blockchain payment stack built around a small trusted core, explicit consensus rules, a durable chainstate, a thin desktop client, and a Bitcoin-style architecture adapted to Atho’s own hashing, signature, and address choices.

The repository is organized as a multi-crate workspace:

- `atho-core` for protocol types, constants, addresses, blocks, transactions, genesis, and consensus rules
- `atho-crypto` for the Falcon boundary and secret-handling primitives
- `atho-storage` for chainstate, UTXO state, validation, LMDB persistence, and recovery
- `atho-wallet` for HD wallet logic, mnemonic handling, keypooling, and encrypted wallet datafiles
- `atho-p2p` for the wire protocol, handshake state machine, peer/session logic, and headers-first sync scaffolding
- `atho-rpc` for the local RPC surface and transport
- `atho-node` for runtime orchestration, mining, mempool, service ownership, and integration
- `atho-qt` for the thin desktop client

## Status

Atho is an active buildout, not a finished production network.

Current posture:

- local consensus, storage, mining, wallet, RPC, and Qt lifecycle paths have substantial sandbox coverage
- the repo now has a centralized documentation system under [`docs/`](docs/index.md)
- the local node and Qt client run through a real RPC path instead of UI-owned chainstate shortcuts
- the network layer has a real protocol foundation, but the live TCP peer runtime is still incomplete

Production-readiness summary:

- overall readiness: `7/10`
- local core consensus path: `strong`
- full product readiness: `not yet production ready`

The detailed status, open blockers, and remaining risks live in:

- [`docs/production-readiness/current-status.md`](docs/production-readiness/current-status.md)
- [`docs/production-readiness/roadmap.md`](docs/production-readiness/roadmap.md)

## Design Principles

- keep the trusted core small
- keep consensus deterministic and explicit
- keep validation on one canonical path
- keep the GUI thin and backend-owned
- keep storage durable and fail-closed
- keep the stack boring, compact, and auditable
- keep protocol evolution explicit through versioning and activation heights

## Quick Start

Build the workspace:

```bash
cargo check
```

Run tests:

```bash
cargo test
```

Run the node daemon:

```bash
cargo run -p atho-node --bin athod -- run mainnet
```

Run the Qt client against a local managed node:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

More operational commands live in [`docs/operations/commands.md`](docs/operations/commands.md).

## Repository Layout

```text
crates/
  atho-core/      protocol types, consensus rules, genesis, blocks, txs, addresses
  atho-crypto/    Falcon boundary and secret handling
  atho-storage/   LMDB storage, chainstate, validation, UTXO state
  atho-wallet/    HD wallet, mnemonic, keypool, encrypted wallet datafiles
  atho-p2p/       wire protocol, handshake, peer/session logic, sync scaffolding
  atho-rpc/       local RPC request/response and transport
  atho-node/      runtime, service, miner, mempool, orchestration
  atho-qt/        thin desktop client
docs/             all project documentation, specs, readiness notes, and whitepaper
scripts/          build and packaging helpers
dev/              local sandbox state, logs, databases, chain exports, quarantine
dist/             staged release artifacts
```

## Documentation Map

Start here:

- [`docs/index.md`](docs/index.md)

Key sections:

- [`docs/overview/project-overview.md`](docs/overview/project-overview.md)
- [`docs/architecture/system-architecture.md`](docs/architecture/system-architecture.md)
- [`docs/protocol/network-and-identity.md`](docs/protocol/network-and-identity.md)
- [`docs/consensus/consensus-rules.md`](docs/consensus/consensus-rules.md)
- [`docs/storage/chainstate-and-persistence.md`](docs/storage/chainstate-and-persistence.md)
- [`docs/node-runtime/node-runtime-and-p2p.md`](docs/node-runtime/node-runtime-and-p2p.md)
- [`docs/wallet/wallet-model.md`](docs/wallet/wallet-model.md)
- [`docs/gui-client/qt-client.md`](docs/gui-client/qt-client.md)
- [`docs/testing-audits/testing-and-hardening.md`](docs/testing-audits/testing-and-hardening.md)
- [`docs/whitepaper/atho-whitepaper-apa.md`](docs/whitepaper/atho-whitepaper-apa.md)

## Production Notes

The strongest parts of the stack today are the local validation core, storage integrity checks, replay/restart handling, and RPC-driven Qt synchronization.

The weakest parts are still:

- live TCP peer runtime completeness
- compact-block and downloader work
- pruning and snapshot lifecycle coverage
- schema migration tooling
- canonical wallet history sourcing
- OS-level GUI automation

Those are documented explicitly instead of being hidden or overstated.

## Reference Materials

Historical PDFs, planning references, and vendored third-party documentation are centralized under [`docs/reference/`](docs/reference/reference-materials.md). The detailed whitepaper and subsystem manuals live under `docs/`, not at repo root.
