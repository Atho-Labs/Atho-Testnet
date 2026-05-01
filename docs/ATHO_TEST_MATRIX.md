# Atho Test Matrix

This matrix tracks current verification coverage, active release gates, and remaining gaps.

Status values:

- `green`: implemented and exercised
- `yellow`: partially implemented or only lightly covered
- `red`: still a release blocker

| Area | Current Coverage | Command / Location | Status | Release Gate |
| --- | --- | --- | --- | --- |
| Error registry | Central `ATHO-*` descriptors, doc sync, JSON sync, uniqueness checks | `cargo test -p atho-errors` | green | yes |
| Core consensus constants | Network IDs, prefixes, genesis metadata, PoW bounds, subsidy schedule | `cargo test -p atho-core` | green | yes |
| Falcon signatures | exact-size enforcement, verification regression coverage | `cargo test -p atho-crypto` | green | yes |
| Transaction validation | malformed witness rejection, oversized witness rejection, version gating | `cargo test -p atho-storage` | green | yes |
| UTXO accounting | apply, spend, rollback, atomic rollback on failure | `cargo test -p atho-storage` | green | yes |
| Reorg selection | work-based branch preference, rollback/reconnect | `cargo test -p atho-storage` and `cargo test -p atho-node` | green | yes |
| Snapshot export/import | deterministic bundle export and import-after-restart | `cargo test -p atho-node node::tests::node_imports_snapshot_bundle_and_keeps_mining_after_restart -- --exact` | green | yes |
| Pruning memory-tail logic | retained-window pruning and snapshot export after pruning | `cargo test -p atho-storage chainstate::tests::chainstate_prunes_old_history_after_retention_window -- --exact` and `cargo test -p atho-storage chainstate::tests::snapshot_bundle_uses_canonical_storage_after_pruning_memory_tail -- --exact` | green | yes |
| Prune-test isolation | dedicated network id, magic, ports, address prefix, DB root, runtime parsing, restart flow | `cargo test -p atho-core genesis::tests::genesis_state_is_network_scoped -- --exact`, `cargo test -p atho-node node::tests::prunetest_node_mines_and_restarts_in_an_isolated_database_root -- --exact` | green | yes |
| Database migration | schema v2 forward migration and fail-closed unsupported schema | `cargo test -p atho-storage db::tests::schema_version_two_migrates_forward_in_place -- --exact` | green | yes |
| Corruption quarantine | malformed snapshot, tip mismatch, incomplete history quarantine | `cargo test -p atho-storage` | green | yes |
| P2P handshake | version/verack path, wrong genesis rejection, oversized limits | `cargo test -p atho-p2p` | green | yes |
| Real TCP mesh | multi-node sync, relay, reconnect, 25-node convergence | `cargo test -p atho-node tcp_runtime_25_node_cluster_converges_restarts_and_recovers -- --exact` | green | yes |
| Compact blocks | short-id reconstruction and missing-tx recovery path | `cargo test -p atho-p2p` and `cargo test -p atho-node` | green | yes |
| RPC error handling | structured `RpcError` serialization and service-path mapping | `cargo test -p atho-rpc` and `cargo test -p atho-node service::tests::block_template_exposes_canonical_header_bytes_for_miners -- --exact` | green | yes |
| Mining CPU path | canonical solver reference, stale/cancel fallback coverage | `cargo test -p atho-node` | green | yes |
| Mining GPU path | probe info, strict GPU mode, auto fallback, coded failures | `cargo test -p atho-node --features gpu-native` and `cargo test -p atho-gpu-native --features gpu-native` | yellow | yes |
| Wallet safety | mnemonic/datafile validation and sanitized errors | `cargo test -p atho-wallet` | green | yes |
| Address/network isolation | visible-prefix decode, HPK prefix separation, wrong-prefix rejection | `cargo test -p atho-core address::tests::prunetest_addresses_round_trip_as_prunetest -- --exact` | green | yes |
| Parser fuzz targets | fuzz target compilation and parser hardening scaffolding | `cargo check --manifest-path fuzz/Cargo.toml --all-targets` | yellow | yes |
| Repair CLI | explicit user-facing repair entrypoint | pending | red | yes |
| Reindex CLI | explicit chainstate rebuild entrypoint | pending | red | yes |
| Snapshot sync over P2P | peer-served chunked sync with validation | pending | red | yes |
| DNS seed bootstrap | crawler/seed daemon and bootstrap-only startup path | pending | red | yes |
| Hostile mesh | Sybil/eclipse/flood simulations at scale | pending | red | yes |
| 24h soak | scheduled long-run convergence and memory/disk health | pending | red | yes |
| Cross-OS desktop smoke | packaged Qt startup automation on macOS/Linux/Windows | pending | red | yes |

## Current CI Mapping

### Quick

- `cargo test -p atho-errors`
- `cargo test -p atho-core`
- `cargo test -p atho-crypto`
- `cargo test -p atho-storage`
- `cargo test -p atho-p2p`
- `cargo test -p atho-rpc`
- `cargo test -p atho-wallet`
- `cargo test -p atho-node`
- `cargo check --manifest-path fuzz/Cargo.toml --all-targets`

### Nightly

- full workspace test run
- prune-test restart flow
- pruning snapshot/export paths
- 25-node TCP convergence test
- cross-OS node build smoke

## Interpretation

The matrix is intentionally conservative.

- `green` means there is current code and current automated verification.
- `yellow` means the path exists but coverage depth is still below production claim quality.
- `red` means the feature is still a release blocker or only a plan item.
