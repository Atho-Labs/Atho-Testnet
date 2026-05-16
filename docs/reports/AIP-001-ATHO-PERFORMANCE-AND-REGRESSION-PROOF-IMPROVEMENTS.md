# AIP-001: Atho Performance and Regression-Proof Improvements

## Status

**Partially Implemented**

This proposal is now backed by code changes in the current `Atho-Testnet-main` checkout, but several improvement areas remain proposed rather than fully implemented.

## Summary

This AIP defines a safe optimization pass for Atho focused on reducing redundant work, centralizing validation helpers, improving rejection ordering, and increasing regression coverage without weakening consensus or changing deterministic protocol behavior.

The implemented part of this pass concentrates on validation hot paths because they are consensus-adjacent, frequently executed, and easy to regress if handled carelessly.

## Motivation

Atho’s audits and adversarial testing surfaced a familiar pattern: the codebase is already doing many important safety checks correctly, but several paths still do more work than they need to.

The motivating issues were:

- redundant header validation work between contextual and non-contextual block validation
- repeated transaction-preparation work in contextual block validation
- repeated witness parsing and signer-group reconstruction
- rejection ordering that allowed oversized blocks to reach contextual PoW checks before cheaper size rejection
- limited benchmark evidence around hot cryptographic paths
- the need for stronger regression coverage around every safe optimization

This AIP is meant to tighten those paths without changing txid, block hash, address format, signature digest format, monetary policy, or consensus rules.

## Goals

- Make Atho faster.
- Make Atho safer.
- Reduce wasted work.
- Reduce duplicate validation.
- Improve sync-adjacent validation efficiency.
- Improve block and tx validation throughput.
- Improve validation ordering.
- Improve caching and reuse only where safe.
- Improve benchmark coverage.
- Improve regression tests.
- Preserve consensus correctness.
- Preserve security.
- Preserve deterministic behavior.

## Non-Goals

- Does not change monetary policy.
- Does not change txid format.
- Does not change block hash format.
- Does not change address format.
- Does not change signature message format.
- Does not remove consensus validation.
- Does not trust peer data.
- Does not rewrite the whole project.
- Does not add unrelated product features.

## Audit Findings Used

This proposal is grounded in the earlier production-readiness and adversarial passes, especially:

- contextual block validation was duplicating header checks and doing more work than necessary
- block-validation paths were reparsing transaction witness/signature structure after already preparing it
- oversized-block rejection ordering needed to fail cheap before expensive
- Falcon malformed-input coverage needed to be stronger
- wallet file permissions needed explicit hardening on Unix
- canonical raw-transaction parsing needed a direct regression
- the project still carries a separate high-severity legacy locking-script ownership issue that is **not** solved by this AIP and remains a production blocker

## Improvement Areas

### Section 1: Consensus Validation Improvements

Implemented:

- shared block header-shape validation helper
- shared block size/merkle/witness/target-bounds validation helper
- shared prepared-transaction collection for block validation
- early cheap rejection of oversized blocks in contextual validation

Proposed:

- further reuse of prepared block-level metadata across sync, miner, and mempool paths
- additional benchmarking for block validation throughput

### Section 2: Transaction Validation Improvements

Implemented:

- internal helpers now reuse prepared transaction validation state during contextual block validation instead of reparsing witness/signature structure per transaction
- exact-fee and minimum-fee contextual checks now share internal prepared-state helpers

Proposed:

- batched UTXO reads for multi-input transactions where storage boundaries allow it safely
- short-term invalid-transaction caching for mempool policy

### Section 3: Falcon Signature Performance and Safety Improvements

Implemented:

- added malformed Falcon public-key/signature regression coverage
- preserved exact verification semantics while reducing duplicate witness preparation in block-validation paths

Proposed:

- explicit signature-verification cache keyed by exact public-key/message/signature/domain bytes
- broader fuzzing for Falcon decode/verify inputs

### Section 4: Hashing and Serialization Improvements

Implemented:

- kept canonical raw-transaction regression coverage for trailing-byte junk
- avoided changing any consensus hashing or serialization formats

Proposed:

- broader serialization microbenchmarks
- more coverage around alternative malformed encodings

### Section 5: UTXO and Database Performance Improvements

Implemented:

- no database layout changes in this AIP
- preserved existing rollback/reload/reorg tests while optimizing validation around them

Proposed:

- batched storage-backed UTXO lookup experiments
- additional throughput benchmarks for UTXO reads/writes

### Section 6: Mempool Performance Improvements

Implemented:

- no direct mempool cache redesign in this pass

Proposed:

- invalid-tx short-term cache
- per-peer invalid admission penalty hooks
- mempool/block-rule consistency regression expansion

### Section 7: Mining and Block Template Improvements

Implemented:

- no mining-template algorithm change in this pass

Proposed:

- more explicit prevalidated mempool metadata reuse during template construction
- template rebuild benchmarks

### Section 8: Networking and P2P Speed Improvements

Implemented:

- no wire-protocol behavior change in this pass

Proposed:

- duplicate message suppression review
- peer-scoring and stale-peer benchmark work
- malformed-message adversarial harness expansion

### Section 9: Sync Speed Improvements

Implemented:

- no sync scheduler redesign in this pass

Proposed:

- explicit sync-from-zero hostile-peer test plan
- scheduling and in-flight request benchmarking

### Section 10: API/RPC Performance and Safety Improvements

Implemented:

- retained transaction-broadcast regression coverage
- kept canonical raw-transaction rejection under test

Proposed:

- endpoint latency profiling
- broader malformed-request and pagination benchmarks

### Section 11: Wallet Performance and Safety Improvements

Implemented:

- explicit owner-only wallet datafile permissions on Unix

Proposed:

- more wallet recovery and UTXO-selection regressions
- wallet scanning and fee-estimation profiling

### Section 12: Startup, Config, and Runtime Improvements

Implemented:

- no startup/config pipeline rewrite in this pass

Proposed:

- startup summary and network-mode validation pass
- shutdown-path profiling

### Section 13: Logging Performance Improvements

Implemented:

- no logging pipeline change in this pass

Proposed:

- repeated-error rate limiting
- hot-loop logging review under attack load

### Section 14: Regression-Proof Test Plan

Implemented in this pass:

- contextual oversized-block rejection regression
- malformed Falcon verification regression
- canonical raw-transaction trailing-byte regression
- wallet datafile permission regression
- attack-harness reruns on regnet and testnet

Full plan is documented in `ATHO_REGRESSION_TEST_PLAN.md`.

### Section 15: Benchmark Plan

Implemented:

- current Falcon hot-path benchmark snapshot recorded in `ATHO_BENCHMARK_RESULTS.md`

Proposed:

- add validation, serialization, mempool, template, API, and sync benchmarks

### Section 16: Implementation Priority

This pass implemented the first priority group and part of the second:

1. Safety-preserving cleanup:
   - shared validation helpers
   - cheap-before-expensive rejection ordering
   - regression additions
2. Database and validation speed:
   - reduced repeated transaction preparation in block validation
   - reduced repeated witness parsing in contextual block validation

Later priorities remain proposed.

## Implemented Changes In This AIP

### 1. Shared block header-shape validation

Introduced in [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:825) and [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:849).

This centralizes:

- empty block check
- block version check
- network-id check
- height check
- timestamp check
- block size/weight/vsize check
- merkle-root check
- witness-root check
- target-bounds check

### 2. Prepared non-coinbase transaction reuse in block validation

Implemented in [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:871) and consumed from [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:945).

This removes duplicated transaction preparation across:

- block structure validation
- contextual block validation
- parallel signature verification

### 3. Prepared-state contextual transaction helpers

Implemented in [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:597) and [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:630).

This allows the contextual block-validation loop to reuse prepared signer/input mapping instead of rebuilding it.

### 4. Contextual oversized-block fail-fast path

Reinforced in [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:2073) and the live contextual path around [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:1034).

### 5. Supporting hardening that this AIP builds on

- wallet file permission hardening in [datafile.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet/datafile.rs:267)
- malformed Falcon verification regression in [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs:407)
- canonical raw-transaction trailing-byte regression in [service.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-node/src/service.rs:4403)
- corrected oversized-block adversarial harness input in [atho-attack.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-node/src/bin/atho-attack.rs:202)

## Safety Requirement

All implemented optimizations preserve the same validation rules:

- no consensus checks were removed
- no txid or block-hash logic changed
- no signature-domain behavior changed
- no monetary policy or fee schedule changed
- final block acceptance still performs full contextual validation
- final Falcon verification still occurs before block acceptance completes

## Expected Benefits

Qualitative improvements from the implemented part:

- one shared header/shape path instead of duplicated contextual/non-contextual checks
- one prepared-transaction pass per non-coinbase block transaction instead of multiple witness/signature-structure rebuilds
- one less witness parse in the contextual block-validation loop
- cheaper rejection of oversized blocks before deeper contextual work
- stronger regression evidence around the changed hot paths

## Remaining Proposed Work

- invalid-tx cache for mempool policy
- batched storage-backed UTXO lookups
- API latency and pagination profiling
- sync pipeline benchmarking
- broader criterion benches across validation and serialization
- P2P malformed-message and spam harness expansion

## Compatibility

This AIP is intended to be **consensus-compatible** with existing Atho nodes.

It does **not** address the separate legacy locking-script ownership weakness identified in the adversarial audit. That remains outside the scope of this performance-focused AIP and still requires a deliberate consensus and deployment decision before production.

## Activation / Rollout

No network activation is required for the implemented subset of this AIP.

These changes are local implementation improvements and regression additions only.

## Reference Files

- `crates/atho-storage/src/validation.rs`
- `crates/atho-node/src/bin/atho-attack.rs`
- `crates/atho-crypto/src/falcon.rs`
- `crates/atho-wallet/src/wallet/datafile.rs`
- `crates/atho-node/src/service.rs`
- `ATHO_OPTIMIZATION_REPORT.md`
- `ATHO_REGRESSION_TEST_PLAN.md`
- `ATHO_BENCHMARK_RESULTS.md`
