# Atho Performance Benchmark Report

## Executive Summary

This report captures the benchmark work completed during the ground-up optimization pass. The theme of the results is straightforward:

- Atho has a healthier validation path than before because repeated work was removed in storage validation.
- Falcon verification remains one of the dominant CPU costs.
- P2P codec overhead is low in isolation.
- The dedicated end-to-end node benchmark harness is not yet reliable enough to be a release-signoff tool.

## Code Changes Reflected In This Report

### Validation hot-path cleanup

File: `crates/atho-storage/src/validation.rs`

- cached `txid` in prepared transaction validation state
- removed witness signer-group cloning in the verification path
- reused parsed canonical UTXO lock digest instead of reparsing
- reused prepared txids during contextual block validation

### P2P codec strictness improvement

File: `crates/atho-p2p/src/codec.rs`

- exact-length frame enforcement
- explicit trailing-bytes rejection

This second change is primarily a security/strictness improvement, but it also prevents wasteful follow-on work on malformed frames.

## Benchmark Commands Run

```bash
cargo bench -p atho-crypto --bench falcon_hot_paths -- --noplot
cargo bench -p atho-p2p --bench network_hot_paths -- --noplot
cargo run -p atho-node --bin atho-benchmark -- --network regnet --tx-count 64 --inputs-per-tx 1 --samples 2
```

The final `atho-benchmark` command did not terminate cleanly in this environment and was manually stopped. That result is documented below as a tooling blocker.

## Results

### Falcon hot paths

| Benchmark | Current result | Notes |
| --- | --- | --- |
| `falcon_generate_from_seed` | `15.572 ms .. 16.890 ms` | Seed-based keygen remains relatively expensive |
| `falcon_sign_transaction` | `850.53 us .. 944.74 us` | Signing is still a meaningful local-wallet cost |
| `falcon_verify_transaction` | `130.23 us .. 155.04 us` | Verification remains a key validation hot path |

### P2P network hot paths

| Benchmark | Current result | Notes |
| --- | --- | --- |
| `p2p_wire_encode_version` | `1.9089 us .. 1.9731 us` | Wire encode is lightweight |
| `p2p_wire_decode_version` | `1.8317 us .. 1.8629 us` | Decode is lightweight even with strict framing |
| `p2p_downloader_assignments_128` | `55.156 us .. 56.752 us` | Scheduler assignment cost is modest in isolation |

## Before / After Story

For this pass, the strongest measurable story is **qualitative plus micro-benchmark evidence**, not a single large "X percent faster" headline:

- transaction/block validation now does less repeated txid and witness-preparation work,
- P2P decoding now rejects malformed extra bytes immediately,
- and the micro-benches show that network codec cost is not currently the dominant problem.

What we do **not** have yet is a reliable end-to-end benchmark that can turn those internal improvements into a clean "block validation improved by N%" claim. The current `atho-benchmark` harness needs repair before that claim would be honest.

## Bottlenecks Still Remaining

### 1. Falcon verification cost

Falcon verification is still expensive enough to dominate multi-input transaction and block validation workloads.

### 2. Mempool scaling policy

The mempool currently lacks a well-documented hard memory/expiry/eviction model, which matters as much for performance as for DoS resistance.

### 3. End-to-end benchmark reliability

The node benchmark harness hung during a modest regnet run in this environment. That prevents repeatable throughput measurement across:

- tx validation
- mempool admission
- block validation
- sync
- miner template generation

### 4. Mainnet network readiness

Mainnet seed/peer configuration is empty, which means throughput and propagation performance cannot yet be signed off as a real deployment story.

## Security-Correctness Tradeoff Review

No performance change in this pass weakened consensus or widened acceptance:

- transaction and block validation remain strict,
- final Falcon verification remains intact,
- canonical payment lock enforcement remains intact,
- P2P decoding became stricter, not looser.

Where performance slowed down in benchmark comparisons, that slowdown is acceptable because the code path remains correctness-first and the pass intentionally prioritized removing ambiguity and duplicated work before attempting higher-risk parallelization.

## Recommended Next Benchmark Work

1. Repair `atho-benchmark` so it terminates cleanly and emits stable results.
2. Add dedicated criterion benches for:
   - tx canonical decode
   - tx validation by input count
   - block validation by tx count
   - UTXO batch reads/writes
   - mempool admission and template selection
3. Add scale benches for:
   - 1,000 / 10,000 mempool entries
   - large explorer address history queries
   - sync under mixed-quality peers
4. Add regression thresholds to catch clear benchmark cliffs in CI or release gating.

## Overall Performance Readiness

### Safe claim

**Atho is fast enough for continued extended testnet work.**

### Unsafe claim

It would not be honest to call the current benchmark story mainnet-grade. Too much of the end-to-end throughput story is still inferred instead of directly proven.

## Final Take

This pass made Atho's hot paths cleaner and safer. The biggest performance gains came from removing wasted work in validation, not from risky architectural rewrites. The next step is not blind optimization; it is better measurement, a bounded mempool policy, and more stable end-to-end performance tooling.
