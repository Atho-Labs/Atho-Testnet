# Atho Benchmark Results

## Scope

This file records the benchmark evidence collected during the current optimization pass.

## Environment Notes

- Repository: `Atho-Testnet-main`
- Benchmark command:

```bash
cargo bench -p atho-crypto --bench falcon_hot_paths -- --sample-size 10
```

- Criterion benchmark profile
- Sample size: `10`
- Hardware details were not captured in this pass

## Current Snapshot

### `falcon_generate_from_seed`

- Result: `9.4130 ms .. 9.6687 ms`

### `falcon_sign_transaction`

- Result: `477.08 µs .. 498.34 µs`

### `falcon_verify_transaction`

- Result: `59.478 µs .. 66.154 µs`

## Before / After Status

- **Before baseline:** not captured in this pass
- **After snapshot:** captured
- **Improvement percentage:** not claimed

Because this pass focused first on correctness-preserving validation refactors, I recorded an after snapshot but did not fabricate a before/after percentage.

## Interpretation

The Falcon verify path is already much cheaper than key generation and materially cheaper than signing, which is useful context when prioritizing future optimizations:

- block and mempool hot paths should still minimize unnecessary verification triggers
- reducing repeated witness/signature preparation remains worthwhile even when raw Falcon verification itself is comparatively fast

## Missing Benchmarks

The following are still needed:

- transaction validation throughput
- block validation throughput
- tx serialization / deserialization
- block serialization / deserialization
- txid calculation
- UTXO read throughput
- UTXO write throughput
- mempool admission throughput
- block template build time
- API hot endpoint latency
- sync throughput

## Recommended Next Commands

```bash
cargo bench -p atho-crypto --bench falcon_hot_paths -- --sample-size 30
```

When additional benches exist or are added:

```bash
cargo bench -p atho-storage
cargo bench -p atho-node
```

## Caution

No benchmark should be used to justify skipping validation or loosening deterministic rules. Benchmarks are only valid if the full regression suite remains green.
