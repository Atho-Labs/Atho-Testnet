# Atho Unknown Risks Audit

Date: 2026-04-29

## Ratings

| Area | Rating | Notes |
|---|---:|---|
| Overall unknown risks readiness | 6/10 | Core correctness is strong, but several cross-layer assumptions and lifecycle meanings still need to be made explicit. |
| Security-hardening | 8/10 | Two concrete leaks were fixed in this pass; the remaining issues are mostly lifecycle/spec gaps rather than direct consensus breaks. |
| Determinism | 7/10 | Consensus core is deterministic, but wallet and RPC status composition still have state-drift edges. |
| UTXO correctness | 8/10 | Core UTXO accounting is solid; wallet-side spendability and snapshot interpretation needed tightening. |
| Transaction lifecycle correctness | 8/10 | Transaction consensus is strong; wallet visibility and refresh timing still have edge cases. |
| Block creation/validation correctness | 8/10 | Block path remains sound; the main risks are around readiness interpretation and operator flow. |
| Coinbase/reward/monetary-policy correctness | 8/10 | No monetary policy bug was found in this pass. |
| Mempool correctness | 7/10 | The mempool itself is fine, but wallet invalidation depends on coarse signals. |
| Signature/witness verification correctness | 8/10 | No new signature bug was found in this pass. |

## Exact Systems Tested

| System | What Was Checked |
|---|---|
| `crates/atho-qt/src/app.rs` | Wallet scan readiness, wallet spendability height, startup gating, mining readiness, send/mining UI lifecycle. |
| `crates/atho-qt/src/connection.rs` | Managed local-node status, remote RPC fallback status, partial readiness semantics. |
| `crates/atho-qt/src/app/startup.rs` | Startup-screen wording and gate behavior. |
| `crates/atho-qt/src/app/wallet_ledger.rs` | Spendability classification from a supplied height. |
| `crates/atho-node/src/runtime.rs` | Runtime/start ordering and lifecycle meaning. |
| `crates/atho-node/src/orchestrator.rs` | Start/prime sequencing. |
| `crates/atho-node/src/service.rs` | Node status, RPC bridge, refresh behavior, peer graph seeding. |
| `crates/atho-node/src/sync.rs` | Headers-first sync and compact-block handling. |
| `crates/atho-node/src/tcp_p2p.rs` | P2P bootstrapping and listener startup sequencing. |
| `docs/architecture/lifecycle-flows.md` | Lifecycle wording and implicit readiness assumptions. |
| `docs/node-runtime/rpc-and-client.md` | RPC readiness and client/backend semantics. |
| `docs/node-runtime/node-runtime-and-p2p.md` | Network readiness, sync, and operator diagnostics. |
| `docs/production-readiness/current-status.md` | Explicitly documented maturity gaps. |
| `docs/wallet/wallet-model.md` | Wallet ownership boundaries and history limitations. |

## Exact State-Injection Campaigns Performed

| Campaign | Purpose | Result |
|---|---|---|
| Synthetic `ConnectionStatus` with `sync_best_height` far above `block_count` | Prove wallet spendability must follow local chain height only. | Exposed and fixed a wallet maturity leak. |
| Mock RPC server that returns `GetNodeStatus = MethodNotFound` but valid network/block/mempool answers | Prove partial RPC reachability must not masquerade as readiness. | Exposed and fixed a readiness leak. |
| Mock RPC server returning valid `GetNodeStatus` | Confirm the normal remote status path still works. | Passed. |
| Wallet scan readiness check with invalid RPC address | Confirm the scan gate still blocks when RPC is not ready. | Passed. |
| Full `atho-qt` library suite | Confirm the current UI and connection changes did not regress other library tests. | `cargo test -p atho-qt --lib` passed: 28 passed, 1 ignored. |

## What Was Fixed

| Severity | Issue | File | Status |
|---|---|---|---|
| High | Wallet spendability used `max(sync_best_height, block_count)`, which could make immature outputs appear spendable before the local canonical height actually reached them. | `crates/atho-qt/src/app.rs:547`, `crates/atho-qt/src/app.rs:762`, `crates/atho-qt/src/app.rs:1073` | Fixed. |
| High | Remote RPC fallback treated `GetNetwork` reachability as readiness, which could report `running/connected/headers_synced` even when `GetNodeStatus` was unavailable. | `crates/atho-qt/src/connection.rs:426` to `crates/atho-qt/src/connection.rs:484` | Fixed. |
| Medium | Remote RPC fallback discarded `mempool_total_fee_atoms`, reducing observability. | `crates/atho-qt/src/connection.rs:457` to `crates/atho-qt/src/connection.rs:483` | Fixed. |

## Top Hidden Assumptions Found

1. Wallet maturity should use the local canonical block height, not the best-known peer height.
2. `GetNodeStatus` is the only trustworthy readiness signal; `GetNetwork` alone is not enough.
3. A stable mempool count implies a stable mempool shape. It does not.
4. A wallet scan can be treated as a single snapshot even though it is assembled from multiple RPC calls. It cannot.
5. The UI can say "synchronized" without a crisp spec for whether that means local-tip synced or best-known-tip synced.

## Top Abstraction Leaks Found

1. Wallet spendability was leaking network-sync state into local UTXO maturity.
2. RPC reachability was leaking into node readiness.
3. Wallet refresh invalidation was keyed to coarse status deltas instead of a mempool fingerprint.
4. Startup-screen wording implied a stronger readiness contract than the code actually enforced.
5. `sync_best_height` is used both as a display number and, indirectly, as a readiness hint, which makes its meaning too slippery.

## Top Unhandled Lifecycle States Found

1. Node reachable but not authoritative enough to claim readiness.
2. Wallet loaded while the node is still syncing headers.
3. Wallet scan in flight while mempool contents change but the transaction count stays the same.
4. Wallet scan assembled from one status, one UTXO read, one mempool reservation read, and one history read, all potentially from different moments.
5. Startup path attached to a process that answers the right network label but is not necessarily the intended local node instance.

## Top Order-Of-Operations Risks Found

1. Wallet scan height was computed from the wrong semantic source before the fix.
2. Wallet scan reads status, UTXOs, mempool reservations, and history as separate calls with no snapshot token.
3. The GUI can move from startup gating to the main shell after the first scan even if the broader sync semantics are not clearly defined.
4. Local-node probe logic decides whether to spawn based on `GetNetwork` only.
5. Mining is gated on `connected` but not on an explicit `headers_synced` contract, so the UI can expose mining while the node may still be catching up.

## Top Operator / UX Risk Paths Found

1. Users can see spendable funds too early if wallet maturity is interpreted against the wrong height. Fixed.
2. Users can see a backend as connected even when the RPC surface is only partially responsive. Fixed.
3. Users can assume "Refreshing wallet" means the wallet is fully synchronized, but the code does not enforce a crisp sync definition.
4. Users can attach to the wrong local process if it answers the expected network label on the expected RPC port.
5. Users can act on stale wallet/mempool state when mempool contents change without a count change.

## Top Missing Invariants Found

1. Wallet spendability must be derived from the local canonical height only.
2. A readiness signal must be defined separately from a reachability signal.
3. Wallet refresh invalidation must track mempool content, not just mempool count.
4. Wallet snapshot assembly should have a defined consistency boundary.
5. Startup and mining readiness should be explicitly defined in terms of local tip sync, not just generic connected/running flags.

## Top Missing Specifications Found

1. What exactly does `connected` mean in the GUI?
2. What exactly does `running` mean in the GUI?
3. What exactly does `headers_synced` mean relative to wallet readiness?
4. Does wallet usability require local-tip sync, best-known-tip sync, or only RPC reachability?
5. Should mining be blocked until the node is headers-synced?
6. Is the `GetNetwork` probe sufficient to decide that the intended local node is already present?
7. Is wallet scanning allowed to assemble state from multiple RPC moments, or should it be atomic/versioned?

## Top Observability Blind Spots Found

1. The startup watcher logs that the node is "still starting" after a timeout, but the user-facing state is not equally explicit.
2. A mempool reshuffle with the same count can leave the wallet cache stale without an obvious signal.
3. The fallback RPC path previously looked healthy enough to advance UI state even when node-status information was missing. Fixed.
4. Wallet scan logs include a height value, but that height meaning was previously ambiguous. Fixed in code, but the documentation still needs to say it clearly.
5. The current docs mention readiness and lifecycle, but they do not define the status fields tightly enough for operators.

## Top Weird / Abstract Failure Cases Found

1. Peer best height jumps ahead of local block count and the wallet mistakenly classifies immature UTXOs as available. Fixed.
2. RPC can answer `GetNetwork` but not `GetNodeStatus`, creating a false sense of readiness. Fixed.
3. Mempool contents can change while the transaction count stays constant, leaving stale wallet reservations.
4. A user can be shown a "ready" wallet while the node is still only partially synced.
5. A local process can answer the expected network label on the expected port but still not be the intended node instance.

## Exact Issues Still Open

| Severity | Issue | Why It Matters | Needs |
|---|---|---|---|
| High | Wallet cache invalidation uses block count, mempool count, and tip hash only. It does not use a mempool fingerprint, so a one-for-one mempool replacement can leave stale wallet reservations. | Wallet balances and sendability can drift without an obvious refresh trigger. | New test harness or a status fingerprint field. |
| High | Wallet scan state is assembled from multiple RPC calls with no snapshot token. | The GUI can show a self-inconsistent wallet view under reorg or mempool churn. | Atomic snapshot API or versioned snapshot contract. |
| Medium | The UI still does not define whether wallet readiness requires `headers_synced`. | Operators may act on a wallet that is usable locally but not clearly synchronized. | Documentation and likely a product decision. |
| Medium | Mining readiness is still based on `connected` rather than a tighter sync contract. | Mining can start on a backend that is reachable but not clearly synced. | Documentation or a stricter gating rule. |
| Medium | Local-node probing only checks `GetNetwork` before deciding to skip spawn. | The app can attach to the wrong local process if it speaks the right network label. | Identity check or operator guidance. |
| Low | The startup watcher does not surface a distinct user-facing timeout state after long bootstrap delay. | Troubleshooting is slower. | Better diagnostics. |

## Exact Issues Fixed In This Pass

1. Wallet spendability now uses local block height only.
2. Remote RPC fallback no longer claims readiness without `GetNodeStatus`.
3. Remote RPC fallback now preserves `mempool_total_fee_atoms`.

## Exact Files / Modules / Functions Implicated

| File | Functions / Areas |
|---|---|
| `crates/atho-qt/src/app.rs` | `refresh_wallet_cache`, `build_wallet_scan_snapshot`, `wallet_scan_rpc_ready`, `wallet_scan_height`, `apply_connection_status`, `wallet_readiness_blocks_main_ui`, `start_mining_job`, startup gating. |
| `crates/atho-qt/src/connection.rs` | `collect_rpc_status`, `status`, `spawn_status_monitor`, partial readiness fallback. |
| `crates/atho-qt/src/app/startup.rs` | `render_wallet_preparation_screen`. |
| `crates/atho-qt/src/app/wallet_ledger.rs` | `summarize_wallet_utxos`. |
| `crates/atho-node/src/service.rs` | `status`, `node_status`, `refresh_runtime_views`. |
| `crates/atho-node/src/orchestrator.rs` | `start`, `stop`. |
| `crates/atho-node/src/runtime.rs` | `run_with_config`, `start`, `stop`. |
| `crates/atho-node/src/tcp_p2p.rs` | `bind_shared`, bootstrap sequencing. |
| `docs/node-runtime/rpc-and-client.md` | RPC readiness and current limitations. |
| `docs/architecture/lifecycle-flows.md` | Wallet and startup lifecycle wording. |
| `docs/production-readiness/current-status.md` | Readiness framing and remaining gaps. |
| `docs/wallet/wallet-model.md` | Wallet ownership boundaries and history limitations. |

## Exact Campaigns Performed

| Type | Commands / Inputs |
|---|---|
| Targeted regression | `cargo test -p atho-qt wallet_scan_height_tracks_local_block_count -- --nocapture` |
| Targeted regression | `cargo test -p atho-qt wallet_scan_waits_for_rpc_readiness -- --nocapture` |
| Targeted regression | `cargo test -p atho-qt local_flag_uses_rpc_status_path_when_rpc_is_available -- --nocapture` |
| Targeted regression | `cargo test -p atho-qt rpc_status_without_node_status_does_not_claim_readiness -- --nocapture` |
| State injection | Synthetic status with `sync_best_height` much higher than `block_count`. |
| State injection | Mock RPC with `GetNodeStatus = MethodNotFound` plus valid network/block/mempool responses. |

## Things We Had Not Thought About

1. Wallet maturity can silently drift if a display height is reused as an accounting height.
2. Reachability and readiness are not the same thing, but the fallback RPC path had blurred them.
3. Mempool churn can be invisible to a dirty-flag that only watches counts.
4. A wallet scan can be internally inconsistent even when every individual RPC call is valid.
5. The current docs do not define the exact readiness boundary for wallet usability.

## Final Roadmap

1. Add a versioned or atomic wallet snapshot API so status, UTXOs, mempool reservations, and history come from one backend state boundary.
2. Add a mempool fingerprint or change token so wallet cache invalidation is driven by content, not just count.
3. Define `connected`, `running`, `headers_synced`, and wallet-ready semantics explicitly in docs and code.
4. Decide whether mining should require `headers_synced` and make that policy explicit.
5. Tighten local-node probe identity so `GetNetwork` alone does not decide whether an existing process is the intended backend.
6. Add diagnostics for stale-wallet and partial-readiness states so operators can see why the GUI is waiting or stale.

## Final Decision

- Safe to merge: No
- Needs more testing: Yes
- Blockers: atomic wallet snapshot semantics, mempool fingerprinting for wallet refresh, and explicit readiness specs for wallet/mining behavior.
