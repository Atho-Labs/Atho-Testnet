# Atho Falcon-512 Benchmarks

## Benchmark Command

- `cargo bench -p atho-crypto --bench falcon_hot_paths -- --sample-size 10`

## Snapshot

- `falcon_generate_from_seed`: `9.4490 ms .. 9.7379 ms`
- `falcon_sign_transaction`: `509.29 µs .. 550.63 µs`
- `falcon_verify_transaction`: `61.602 µs .. 64.348 µs`

## Notes

- This is a single post-hardening snapshot, not a full before/after campaign.
- Criterion reported no statistically significant change versus the previous local baseline it found.
- Additional benchmark gaps remain:
  - malformed-input rejection throughput
  - grouped-input signature verification throughput
  - full block Falcon verification scaling
  - parallel verification scaling under larger transaction bundles
