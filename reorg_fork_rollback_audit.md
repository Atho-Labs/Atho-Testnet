# Atho Reorg, Fork, Rollback, and Chain-State Safety Audit

## Ratings

- Overall reorg safety: 9/10
- Overall fork-handling safety: 9/10
- Overall rollback correctness: 9/10
- Parent/child/orphan handling: 8/10
- Cache invalidation safety: 9/10
- Mempool-after-reorg correctness: 9/10
- Wallet/GUI-after-reorg correctness: 8/10

## Executive Summary

The chain-transition path is materially safer after this audit.

Three high-risk issues were found and fixed:

1. `Chainstate::select_branch` could leave the in-memory chainstate partially mutated if the candidate branch failed after disconnecting the current suffix.
2. Buffered branch reconstruction was too peer-local, so parent arrival on a different peer could fail to wake the child branch buffered elsewhere.
3. Wallet cache invalidation and wallet scan state were too weakly tied to backend state, so tip/count changes alone could miss mempool drift or backend transitions.

The full release workspace test suite passed after the fixes, including the reorg/fork tests, sync tests, wallet readiness tests, and the new rollback regression.

## Exact Tests Run

- `cargo test -p atho-storage --lib select_branch_restores_exact_state_after_candidate_commit_failure -- --nocapture`
- `cargo test -p atho-node --lib cross_peer_branch_blocks_reconstruct_after_parent_arrives -- --nocapture`
- `cargo test -p atho-qt --lib wallet_cache_invalidates_when_mempool_fingerprint_changes -- --nocapture`
- `cargo test -p atho-qt --lib wallet_scan_waits_for_rpc_readiness -- --nocapture`
- `cargo test -p atho-qt --lib local_flag_uses_rpc_status_path_when_rpc_is_available -- --nocapture`
- `cargo test -p atho-qt --lib rpc_status_without_node_status_does_not_claim_readiness -- --nocapture`
- `cargo test --workspace --release --all-features`

## Exact Scenarios Tested

- One-block, multi-block, and longer-branch reorgs.
- Branch choice by accumulated work rather than raw height.
- Candidate-branch commit failure during `select_branch`.
- Parent arriving after child blocks were buffered.
- Parent arriving on a different peer than the buffered child branch.
- Invalid tip-height mismatch staying buffered.
- Node restart and chainstate reload paths.
- TCP runtime sync, relay, and reorg convergence paths.
- Wallet scan readiness and wallet cache invalidation after backend state changes.
- Local RPC fallback behavior when `NodeStatus` is not available.
- Release workspace regression coverage across storage, node, Qt, p2p, RPC, wallet, and crypto crates.

## Exact Hidden / Abstract Edge Cases Tested

- Branch rollback after a forced commit fault.
- Cross-peer orphan resolution, where the parent arrives on a different peer from the child branch.
- Chainstate restore after partial branch application.
- Wallet scan state changing mid-refresh.
- Wallet balance readiness following the canonical local block height instead of peer-advertised height.
- Partial RPC status responses that can report reachability without authoritative readiness.
- Reorgs after restart and after reconnect.
- Sync convergence after buffered branch replay.

## Issues Found

| Severity | Area | Issue | Status |
|---|---|---|---|
| High | Chainstate rollback | `select_branch` could leave partial in-memory state after candidate commit failure | Fixed |
| High | Branch buffering | Branch reconstruction was effectively peer-local and could miss cross-peer parent arrival | Fixed |
| High | Wallet state | Cache invalidation used tip/count heuristics that missed backend drift | Fixed |
| Medium | RPC fallback identity | Partial RPC status remains a reachability-only path without authoritative network identity | Open, non-consensus |
| Medium | Orphan lifecycle | Buffered/orphan branch lifetime and eviction policy are still implicit rather than fully specified | Open, non-consensus |
| Low | Restore failure semantics | Restoring exact state after a storage write failure still depends on recovery behavior being clear | Open, operational |

## Issues Fixed

### 1. Exact rollback restore in `Chainstate::select_branch`

File: `crates/atho-storage/src/chainstate.rs`

Function: `Chainstate::select_branch`

Fix:
- Captured the original tip, height, blocks, UTXO set, and undo stack before disconnecting the current suffix.
- On candidate connect failure, restored the exact original state instead of trying to replay the disconnected suffix.
- Persisted the restored state after in-memory restoration.

Regression:
- `select_branch_restores_exact_state_after_candidate_commit_failure`

### 2. Cross-peer buffered branch reconstruction

File: `crates/atho-node/src/sync.rs`

Functions:
- `process_buffered_branches_for_peer`
- `process_buffered_branches`
- `recoverable_branch_error`

Fix:
- Re-scans buffered branches across all peers after any new block is processed.
- Keeps scanning until no more progress is possible.
- Treats `ForkPointUnavailable` as a recoverable branch condition for buffered tips instead of trapping the branch forever.

Regression:
- `cross_peer_branch_blocks_reconstruct_after_parent_arrives`

### 3. Wallet cache invalidation and scan consistency

Files:
- `crates/atho-node/src/node.rs`
- `crates/atho-node/src/service.rs`
- `crates/atho-rpc/src/response.rs`
- `crates/atho-rpc/src/server.rs`
- `crates/atho-qt/src/connection.rs`
- `crates/atho-qt/src/app.rs`
- `crates/atho-qt/src/view.rs`

Fixes:
- Added `mempool_fingerprint` to node status propagation.
- Wired that fingerprint through RPC, Qt connection status, and the view model.
- Invalidated wallet caches when the fingerprint changes.
- Changed wallet scan height to follow the local canonical block count.
- Added a snapshot token so wallet scans are rejected and retried if backend state changes mid-refresh.
- Treated readiness as false unless the node is authoritative enough for wallet/mining use.

Regressions:
- `wallet_cache_invalidates_when_mempool_fingerprint_changes`
- `wallet_scan_waits_for_rpc_readiness`
- `local_flag_uses_rpc_status_path_when_rpc_is_available`
- `rpc_status_without_node_status_does_not_claim_readiness`

## Exact Consensus-Risk Findings

- No open consensus split risk remained after the fixes and the release workspace sweep.
- The only consensus-adjacent rollback risk found was the partial-state restoration issue in `select_branch`, and that is now fixed.
- Branch selection remains work-based, and the work comparison path is covered by release tests.

## Exact Stale-Cache Findings

- Stale wallet cache invalidation was the main cache issue found in this audit.
- The mempool fingerprint now guards against backend drift that count/tip heuristics missed.
- The wallet scan snapshot token now detects mid-refresh state changes and fails closed.
- No stale UTXO, signature, or branch-selection cache regression surfaced in the covered tests.

## Exact Rollback Corruption Findings

- Before the fix, a failed candidate branch connect could leave a partially mutated state if rollback relied on replaying the disconnected suffix.
- Exact snapshot restoration eliminates that corruption mode.
- The new regression confirms the restored height, tip hash, block count, and UTXO count match the pre-reorg state.

## Exact Branch-Selection Ambiguity Findings

- Raw height alone is not a safe branch choice metric.
- The existing work-based comparison is correct and was explicitly retested.
- The new rollback regression had to use a 3-block fork to avoid a tie/no-op fixture.

## Top 20 Highest-Risk Reorg / Fork Blockers

1. Exact rollback restore after candidate branch failure.
2. Cross-peer branch reconstruction when the parent arrives on a different peer.
3. Wallet cache invalidation tied only to tip/count changes.
4. Wallet scan state assembled from multiple RPC calls without a snapshot token.
5. Ambiguous readiness semantics around `headers_synced`.
6. Peer-local orphan lifetime and cleanup policy.
7. Partial RPC status paths that are reachability-only, not authoritative.
8. Restore-path persistence failure semantics.
9. Branch tie-break documentation for equal-work forks.
10. Explicit orphan eviction policy for never-completing branches.
11. Restart immediately after branch switch.
12. Reorg during wallet scan refresh.
13. Parent arrival during branch buffer processing.
14. Stale branch buffers surviving longer than intended.
15. GUI state advancing ahead of backend state.
16. Miner/template state being built from stale tip assumptions.
17. Missing explicit docs for reorg readiness and wallet spendability.
18. Need for a dedicated reorg fuzz harness over parent/child arrival order.
19. Need for a restart-plus-reorg soak harness.
20. Need for a peer churn / orphan pressure stress harness.

## Things We Had Not Thought About

- Rollback logic must restore exact state, not just reverse the last few operations, when a candidate branch can fail mid-apply.
- Child blocks buffered on one peer can be unblocked by a parent arriving on another peer, so branch recovery cannot stay peer-local.
- Wallet readiness is not the same thing as reachability; partial RPC responses can be “alive” without being authoritative.
- Wallet scan consistency needs a backend state token, not just count or tip comparisons.
- Canonical local chain height must drive spendability checks, not peer-advertised best height.

## Files / Modules / Functions Implicated

- `crates/atho-storage/src/chainstate.rs::select_branch`
- `crates/atho-storage/src/chainstate.rs::restore_chainstate_state`
- `crates/atho-storage/src/chainstate.rs::select_branch_restores_exact_state_after_candidate_commit_failure`
- `crates/atho-node/src/sync.rs::process_buffered_branches_for_peer`
- `crates/atho-node/src/sync.rs::process_buffered_branches`
- `crates/atho-node/src/sync.rs::recoverable_branch_error`
- `crates/atho-node/src/sync.rs::cross_peer_branch_blocks_reconstruct_after_parent_arrives`
- `crates/atho-node/src/node.rs::mempool_fingerprint`
- `crates/atho-node/src/service.rs::node_status`
- `crates/atho-rpc/src/response.rs::NodeStatus`
- `crates/atho-rpc/src/server.rs::status`
- `crates/atho-qt/src/connection.rs::collect_rpc_status`
- `crates/atho-qt/src/connection.rs::connection_status_from_node_status`
- `crates/atho-qt/src/app.rs::apply_connection_status`
- `crates/atho-qt/src/app.rs::build_wallet_scan_snapshot`
- `crates/atho-qt/src/app.rs::connection_snapshot_token`
- `crates/atho-qt/src/app.rs::wallet_scan_height`
- `crates/atho-qt/src/view.rs::ViewModel`

## Exact Issues Still Open

- No consensus-critical blockers remain in the covered reorg/fork/rollback paths.
- The remaining items are specification, lifecycle, and observability hardening.
- The biggest open non-consensus risks are orphan buffer lifecycle clarity and explicit readiness semantics.

## Final Roadmap

1. Add a dedicated reorg/fork fuzz harness over parent/child arrival order, restart timing, and orphan churn.
2. Document readiness semantics for `connected`, `running`, and `headers_synced`.
3. Define orphan buffer lifetime and eviction policy explicitly.
4. Add a restart-plus-reorg soak test that exercises partial branch switches.
5. Add a peer churn / orphan pressure stress test.
6. Add UI tests that assert wallet spendability never depends on peer-advertised height.
7. Add a persistence-failure regression for restore-path IO errors if the recovery model needs to be stricter.

## Final Decision

- Safe to merge: Yes for the covered reorg/fork/rollback fixes
- Needs more testing: Yes for the remaining spec and stress-harness gaps
- Blockers: None found in the covered consensus-critical chain-transition paths
