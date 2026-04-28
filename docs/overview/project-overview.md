# Project Overview

## What Atho Is

Atho is a Bitcoin-style public UTXO payment stack implemented in Rust.

It includes:

- a consensus core
- a full validating node
- durable chainstate storage
- a wallet system with HD derivation and encrypted datafiles
- a local RPC interface
- a thin desktop client
- a developing peer-to-peer network layer

## Mission

Atho aims to be a small, explicit, and production-minded blockchain stack where every important rule is local, deterministic, and auditable.

The project biases toward:

- boring architecture
- explicit constants
- deterministic validation
- one canonical rule path
- compact modules
- low abstraction overhead
- backend-owned truth

## What Problem Atho Is Solving

Atho is not trying to be a general smart-contract platform or a privacy-first Layer 1 launch.

Its narrower goal is:

- a direct payment chain
- a public UTXO ledger
- independent local verification
- post-quantum signature readiness
- a software stack that feels closer to Bitcoin Core discipline than to application-platform sprawl

## Major Design Choices

### Bitcoin-style UTXO core

Chosen because:

- UTXO state transitions are compact and explicit
- replay, rollback, and reorg logic are easier to reason about than account-style global mutation
- wallet ownership and spendability are inspectable through discrete outputs

Tradeoff:

- richer stateful application logic is intentionally not a Layer 1 goal

### Rust implementation

Chosen because:

- memory safety matters in consensus and networking code
- the workspace benefits from explicit ownership and type boundaries
- performance-sensitive code can still stay close to the metal

Tradeoff:

- vendored or specialized cryptographic integrations require careful path and build hygiene

### Thin desktop client

Chosen because:

- the node should remain the authority for chainstate, validation, and mining
- UI state should reflect backend truth instead of embedding a second blockchain implementation
- restart and synchronization behavior is easier to harden when the GUI is a client

Tradeoff:

- wallet history and client UX are dependent on stable backend interfaces

### Explicit versioning and activation scaffolding

Chosen because:

- silent consensus drift is unacceptable
- upgrades need one clear height-gated routing mechanism
- storage versioning and ruleset versioning should be inspectable in code

Tradeoff:

- the current stack includes placeholders for future rulesets before those rules exist

## Non-Goals At Current Stage

- public mainnet launch readiness
- fully decentralized peer mesh operation
- snapshot sync deployment
- complex Layer 1 scripting
- private transaction schemes

## Code Map

Primary implementation locations:

- `crates/atho-core/src/`
- `crates/atho-storage/src/`
- `crates/atho-node/src/`
- `crates/atho-wallet/src/`
- `crates/atho-p2p/src/`
- `crates/atho-rpc/src/`
- `crates/atho-qt/src/`

Related documentation:

- [System Architecture](../architecture/system-architecture.md)
- [Blocks and Consensus](../protocol/blocks-and-consensus.md)
- [Current Production Status](../production-readiness/current-status.md)
