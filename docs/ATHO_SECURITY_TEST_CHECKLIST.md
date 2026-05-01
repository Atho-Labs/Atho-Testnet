# Atho Security Test Checklist

This checklist is the operator and release-facing security bar for Atho.

Use it before public testnet promotion and before any mainnet-readiness claim.

## Consensus

- [x] Invalid PoW is rejected.
- [x] Wrong previous-hash blocks are rejected.
- [x] Wrong network/genesis identifiers are rejected.
- [x] Invalid Falcon signatures are rejected.
- [x] Oversized or truncated witness payloads are rejected early.
- [x] Reorg branch choice is work-based, not raw-height-based.
- [x] Snapshot import validates tip identity before acceptance.
- [ ] Full pruning-window rollback rejection is covered by dedicated prune-window tests.
- [ ] Coinbase maturity edge cases are covered across deeper reorgs and prune boundaries.

## Network Separation

- [x] Mainnet, testnet, regnet, and prunetest have distinct IDs.
- [x] Mainnet, testnet, regnet, and prunetest have distinct P2P magic values.
- [x] Mainnet, testnet, regnet, and prunetest have distinct ports.
- [x] Mainnet, testnet, regnet, and prunetest have distinct address prefixes.
- [x] Mainnet, testnet, regnet, and prunetest have distinct database roots.
- [ ] Wrong-network RPC/template/block-injection coverage is still broader-release work.

## P2P Abuse Resistance

- [x] Wrong-genesis handshakes are rejected.
- [x] Oversized P2P payloads are rejected before allocation.
- [x] Pre-handshake invalid messages disconnect peers.
- [x] Multi-node TCP convergence and reconnect tests exist.
- [ ] DNS seed crawler and bootstrap-only coverage are still missing.
- [ ] Eclipse/Sybil hostile-mesh coverage is still missing at the target scale.
- [ ] Long-run public-network churn and flood soak is still missing.

## Storage and Recovery

- [x] Malformed snapshots fail closed.
- [x] Incomplete persisted history is quarantined and rebuilt.
- [x] Persisted tip mismatches fail closed.
- [x] Unsupported schema versions fail closed.
- [x] Schema v2 migration forward path is tested.
- [ ] Explicit `repair` CLI is still missing.
- [ ] Explicit `reindex` CLI is still missing.
- [ ] Crash-during-prune and prune-window rollback refusal still need dedicated automation.

## Wallet and Secrets

- [x] Wallet datafile errors use structured `ATHO-*` codes.
- [x] Wallet errors avoid leaking mnemonic or secret material.
- [x] Wrong-format mnemonic and corrupted wallet data are rejected.
- [ ] Cross-network wallet address rejection needs broader end-to-end coverage through UI and RPC.

## RPC and User-Facing Errors

- [x] RPC errors serialize as structured typed responses.
- [x] Critical runtime/storage/mining paths map to explicit `ATHO-*` codes.
- [x] GPU failures are coded and surfaced to CLI/Qt.
- [ ] Full malformed-input coverage for every RPC endpoint is still incomplete.
- [ ] Stable structured error-code assertions for every launcher/CLI workflow are still incomplete.

## Mining

- [x] CPU miner is the canonical reference path.
- [x] GPU path cannot silently replace canonical CPU validation.
- [x] `gpu` requires a real GPU.
- [x] `auto` prefers GPU and falls back to CPU with a reason.
- [x] Canonical header bytes for miners are tested.
- [x] Prunetest network mines and reloads through an isolated root.
- [ ] Per-device autotuning is still missing.
- [ ] Multi-runtime GPU backend selection beyond OpenCL is still missing.

## Logging

- [x] Central error registry exists.
- [x] Structured error log lines include code, severity, and module.
- [x] Wallet error paths avoid leaking secret material.
- [ ] Full repo-wide elimination of ad hoc string-only operator errors is still incomplete.

## Release Blockers Still Open

- [ ] Dedicated prune-window rollback tests
- [ ] repair CLI
- [ ] reindex CLI
- [ ] snapshot sync over P2P
- [ ] DNS seed infrastructure
- [ ] hostile mesh automation
- [ ] 24h+ soak automation
- [ ] cross-OS desktop smoke automation

## Minimum Commands Before a Release Review

```bash
cargo test -p atho-errors
cargo test -p atho-core
cargo test -p atho-crypto
cargo test -p atho-storage
cargo test -p atho-p2p
cargo test -p atho-rpc
cargo test -p atho-wallet
cargo test -p atho-node
cargo check --manifest-path fuzz/Cargo.toml --all-targets
```

## Additional Hardening Commands

```bash
cargo test -p atho-node tcp_runtime_25_node_cluster_converges_restarts_and_recovers -- --exact
cargo test -p atho-node node::tests::prunetest_node_mines_and_restarts_in_an_isolated_database_root -- --exact
cargo test -p atho-storage chainstate::tests::snapshot_bundle_uses_canonical_storage_after_pruning_memory_tail -- --exact
```
