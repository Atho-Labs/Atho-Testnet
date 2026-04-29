# Optimizations and Max Parallelization Speed

This note collects the speed work that has already been applied to Atho and the next safe optimizations worth considering.

The goal is simple:

- maximize throughput
- keep validation deterministic
- keep consensus rules intact
- avoid unsafe cache reuse
- benchmark only in sandbox state

## What We Already Did

The current optimization pass focused on the hottest paths:

- raised the consensus block cap to 3,000,000 vbytes with an about 12 MB raw cap
- kept SigWit-style vbyte accounting
- added full mempool prevalidation before admission
- cached transaction and block size metadata in the mempool
- used validated mempool entries as the miner fast path
- added exact safe signature verification caching rules
- parallelized Falcon-512 verification for cache misses
- added sighash reuse so signatures are not rebuilt repeatedly
- batched UTXO reads and writes during block validation
- enforced duplicate txid and duplicate input checks
- enforced raw and vbyte block size limits in validation
- improved compact block relay and header-first sync behavior
- added a sandbox wipe command that requires explicit network selection
- hardened regression and race-condition testing around shared test state
- wrote a benchmark status file so sandbox benchmark work can be tracked separately

## How We Get Maximum Safe Speed

The fastest safe design is a pipeline:

1. validate structure early
2. reject bad data before expensive work
3. cache exact safe results
4. batch database reads and writes
5. parallelize CPU-heavy work
6. merge results deterministically
7. recheck consensus-critical state before accept/apply

That means:

- do not verify signatures before size, serialization, and network checks
- do not touch the database one transaction at a time
- do not trust a cache unless the cache key includes every consensus-relevant input
- do not mutate shared state inside parallel workers
- do not skip final consensus checks for speed

## Parallelization Strategy

The best parallel targets are the parts that are CPU-heavy and independent:

- Falcon-512 signature verification
- sighash reconstruction for cache misses
- transaction structural decoding
- UTXO lookups for independent inputs
- block template candidate sorting and scoring
- compact block reconstruction lookups

The safe pattern is:

```rust
let results: Vec<_> = jobs.par_iter().map(verify_one).collect();
merge_results(results);
```

Avoid patterns like:

```rust
jobs.par_iter().for_each(|job| {
    shared_cache.lock().unwrap().insert(job.key, verify(job));
});
```

That second pattern is slower and much easier to race.

## Additional Optimizations To Consider

These are the next safe improvements worth evaluating:

- benchmark harness automation for signatures, blocks, mempool, and network propagation
- autotuning worker counts based on available cores and workload size
- UTXO read ordering by outpoint locality to reduce cache misses
- prefetching hot UTXO rows before parallel signature work starts
- incremental block template maintenance instead of rebuilding from scratch
- per-peer relay scoring with dynamic backpressure
- compact block missing-tx batching instead of single-request churn
- hot-cache metrics for signature cache, sighash cache, and UTXO cache
- zero-copy decoding for transaction and block bytes where possible
- memory pooling for repeated validation allocations
- read-only database snapshots for validation windows
- faster reorg invalidation of mempool and cache entries
- block assembly work queues that keep the miner busy while new mempool transactions arrive

## Best-Practice Benchmarks

If you want the maximum credible throughput number, measure it this way:

- use release builds only for final results
- wipe sandbox state before each benchmark run
- test cold-cache and warm-cache modes separately
- keep the same hardware, OS, compiler, and dataset
- compare before/after on the same block size and transaction mix
- include 1 core, 2 cores, 4 cores, 8 cores, 16 cores if available, and all cores
- record signatures per second, TPS simulated, and propagation time

## Guardrails

Do not trade speed for correctness.

Never:

- change tokenomics in a speed pass
- weaken proof-of-work validation
- skip raw or vbyte size checks
- trust peer-provided validation flags
- trust stale UTXO or signature cache data across reorgs
- benchmark against mainnet data

If a speedup depends on consensus shortcuts, it is not a valid optimization.

## Current Status

The current codebase already has the major safe speed levers in place:

- faster validation staging
- exact signature caching rules
- parallel Falcon verification
- UTXO batching
- compact relay support
- sandbox-safe wipe and test flow

The remaining work is mostly measurement, tuning, and proving the best batch sizes and worker counts for the real hardware and sandbox network shape.

## Lifecycle Semantics We State Explicitly

These rules are now documented so readiness does not stay implied:

- `headers_synced` means the local header chain has caught up to the peer-reported header view. It does not mean the wallet is spendable or that the node is fully ready for mining actions.
- `handshake_ready` means the P2P transport handshake finished. It does not mean the peer is safe to trust for consensus state.
- `wallet_ready` means the Qt wallet has a stable local scan snapshot and the RPC backend has passed the readiness gate.
- Wallet spendability uses the local canonical block count, not a peer-advertised height.
- Orphan and branch buffers are peer-local, in-memory only, and non-consensus state.
- Orphan and branch buffers are dropped on peer disconnect.
- Buffered branches are re-evaluated globally after each accepted block so a parent arriving on one peer can still unblock children buffered from another peer.
- Pending compact-block reconstruction state is transient and is cleared when the block is accepted, rejected, or the peer disconnects.

## Benchmark Harness

The dedicated benchmark harness is now implemented as `atho-benchmark`.
It measures:

- block validation
- mempool admission throughput
- full-block propagation latency
- compact-block propagation latency

It runs against sandbox data only, starts from a wiped temp root by default, and can write `benchmark.md` directly.
