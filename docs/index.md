# Atho Documentation Index

This directory is the canonical documentation tree for the Atho repository.

It is designed to answer three questions clearly:

1. What is Atho?
2. How does Atho work today?
3. What still blocks production readiness?

## Start Here

- [Project Overview](overview/project-overview.md)
- [Repository Structure](overview/repository-structure.md)
- [System Architecture](architecture/system-architecture.md)
- [Lifecycle Flows](architecture/lifecycle-flows.md)

## Protocol And Consensus

- [Network and Identity](protocol/network-and-identity.md)
- [Transactions](protocol/transactions.md)
- [Blocks and Consensus](protocol/blocks-and-consensus.md)
- [Addresses and Keys](protocol/addresses-and-keys.md)
- [Consensus Rules](consensus/consensus-rules.md)
- [Proof of Work and Emission](consensus/proof-of-work-and-emission.md)
- [Versioning and Activations](consensus/versioning-and-activations.md)
- [Reorg, Fork, and Pruning Rules](consensus/reorg-fork-pruning.md)

## Implementation

- [Chainstate and Persistence](storage/chainstate-and-persistence.md)
- [Node Runtime and P2P](node-runtime/node-runtime-and-p2p.md)
- [Mining and Mempool](node-runtime/mining-and-mempool.md)
- [RPC and Client Backend](node-runtime/rpc-and-client.md)
- [Wallet Model](wallet/wallet-model.md)
- [Qt Client](gui-client/qt-client.md)
- [Qt Reference Map](gui-client/qt-reference-map.md)
- [Cryptography](crypto/cryptography.md)
- [Crypto Migration Report](crypto/migration-report.md)

## Operations And Delivery

- [Quick Start](../quickstart.md)
- [Commands](operations/commands.md)
- [Runtime Model](operations/runtime-model.md)
- [Optimizations and Max Parallelization Speed](operations/optimizations-and-max-parallelization-speed.md)
- [Linux Quick Start](operations/linux-quick-start.md)
- [macOS Quick Start](operations/macos-quick-start.md)
- [Windows Quick Start](operations/windows-quick-start.md)
- [VPS Full Node](operations/vps-full-node.md)
- [Launch Checklist](operations/launch-checklist.md)
- [Dev Workspace](operations/dev-workspace.md)
- [Troubleshooting](operations/troubleshooting.md)
- [Build and Packaging](build-deployment/packaging.md)

## Quality, Testing, And Status

- [Testing and Hardening](testing-audits/testing-and-hardening.md)
- [Current Production Status](production-readiness/current-status.md)
- [Roadmap to Production](production-readiness/roadmap.md)
- [Release Notes](production-readiness/release-notes.md)
- [Historical Rebuild Roadmap](project/rebuild-roadmap.md)

## Research And Reference

- [Reference Materials](reference/reference-materials.md)
- [APA Whitepaper](whitepaper/atho-whitepaper-apa.md)

## Current Status Snapshot

As of the latest sandbox hardening pass:

- local consensus and storage paths are heavily tested
- the Qt client follows the real backend tip through RPC
- the Qt settings page now exposes operator-local peer and traffic diagnostics
- the node can mine, validate, persist, reload, and reorg locally
- DNS seeds are intentionally blank
- the live TCP peer runtime exists, but still needs broader public-network hardening and soak coverage

For the explicit rating, blockers, and missing pieces, use the production-readiness section instead of inferring from the code layout.
