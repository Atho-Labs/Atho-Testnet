# Testing and Hardening

## Strategy

Atho is being hardened through layered testing instead of relying on one test style:

- unit tests for protocol and wallet primitives
- integration tests for node, storage, RPC, P2P, and Qt behavior
- lifecycle tests for mining, sending, restart, and UI/backend synchronization
- adversarial mutation campaigns
- targeted attack harnesses

Why:

- consensus correctness failures and lifecycle regressions appear in different places

## What Has Been Exercised

Recent sandbox execution has covered:

- fresh startup
- genesis initialization
- block mining and acceptance
- transaction creation, signing, mempool admission, and confirmation
- UTXO maturation and spendability
- restart and reload
- chainstate quarantine/recovery
- reorg and branch-selection tests
- RPC-driven Qt tip synchronization
- Qt wallet create/open/send/receive/mining lifecycle
- live TCP two-node sync/reorg and transaction relay
- 25-node live cluster convergence and restart recovery
- P2P handshake, message framing, inventory handling, and headers-first logic

## Recent Executed Commands

Validated command set:

```bash
cargo test -p atho-core -- --test-threads=1
cargo test -p atho-wallet -- --test-threads=1
cargo test -p atho-p2p -- --test-threads=1
cargo test -p atho-storage -- --test-threads=1
cargo test -p atho-node -- --test-threads=1
cargo test -p atho-qt -- --test-threads=1
cargo run -p atho-node --bin atho-attack -- --network regnet
cargo run --release -p atho-node --bin atho-adversarial -- --cases 52000 --seed 12345
```

## Most Recent Result Snapshot

Passing counts from the most recent documented full-stack pass:

- `atho-core`: `35 passed`
- `atho-wallet`: `25 passed`
- `atho-p2p`: `25 passed`
- `atho-storage`: `25 passed`
- `atho-node`: `42 passed`
- `atho-qt`: `25 passed`
- targeted attack sweep: `19/19 passed`

Adversarial campaign result:

- `campaign_cases=52300`
- `campaign_unexpected_accept=0`
- `campaign_unexpected_reject=0`
- `campaign_panics=0`
- `campaign_mismatches=0`
- `campaign_silent_accepts=0`
- `campaign_silent_state_divergence=0`

## What These Tests Prove

They materially raise confidence in:

- local consensus determinism
- storage replay/recovery
- Qt tip synchronization through the real backend path
- mining and transaction lifecycle correctness
- rejection of malformed or adversarial local inputs

## What They Do Not Yet Prove

- full pruning lifecycle safety
- snapshot sync correctness
- schema migration safety
- OS-level GUI interaction correctness
- active post-V1 consensus upgrade execution

## Current Highest-Value Missing Tests

1. 50-node real-socket soak
2. longer restart / reconnect soak across live peers
3. pruning + restart execution
4. snapshot sync lifecycle
5. schema migration tests
6. GUI automation at the window/control level
7. long-run public-network soak
8. activation-boundary execution with a real V2 ruleset

## Related Documentation

- [Current Production Status](../production-readiness/current-status.md)
- [Roadmap to Production](../production-readiness/roadmap.md)
