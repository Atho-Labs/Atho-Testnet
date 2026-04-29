# Atho Network and Validation Optimization Benchmark

## Hardware
- CPU: Apple M4-class ARM64
- Core count: 10
- RAM: 16 GB
- Disk: Local SSD
- OS: macOS 24.6.0 (Darwin 24.6.0)
- Rust version: rustc 1.94.1 (e408947bf 2026-03-25)
- Build profile: release Criterion runs measured in sandbox-safe temp roots; `cargo test --workspace --release --all-features` passed
- Commit hash: `33efb08f2afebe73b9864670df2dbaafee5e1641`

## Network Parameters
- Block time: 75 seconds
- Vbyte cap: 3,000,000 vbytes
- Raw cap: about 12 MB
- Average tx size tested: not captured in this workspace
- Signature scheme: Falcon-512
- Transaction model: public UTXO
- Sizing model: SigWit-style vbytes

## Measured Hot Paths
### Core Serialization And Hashing
| Benchmark | Criterion Result | Notes |
|---|---|---|
| `transaction_canonical_bytes` | `[27.137 ns, 27.873 ns, 28.818 ns]` | Canonical transaction byte serialization |
| `transaction_txid` | `[240.99 ns, 253.50 ns, 268.28 ns]` | SHA3-384 txid derivation |
| `block_canonical_bytes` | `[312.20 ns, 334.80 ns, 363.36 ns]` | Canonical block byte serialization |
| `block_hash` | `[547.18 ns, 553.02 ns, 559.73 ns]` | Header hash derivation |

### Falcon-512
| Benchmark | Criterion Result | Notes |
|---|---|---|
| `falcon_generate_from_seed` | `[3.5866 ms, 3.7787 ms, 4.0236 ms]` | Deterministic keygen hot path |
| `falcon_sign_transaction` | `[200.83 µs, 212.53 µs, 226.14 µs]` | Transaction signing hot path |
| `falcon_verify_transaction` | `[22.724 µs, 24.025 µs, 25.518 µs]` | Validation-side signature verification |

### P2P Hot Paths
| Benchmark | Criterion Result | Notes |
|---|---|---|
| `p2p_wire_encode_version` | `[520.22 ns, 535.45 ns, 552.47 ns]` | Framed wire encode |
| `p2p_wire_decode_version` | `[579.60 ns, 600.78 ns, 624.21 ns]` | Framed wire decode |
| `p2p_downloader_assignments_128` | `[27.692 µs, 28.735 µs, 30.097 µs]` | Peer assignment scheduler |

### Wallet Datafile
| Benchmark | Criterion Result | Notes |
|---|---|---|
| `wallet_datafile_save` | `[196.53 ms, 204.23 ms, 213.06 ms]` | Encrypted wallet persistence |
| `wallet_datafile_load` | `[161.59 ms, 167.95 ms, 175.17 ms]` | Wallet reload and decode |

## Chain Wipe Confirmation
- Wiped before pre-benchmark: verified on a disposable `/tmp` sandbox root with `athod wipe --network regnet --data-dir <tmp> --all`
- Wiped before post-benchmark: verified on a disposable `/tmp` sandbox root and mainnet wipe refusal was separately confirmed
- Database paths: sandbox roots only; production paths left untouched
- Cold-cache mode: not rerun in this turn
- Warm-cache mode: not rerun in this turn

## Pre-Optimization Baseline
| Test | Tx Count | Signature Count | Time | Signatures/sec | TPS Simulated | Notes |
|---|---:|---:|---:|---:|---:|---|
| Not captured | - | - | - | - | - | The original before/after benchmark harness is still not present in this workspace |

## Post-Optimization Results
| Test | Tx Count | Signature Count | Time | Signatures/sec | TPS Simulated | Notes |
|---|---:|---:|---:|---:|---:|---|
| Criterion hot paths | - | - | See measured sections above | - | - | Sandbox-safe microbenchmarks were captured for core, Falcon, P2P, and wallet hot paths |

## Improvement Summary
| Area | Before | After | Improvement |
|---|---:|---:|---:|
| Signature verification | Not measured | `[22.724 µs, 24.025 µs, 25.518 µs]` | Criterion microbenchmark captured |
| Block validation | Not measured | Not directly benchmarked | Covered by release/all-features regression tests |
| Mempool admission | Not measured | Not directly benchmarked | Covered by release/all-features regression tests |
| UTXO batching | Not measured | Not directly benchmarked | Covered by release/all-features regression tests |
| Compact relay | Not measured | Not directly benchmarked | Covered by release/all-features regression tests |

## Network Propagation Results
| Test | Block Size | Peers | Propagation Time | Missing Tx Rate | Notes |
|---|---:|---:|---:|---:|---|
| Integration coverage | - | - | - | - | TCP runtime, compact-block recovery, headers-first sync, relay, and 25-node cluster tests passed in release/all-features builds; no dedicated propagation Criterion harness exists yet |

## Race Condition Results
- Result: Pass
- Notes: `cargo test --workspace --release --all-features` passed. Shared test-state locks remain in place, and the Qt funding path regression exposed by the new ownership check was fixed and retested.

## Regression Results
- Result: Pass
- Notes: `cargo test -p atho-core`, `cargo test -p atho-storage`, `cargo test -p atho-node --bin athod`, `cargo test -p atho-node --lib`, `cargo test -p atho-qt --lib`, `cargo test -p atho-qt --bin atho-qt`, `cargo test --workspace`, and `cargo test --workspace --release --all-features` all passed. A new regression test now rejects wrong public keys for standard 32-byte payment outputs.

## Final Decision
- Safe to merge: No
- Needs more testing: Yes
- Blockers: Dedicated node validation, mempool, and full block-propagation benchmark harnesses are still missing, so the benchmark coverage is incomplete even though the hot-path Criterion results and release regressions are green.
