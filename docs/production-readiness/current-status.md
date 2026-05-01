# Current Production Status

## Overall Rating

- overall production-readiness: `8/10`
- local lifecycle completeness: `8/10`

Interpretation:

- the stack is materially beyond prototype stage
- the local consensus, storage, wallet, miner, RPC, and desktop paths are strong
- the full product still needs broader public-network and delivery hardening before it should be called production-ready

## Subsystem Ratings

| Area | Rating | Notes |
| --- | ---: | --- |
| Chain/bootstrap | 8/10 | startup, recovery, restart, and genesis paths are exercised |
| Mining | 8/10 | local mining lifecycle works and is tested |
| Block lifecycle | 8/10 | candidate assembly, validation, acceptance, and replay are exercised |
| Transaction lifecycle | 8/10 | signing, mempool, mining, confirmation, and history are exercised |
| UTXO lifecycle | 8/10 | apply, spend, rollback, maturity, and reload are covered |
| Mempool lifecycle | 8/10 | admission, conflicts, removal, and revalidation are covered |
| Wallet lifecycle | 8/10 | create/open/import/send/receive/history work through the backend path |
| Validation lifecycle | 8/10 | one canonical path with strong adversarial coverage |
| Persistence/restart | 8/10 | atomic local commits and recovery exist |
| Reorg lifecycle | 7/10 | deterministic local branch handling exists; deep pruned history is conservative |
| Fork lifecycle | 7/10 | local branch selection and rejection logic exist |
| Version/activation | 7/10 | centralized scaffolding exists; no active V2 deployment yet |
| Pruning lifecycle | 7/10 | isolated prune-test network exists and snapshot/pruning regressions are stronger, but lifecycle tooling is still incomplete |
| API/backend lifecycle | 8/10 | local RPC and service ownership are strong; public RPC is denied by default |
| GUI / Qt lifecycle | 8/10 | functional, backend-synced, and operationally cleaner |
| Qt sync-to-tip correctness | 8/10 | real RPC tip propagation is tested |

## What Is Done

- explicit network, version, and activation constants
- canonical local validation path
- atomic LMDB-backed chainstate commit model
- local corruption quarantine and recovery
- HD wallet with encrypted datafiles
- canonical wallet history API
- RPC-driven Qt client
- managed local-node Qt startup through the real RPC path
- real mined-block, send, confirm, restart, and rehydrate lifecycle coverage
- live TCP peer runtime with real-socket sync, reorg, restart, and transaction relay coverage
- 25-node live cluster convergence and recovery coverage
- isolated `prunetest` network for pruning, restart, and recovery regression work

## What Is Still Incomplete

- compact block burst hardening
- broader parallel downloader stress coverage
- peer-served snapshot sync
- pruning lifecycle hardening
- schema migration breadth and repair tooling
- OS-level GUI automation in CI
- active post-V1 upgrade execution
- long-run public-network soak coverage

## Highest-Risk Remaining Blockers

1. pruning and snapshot lifecycle coverage are incomplete
2. schema migration and repair coverage are still narrow
3. release/distribution hardening is incomplete
4. GUI automation is not yet at a product-verification level
5. long-run public-network soak coverage is incomplete
6. post-V1 upgrade execution is still not live

## Explicit Honesty Statement

What should not be claimed today:

- that Atho is fully production-ready
- that the peer network is fully hardened for hostile public deployment
- that pruning and snapshot modes are fully complete
- that upgrade/migration coverage is complete

## Related Documentation

- [Testing and Hardening](../testing-audits/testing-and-hardening.md)
- [Roadmap to Production](roadmap.md)
