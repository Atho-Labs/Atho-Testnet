# Roadmap to Production

## Priority Order

This roadmap is ordered by risk reduction and payoff, not by novelty.

## 1. Finish The Live Peer Runtime

Build:

- TCP inbound/outbound loop
- timeouts and disconnect handling
- handshake integration
- real headers-first sync across processes

Why first:

- the current network layer is a strong foundation, but public-node claims are premature without a real runtime

## 2. Replace Wallet Ledger Reconstruction

Build:

- canonical backend history API
- wallet-history queries that do not depend on TSV exports
- Qt history sourced from backend truth

Why second:

- it is the biggest remaining product-integrity gap in the wallet/GUI boundary

## 3. Harden Pruning And Snapshot Paths

Build:

- execution coverage for pruning
- snapshot sync scaffolding
- pruning/reorg interaction tests

Why:

- storage maturity is incomplete without state-lifecycle coverage beyond the full-history path

## 4. Add Schema Migration And Repair Tooling

Build:

- versioned migrations
- reindex/repair commands
- explicit operator guidance for upgrade failures

Why:

- reject-and-quarantine is safe, but not sufficient for long-lived deployments

## 5. Add Live Multi-Node Integration Tests

Build:

- two-node TCP handshake
- block relay
- tx relay
- reorg over live peers

Why:

- the network layer needs end-to-end proof, not just module-level tests

## 6. Add OS-Level Qt Automation

Build:

- automated navigation
- send/receive interaction tests
- status and tip checks through the rendered client

Why:

- method-level UI tests are not a full product verification substitute

## 7. Activate A Real V2 Test Ruleset

Build:

- one real post-V1 rule change
- one-block-before / at / after activation tests
- replay and restart behavior around activation

Why:

- upgrade scaffolding is only proven once a real rule passes through it

## 8. Strengthen Delivery And Release Discipline

Build:

- signing or checksum workflow
- cross-platform packaging validation
- release CI gates

Why:

- operational trust also depends on reproducible and inspectable delivery

## Related Documentation

- [Current Production Status](current-status.md)
- [Build and Packaging](../build-deployment/packaging.md)
