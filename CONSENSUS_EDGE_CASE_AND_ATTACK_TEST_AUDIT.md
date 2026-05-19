# Atho Full Edge-Case and Adversarial Consensus Test Audit

## 1. Executive Summary

### Verdict

- **Overall edge-case coverage grade:** **7/10**
- **Overall adversarial testing grade:** **6/10**
- **Overall mainnet safety grade:** **6/10**
- **Decision:** **MAINNET DELAY RECOMMENDED**

### Why

The good news first: after the recent consensus hardening pass, I did **not** reproduce an active inflation bug, an obvious double-spend acceptance path, an invalid-block acceptance path, or a valid-block rejection bug in the current validator/miner/sync/storage path that I exercised locally.

The blunt part: Atho is now in the uncomfortable middle ground where the **core rules look materially stronger**, but the **proof machinery around them is still incomplete**. The biggest problems in this pass were not “the chain is obviously broken,” but rather:

1. the **fuzz gate is currently broken** and does not compile,
2. the built-in **adversarial runner has drifted from current consensus rules**,
3. there is **no property-based invariant suite**,
4. crash-fault injection is **too narrow** to prove full atomicity across all critical windows.

That is enough for me to say **do not call mainnet ready yet**, even though the consensus code itself looks much healthier than before.

### Evidence executed in this pass

Commands I ran:

- `cargo test -p atho-node --lib -- --test-threads=1` -> **198 passed**
- `cargo check --workspace` -> **passed**
- `python3 -m unittest tests.test_runtime_launcher` -> **15 passed**
- `cargo check --manifest-path fuzz/Cargo.toml --all-targets` -> **failed**

Important failure from the fuzz build gate:

- `fuzz/fuzz_targets/common.rs:264` constructs `BlockHeader` without `founders_hash_sha3_384` and `founders_hash_sha3_512`, so the documented fuzz build check in `docs/testing.md:37-43` is currently false.

### Top 25 Missing Edge-Case Tests

1. Exact PoW boundary test where `hash == target`
2. Timestamp exactly at maximum future drift
3. Timestamp one second beyond maximum future drift in full node sync flow
4. `u64::MAX` timestamp arithmetic safety
5. `u64::MAX` height arithmetic in chain selection helpers
6. Reorg replay property: replay-from-genesis equals post-reorg live state for random valid branches
7. Property test that failed block validation leaves UTXO set bit-identical
8. Property test that miner-produced candidate block always passes normal validator
9. LMDB value decoder fuzz target
10. Chainstate loader fuzz target from corrupted persisted bytes
11. Explorer snapshot loader fuzz target
12. Falcon public key parser fuzz target
13. Falcon signature parser fuzz target
14. Compact-block reconstruction differential test for tx variants with same `txid` but different malformed witness bytes
15. Crash injection after block DB write but before tx index update
16. Crash injection after tx index write but before UTXO updates
17. Crash injection after UTXO deletions but before UTXO insertions
18. Crash injection after UTXO insertions but before tip metadata update
19. Crash injection during reorg disconnect phase
20. Crash injection during reorg reconnect phase
21. Cross-network transaction replay tests at the API/raw-hex boundary for all networks
22. Exact fee-floor minus-one / equal / plus-one tests at block acceptance boundary
23. Multi-input same-signer-group / multi-signer-group randomized witness property tests
24. Snapshot bootstrap negative tests for wrong hash + wrong network + stale tip in one matrix
25. Full adversarial campaign CI lane with deterministic short-case budget

### Top 25 Highest-Risk Failure Modes

1. Broken fuzz gate silently reduces parser coverage
2. Stale adversarial harness gives false confidence about hostile scenarios
3. Missing property tests leave invariant regressions hard to catch
4. Narrow crash fault injection can miss silent atomicity bugs
5. Hidden production mutation helper (`Node::dev_seed_chainstate`) could be reused unsafely in future tooling
6. Startup/load paths still include panic-oriented convenience entrypoints
7. Snapshot bootstrap is hash-pinned, not signed-distribution verified
8. Local clock skew can still temporarily reject valid future-near-tip blocks
9. Compact-block logic depends on mempool transaction identity remaining aligned with witness commitment rules
10. Differential drift between API-reported balances and live UTXO scan is only partially covered
11. No proof that all malformed LMDB values fail closed under fuzz mutation
12. No proof that all malformed RPC payloads remain non-panicking under extended fuzz runs
13. No proof that all malformed P2P payloads remain non-panicking under extended fuzz runs
14. No randomized long-branch replay property across repeated reorgs
15. No long-duration soak of checkpoint-anchored sync under churn
16. No exact equality proof for PoW `<=` semantics
17. No randomized stress proving mempool/miner/validator agreement across large tx sets
18. No automated “reindex equals live state” invariant on every CI cycle
19. No automated “trusted snapshot load then full sync then reindex” equivalence test
20. Adversarial runner still uses non-canonical 4-byte locks in its base fixtures
21. Some operationally dangerous helper paths are only hidden by convention, not compile gating
22. Broken fuzz build means new header/serialization changes can outpace hostile-input coverage
23. No property test for coinbase reward equality across arbitrary fee combinations
24. No exhaustive malformed UTF-8 / invalid Base56 API regression matrix
25. No committed baseline for long-running fuzz or adversarial campaign durations

### Top 25 Areas Attackers Would Target First

1. Full transaction decoder
2. Witness decoder
3. Block decoder
4. Compact-block reconstruction
5. P2P frame decoder
6. Network message decoder
7. RPC request decoder
8. Snapshot bootstrap loader
9. LMDB persisted value decoding
10. Reorg rollback path
11. Coinbase reward calculation boundary
12. Fee accounting overflow/underflow
13. Timestamp/future drift logic
14. Wrong-network replay surfaces
15. Falcon signature parsing and grouping
16. Witness input-reference binding
17. Mempool/miner candidate selection under concurrent churn
18. Admin/debug/developer helper surfaces
19. Crash recovery / unclean shutdown restart
20. Explorer/index snapshots as derived state
21. Pruning + restart + reorg combinations
22. Startup snapshot bootstrap with operator-supplied hash
23. Differential block template vs validator logic
24. Hidden fast paths / trusted modes / skip flags
25. Test harness drift that masks real regressions

### Top 25 Fixes Required Before Mainnet

1. Repair the fuzz crate so `cargo check --manifest-path fuzz/Cargo.toml --all-targets` passes
2. Add fuzz targets for LMDB value decode and chainstate loader
3. Add fuzz targets for Falcon public key and signature byte parsing
4. Update `atho-adversarial` base fixtures to use canonical 32-byte payment locks
5. Replace stale adversarial “valid” fixtures that are now consensus-invalid
6. Add a deterministic short adversarial campaign CI target
7. Add `proptest` or `quickcheck` invariant suites for UTXO and replay correctness
8. Expand commit fault injection beyond `BeforeCommit`
9. Add crash-fault simulation around reorg disconnect/reconnect windows
10. Add exact PoW equality boundary tests
11. Add exact future-drift boundary tests in sync/runtime paths
12. Add replay-from-genesis equivalence tests after reorg
13. Add differential tests comparing mined candidate blocks to normal validator across randomized mempools
14. Add malformed LMDB restart regression fixtures
15. Add snapshot bootstrap corruption regression fixtures
16. Make hidden mutation helpers test-only or feature-gated
17. Prefer fallible startup paths in default local tooling where possible
18. Wire fuzz compile check into required CI
19. Add a nightly fuzz execution job, not just compile-check
20. Add a nightly adversarial runner job with bounded case count
21. Add a nightly replay/reindex equivalence job
22. Add clock-skew simulation tests
23. Add exact fee-floor boundary tests at block-context validation level
24. Add API regression tests for invalid UTF-8 / Base56 / trailing garbage
25. Refresh `docs/testing.md` only after the documented commands genuinely pass

### Top 25 Regression Tests Required Before Mainnet

1. Fuzz crate build regression
2. Canonical adversarial fixture regression
3. PoW equality-at-target regression
4. Future-drift exact-boundary regression
5. Future-drift plus-one-second regression
6. Coinbase reward with computed fees only regression
7. Block metadata fee fields ignored by consensus regression
8. tx-PoW changes witness commitment regression
9. Compact-block short ID differentiates tx-PoW variants regression
10. Empty-witness coinbase must have zero tx-PoW regression
11. Malformed witness bytes alter commitment regression
12. Failed block leaves UTXO snapshot unchanged regression
13. Failed reorg leaves canonical chain unchanged regression
14. Crash-before-commit leaves snapshot unchanged regression
15. Crash-after-block-record-before-state-write regression
16. Snapshot hash mismatch fails closed regression
17. Wrong-network mining reward address rejected regression
18. Missing mainnet/testnet reward address rejected regression
19. Noncanonical raw transaction bytes rejected regression
20. Trailing-byte raw transaction bytes rejected regression
21. Wrong-network replay raw transaction rejected regression
22. Reindex result equals live state regression
23. Explorer snapshot rebuild equals live-derived state regression
24. Hidden dev seed helper absent from release feature set regression
25. Fuzz/common BlockHeader constructor stays in sync with header schema regression

## 2. Unknown Unknowns Review

### Summary

This pass intentionally hunted for “things that are probably fine until they suddenly are not.”

The most important hidden-assumption findings were:

| Finding | Why it is dangerous | Reachable? | Exploit / trigger | Fix | Proof test |
|---|---|---|---|---|---|
| Broken fuzz gate in `fuzz/fuzz_targets/common.rs:264` | Parser coverage exists only on paper if the fuzz crate does not compile | Yes | Header/schema changes land; fuzz jobs silently stop being meaningful | Update fuzz fixtures for current `BlockHeader` schema | CI build check on fuzz crate |
| `atho-adversarial` still models “valid” 4-byte locks at `crates/atho-node/src/bin/atho-adversarial.rs:53-55`, `:339-358` | Hostile campaign can misclassify invalid modern transactions/coinbases as valid fixtures | Yes | Engineers trust stale campaign output | Rebuild adversarial base fixtures with canonical 32-byte locks | Runner regression with one known-good canonical fixture |
| Many adversarial block cases use `validate_block_without_pow` (`crates/atho-node/src/bin/atho-adversarial.rs:1037+`) | Coverage misses full PoW acceptance path and can overstate block-level adversarial confidence | Yes | False sense of hostile block coverage | Split no-PoW structure tests from full validator tests and run both | CI adversarial matrix |
| No repo-wide `proptest` / `quickcheck` usage found | Invariant drift can sneak past example-based tests | Yes | Replay / UTXO / fee accounting regress subtly over time | Add property suites for UTXO, replay, reward, encoding | Property test CI lane |
| Crash fault injection only has `CommitFaultPoint::BeforeCommit` at `crates/atho-storage/src/db.rs:235-236` | Atomicity is only proven for one failure window | Test-only but coverage-critical | Bug appears after a different partial-write phase | Add fault points for delete/add/index/tip-update stages | Per-fault crash regression tests |
| `Node::dev_seed_chainstate` is public in non-test builds at `crates/atho-node/src/node.rs:462-474` | Hidden mutation helpers can leak into operational tooling later | Not remotely reachable today | Future tool / debug RPC accidentally exposes it | Gate behind `#[cfg(test)]` or explicit dev feature | Release build symbol/API regression |
| Convenience startup paths panic on load failure (`crates/atho-storage/src/chainstate.rs:179-182`, `crates/atho-node/src/node.rs:490-492`) | Bad snapshot / corrupt local state becomes hard crash instead of surfaced error | Yes | Misconfigured snapshot or unrecoverable disk state | Prefer fallible startup in user-facing/default service constructors | Startup failure integration test |
| Testing docs claim fuzz build should pass (`docs/testing.md:37-43`) when it does not | Team can believe a gate exists when it is already broken | Yes | False release confidence | Keep docs and gates aligned; add CI required check | Docs command test in CI |

## 3. Boundary Value Testing

### Coverage status

Current boundary coverage is **good in core integer accounting and basic transaction structure**, **moderate in reorg/depth/checkpoint logic**, and **weak in extreme numeric / randomized boundary generation**.

### Amount boundaries

**Covered now**

- zero-value output rejection: `crates/atho-storage/src/validation.rs:373-375`
- dust rejection and exact dust-floor coverage: `crates/atho-storage/src/validation.rs:386-395`, tests around `:1736`, `:1868`
- fee floor calculations and examples: `crates/atho-core/src/consensus/tx_policy.rs:118`, tests around `:514-735`
- overflow-safe output sum: `crates/atho-core/src/transaction.rs` tests around `checked_output_value_atoms_rejects_overflow`
- indefinite tail-emission policy explicitly tested: `crates/atho-core/src/consensus/subsidy.rs:115-137`

**Missing / should be added**

- exact `fee floor - 1 / == / + 1` tests in contextual block acceptance, not just tx policy helpers
- `u64::MAX` input/output accumulation randomized tests
- “sum outputs greater than inputs by 1 atom” property tests across many shapes
- malformed numeric strings at API/raw-RPC boundaries
- no finite max-supply tests are intentionally replaced by “no cap remains” tests; that is correct for Atho’s current policy, but the report and CI should say so explicitly

### Height boundaries

**Covered now**

- maturity and confirmation logic: `crates/atho-node/src/bin/atho-adversarial.rs:1334+`
- halving/tail-emission schedule tests: `crates/atho-core/src/consensus/subsidy.rs`
- max reorg depth exact boundary / +1 rejection: `crates/atho-storage/src/chainstate.rs:2527-2577`
- finalized checkpoint boundary tests: `crates/atho-storage/src/chainstate.rs:2587+`

**Missing / should be added**

- randomized very-high-height reward and difficulty tests
- `u64::MAX` height arithmetic guards in replay helpers
- reorg across exact maturity boundary with mixed coinbase and non-coinbase spends

### Time boundaries

**Covered now**

- timestamp zero rejected
- median-time-past floor enforced
- future-drift ceiling enforced: `crates/atho-storage/src/validation.rs:1234-1238`
- exact future-drift rejection test added in `crates/atho-storage/src/validation.rs:2485`

**Missing / should be added**

- exact equality test at `maximum_timestamp`
- one-second-under-boundary test in sync/block acceptance
- clock-skew / restart / NTP regression tests
- timestamp arithmetic overflow tests

### Size boundaries

**Covered now**

- tx raw size / vsize checks
- block raw size / vsize / weight checks
- excessive tx-count allocation guard in block decode: `crates/atho-core/src/block.rs:419-423`
- witness ref count / signature len / pubkey len parsing guards: `crates/atho-core/src/transaction.rs:262-329`
- compact-block tx-count guard: `crates/atho-p2p/src/protocol.rs:849+`

**Missing / should be added**

- max-inputs exact boundary tests with randomized witness grouping
- one-byte-under / exact / one-byte-over block size cases in one parameterized matrix
- max-output-count exact boundary / +1 randomized tests across rule activations

## 4. Serialization Edge-Case Testing

### What looks strong

- `Transaction::from_full_bytes()` is now strict and rejects truncated tx-PoW tails: `crates/atho-core/src/transaction.rs:720-814`
- trailing junk is rejected and tested in protocol fixtures and node service raw-tx tests
- block canonical decode rejects trailing garbage and oversized tx-count preallocation risk
- witness commitment now binds tx-PoW and malformed witness bytes

### What is still missing

1. No property suite for `decode(encode(x)) == x` across randomized transaction and block strategies
2. No explicit regression for endian confusion at the API/raw hex boundary
3. No dedicated malformed UTF-8 / overlong UTF-8 API request matrix
4. No differential test between wallet-produced tx bytes and node-produced tx bytes across many randomized spends
5. Fuzz build currently broken, which directly weakens serialization confidence

### Current judgment

- **Serialization safety grade:** **7/10**
- Core codec behavior looks disciplined now.
- The proof story is dragged down by the broken fuzz gate and absent property tests.

## 5. Transaction Edge-Case Testing

### What is covered well

- no inputs / no outputs
- duplicate input rejection
- dust rejection
- zero-value output rejection
- fee floor checks
- tx size checks
- witness grouping shape checks
- wrong signer pubkey rejection
- wrong network ownership rejection
- missing UTXO rejection
- immature spend rejection
- tx-PoW exact-bits validation and network binding

Primary enforcement lives in `crates/atho-storage/src/validation.rs:360-720`.

### What is still weak or missing

- randomized multi-signer grouped witness strategies
- block-level same-input-across-two-transactions fuzz/property testing
- malformed unlocking-script payload matrices at the API/raw-tx entrypoint
- explicit “same tx semantic content, different noncanonical encodings” rejection property
- malformed witness bytes under long fuzz runs

### Current judgment

- **Transaction edge-case grade:** **8/10**

This is one of the strongest areas right now.

## 6. Coinbase Edge-Case Testing

### What improved

Coinbase shape is now much tighter:

- canonical payment lock required
- exactly one output required
- empty witness required
- `tx_pow_nonce == 0`
- `tx_pow_bits == 0`

See `crates/atho-storage/src/validation.rs:816-834`.

### What is covered

- overclaim rejection
- duplicate coinbase rejection
- coinbase-not-first rejection
- wrong reward amount rejection
- legacy lock rejection
- maturity checks

### What is still missing

- explicit underclaim test (accepted or rejected intentionally; document policy)
- reorg across coinbase maturity exact boundary with restored mempool transactions
- randomized fee combinations proving coinbase reward equals subsidy plus computed fees only

### Current judgment

- **Coinbase edge-case grade:** **8/10**

## 7. Block Edge-Case Testing

### What is covered

- empty block rejection
- no coinbase rejection
- wrong network rejection
- wrong height rejection
- wrong parent rejection
- wrong merkle root rejection
- wrong witness root rejection
- duplicate txid rejection
- duplicate input across block rejection
- invalid tx inside block rejection
- future timestamp rejection
- invalid PoW rejection
- oversized block rejection

### What is still missing

- exact `hash == target` acceptance
- exact max-size / max-weight boundary matrices
- malformed-body-after-valid-header randomized corpus
- crash-in-middle-of-contextual-validation failpoint proof

### Current judgment

- **Block edge-case grade:** **8/10**

## 8. UTXO and State Mutation Edge Cases

### What is strong

- contextual validation uses an overlay before final mutation
- failed blocks do not mutate the live UTXO set in the tested paths
- chainstate reorg rollback has journaled restoration tests
- commit fault injection proves rollback on `BeforeCommit`

Notable tests:

- `crates/atho-storage/src/chainstate.rs:3134+`
- `crates/atho-storage/src/chainstate.rs:3547+`

### What is still missing

- failpoints after UTXO deletion but before UTXO insertion
- failpoints after UTXO insertion but before metadata tip update
- failpoints around tx index writes and peer/address index updates
- proof that restart after each partial-write window equals clean replay-from-genesis

### Current judgment

- **UTXO atomicity grade:** **7/10**

The design is much better now, but the injected-fault matrix is still too narrow to claim “production-proven atomicity.”

## 9. Reorg and Fork Edge Cases

### What is covered

- higher-work branch preferred over raw height
- deep reorg boundary enforcement
- finalized checkpoint conflict rejection
- rollback after candidate validation failure
- rollback after commit failure
- buffered side-branch and cross-peer reconstruction

This is now one of Atho’s stronger areas.

### What is still missing

- property-based replay equivalence after arbitrary valid branch switches
- crash during disconnect and reconnect phases
- mixed maturity-boundary and halving-boundary reorg matrices

### Current judgment

- **Reorg safety grade:** **8/10**

## 10. Mempool Edge-Case Testing

### What is covered

- invalid consensus txs rejected
- wrong-network and invalid-signature paths rejected
- dust and fee-floor mismatches rejected
- mempool does not mutate chainstate
- mining view revalidates entries instead of trusting cached admission blindly
- invalid/stale unchecked entries are skipped at mining time

### What is still missing

- longer parent/child randomized chains
- same semantic tx under alternate malformed encodings
- very-large mempool differential tests between mempool ordering and miner selection
- more explicit restore-after-reorg validity matrix

### Current judgment

- **Mempool/miner alignment grade:** **8/10**

## 11. Miner Edge-Case Testing

### What is strong

- candidate blocks now require explicit payout address on mainnet/testnet
- candidate blocks compute fees and coinbase reward locally
- final mined block is still connected through normal validator logic

### What is still missing

- large randomized mempool differential tests proving candidate block always passes validator
- more direct tests for mempool mutation during block assembly
- long-running CPU/GPU nonce result differential proof

### Current judgment

- **Miner edge-case grade:** **7/10**

## 12. PoW and Difficulty Edge Cases

### What is covered

- target bounds
- retarget clamping
- network-specific stall reset behavior
- minimum next-block timestamp rules
- branch work comparison

### What is still missing

- exact `hash == target` acceptance regression
- more explicit endianness abuse tests
- timestamp manipulation matrix around retarget boundary conditions

### Current judgment

- **PoW/difficulty grade:** **7/10**

## 13. Network Separation Edge Cases

### What is strong

- network-scoped address decoding
- network-scoped tx signing digests
- wrong-network block rejection
- wrong-network UTXO ownership rejection
- separate storage roots per network
- finalized checkpoint logic is network-local

### What is still missing

- fuller mixed raw-hex replay matrix across all three networks at external endpoints
- explicit regression that dev-only mining reward defaults never appear on mainnet/testnet

### Current judgment

- **Network separation grade:** **8/10**

## 14. Storage and Crash Edge-Case Testing

### What is strong

- startup self-check clears stale commit journal
- recoverable local-state errors trigger quarantine/rebuild path
- schema mismatch fails closed
- raw block-file corruption is indexed safely through metadata

### What is still missing

- multi-point crash injection coverage
- partial-write equivalence proof after every critical mutation window
- LMDB decode fuzzing

### Current judgment

- **Storage crash safety grade:** **6/10**

This is the biggest remaining proof gap after fuzzing/property coverage.

## 15. API/RPC Edge-Case Testing

### What I found

External transaction and block inputs still route through normal validation paths. I did **not** find an API or RPC path that directly marks UTXOs spent or bypasses consensus checks.

Notable evidence:

- raw tx broadcast routes in `crates/atho-node/src/service.rs:375` and `:3001+`
- block submission route in `crates/atho-node/src/service.rs:420`
- API tests for malformed/hidden/oversized inputs passed in the full `atho-node --lib` run

### Remaining concerns

- admin/debug operational surfaces still deserve separate deployment hardening review
- malformed external payload fuzzing exists conceptually, but the fuzz gate is broken

### Current judgment

- **API/RPC consensus safety grade:** **7/10**

## 16. Legacy and Dead Code Edge Cases

### Keyword review table

| Keyword | File | Function | Risk | Reachable? | Required Action |
|---|---|---|---|---|---|
| `legacy` | `crates/atho-storage/src/validation.rs:466-470` | `canonical_payment_lock` | Low | Yes | Keep fail-closed; retain tests |
| `skip_pow` | `crates/atho-storage/src/validation.rs:940-960`, `:1016-1025` | `validate_block_without_pow` | Medium | Test/dev only in practice | Keep out of production entrypoints; document clearly |
| `trusted` | `crates/atho-node/src/node.rs:1497+` and snapshot bootstrap paths | snapshot bootstrap | Medium | Yes | Prefer signed snapshot metadata or stronger operator warnings |
| `unchecked` | `crates/atho-node/src/mempool.rs:656` | `insert_unchecked` | Low | `pub(crate)` only | Keep internal; do not expose remotely |
| `dev` | `crates/atho-node/src/node.rs:462-474` | `dev_seed_chainstate` | Medium | Yes | Gate behind test/dev feature |
| `fast` | `crates/atho-node/src/sync.rs:432-477`, `:2963+` | fast body download | Medium | Yes | Keep validation lag tests; add soak |
| `cache` | `crates/atho-node/src/service.rs` / explorer caches | explorer/mempool caches | Low | Yes | Continue differential cache-vs-live tests |
| `panic!` | `crates/atho-storage/src/chainstate.rs:179-182`, `crates/atho-node/src/node.rs:490-492` | `load_or_new` convenience paths | Medium | Yes | Prefer fallible startup in user-facing constructors |
| `fallback` | GPU/cookie/sync paths | multiple | Low | Yes | Keep explicit tests; mostly fail-safe so far |
| `default` | config/helpers | multiple | Low | Yes | Audit defaults when consensus schema changes |

### Bottom line

I did **not** find a live legacy consensus bypass comparable to the old non-32-byte lock risk from earlier audits. That specific issue appears closed now. The remaining “legacy/dead-code” concern is more about **stale test harnesses and helper surfaces** than about the main validator accepting old rules.

## 17. Fuzz Testing Plan

### Existing fuzz targets

Current fuzz crate targets include:

- `p2p_frame_decode`
- `p2p_message_roundtrip`
- `tx_witness_parse`
- `tx_decode`
- `tx_roundtrip`
- `sighash`
- `block_decode`
- `block_template_decode`
- `block_validate`
- `mempool_admission`
- `compact_block_reconstruct`
- `network_message_decode`
- `rpc_request_decode`
- `address_decode`

Source: `fuzz/Cargo.toml`.

### Gaps to add

1. Falcon public key decode
2. Falcon signature decode
3. LMDB UTXO value decode
4. LMDB chainstate snapshot decode
5. explorer snapshot decode
6. config/rpcauth parse
7. cookie-auth file parse
8. block-record file-location decode
9. reorg undo journal decode
10. API path parameter decode with invalid UTF-8 / Base56

### Current status

- **Fuzz coverage grade:** **4/10**

The target list is promising.
The actual gate is not.

## 18. Property-Based Testing Plan

### Current status

Repo-wide search found **no `proptest` or `quickcheck` usage** under `crates/` or `fuzz/`.

### Minimum property suite to add

1. UTXO transition equivalence for valid blocks
2. Failed block leaves UTXO set unchanged
3. Replay-from-genesis equals live state after arbitrary valid chain prefixes
4. Reorg replay equals active state after branch switch
5. Coinbase reward equals subsidy plus fees
6. `decode(encode(tx)) == tx`
7. `decode(encode(block)) == block`
8. `txid` stable across roundtrip
9. block hash stable across roundtrip
10. merkle root changes when tx order changes
11. witness root changes when committed witness/tx-PoW content changes
12. miner candidate block always passes normal validator

### Current status

- **Property-test coverage grade:** **2/10**

## 19. Differential Testing Plan

### Current differential coverage I observed

- compact-block reconstruction vs full block path
- explorer incremental rebuild vs full rebuild
- chainstate reorg rollback vs preserved snapshot
- service `gettxoutsetinfo` vs listed UTXO stats
- branch work comparison vs reference logic in PoW tests

### High-value differential tests still missing

1. miner fee calculation vs validator fee calculation on randomized mempools
2. wallet-built raw tx bytes vs node parse/re-serialize across random spends
3. CPU miner vs validator PoW equality corpus
4. chain replay from blocks vs persisted LMDB snapshot across randomized histories
5. API-reported balance vs direct UTXO scan over randomized address histories

## 20. Regression Test Requirements

| Bug / Risk | Test Name | File | Fails Before Fix? | Passes After Fix? | CI Required? |
|---|---|---|---|---|---|
| Broken fuzz header fixture | `fuzz_block_header_schema_compiles` | `fuzz/fuzz_targets/common.rs` | Yes | Not yet | Yes |
| Stale adversarial 4-byte lock fixtures | `adversarial_valid_fixture_uses_canonical_locks` | `crates/atho-node/src/bin/atho-adversarial.rs` | Yes | Not yet | Yes |
| Missing PoW equality boundary proof | `pow_accepts_hash_equal_to_target` | `crates/atho-core/src/consensus/pow.rs` | N/A | Not yet | Yes |
| Missing drift exact-boundary proof | `future_timestamp_at_exact_drift_limit_is_accepted` | `crates/atho-storage/src/validation.rs` | N/A | Not yet | Yes |
| Missing crash-window coverage | `commit_fault_after_utxo_delete_restores_state` | `crates/atho-storage/src/db.rs` / `chainstate.rs` | N/A | Not yet | Yes |
| Hidden dev helper in release builds | `release_build_omits_dev_seed_chainstate` | `crates/atho-node/src/node.rs` | N/A | Not yet | Yes |
| Panic-oriented startup convenience path | `default_service_startup_surfaces_recoverable_load_errors` | `crates/atho-node/src/service.rs` / `node.rs` | N/A | Not yet | Yes |
| Missing replay equivalence proof | `reorg_replay_equals_fresh_replay_property` | `crates/atho-storage/src/chainstate.rs` | N/A | Not yet | Yes |

## 21. CI / Mainnet Gate Requirements

Mainnet should not launch until all of these are true:

- `cargo check --workspace` passes
- `cargo test -p atho-node --lib -- --test-threads=1` passes
- core/storage/p2p consensus suites pass
- launcher tests pass
- fuzz crate compiles
- nightly fuzz execution runs for a minimum budget
- deterministic short adversarial campaign passes
- property tests pass
- replay/reindex equivalence job passes
- crash-fault injection matrix passes
- miner-produced blocks pass validator under randomized mempool scenarios
- no test harness is stale relative to current consensus schema

## 22. Severity Rating

| Severity | File | Function / Area | Description | Attack / Failure Scenario | Consensus Impact | Fix | Test Required | Mainnet Blocked? |
|---|---|---|---|---|---|---|---|---|
| High | `fuzz/fuzz_targets/common.rs:264` | fuzz fixture construction | Fuzz gate does not compile after header schema expansion | Parser regressions slip past because fuzz CI is dead | Indirect but serious | Update fixture to current `BlockHeader` | fuzz build check | No, but delay |
| High | `crates/atho-node/src/bin/atho-adversarial.rs:53-55`, `:339-358` | adversarial harness fixtures | Built-in hostile runner uses non-canonical 4-byte locks as “valid” base data | Audit harness gives false positives/false negatives | Test-quality risk | Rebuild fixtures with canonical 32-byte locks | adversarial harness regression | No, but delay |
| High | repo-wide absence | property/invariant suite | No property-based proof for UTXO/replay/reward invariants | Subtle regressions survive example tests | Consensus regression risk | Add `proptest` suite | property CI lane | No, but delay |
| High | `crates/atho-storage/src/db.rs:235-236` | commit fault injection | Only one crash fault point exists | Silent corruption bug in an untested mutation window | Potential consensus-state corruption | Add more failpoints | crash matrix | No, but delay |
| Medium | `crates/atho-node/src/node.rs:462-474` | `dev_seed_chainstate` | Hidden state-mutation helper compiled into non-test builds | Future debug tool or route exposes it unsafely | Local state corruption if misused | Gate to tests/dev feature | release symbol/API regression | No |
| Medium | `crates/atho-storage/src/chainstate.rs:179-182`, `crates/atho-node/src/node.rs:490-492` | convenience startup | Panic-oriented startup path instead of fallible error propagation | Misconfigured snapshot or bad local state crashes process | Availability risk; fail-closed | Prefer fallible startup for defaults | startup integration test | No |
| Medium | `docs/testing.md:37-43` | testing documentation | Docs claim fuzz build should pass when it currently fails | Teams believe a gate exists when it does not | Process risk | Keep docs in lockstep with live commands | docs command CI | No |
| Medium | `crates/atho-node/src/bin/atho-adversarial.rs:1037+` | block campaign validation mode | Many adversarial cases use `validate_block_without_pow` | Hostile block coverage skips full PoW acceptance path | Coverage risk | Split no-PoW vs full-PoW matrices | adversarial CI | No |
| Medium | repo-wide | long fuzz execution | Fuzz targets exist but no executed baseline was proven in this pass | Malformed input bugs remain undiscovered | Coverage risk | Nightly fuzz execution | nightly fuzz job | No |
| Low | `crates/atho-node/src/service.rs:210+` / `:263+` | constructor split | Safer `try_new` exists, but many local helpers still use `new` | Tests/tooling may exercise harsher panic path than runtime | Low | Prefer `try_new` where practical | local tooling regression | No |

## 23. Subsystem Grades

| Subsystem | Grade | Mainnet Ready? | Missing Edge Cases | Biggest Risk | Required Fix |
|---|---:|---|---|---|---|
| Edge-case coverage | 7/10 | No | property/fuzz/crash gaps | false confidence | repair gates and add invariants |
| Block validation edge cases | 8/10 | Mostly | exact PoW equality, max-boundary matrices | boundary drift | add boundary/property tests |
| Transaction validation edge cases | 8/10 | Mostly | randomized grouped witness strategies | subtle witness regressions | add property tests |
| Coinbase edge cases | 8/10 | Mostly | underclaim and randomized fee matrix | policy drift | add reward equality properties |
| Monetary policy edge cases | 7/10 | Mostly | randomized high-height checks | schedule drift | add property/boundary tests |
| UTXO atomicity | 7/10 | No | multi-point crash injection | silent partial-write corruption | extend failpoints |
| Reorg safety | 8/10 | Mostly | arbitrary replay equivalence | replay mismatch | add property replay tests |
| Serialization safety | 7/10 | No | fuzz build broken, no roundtrip property suite | parser regressions | fix fuzz + add properties |
| Falcon signature safety | 7/10 | Mostly | dedicated parser fuzzing | malformed byte edge cases | add Falcon fuzz targets |
| PoW/difficulty safety | 7/10 | Mostly | exact target equality and skew tests | boundary ambiguity | add edge tests |
| Mempool/miner alignment | 8/10 | Mostly | large randomized differential tests | ordering/fee drift | add miner-vs-validator differential |
| Storage crash safety | 6/10 | No | partial-write matrix incomplete | silent restart corruption | expand fault injection |
| Network separation | 8/10 | Mostly | fuller external replay matrix | route-level drift | add cross-network raw input tests |
| Legacy bypass resistance | 8/10 | Mostly | stale harnesses | test-quality drift | update adversarial fixtures |
| Fuzz coverage | 4/10 | No | broken compile gate | no real hostile parser confidence | repair and execute fuzz |
| Property test coverage | 2/10 | No | absent | invariant regressions | add `proptest` |
| Regression test coverage | 7/10 | Mostly | missing new gate regressions | future refactors | add targeted regressions |
| CI mainnet gate quality | 5/10 | No | broken fuzz gate, no nightly hostile runs | false green | strengthen CI |

## 24. Final Mainnet Decision

## MAINNET DELAY RECOMMENDED

### Why not blocked outright

In this pass I did **not** reproduce:

- invalid block acceptance
- valid block rejection due to an obvious deterministic bug
- inflation from coinbase overclaim
- double-spend acceptance
- immature coinbase spend acceptance
- signature bypass
- mempool/block consensus mismatch that lets bad state in

The recent consensus fixes materially improved the chain.

### Why not ready

I still cannot sign “mainnet ready” because:

1. the **fuzz gate is currently broken**,
2. the built-in **adversarial runner is stale relative to current consensus rules**,
3. there is **no property-based invariant suite**,
4. crash/failpoint coverage is **not broad enough** to prove full atomicity.

That is not the same as “I found an active inflation bug.” It means the remaining risk is now **proof and coverage debt**, not obviously broken consensus logic. That is still enough to delay a production launch.

### Before mainnet

Must fix before launch:

- fuzz crate compile failure
- stale adversarial harness fixtures
- missing property-based invariants
- narrow crash-fault matrix

Should fix before or immediately after:

- hide or feature-gate `dev_seed_chainstate`
- make default local service constructors prefer fallible startup paths
- add exact PoW equality and time-boundary regressions

## 25. Final Deliverables

This report provides:

1. `CONSENSUS_EDGE_CASE_AND_ATTACK_TEST_AUDIT.md`
2. edge-case coverage review by subsystem
3. adversarial transaction and block testing assessment
4. monetary and replay boundary review
5. UTXO/crash/reorg safety review
6. serialization/fuzz audit
7. Falcon/parser hostile-input review
8. API/RPC consensus-boundary review
9. legacy/dead-code keyword risk table
10. missing-test report
11. critical/high-priority finding list
12. subsystem grade table
13. mainnet launch decision

## Closing note

The overall picture is encouraging: Atho is no longer failing because the core consensus code is obviously flimsy. The next tranche of work is about making the **proof that it stays correct** hard to fake:

- working fuzz gates,
- current adversarial fixtures,
- property invariants,
- broader crash-fault injection.

That is the difference between “looks strong in local review” and “I’m comfortable staking mainnet confidence on it.”
