# Atho Architecture and Topology Audit

Date: 2026-05-07

## Scope

This audit reviewed the Atho codebase for:

- topology and module boundaries
- hot-path performance
- consensus isolation
- P2P and sync structure
- storage and indexing layout
- lock/contention risks
- API and explorer isolation
- Bitcoin-style reliability and long-term maintainability

## Current Topology Summary

The project is already organized around crate boundaries instead of a single
monolithic `src/` tree. In practice, this gives Atho the same separation the
requested topology was aiming for:

- `atho-core`
  - canonical protocol types, consensus rules, PoW, subsidy, transaction policy,
    signatures, encoding, network identity
- `atho-storage`
  - LMDB-backed storage, chainstate persistence, UTXO set, validation helpers
- `atho-p2p`
  - networking transport, handshake, codec, peer/session logic, relay, sync helpers
- `atho-node`
  - orchestrator, runtime, mempool, mining, API, explorer index, service façade
- `atho-wallet`
  - wallet snapshot and recovery helpers
- `atho-rpc`
  - RPC command/request/response surface
- `atho-qt`
  - client UI layer

This is a healthy structure overall. Consensus, storage, networking, API, and UI
are already separated at the crate level.

## Architectural Strengths

1. Consensus logic is centralized in `atho-core` instead of leaking into the API
   or UI surface.
2. Storage responsibilities are isolated in `atho-storage`.
3. Explorer read paths are separated behind an index layer instead of forcing
   request-time full-chain scans.
4. The node/service layer is mostly acting as a façade around validated runtime
   actions rather than bypassing validation.
5. The crate layout is much closer to Bitcoin-style subsystem isolation than a
   typical application-shaped Rust project.

## Hot Paths Reviewed

The following hot paths were reviewed:

- transaction deserialization
- transaction validation
- transaction PoW generation and verification
- signature verification
- difficulty retarget calculation
- UTXO lookup and block connection
- mempool admission and revalidation
- block template construction
- explorer index rebuild
- API chain/network stats refresh
- P2P message handling and shared service locking

## Improvements Made

### 1. Difficulty retarget hot path no longer clones header vectors

The PoW retargeting path in `atho-core/src/consensus/pow.rs` previously cloned
block headers into temporary vectors before computing median time past and the
next target.

That temporary cloning was removed from the active code path. Retarget
calculation now operates directly over borrowed block slices.

Why this matters:

- retarget logic sits on the validation/mining path
- this removes repeated header allocations and copies
- the change preserves consensus behavior

### 2. Reference-equivalence guard added

A regression test was added so the optimized path must match the previous
clone-based reference logic exactly:

- `consensus::pow::tests::next_target_matches_clone_based_reference_path`

### 3. Benchmark pair added

The benchmark suite now includes a direct comparison between:

- `pow_next_target_clone_heavy_reference`
- `pow_next_target_current`

This makes the topology improvement measurable instead of anecdotal.

## Benchmark Results

### Difficulty retarget path

- clone-heavy reference:
  - `6.3653 µs .. 6.7284 µs .. 7.2551 µs`
- current optimized path:
  - `2.6098 µs .. 2.7868 µs .. 3.0295 µs`

That is roughly a 58% median reduction in retarget-path latency.

### Previously measured TX PoW improvement

From the earlier TX PoW optimization pass:

- single-thread solve:
  - `188.06 ms .. 264.34 ms .. 330.25 ms`
- auto-thread solve:
  - `86.070 ms .. 123.68 ms .. 153.16 ms`

That remains roughly a 53% median improvement.

## Bottlenecks and Risks Found

### 1. Canonical chain retained as in-memory `Vec<Block>`

`atho-storage::Chainstate` keeps the active in-memory chain as `Vec<Block>`.

This is simple and deterministic, which is good, but it becomes a scaling risk:

- memory growth tracks retained canonical block history
- some chainwork / difficulty / branch operations still reason over full block slices
- it is not the long-term ideal for Bitcoin-style node scale

This is the single biggest topology concern found.

### 2. Explorer rebuild is full-chain/full-UTXO when rebuilding

Request-time explorer reads are indexed, which is good.

However, `ExplorerIndex::rebuild` still:

- loads canonical blocks
- walks the chain
- walks the UTXO snapshot

That is acceptable for rebuilds and startup recovery, but it is not cheap. It is
the right place for future incremental indexing work.

### 3. Wallet activity path still uses canonical full-chain reads

`NodeService::wallet_activity` reconstructs wallet activity from
`canonical_blocks()`.

This is safe and deterministic, but it is not a cheap path if called often on a
larger chain.

### 4. P2P runtime uses shared `Arc<Mutex<NodeService>>`

`tcp_p2p.rs` makes heavy use of `Arc<Mutex<NodeService>>`.

This is workable and simple, but it is the main contention risk under load:

- many peer events lock the same state object
- service lock scope can grow as features are added
- this is the place most likely to need future lock splitting

### 5. Some read APIs clone more than they need

Examples include mempool entry/transaction snapshot helpers that return cloned
vectors and objects. This is fine for correctness, but it adds avoidable
allocation overhead on read-heavy paths.

## Full-Chain Scan Findings

No evidence was found that the public explorer/API does full-chain scans on
normal request paths.

That is a strong architectural property.

What does still walk the chain:

- explorer index rebuilds
- wallet activity derivation
- some chain-stat snapshot/rebuild paths

Those are acceptable for maintenance/rebuild flows, but they are still
important scaling targets.

## Consensus Topology Findings

Consensus remains properly centralized in `atho-core`:

- PoW rules
- subsidy schedule
- transaction policy / TX PoW
- signature hashing and validation rules
- network identity and address logic

No new duplicated consensus logic was introduced by the optimization pass.

## Coupling Findings

### Good coupling

- `atho-node` depends on `atho-core`, `atho-storage`, and `atho-p2p` in the
  expected direction
- UI and RPC do not appear to own consensus logic

### Risky coupling

- `NodeService` is simultaneously:
  - RPC/API façade
  - explorer cache owner
  - runtime diagnostics formatter
  - P2P shared state sink

This is not broken, but it is a concentration point. Future scale work should
split hot runtime state from presentation/cache state.

## Storage and Indexing Findings

### Good

- network-specific storage paths exist
- persisted block/tx lookups exist
- UTXO snapshot and chainstate commit are already batch-oriented
- explorer index exists specifically to avoid request-time chain scans

### Future improvement target

Split consensus-critical chainstate storage from explorer/cache storage more
aggressively if startup or rebuild time becomes a problem.

## Locking and Concurrency Findings

Most important risk:

- `tcp_p2p.rs` shared `NodeService` mutex

Recommended next architecture step:

1. keep consensus commit state narrow
2. split explorer/API cache refresh paths from peer relay state
3. reduce the amount of work done while holding the service lock

This should be treated as the main future concurrency optimization target.

## Regression Results

The optimization pass was validated with:

- `cargo test -p atho-core -- --nocapture`
- `cargo test -p atho-node --test protocol_fixtures -- --nocapture`

Both passed after the retarget-path optimization.

## Final Readiness Status

### Topology readiness

PASS

The crate structure is already strong and much closer to the requested model
than expected.

### Performance readiness

PASS, with clear future targets

Safe improvements landed and benchmarked well. The current structure is serviceable,
but the shared service lock and in-memory canonical block vector are the two
largest long-term pressure points.

### Security / consensus readiness

PASS

No consensus behavior changed in the retarget optimization. The new test locks
that in explicitly.

## Recommended Next Steps

1. Introduce a lighter-weight header/tip history view so difficulty/chainwork
   calculations do not conceptually depend on whole `Block` objects.
2. Incrementalize explorer index rebuilds further so they advance from tip deltas
   instead of full rebuilds.
3. Split `NodeService` state into:
   - consensus/runtime state
   - peer/runtime diagnostics
   - API/explorer presentation caches
4. Reduce clone-heavy read APIs in mempool and service paths where possible.
5. Add allocation-focused benches for:
   - explorer rebuild
   - wallet activity derivation
   - mempool snapshot reads
