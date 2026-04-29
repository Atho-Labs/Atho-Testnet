# Atho Consensus Security Report

## Build Info
- Commit hash: `377afd2815b5d50238750c33412a3c0cc846e104`
- Rust version: `rustc 1.94.1 (e408947bf 2026-03-25)`
- Build profile: `release`
- OS: `Darwin 24.6.0 arm64`
- CPU: `Apple Silicon, 10 cores`
- RAM: `16 GiB`
- Test date: `2026-04-29`

## Consensus Parameters Tested
- Network: `Mainnet`, `Testnet`, `Regnet`
- Block time: `75 seconds`
- Vbyte block cap: `3,000,000`
- Raw block cap: `12,000,000 bytes`
- Monetary policy constants: `50 ATHO` genesis reward, `1,680,000` block halving interval, `168,000,000 ATHO` max supply
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
| Monetary policy | Pass | Max-supply and halving boundary checks passed |
| Signature and witness verification | Pass | Invalid signatures rejected; exact cache rules remained safe |
| Mempool behavior | Pass | Conflict handling, revalidation, and policy checks passed |
| P2P / sync / propagation | Pass | Handshake, relay, sync, and reorg tests passed |
| Storage / restart / recovery | Pass | Snapshot, quarantine, and rollback tests passed |
| Fuzz / stress | Partial | Direct fuzz runs were clean, but dedicated consensus fuzz targets are still missing |

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
- Dedicated consensus fuzz targets for `tx_decode`, `tx_roundtrip`, `sighash`, `block_decode`, `block_validate`, `mempool_admission`, `compact_block_reconstruct`, and `network_message_decode` are still missing.
- `cargo fuzz` is not installed in this environment, so fuzzing was run through direct libFuzzer binaries.
- Sanitizer-backed fuzzing, Miri, and TSAN/ASAN were not available in this run.
- There is still no dedicated end-to-end benchmark harness for full block validation, mempool throughput, or propagation latency.

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
- The fuzz targets currently present in the repo are useful, but they do not yet cover the full consensus attack surface.

## Top 25 Highest-Risk Blockers
1. No dedicated `tx_decode` fuzz target.
2. No dedicated `tx_roundtrip` fuzz target.
3. No dedicated `sighash` fuzz target.
4. No dedicated `block_decode` fuzz target.
5. No dedicated `block_validate` fuzz target.
6. No dedicated `mempool_admission` fuzz target.
7. No dedicated `compact_block_reconstruct` fuzz target.
8. No dedicated `network_message_decode` fuzz target.
9. `cargo fuzz` is missing in the local environment.
10. No sanitizer-backed fuzz run was available.
11. No Miri run was available.
12. No TSAN run was available.
13. No ASAN run was available.
14. No differential validator baseline exists in the repo.
15. No dedicated full block validation benchmark harness exists.
16. No dedicated mempool throughput benchmark harness exists.
17. No dedicated propagation benchmark harness exists.
18. No dedicated header-first sync benchmark harness exists.
19. No cold-cache / warm-cache benchmark harness exists for the full node.
20. No long-duration consensus soak test is wired into CI.
21. No replay corpus for malformed storage records is persisted in CI.
22. No cache-poisoning stress harness for reorgs is wired into CI.
23. No partial-write fault-injection harness covers every storage path.
24. No adversarial P2P corpus regression gate exists.
25. No automated release-mode attack harness gate is enforced in CI.

## Top 25 Missing Hardening Steps
1. Add `tx_decode` fuzzing.
2. Add `tx_roundtrip` fuzzing.
3. Add `sighash` fuzzing.
4. Add `block_decode` fuzzing.
5. Add `block_validate` fuzzing.
6. Add `mempool_admission` fuzzing.
7. Add `compact_block_reconstruct` fuzzing.
8. Add `network_message_decode` fuzzing.
9. Install or vendor `cargo fuzz` for this repo.
10. Add sanitizer-backed fuzz CI.
11. Add Miri coverage for parser and validator hot paths.
12. Add TSAN coverage for cache and reorg logic.
13. Add ASAN coverage for parsers and network decoders.
14. Add a reference-validator differential test harness if one exists.
15. Add a dedicated block-validation benchmark runner.
16. Add a dedicated mempool benchmark runner.
17. Add a dedicated propagation benchmark runner.
18. Add a dedicated header-first sync benchmark runner.
19. Add cold-cache and warm-cache benchmark modes for consensus paths.
20. Add long-run reorg soak tests.
21. Add persistent malformed-state replay corpora.
22. Add cache-poisoning and restart-replay stress tests.
23. Add partial-write fault injection across all storage entry points.
24. Add adversarial P2P corpus persistence and replay.
25. Gate release-mode attack and regression harnesses in CI.

## Final Roadmap
1. Add the missing consensus fuzz targets.
2. Wire fuzzing into CI with sanitizers.
3. Add benchmark harnesses for block validation, mempool, and propagation.
4. Add more reorg and cache-poisoning stress tests.
5. Add a release-mode regression gate for the attack and adversarial harnesses.

## Final Decision
- Safe to merge: Yes
- Needs more testing: Additional fuzz target coverage and benchmark harnesses would still improve confidence
- Blockers: None remaining in the current consensus and validation code paths
