# Roadmap to Production

## Priority Order

This roadmap is ordered by operational risk reduction and payoff, not by novelty.

## 1. Harden The Live Peer Runtime

Build:

- long multi-peer soak coverage
- latency and packet-loss harnesses
- ban/subnet enforcement integration tests
- broader propagation benchmarks

Why first:

- the real runtime exists now, but public-node claims are still premature without broader churn and soak coverage

## 2. Harden Pruning And Snapshot Paths

Build:

- deeper pruning execution coverage
- peer-served snapshot sync scaffolding
- pruning/reorg interaction tests

Why:

- storage maturity is incomplete without state-lifecycle coverage beyond the full-history path

## 3. Expand Schema Migration And Repair Tooling

Build:

- broader versioned migrations
- reindex/repair commands
- explicit operator guidance for upgrade failures

Why:

- reject-and-quarantine is safe, but not sufficient for long-lived deployments

## 4. Add Live Multi-Node Integration Soaks

Build:

- reorg over live peers
- multi-peer propagation races
- long reconnect churn coverage

Why:

- the network layer now needs scale and churn proof, not just existence proof

## 5. Add OS-Level Qt Automation

Build:

- automated navigation
- send/receive interaction tests
- status and tip checks through the rendered client

Why:

- method-level UI tests are not a full product verification substitute

## 6. Activate A Real V2 Test Ruleset

Build:

- one real post-V1 rule change
- one-block-before / at / after activation tests
- replay and restart behavior around activation

Why:

- upgrade scaffolding is only proven once a real rule passes through it

## 7. Strengthen Delivery And Release Discipline

Build:

- signing or checksum workflow
- cross-platform packaging validation
- release CI gates

Why:

- operational trust also depends on reproducible and inspectable delivery

## Related Documentation

- [Current Production Status](current-status.md)
- [Build and Packaging](../build-deployment/packaging.md)
