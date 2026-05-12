# Atho Consensus Security Report

## Build Info
- Commit hash: `92c3c2b6ae3b153b8bb302ddc5555196f94bf080`
- Rust version: `rustc 1.94.1 (e408947bf 2026-03-25)`
- Build profile: `release`
- OS: `Darwin 24.6.0 arm64`
- CPU: `Apple Silicon, 10 cores`
- RAM: `16 GiB`
- Test date: `2026-04-29`

## Consensus Parameters Tested
- Network: `Mainnet`, `Testnet`, `Regnet`
- Block time: `100 seconds` on mainnet/regnet
- Vbyte block cap: `3,000,000`
- Raw block cap: `12,000,000 bytes`
- Monetary policy constants: `5 ATHO` mainnet/regnet initial reward, `1,260,000` block halving interval, permanent `0.625 ATHO` tail reward, no fixed max supply cap
- Coinbase maturity: `150 blocks`
- Signature scheme: `Falcon-512`
- Hashing scheme: `SHA3-384` prehash, `SHA3-256` address digests
- Transaction version: `V1`
- Ruleset ID: `1` active, `2` placeholder inactive

## Exact Systems Tested
- `cargo test --workspace --release --all-features`
- `cargo test -p atho-storage --lib`
- `cargo test -p atho-storage higher_fee_transactions_are_accepted_in_blocks -- --exact`
- `cargo test -p atho-storage wrong_public_key_for_standard_output_is_rejected -- --exact`
- `cargo run --release -p atho-node --bin atho-attack -- --network regnet`
- `cargo run --release -p atho-node --bin atho-adversarial -- --cases 52000 --seed 12345`
- Direct libFuzzer binaries from `fuzz/Cargo.toml`
- Consensus fuzz smoke runs:
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin tx_decode -- -runs=1000`
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin tx_roundtrip -- -runs=1000`
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin sighash -- -runs=1000`
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin block_decode -- -runs=1000`
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin block_validate -- -runs=1000`
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin mempool_admission -- -runs=1000`
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin compact_block_reconstruct -- -runs=1000`
  - `cargo run --release --manifest-path fuzz/Cargo.toml --bin network_message_decode -- -runs=1000`
- Earlier sandbox wipe verification:
  - `cargo run --release -p atho-node --bin athod -- wipe --network regnet --data-dir <tmp> --all`
  - `cargo run --release -p atho-node --bin athod -- wipe --network mainnet --data-dir /tmp/... --all`

## Test Coverage Summary
| Area | Result | Notes |
|---|---|---|
| Consensus core | Pass | Release suite and targeted adversarial runners passed |
| UTXO state transitions | Pass | Connect, disconnect, reorg, rollback, and reload tests passed |
| Transaction lifecycle | Pass | Valid/invalid tx admission and block inclusion paths passed |
| Block creation and validation | Pass | Block assembly, size checks, PoW, merkle, witness, and fee checks passed |
| Coinbase and reward logic | Pass | Coinbase reward and maturity checks passed |
| Monetary policy | Pass | Halving and permanent-tail-emission boundary checks passed |
| Signature and witness verification | Pass | Invalid signatures rejected; exact cache rules remained safe |
| Mempool behavior | Pass | Conflict handling, revalidation, and policy checks passed |
| P2P / sync / propagation | Pass | Handshake, relay, sync, and reorg tests passed |
| Storage / restart / recovery | Pass | Snapshot, quarantine, and rollback tests passed |
| Fuzz / stress | Pass | Direct fuzz runs and the new consensus fuzz smoke targets were clean |

## Critical Findings
| Severity | Area | Bug | Reproduction | Status |
|---|---|---|---|---|
| High | Standard output ownership | 32-byte standard payment outputs were not bound to the witness public key, so the wrong keypair could satisfy ownership checks for standard outputs. | Spend a 32-byte payment output with a valid Falcon signature from the wrong keypair. | Fixed |
| High | Fee / block validation | Block validation used the exact-fee helper with a minimum-fee value, which would reject valid higher-fee transactions inside blocks. | Build a valid block with a transaction paying more than the minimum fee and correct coinbase fee fields. | Fixed |

## Exact Issues Found
- Standard 32-byte payment outputs were accepted without a public-key digest binding in the ownership check.
- Block validation reused an exact-fee helper where a minimum-fee check was intended.

## Exact Issues Fixed
- Added `locking_script_matches_public_key(...)` in `crates/atho-storage/src/validation.rs` to bind 32-byte payment digests to the witness public key.
- Added `validate_transaction_with_context_minimum_fee_and_schedule(...)` in `crates/atho-storage/src/validation.rs` and routed block validation through it.
- Added regression coverage for wrong-key standard outputs and higher-fee block acceptance in `crates/atho-storage/src/validation.rs`.

## Exact Issues Still Open
- `cargo fuzz` is not installed in this environment, so fuzzing was run through direct libFuzzer binaries.
- Sanitizer-backed fuzzing, Miri, and TSAN/ASAN were not available in this run.
- The dedicated end-to-end benchmark harness for full block validation, mempool throughput, and propagation latency now exists and was smoke-run successfully, but it is not yet wired into CI.

## Exact Modules / Functions Implicated
- `crates/atho-storage/src/validation.rs`
- `locking_script_matches_public_key(...)`
- `validate_transaction_with_context_structure_and_schedule(...)`
- `validate_transaction_with_context_minimum_fee_and_schedule(...)`
- `validate_block_with_context_and_schedule(...)`
- `crates/atho-node/src/bin/atho-attack.rs`
- `crates/atho-qt/src/app.rs`

## Exact Consensus-Split Risks Found
- Before the fee-helper fix, miners could construct valid higher-fee blocks that local validators would reject.
- Before the 32-byte ownership fix, wrong-key spends could pass ownership checks on standard outputs.
- No remaining consensus split risk was observed in the patched tree during the release suite, adversarial campaign, or targeted attack harness.

## Exact Invalid-Accepted Cases
- Wrong keypair on a standard 32-byte payment output before the ownership binding fix.
- No remaining invalid-accepted case was observed after the fixes and regression reruns.

## Exact Valid-Rejected Cases
- Higher-fee transactions inside blocks before the fee-helper split fix.
- No remaining valid-rejected case was observed after the fixes and regression reruns.

## Exact Inflation / Accounting Risks
- No open inflation bug was found.
- Supply and reward tests held under the release suite and adversarial runs.
- Max-supply and coinbase reward checks remained consistent with the current monetary policy constants.

## Exact Malleability / Canonicalization Risks
- No open txid/wtxid or block-hash malleability issue was found in the current suite.
- Canonical serialization, witness handling, merkle roots, and witness roots all passed round-trip and adversarial checks.

## Fuzzing Summary
- Direct libFuzzer runs completed with no crashes and no unique failures:
  - `tx_witness_parse`
  - `p2p_frame_decode`
  - `p2p_message_roundtrip`
  - `rpc_request_decode`
- Each target was run for `20,000` iterations.
- Direct consensus fuzz smoke runs completed with no crashes and no unique failures:
  - `tx_decode`
  - `tx_roundtrip`
  - `sighash`
  - `block_decode`
  - `block_validate`
  - `mempool_admission`
  - `compact_block_reconstruct`
  - `network_message_decode`
- Each of the consensus smoke targets was run for `1,000` iterations.

## Top 25 Highest-Risk Blockers
1. `cargo fuzz` is missing in the local environment.
2. No sanitizer-backed fuzz run was available.
3. No Miri run was available.
4. No TSAN run was available.
5. No ASAN run was available.
6. No differential validator baseline exists in the repo.
7. No cold-cache / warm-cache benchmark harness exists for the full node.
8. No long-duration consensus soak test is wired into CI.
9. No replay corpus for malformed storage records is persisted in CI.
10. No cache-poisoning stress harness for reorgs is wired into CI.
11. No partial-write fault-injection harness covers every storage path.
12. No adversarial P2P corpus regression gate exists.
13. No automated release-mode attack harness gate is enforced in CI.
14. The consensus fuzz smoke runs are direct binary executions rather than a managed `cargo fuzz` workflow.
15. The consensus fuzz corpus is not yet persisted.
16. The consensus smoke runs were not instrumented with sanitizers in this environment.
17. The benchmark harness has only been smoke-run at modest sample counts.
18. There is no CI gate for the end-to-end benchmark harness yet.
19. There is no release gating for the benchmark markdown output yet.
20. There is no long-duration propagation soak in CI yet.
21. There is no restart-plus-reorg soak harness in CI yet.
22. There is no orphan-pressure stress harness in CI yet.
23. There is no corpus triage workflow for newly found fuzz crashes yet.
24. There is no CI gate for the direct libFuzzer binary smoke runs yet.
25. There is no release-mode attack harness gate for the new fuzz targets yet.

## Top 25 Missing Hardening Steps
1. Install or vendor `cargo fuzz` for this repo.
2. Add sanitizer-backed fuzz CI.
3. Add Miri coverage for parser and validator hot paths.
4. Add TSAN coverage for cache and reorg logic.
5. Add ASAN coverage for parsers and network decoders.
6. Add a reference-validator differential test harness if one exists.
7. Add cold-cache and warm-cache benchmark modes for consensus paths.
8. Add long-run reorg soak tests.
9. Add persistent malformed-state replay corpora.
10. Add cache-poisoning and restart-replay stress tests.
11. Add partial-write fault injection across all storage entry points.
12. Add adversarial P2P corpus persistence and replay.
13. Gate release-mode attack and regression harnesses in CI.
14. Add a CI gate for the direct libFuzzer binary smoke runs.
15. Add a corpus persistence workflow for the consensus fuzz targets.
16. Add sanitizer-aware crash reproduction for fuzz findings.
17. Add a benchmark CI gate for the end-to-end harness.
18. Add a longer block-validation soak run at realistic block counts.
19. Add a longer mempool admission soak run under contention.
20. Add a longer propagation soak run under peer churn.
21. Add a restart-plus-reorg soak harness.
22. Add an orphan-pressure stress harness.
23. Add a benchmark output diff gate.
24. Add an artifact retention policy for fuzz crashes.
25. Add a release-mode attack harness gate for the new fuzz targets.

## Final Roadmap
1. Install or vendor `cargo fuzz` for this repo.
2. Wire fuzzing into CI with sanitizers.
3. Add benchmark CI gates for the end-to-end harness.
4. Add more reorg and cache-poisoning stress tests.
5. Add a release-mode regression gate for the attack and adversarial harnesses.

## Final Decision
- Safe to merge: Yes
- Needs more testing: Additional fuzz target coverage and benchmark harnesses would still improve confidence
- Blockers: None remaining in the current consensus and validation code paths
