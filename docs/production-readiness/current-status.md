# Current Production Status

## Overall Rating

- overall production-readiness: `7/10`
- local lifecycle completeness: `8/10`

Interpretation:

- the stack is materially beyond prototype stage
- the core local validation path is strong
- the full product is still not ready for production deployment

## Subsystem Ratings

| Area | Rating | Notes |
| --- | ---: | --- |
| Chain/bootstrap | 8/10 | startup, recovery, restart, and genesis paths are exercised |
| Mining | 8/10 | local mining lifecycle works and is tested |
| Block lifecycle | 8/10 | candidate assembly, validation, acceptance, and replay are exercised |
| Transaction lifecycle | 7/10 | signing, mempool, mining, and confirmation work; history model still needs cleanup |
| UTXO lifecycle | 8/10 | apply, spend, rollback, maturity, and reload are covered |
| Mempool lifecycle | 8/10 | admission, conflicts, removal, and revalidation are covered |
| Wallet lifecycle | 7/10 | create/open/import/send/receive work; history remains indirect |
| Validation lifecycle | 8/10 | one canonical path with strong adversarial coverage |
| Persistence/restart | 8/10 | atomic local commits and recovery exist |
| Reorg lifecycle | 7/10 | deterministic local branch handling exists; deep pruned history is conservative |
| Fork lifecycle | 7/10 | local branch selection and rejection logic exist |
| Version/activation | 7/10 | centralized scaffolding exists; no active V2 execution yet |
| Pruning lifecycle | 5/10 | constants and guardrails exist, but coverage is limited |
| API/backend lifecycle | 8/10 | local RPC and service ownership are strong |
| GUI / Qt lifecycle | 7/10 | functional and backend-synced, but not OS-automated |
| Qt sync-to-tip correctness | 8/10 | real RPC tip propagation is tested |

## What Is Done

- explicit network, version, and activation constants
- canonical local validation path
- atomic LMDB-backed chainstate commit model
- local corruption quarantine and recovery
- HD wallet with encrypted datafiles
- RPC-driven Qt client
- real mined-block, send, confirm, restart, and rehydrate lifecycle coverage
- P2P message, handshake, and sync foundation

## What Is Still Incomplete

- live TCP peer runtime in the daemon
- compact block relay
- parallel block downloader
- snapshot sync
- pruning lifecycle hardening
- schema migration tooling
- canonical wallet history API
- OS-level GUI automation
- active post-V1 upgrade execution

## Highest-Risk Remaining Blockers

1. live socketed peer runtime is incomplete
2. wallet history still depends on ledger reconstruction rather than a canonical backend history API
3. pruning and snapshot lifecycle coverage are incomplete
4. schema migrations do not exist yet
5. release/distribution hardening is incomplete
6. GUI automation is not yet at a product-verification level

## Explicit Honesty Statement

What should not be claimed today:

- that Atho is production-ready
- that the peer network is hardened for public deployment
- that pruning and snapshot modes are fully complete
- that wallet history architecture is final

## Related Documentation

- [Testing and Hardening](../testing-audits/testing-and-hardening.md)
- [Roadmap to Production](roadmap.md)
