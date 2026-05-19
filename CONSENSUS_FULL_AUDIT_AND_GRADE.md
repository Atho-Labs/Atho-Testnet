# Atho Full Consensus Audit and Production Grade

## 1. Executive Summary

### Overall Verdict

- Overall production readiness grade: **4/10**
- Overall consensus safety grade: **4/10**
- Mainnet launch decision: **MAINNET BLOCKED**
- External audit recommended: **Yes**

### Why launch is blocked

The codebase has a strong amount of localized validation logic and much better reorg handling than a toy chain, but I cannot call it production-safe because several consensus-critical rules are either under-committed or rely on state that is not fully verified at restart.

The two biggest blockers are:

1. **Consensus-relevant block data is not fully committed by the block header.**
   - `tx_pow_nonce` / `tx_pow_bits` are validated during block acceptance, but they are **not** committed by either the txid merkle root or the witness root.
   - `fees_total_atoms` / `fees_miner_atoms` are used during block validation, but they are **not** part of the canonical block bytes used by block-file storage, and they are not committed by the header.
   - Files: [crates/atho-core/src/transaction.rs](crates/atho-core/src/transaction.rs), [crates/atho-core/src/block.rs](crates/atho-core/src/block.rs), [crates/atho-storage/src/validation.rs](crates/atho-storage/src/validation.rs)

2. **The miner reward output is deterministically derived from public data.**
   - `reward_target_for_height()` derives a Falcon keypair from `sha3_384(network.domain_tag || height)`, which means **anyone can derive the private key and steal the reward** for that height once mature.
   - File: [crates/atho-node/src/mining.rs](crates/atho-node/src/mining.rs)

### Top 10 Highest-Risk Findings

1. **Critical:** block header does not commit `tx_pow_nonce` / `tx_pow_bits`, but validation depends on them.
2. **Critical:** mining rewards are sent to deterministic public keys derived from height and network.
3. **Critical:** startup consistency checks do not verify that persisted LMDB UTXOs match canonical block history.
4. **High:** coinbase witness bytes are not explicitly constrained, and malformed witness bytes collapse to the same witness commitment as empty witness bytes.
5. **High:** `Transaction::from_full_bytes()` accepts truncated full-transaction encodings with missing tx-PoW fields and defaults them to zero.
6. **High:** canonical block/transaction decoders allocate from attacker-controlled counts before enforcing sane bounds.
7. **High:** there is no upper bound on future block timestamps; only the lower MTP rule is enforced.
8. **High:** compact-block reconstruction uses txid-based short IDs even though tx-PoW fields are not committed by txid.
9. **Medium:** consensus-vs-policy comments are misleading in places where dust and fee floors are actually enforced during block validation.
10. **Medium:** miner template construction does not self-validate the completed candidate before exposing it to miners or RPC clients.

### Top 10 Fixes Required Before Mainnet

1. Commit **all** consensus-relevant transaction bytes into the block header commitment path.
2. Replace deterministic mining reward keys with operator-configured or wallet-controlled reward destinations.
3. Add a persisted UTXO integrity root or deterministic startup rebuild/verification path.
4. Make coinbase witness and tx-PoW fields strict: empty witness only, zero tx-PoW only.
5. Make `Transaction::from_full_bytes()` strict: missing tx-PoW tail must be rejected.
6. Bound all pre-allocation counts in manual decoders before `Vec::with_capacity(...)`.
7. Add a deterministic future timestamp ceiling.
8. Remove `fees_total_atoms` / `fees_miner_atoms` from consensus validity, or serialize and commit them canonically.
9. Make compact-block reconstruction use a commitment that includes the same bytes block validity depends on.
10. Add missing regression tests listed in Section 17.

### Top 10 Tests That Must Exist Before Mainnet

1. Block with altered tx-PoW fields but unchanged header commitments must be rejected or impossible to construct.
2. Block with altered `fees_total_atoms` / `fees_miner_atoms` and unchanged header must not create an alternate validity outcome.
3. Future-dated block above allowed drift must be rejected.
4. Coinbase with non-empty witness must be rejected.
5. Coinbase with non-zero `tx_pow_nonce` / `tx_pow_bits` must be rejected.
6. Full transaction encoding missing tx-PoW tail must be rejected.
7. Corrupted persisted UTXO set on dirty restart must be detected and repaired or fail closed.
8. Compact block reconstruction with same `txid` but different tx-PoW bytes must not silently accept the wrong variant.
9. Candidate block produced by the miner must round-trip through block validation successfully.
10. Two-coinbase and coinbase-not-first blocks must be explicitly tested.

## 2. Consensus-Critical System Map

| Area | File | Function/Class | Consensus Role | Risk Level |
|---|---|---|---|---|
| Block header hashing | `crates/atho-core/src/block.rs` | `BlockHeader::canonical_bytes`, `BlockHeader::block_hash` | Defines block identity and PoW hash input | Critical |
| Block body commitments | `crates/atho-core/src/block.rs` | `merkle_root`, `witness_root`, `Block::from_canonical_bytes` | Binds transactions to headers | Critical |
| Transaction serialization | `crates/atho-core/src/transaction.rs` | `base_bytes`, `full_bytes`, `from_full_bytes`, `txid`, `wtxid`, `witness_commitment_hash` | Defines txids, witness commitments, storage decode | Critical |
| Signature digest rules | `crates/atho-core/src/consensus/signatures.rs` | `transaction_signing_digest`, `transaction_signing_digest_for_input_indexes` | Defines the exact Falcon message being signed | Critical |
| Difficulty/chainwork | `crates/atho-core/src/consensus/pow.rs` | `target_for_next_block_with_timestamp`, `meets_target`, `compare_branch_work` | PoW target validation and branch selection | Critical |
| Emission schedule | `crates/atho-core/src/consensus/subsidy.rs` | `block_subsidy_atoms_for_network`, `cumulative_issued_*` | Subsidy and issuance schedule | High |
| Consensus versions | `crates/atho-core/src/consensus/rules.rs` | `rules_at_height`, `is_supported_*_version` | Height-gated ruleset/version enforcement | High |
| Network separation | `crates/atho-core/src/network.rs` | `consensus_id`, `p2p_magic`, `visible_prefix`, `utxo_flag` | Prevents cross-network replay and storage mixing | High |
| Genesis anchoring | `crates/atho-core/src/genesis.rs` | `genesis_state`, `genesis_hash` | Anchors network identity and initial UTXO state | Critical |
| Address ownership | `crates/atho-core/src/address.rs` | `public_key_digest`, `payment_digest_from_locking_script` | Canonical 32-byte ownership lock rules | Critical |
| Fee/dust/tx-PoW | `crates/atho-core/src/consensus/tx_policy.rs` | `minimum_required_fee_atoms`, `required_tx_pow_bits`, `transaction_pow_is_valid_for_bits` | Current relay and block acceptance transaction rules | Critical |
| Transaction validation | `crates/atho-storage/src/validation.rs` | `prepare_transaction_validation`, `validate_transaction_with_context_*` | Structural, contextual, ownership, fee, maturity, tx-PoW checks | Critical |
| Block validation | `crates/atho-storage/src/validation.rs` | `validate_block_with_context_and_schedule`, `validate_contextual_header_precheck` | Full block acceptance path | Critical |
| UTXO state machine | `crates/atho-storage/src/utxo.rs` | `UtxoSet::apply_block`, `disconnect_block`, `is_spendable_at` | UTXO updates, maturity, rollback | Critical |
| Chainstate / reorgs | `crates/atho-storage/src/chainstate.rs` | `connect_block`, `select_branch`, `switch_branch_incrementally`, `replace_with_validated_branch` | Canonical chain selection and reorg recovery | Critical |
| LMDB + flat files | `crates/atho-storage/src/db.rs` | `commit_chainstate`, `replace_chainstate`, `run_startup_consistency_checks`, `apply_utxo_delta` | Persistence of snapshots, UTXOs, blocks, txs | Critical |
| Data-dir separation | `crates/atho-storage/src/path.rs` | `database_dir`, `block_storage_dir`, `rpc_cookie_path` | Network-isolated storage paths | High |
| Miner template assembly | `crates/atho-node/src/mining.rs` | `build_candidate_block`, `reward_target_for_height` | Assembles blocks miners actually solve | Critical |
| Node block acceptance | `crates/atho-node/src/node.rs` | `connect_block`, `submit_block`, `consider_branch` | Consensus entrypoints from runtime and P2P | Critical |
| Mempool admission | `crates/atho-node/src/mempool.rs` | `admit`, `revalidate`, `reserve_inputs` | Policy admission and double-spend blocking | High |
| RPC transaction admission | `crates/atho-node/src/service.rs` | `parse_raw_transaction_hex`, `broadcast_transaction_value` | Externally exposed transaction submission path | High |
| P2P block transport | `crates/atho-p2p/src/protocol.rs` | `MessagePayload::Block`, `compact_block_from_block`, `reconstruct_compact_block` | Wire serialization and compact block reconstruction | Critical |
| Raw block storage | `crates/atho-storage/src/block_files.rs` | canonical block payload storage | Persists block bytes for restart/recovery | High |

### Consensus-Critical Storage Paths

| Storage Path / Dataset | Purpose | Risk Level |
|---|---|---|
| `LMDB meta` | chain snapshot, schema version, storage metadata, runtime state | Critical |
| `LMDB blocks` | per-block metadata and chainwork flags | Critical |
| `LMDB block_heights` | canonical height -> hash map | Critical |
| `LMDB block_transactions` | block -> ordered txid list | High |
| `LMDB transactions` | tx archive records | High |
| `LMDB utxos` | live UTXO set | Critical |
| `blocks/*.dat` flat files | canonical block payload archive | Critical |
| `chainstate.commit-journal` | crash recovery marker | High |

## 3. Block Validation Audit

### Block Validation Rule Matrix

| Rule | Where Enforced | Complete? | Tested? | Failure Mode | Fix Needed |
|---|---|---|---|---|---|
| Non-empty block | `validation.rs:839-841` | Yes | Yes | Empty block accepted if bypassed elsewhere | Keep |
| Block version at height | `validation.rs:842-844` | Yes | Yes (`future_block_version_is_rejected_before_activation`) | Chain split on version drift | Keep |
| Network id match | `validation.rs:845-847` | Yes | Partial | Cross-network block acceptance | Add explicit mainnet/testnet mismatch test |
| Height match | `validation.rs:848-850` | Yes | Partial | Valid block rejected / invalid height accepted | Add explicit bad-height block test |
| Timestamp nonzero | `validation.rs:851-853` | Yes | Partial | Zero timestamp block accepted | Add explicit test |
| Lower timestamp bound (MTP+1) | `validation.rs:1220-1224`, `pow.rs:387-389` | Yes | Yes | Time-travel below MTP | Keep |
| **Upper future timestamp bound** | **Missing** | **No** | **No** | Future-dated blocks distort chain timing/difficulty | Add deterministic future cap |
| Target bounds | `validation.rs:865-868` | Yes | Yes | Out-of-range target accepted | Keep |
| Exact expected target | `validation.rs:1225-1227` | Yes | Yes (`contextual_validation_rejects_unexpected_target`) | Invalid retarget accepted | Keep |
| PoW hash <= target | `validation.rs:1228-1230`, `954-956` | Yes | Yes | Invalid header accepted | Keep |
| Merkle root | `validation.rs:872-875` | Yes | Partial | Body/header mismatch | Add explicit bad-merkle test |
| Witness root | `validation.rs:876-879` | **Partial** | Yes (`header_witness_root_must_match_body_commitment`) | Malformed witness bytes can collapse to same commitment | Commit raw witness bytes or reject non-empty malformed witness in all paths |
| Coinbase first | `collect_prepared...` validates `tx[0]` as coinbase | Partial | Partial | Non-first coinbase rejection is implicit, not explicit | Add explicit test |
| Exactly one coinbase | Remaining txs rejected via `prepare_transaction_validation` on empty inputs | Partial | No explicit test | Second coinbase rejected, but through generic path | Add explicit two-coinbase test |
| Duplicate txid rejection | `validation.rs:910-912` | Yes | Partial | Same txid twice in one block | Add direct duplicate-txid block test |
| Duplicate input rejection across block | `validation.rs:913-915`, `1163-1168` | Yes | Partial | Intra-block double spend | Add direct duplicate-input block test |
| Block size / weight / vsize | `validation.rs:822-830` | Yes | Yes | Oversized block accepted | Keep |
| Fee sum exact | `validation.rs:1196-1200` | **Partial / risky** | Partial | Depends on uncommitted fee metadata | Remove fee metadata from consensus, compute fee exactness directly |
| Coinbase reward exact | `validation.rs:900-909`, `796-819` | **Partial** | Partial | Depends on coinbase fields plus fee metadata | Compare coinbase output directly to subsidy + computed fees |
| UTXO transition only after full validation | `chainstate.rs:248-281` | Yes | Yes | Dirty state after failed block | Keep |

### Block Validation Findings

#### Strengths

- Block validation is centralized instead of being spread across wallet, miner, mempool, and storage paths.
- Contextual validation checks the parent hash, expected target, PoW, duplicate block inputs, fee exactness, and maturity before mutating state.
- Reorg-specific validation reuses the same block validation logic through a UTXO overlay rather than trusting mempool prevalidation.

#### Critical Gaps

1. **Consensus-critical block validity depends on uncommitted fields.**
   - `tx_pow_nonce` / `tx_pow_bits` are checked during transaction validation inside block validation.
   - But the header only commits `txid` merkle root and `witness_commitment_hash`, and the current witness commitment hash excludes tx-PoW fields.
   - Files: `transaction.rs:616-623`, `block.rs:438-477`, `validation.rs:628-633`, `661-666`

2. **Fee metadata is used as if it were consensus input.**
   - `validate_block_with_context_and_schedule()` rejects blocks when `sum_fees != block.fees_total_atoms` or `fees_total_atoms != fees_miner_atoms`.
   - Those fields are not in `Block::canonical_bytes()` and are not committed by the header.
   - Files: `block.rs:269-297`, `validation.rs:1196-1200`

3. **Future timestamp bound is missing.**
   - Current logic only enforces `timestamp >= MTP + 1`.
   - A miner can stamp blocks arbitrarily far into the future and still satisfy validation.

4. **Coinbase witness handling is too loose.**
   - Coinbase validation does not require empty witness or zero tx-PoW fields.
   - Malformed witness bytes are effectively omitted from the witness commitment hash.

### Block Validation Grade: **5/10**

Why:

- The validator checks many important rules correctly.
- But a block must be a self-authenticating consensus object. Right now, the header does not commit all of the data that block acceptance depends on. That is a launch blocker.

## 4. Transaction Validation Audit

### Transaction Rule Matrix

| Rule | Where Enforced | Complete? | Tested? | Failure Mode | Fix Needed |
|---|---|---|---|---|---|
| Supported tx version | `validation.rs:354-358` | Yes | Yes | Version drift | Keep |
| Non-coinbase txs must have inputs | `prepare_transaction_validation()` | Yes | Partial | Empty-input normal tx accepted | Add explicit test |
| Outputs must exist | `validation.rs:359-361` | Yes | Partial | Burn-only or malformed tx accepted | Add explicit test |
| Output count cap | `validation.rs:362-364` | Yes | Yes | DoS/huge fanout | Keep |
| Raw/vsize caps | `validation.rs:365-369` | Yes | Yes | Oversized tx accepted | Keep |
| No zero outputs | `validation.rs:370-372` | Yes | Partial | Value ambiguity / weird accounting | Add explicit test |
| Dust floor | `validation.rs:383-390` | Yes | Yes | Spam / block-policy drift | Keep, but clarify consensus-vs-policy docs |
| Duplicate input rejection | `validation.rs:373-378` | Yes | Partial | Double spend within tx | Add explicit direct test |
| Fee floor | `validation.rs:379-381`, `658-659` | Yes | Yes | Fee-below-floor tx accepted | Keep |
| Canonical 32-byte output locks only | `validation.rs:391-393`, `463-468` | Yes | Yes | Legacy lock accepted | Keep |
| Witness present / parseable | `validation.rs:394-399` | Yes | Yes | Malformed witness bypass | Keep |
| Signer-group coverage of all inputs | `validation.rs:400-455` | Yes | Partial | Uncovered input or duplicate ref | Add targeted test |
| Falcon pubkey/signature lengths | `validation.rs:277-282` | Yes | Yes | Truncated/oversized witness accepted | Keep |
| Falcon signature digest | `validation.rs:287-297`, `signatures.rs:60-81` | Yes | Yes | Wrong-message signature accepted | Keep |
| Ownership digest binding | `validation.rs:688-700` | Yes | Yes | Wrong pubkey spends UTXO | Keep |
| Network/genesis replay protection | `signatures.rs:60-81`, `tx_policy.rs:186-193` | Yes | Yes | Cross-network replay | Keep |
| Maturity / confirmations | `validation.rs:701-703`, `utxo.rs:71-92` | Yes | Partial | Immature spend accepted | Add explicit coinbase maturity test at validator level |
| Exact fee arithmetic | `validation.rs:704-717`, `605-636` | Yes | Yes | Inflation/underflow | Keep |
| Tx-PoW bits/nonce | `validation.rs:545-552`, `628-633`, `661-666` | Yes | Yes | Spam rule bypass | **Commit tx-PoW fields into block commitment path** |
| Canonical full decode strictness | `transaction.rs:668-770` | **No** | No | Truncated full encoding accepted | Make missing tx-PoW tail invalid |
| Allocation bounds before decode | `transaction.rs:719-746`, `TxWitness::from_bytes` | **No** | No | OOM / parser DoS | Bound counts before allocation |

### Transaction Validation Findings

#### Strengths

- Ownership validation is good: the unlocking script must match the canonical 32-byte lock, and the signer pubkey must hash to the same digest.
- Network and genesis hash are mixed into the Falcon signing digest, which is the right move for replay separation.
- Exact fee computation uses checked arithmetic and rejects overspends cleanly.
- Legacy non-32-byte locking scripts are explicitly rejected.

#### High-Risk Gaps

1. **The full transaction decoder is not strict.**
   - `Transaction::from_full_bytes()` defaults missing tx-PoW tail bytes to `(0, 0)` if the payload ends after `lock_time`.
   - File: `transaction.rs:750-760`

2. **Parser allocations are attacker-controlled before caps are enforced.**
   - `Vec::with_capacity(input_count)`, `Vec::with_capacity(output_count)`, and witness group allocations happen before any tight sanity bound derived from remaining bytes.

3. **Tx-PoW is consensus-relevant but not committed into the header.**
   - This is partly a serialization issue, partly a transaction-validity issue.

### Transaction Validation Grade: **7/10**

Why:

- The live validation logic for amounts, ownership, signatures, locks, and fee math is reasonably strong.
- The main downgrades are serialization strictness and the fact that tx-PoW bytes are required for validity but are not committed by the header.

## 5. UTXO Accounting Audit

### UTXO Audit Summary

| Check | Status | Notes |
|---|---|---|
| UTXO creation after valid block | Good | `UtxoSet::apply_block`, `create_outputs` |
| UTXO spend/delete | Good | `spend_inputs`, `UtxoSet::remove` |
| Duplicate UTXO prevention | Good | `UtxoSet::insert`, `db.rs:1446-1448` |
| Atomic DB updates | Good | `commit_chainstate()` LMDB rw txn + block archive append journaling |
| Failed block leaves dirty state | Good in memory | `apply_block()` rolls back via `disconnect_block(undo)` |
| Reorg rollback | Good | Incremental journal + full validated rewrite fallback |
| Startup persisted state verification | **Weak** | Snapshot/tip checked, UTXO contents not checked |
| Dirty restart false-validity risk | **High** | Corrupted UTXOs can survive restart |
| Missing spent-key handling | **Weak** | `apply_utxo_delta()` treats missing key delete as idempotent |

### Deep Findings

1. **In-memory UTXO mutation path is much better than the persisted restart path.**
   - During live operation, validation happens before `apply_block`, and `apply_block()` can roll itself back.
   - Files: `chainstate.rs:248-281`, `utxo.rs:149-190`

2. **Dirty restart trust model is not sufficient for production.**
   - `run_startup_consistency_checks()` only proves that the snapshot tip matches block metadata.
   - It does not prove the persisted UTXO set matches the canonical chain.
   - Files: `db.rs:1006-1063`, `chainstate.rs:1400-1411`

3. **LMDB divergence can be masked.**
   - `apply_utxo_delta()` accepts `LmdbError::NotFound` on spent-key deletes as an idempotent delete.
   - That is practical for dev/recovery scenarios, but unsafe as a permanent production default because it can hide prior UTXO corruption.
   - File: `db.rs:1421-1430`

### UTXO Accounting Grade: **5/10**

Why:

- The live UTXO state machine is decent.
- The restart/recovery trust model is not strong enough for production because persisted UTXO correctness is not fully re-proven after an unclean shutdown or storage tampering event.

## 6. Monetary Policy Audit

### Monetary Policy Findings

- Reward schedule is deterministic and centralized in `subsidy.rs`.
- Checked arithmetic is used for coinbase reward calculation and fee addition.
- Cumulative issuance helpers exist and are tested at schedule boundaries.
- There is **no finite max supply cap** in the current code by design.
  - `max_supply_atoms_for_network()` returns `None`.
  - Files: `subsidy.rs:113-118`, `params.rs:20-21, 40`

### Monetary Policy Risk Notes

| Check | Status | Notes |
|---|---|---|
| Block subsidy calculation | Good | `subsidy.rs:38-65` |
| Halving/tail emission | Good | Tail emission floor is explicit |
| Max supply enforcement | N/A / design-specific | No hard cap exists |
| Fee inclusion in reward | Good | Coinbase reward checked against subsidy + fees |
| Overflow risks | Good | `checked_add`, `checked_sub`, `checked_output_value_atoms` |
| Mainnet/testnet/regtest separation | Neutral | Same schedule across all networks right now |
| Miner overclaim prevention | Good if fee metadata issue fixed | Overclaim blocked by coinbase amount + fee exactness |

### Monetary Policy Grade: **8/10**

Why:

- The issuance math itself is clear and tested.
- This score is not 10/10 because block fee metadata is currently part of the validation decision in an under-committed way, which bleeds into coinbase economics.

## 7. Coinbase Validation Audit

### Findings

| Rule | Status | Notes |
|---|---|---|
| Exactly one coinbase | Partial | Implicitly enforced because only `transactions[0]` may be coinbase and remaining txs must have inputs |
| Coinbase first | Partial | Same as above; needs explicit test |
| Coinbase output count == 1 | Good | `validation.rs:809-811` |
| Coinbase reward exact | Good but coupled to fee metadata | `validation.rs:813-818`, `900-909` |
| Coinbase maturity | Good | `utxo.rs:77-92` |
| Coinbase deterministic txid | Good | Standard transaction base serialization |
| Coinbase witness restrictions | **Missing** | No explicit `witness.is_empty()` rule |
| Coinbase tx-PoW restrictions | **Missing** | No explicit `tx_pow_nonce == 0 && tx_pow_bits == 0` rule |

### Critical Gap

Because the coinbase witness and tx-PoW fields are not explicitly constrained, and malformed non-empty witness bytes can collapse to the same witness commitment as empty bytes, the coinbase path is under-specified.

### Coinbase Validation Grade: **4/10**

Why:

- Reward amount and basic shape are checked.
- The missing coinbase witness/tx-PoW strictness is too large a consensus ambiguity to ignore.

## 8. Falcon-512 Signature Validation Audit

### Findings

| Check | Status | Notes |
|---|---|---|
| Correct message signed | Good | `signatures.rs:60-81` |
| Correct message verified | Good | `validation.rs:287-297` |
| Network binding | Good | `network.consensus_id()` included |
| Genesis binding | Good | `genesis_hash(network)` included |
| Truncated/oversized signature rejected | Good | Exact length checks in witness parser and verifier |
| Wrong public key rejected | Good | Digest must match canonical UTXO lock |
| Wrong address rejected | Good | Lock digest match enforced |
| Replay across networks | Good | Explicit tests exist |
| Batch verification safety | Good enough | No shared batch cache; parallel verification is independent |
| Signature cache correctness | N/A | No cache used |

### Falcon Signature Validation Grade: **8/10**

Why:

- The signature-domain separation and network/genesis binding are strong.
- The main reason this is not higher is that block-level commitment gaps around tx-PoW/witness bytes can still make block payload identity ambiguous even when the Falcon verifier itself is correct.

## 9. Serialization / Hashing Audit

### Critical Serialization Findings

1. **Block header does not commit all consensus-relevant transaction bytes.**
   - `txid()` uses base bytes only: `transaction.rs:600-608`
   - `witness_commitment_hash()` uses base bytes + parsed witness commitment only: `transaction.rs:616-623`
   - `witness_root()` commits only `witness_commitment_hash()`: `block.rs:457-477`
   - `tx_pow_nonce` and `tx_pow_bits` are therefore validated but not committed by the header.

2. **Block fee metadata is not in canonical block bytes.**
   - `Block::full_bytes()` / `Block::canonical_bytes()` do not serialize `fees_total_atoms` or `fees_miner_atoms`.
   - File: `block.rs:269-297`

3. **Full transaction canonical decoder is permissive.**
   - Missing tx-PoW tail defaults to zero instead of rejecting.
   - File: `transaction.rs:750-760`

4. **Manual decoders allocate before bounding counts.**
   - `transaction.rs:719-746`
   - `transaction.rs:246-250`, `273-292`
   - `block.rs:419-425`

5. **Malformed witness bytes are under-committed.**
   - `witness_payload()` returns `None` on malformed bytes.
   - `witness_commitment_hash()` then behaves as if there were no parsed witness.
   - File: `transaction.rs:655-660`, `616-623`

### Serialization and Hashing Grade: **4/10**

Why:

- Header, txid, and block hashing code is deterministic.
- But determinism is not enough when the committed object does not include all the bytes that validity depends on.

## 10. Mempool vs Block Consensus Alignment

### Findings

| Check | Status | Notes |
|---|---|---|
| Mempool rejects invalid consensus txs | Good | Calls `validate_transaction_with_context_for_mempool()` |
| Mempool extra policy on top of consensus | Good | Dust / standard inputs / fee floor layered on |
| Block validation does not trust mempool prevalidation | Good | Revalidates directly against overlay UTXO set |
| Reorg reinsertion | Good | Disconnected txs are re-admitted if still valid |
| Miner candidate assembly uses mempool state | Good | Conflict filtering and size accounting in `mining.rs` |
| Miner template self-validation | **Missing** | Candidate not run back through block validator |
| Policy-vs-consensus clarity | **Poor comments** | Dust/fee floor are described as policy-only in places even though `prepare_transaction_validation()` enforces them during block validation |

### Mempool / Block Alignment Grade: **7/10**

Why:

- The actual code paths are mostly aligned.
- The main downgrades are missing miner self-validation and potentially misleading comments about which rules are policy versus consensus.

## 11. Miner Consensus Audit

### Findings

| Check | Status | Notes |
|---|---|---|
| Uses same subsidy schedule | Good | `mining.rs:45-49` |
| Uses same size/weight/vsize limits | Good | `mining.rs:108-125` |
| Avoids double spends in template | Good | `mining.rs:84-105` |
| Uses same fee calculation model | Good enough | Pulls `fee_atoms` from validated entries |
| Self-validates completed template | **Missing** | Should call block validator before handing block to miners/RPC |
| Reward destination ownership | **Critical failure** | Deterministic public seed-derived reward key |

### Miner Consensus Grade: **3/10**

Why:

- The template assembly mechanics are decent.
- The reward address design is not safe for any real network.

## 12. Difficulty and Proof-of-Work Audit

### Findings

| Check | Status | Notes |
|---|---|---|
| Target calculation | Good | `pow.rs:242-305` |
| Chainwork comparison | Good | `pow.rs:350-373` |
| Target bounds | Good | `pow.rs:391-412` |
| PoW comparison endianness | Good | `hash <= target` on same big-endian byte ordering |
| MTP lower bound | Good | `pow.rs:375-389`, `validation.rs:1220-1224` |
| Future drift ceiling | **Missing** | No upper bound |
| Testnet special rule | Good | `pow.rs:295-303`, tests exist |
| Validation independent of miner | Good | `validation.rs` uses `pow::meets_target` directly |

### PoW and Difficulty Grade: **6/10**

Why:

- Retargeting and chainwork code is well-scoped and tested.
- The missing future timestamp ceiling keeps this from being mainnet-grade.

## 13. Chain Selection and Reorg Audit

### Findings

| Check | Status | Notes |
|---|---|---|
| Best chain by cumulative work | Good | `pow.rs:350-373`, `chainstate.rs:327-332` |
| Height-only bug resistance | Good | Work compare first, height second |
| Max reorg depth | Good | `chainstate.rs:323-325`, `1458-1485` |
| Incremental rollback safety | Good | `switch_branch_incrementally`, `rollback_incremental_branch_switch` |
| Full rewrite fallback | Good | `replace_with_validated_branch` |
| Mempool restoration after reorg | Good | `node.rs:665-683` |
| Crash during reorg | Good coverage | Multiple restart/reorg tests in `chainstate.rs` |
| Dependency on persisted UTXO correctness | **Weak** | Reorg logic assumes loaded UTXOs are canonical |

### Chain Selection and Reorg Grade: **7/10**

Why:

- The reorg code is materially better than typical early-stage chain code.
- The grade is capped by storage-trust issues on restart.

## 14. Network Constants and Environment Separation

### Findings

| Check | Status | Notes |
|---|---|---|
| Unique consensus ids | Good | `network.rs:38-57` |
| Unique magic bytes | Good | `network.rs:104-122` |
| Unique ports | Good | `network.rs:84-102` |
| Unique visible prefixes | Good | `network.rs:124-141` |
| Genesis separation | Good | `genesis.rs` per-network constants |
| DB path separation | Good | `path.rs:11-48` |
| Runtime override of mainnet consensus constants | No evidence found | Sync knobs are runtime-configurable; consensus constants are not |
| Prunetest-only max reorg env override | Acceptable | Non-mainnet only |

### Network Constants Grade: **8/10**

Why:

- Network separation is one of the cleaner parts of the codebase.

## 15. Storage and Database Consensus Safety

### Findings

| Check | Status | Notes |
|---|---|---|
| Atomic commits | Good | `db.rs:694-769`, `774-831` |
| Crash journal | Good | `CommitJournalGuard` usage |
| Block archive + LMDB alignment | Good during normal commit | Both updated in one logical path |
| Schema versioning | Good | `db.rs:911-935` |
| Genesis / network metadata checks | Good | `db.rs:952-1004` |
| Startup state verification | **Insufficient** | Tip/header checked, UTXO contents not checked |
| Corrupted UTXO false-validity risk | **High** | See Findings B and `apply_utxo_delta` |
| Raw block archive self-sufficiency | **Weak** | Canonical raw block bytes omit fee metadata |

### Storage Consensus Safety Grade: **4/10**

Why:

- Normal commit atomicity is decent.
- Restart trust and persistence integrity are not strong enough for production consensus safety.

## 16. Legacy Code and Bypass Audit

### Findings

| Search Area | Result | Risk |
|---|---|---|
| Legacy lock formats | Explicitly rejected (`validation.rs:463-468`) | Low |
| Legacy TSV snapshot/runtime layouts | Quarantined / rejected (`chainstate.rs:1414-1419`, `1488+`) | Low |
| Accept-on-error paths in validation | None found in consensus validator | Low |
| Internal skip-PoW helper | Exists but clearly labeled internal (`validation.rs:1010-1019`) | Medium if misused later |
| Decoder fallback accepting old shape | `Transaction::from_full_bytes()` missing tx-PoW tail default | High |
| Malformed witness collapsing to no witness in commitments | Present | High |

### Legacy and Bypass Risk Grade: **7/10**

Why:

- The codebase is no longer carrying many obvious “legacy accept both formats” landmines.
- The biggest remaining bypass flavor is decoder permissiveness and malformed-witness under-commitment, not classic legacy-compatibility sprawl.

## 17. Required Test Suite

### Block Tests

| Test Name | Purpose | Setup | Expected Result | Code Location | Currently Exists? | Status |
|---|---|---|---|---|---|---|
| `valid_block_accepted` | Baseline block acceptance | Build solved canonical block at next height | Accepted and state mutates | `crates/atho-storage/src/chainstate.rs` | Yes (`chainstate_tracks_tip_and_height`) | Pass |
| `bad_previous_hash_rejected` | Parent binding | Use wrong `previous_block_hash` | Reject with parent mismatch | `crates/atho-node/src/node.rs` or `validation.rs` | Yes (`node_rejects_wrong_parent_hash`) | Pass |
| `bad_height_rejected` | Height binding | Header height != expected height | Reject | `validation.rs` | Partial | Pass (partial) |
| `bad_merkle_root_rejected` | Body/header commitment | Mutate header merkle root | Reject with merkle mismatch | `validation.rs` | No | Missing |
| `bad_pow_rejected` | PoW enforcement | Unsolved block or wrong nonce | Reject with PoW invalid | `validation.rs` | Partial | Pass (partial) |
| `bad_timestamp_rejected_future` | Future drift cap | Timestamp beyond allowed ceiling | Reject | `validation.rs` | No | Missing |
| `oversized_block_rejected` | Size caps | Exceed raw/vsize/weight | Reject | `validation.rs` | Yes (`oversized_block_*`) | Pass |
| `block_with_no_coinbase_rejected` | Coinbase presence | First tx non-coinbase | Reject | `validation.rs` | Partial | Missing explicit |
| `block_with_two_coinbases_rejected` | Unique coinbase | First and later tx are coinbase | Reject | `validation.rs` | No | Missing |
| `coinbase_not_first_rejected` | Coinbase position | Put coinbase at index > 0 | Reject | `validation.rs` | No | Missing |
| `duplicate_txid_rejected` | Duplicate tx detection | Same tx twice | Reject | `validation.rs` | No | Missing |
| `duplicate_input_rejected_in_block` | Intra-block double spend | Two txs spend same outpoint | Reject | `validation.rs` | No | Missing |
| `overclaim_reward_rejected` | Inflation prevention | Coinbase output > subsidy + fees | Reject | `validation.rs` | Partial | Pass (partial) |
| `extra_money_created_rejected` | Fee exactness | Outputs exceed inputs + subsidy | Reject | `validation.rs` | Partial | Pass (partial) |
| `missing_utxo_in_block_rejected` | UTXO existence | Spend non-existent output | Reject | `validation.rs` | Partial | Pass (partial) |
| `spent_utxo_in_block_rejected` | Double-spend across blocks | Spend already-spent output | Reject | `validation.rs` / `chainstate.rs` | Partial | Pass (partial) |
| `immature_coinbase_spend_in_block_rejected` | Maturity rule | Spend immature coinbase | Reject | `validation.rs` | No | Missing |
| `invalid_signature_in_block_rejected` | Signature enforcement | Corrupt Falcon signature | Reject | `validation.rs` | Partial | Pass (partial) |
| `coinbase_with_nonempty_witness_rejected` | Coinbase strictness | Add witness bytes to coinbase | Reject | `validation.rs` | No | Missing |
| `coinbase_with_nonzero_txpow_rejected` | Coinbase strictness | Set `tx_pow_nonce/bits` on coinbase | Reject | `validation.rs` | No | Missing |

### Transaction Tests

| Test Name | Purpose | Setup | Expected Result | Code Location | Currently Exists? | Status |
|---|---|---|---|---|---|---|
| `valid_transaction_accepted` | Baseline tx validity | Canonical spend with correct witness and fee | Accept | `validation.rs` / `mempool.rs` | Yes | Pass |
| `invalid_signature_rejected` | Falcon validity | Corrupt signature bytes | Reject | `validation.rs` | Partial | Pass (partial) |
| `wrong_public_key_rejected` | Ownership binding | Use mismatched Falcon pubkey | Reject | `validation.rs` | Yes (`wrong_public_key_for_standard_output_is_rejected`) | Pass |
| `wrong_address_rejected` | Lock mismatch | Unlocking script differs from UTXO lock | Reject | `validation.rs` | Partial | Pass (partial) |
| `wrong_network_prefix_rejected` | Cross-network replay | Mainnet-signed tx on testnet | Reject | `validation.rs` | Yes | Pass |
| `missing_input_rejected` | UTXO existence | Spend missing outpoint | Reject | `validation.rs` | Partial | Pass (partial) |
| `duplicate_input_rejected` | Intra-tx double spend | Reuse same outpoint twice | Reject | `validation.rs` | No direct test | Missing |
| `negative_output_rejected` | Impossible with `u64` | N/A in Rust type system | N/A | `validation.rs` | N/A | Type-safe |
| `zero_output_rejected` | Zero-value output rule | Output value = 0 | Reject | `validation.rs` | No direct test | Missing |
| `dust_output_rejected` | Dust rule | Output below floor | Reject | `validation.rs` / `mempool.rs` | Yes | Pass |
| `fee_below_floor_rejected` | Fee floor | Fee below required minimum | Reject | `validation.rs` | Partial | Pass (partial) |
| `output_sum_gt_input_sum_rejected` | Inflation prevention | Overspend outputs | Reject | `validation.rs` | Partial | Pass (partial) |
| `oversized_transaction_rejected` | Size cap | Exceed tx raw/vsize cap | Reject | `validation.rs` | Yes | Pass |
| `malformed_full_serialization_rejected` | Parser strictness | Corrupt canonical tx bytes | Reject | `transaction.rs` / `service.rs` | Partial | Pass (partial) |
| `missing_txpow_tail_rejected` | Full decoder strictness | Remove tx-PoW tail from full tx bytes | Reject | `transaction.rs` | No | Missing |
| `extra_unknown_consensus_bytes_rejected` | Canonical parse | Append bytes after tx | Reject | `transaction.rs` / `service.rs` | Partial | Pass (partial) |
| `missing_required_fields_rejected` | Parser hardening | Truncate witness or outputs | Reject | `transaction.rs` | Yes (`witness_payload_rejects_truncated_payload`) | Pass |
| `replay_from_another_network_rejected` | Replay protection | Re-sign for wrong network | Reject | `validation.rs` | Yes | Pass |

### Monetary Tests

| Test Name | Purpose | Setup | Expected Result | Code Location | Currently Exists? | Status |
|---|---|---|---|---|---|---|
| `reward_at_height_0_correct` | Genesis subsidy | Query subsidy at 0 | Exact value | `subsidy.rs` | Yes | Pass |
| `reward_at_normal_height_correct` | Standard era subsidy | Query mid-era height | Exact value | `subsidy.rs` | Yes | Pass |
| `reward_at_halving_boundary_correct` | Boundary behavior | Query first block after halving | Exact value | `subsidy.rs` | Yes | Pass |
| `reward_after_halving_correct` | Post-boundary behavior | Query later height | Exact value | `subsidy.rs` | Yes | Pass |
| `tail_emission_correct` | Tail era | Query deep height | Exact tail amount | `subsidy.rs` | Yes | Pass |
| `max_supply_cannot_be_exceeded` | Cap enforcement | N/A because no hard cap | Not applicable / documented no-cap | `subsidy.rs`, `params.rs` | N/A | Design-specific |
| `coinbase_cannot_claim_more_than_subsidy_plus_fees` | Inflation prevention | Overclaim block | Reject | `validation.rs` | Partial | Pass (partial) |
| `fee_calculation_exact` | No double-count/underflow | Build exact-fee tx/block | Exact fee result | `validation.rs` | Partial | Pass (partial) |
| `atomic_precision_exact` | Integer-only accounting | Sum/format atoms | No rounding drift in consensus | `constants.rs`, `subsidy.rs` | Yes | Pass |
| `no_rounding_inflation` | No float use | Checked integer arithmetic | No inflation | `validation.rs`, `subsidy.rs` | Yes | Pass |

### UTXO Tests

| Test Name | Purpose | Setup | Expected Result | Code Location | Currently Exists? | Status |
|---|---|---|---|---|---|---|
| `utxo_created_after_block_acceptance` | Output creation | Connect valid block | New outputs present | `utxo.rs` / `chainstate.rs` | Yes | Pass |
| `utxo_spent_after_valid_transaction` | Spend path | Connect spending block | Spent output removed | `utxo.rs` | Yes | Pass |
| `utxo_not_spent_after_failed_transaction` | Rollback safety | Fail during apply | Original UTXO preserved | `utxo.rs` / `chainstate.rs` | Yes | Pass |
| `utxo_not_mutated_after_failed_block` | State cleanliness | Invalid block | Chainstate unchanged | `chainstate.rs` | Yes (`invalid_block_is_rejected_without_mutating_chainstate`) | Pass |
| `double_spend_same_block_rejected` | Intra-block conflict | Two txs spend same outpoint | Reject | `validation.rs` | No direct test | Missing |
| `double_spend_across_blocks_rejected` | Cross-block conflict | Spend already-spent UTXO | Reject | `chainstate.rs` | Partial | Pass (partial) |
| `coinbase_maturity_enforced` | Coinbase lockup | Spend immature coinbase | Reject | `validation.rs`, `utxo.rs` | Partial | Pass (partial) |
| `reorg_restores_utxos_correctly` | Reorg accounting | Switch to better fork | Old spends undone, new spends applied | `chainstate.rs` | Yes | Pass |
| `crash_does_not_corrupt_chainstate` | Restart safety | Fault inject commit / dirty restart | Recover or fail closed | `chainstate.rs`, `db.rs` | Partial | Pass (partial) |
| `corrupted_persisted_utxo_detected_on_restart` | Startup integrity | Tamper LMDB UTXO value | Detection and rebuild/fail | `db.rs`, `chainstate.rs` | No | Missing |

### Serialization Tests

| Test Name | Purpose | Setup | Expected Result | Code Location | Currently Exists? | Status |
|---|---|---|---|---|---|---|
| `same_tx_same_txid` | Canonical txid | Encode same tx twice | Same txid | `transaction.rs` | Yes | Pass |
| `same_block_same_hash` | Canonical block hash | Encode same header twice | Same block hash | `block.rs` | Yes | Pass |
| `field_order_cannot_change_hash` | Hash stability | Reorder encoding fields in adversarial test | Different / invalid | `transaction.rs`, `block.rs`, adversarial tests | Partial | Pass (partial) |
| `missing_fields_rejected` | Parse strictness | Truncate tx/block bytes | Reject | `transaction.rs`, `block.rs` | Partial | Pass (partial) |
| `extra_fields_handled_safely` | Parse strictness | Append bytes | Reject | `service.rs`, `transaction.rs`, `block.rs` | Partial | Pass (partial) |
| `legacy_formats_rejected` | Canonical-only decode | Use legacy lock / legacy tx shape | Reject | `validation.rs`, `service.rs` | Partial | Pass (partial) |
| `atx2_canonical_full_tx_enforced` | Full bytes exactness | Non-canonical but parseable full tx | Reject | `transaction.rs`, `service.rs` | Partial | Pass (partial) |
| `binary_codec_roundtrip_exact` | Stable storage/wire bytes | Roundtrip tx/block bytes | Equal bytes | `transaction.rs`, `block.rs` | Yes | Pass |
| `txpow_tail_required_in_full_tx` | Decoder strictness | Omit tx-PoW tail | Reject | `transaction.rs` | No | Missing |
| `fees_metadata_not_needed_for_block_validity` | Remove under-commitment | Validate block from canonical bytes only | Same validity outcome | `validation.rs`, `block.rs` | No | Missing |

### Falcon Tests

| Test Name | Purpose | Setup | Expected Result | Code Location | Currently Exists? | Status |
|---|---|---|---|---|---|---|
| `valid_falcon_signature_accepted` | Baseline | Canonical signed tx | Accept | `validation.rs` | Yes | Pass |
| `invalid_falcon_signature_rejected` | Integrity | Corrupt signature | Reject | `validation.rs` | Partial | Pass (partial) |
| `truncated_signature_rejected` | Parser hardening | Short signature length | Reject | `transaction.rs` / `validation.rs` | Yes (witness parser) | Pass |
| `oversized_signature_rejected` | Parser hardening | Long signature length | Reject | `transaction.rs` | Yes | Pass |
| `signature_for_different_message_rejected` | Digest correctness | Re-sign altered tx | Reject | `validation.rs` | Partial | Pass (partial) |
| `signature_for_different_tx_rejected` | Tx binding | Swap body under same witness | Reject | `validation.rs` | Partial | Pass (partial) |
| `signature_for_different_network_rejected` | Replay protection | Mainnet sig on testnet | Reject | `validation.rs` | Yes | Pass |
| `signature_cache_cannot_bypass_verification` | Cache safety | N/A no cache | No bypass possible | N/A | N/A | N/A |
| `full_signature_storage_retrieval_verified` | Persistence safety | Store/load tx with witness | Same bytes | `transaction.rs`, storage tests | Partial | Pass (partial) |

### Reorg Tests

| Test Name | Purpose | Setup | Expected Result | Code Location | Currently Exists? | Status |
|---|---|---|---|---|---|---|
| `longer_valid_chain_wins` | Canonical fork choice | Better-work branch | Reorg | `chainstate.rs` | Yes | Pass |
| `higher_work_wins_over_height_only` | Chainwork priority | Shorter but more-work branch | Prefer higher work | `pow.rs`, `sync.rs` | Yes | Pass |
| `invalid_longer_chain_rejected` | Reorg validation | Bad candidate branch | Keep current | `chainstate.rs` | Yes | Pass |
| `deep_reorg_beyond_limit_rejected` | Finalization / safety | Exceed max reorg depth | Reject | `chainstate.rs` | Yes | Pass |
| `reorg_rolls_back_utxos_correctly` | Accounting | Disconnect/reconnect branch | Exact UTXO restore | `chainstate.rs` | Yes | Pass |
| `reorg_restores_mempool_transactions` | Runtime correctness | Reorg disconnects mempool-worthy txs | Re-admit valid txs | `node.rs` | Partial | Pass (partial) |
| `reorg_cannot_bypass_coinbase_maturity` | Maturity safety | Reorg around immature spend | Reject | `chainstate.rs`, `validation.rs` | No | Missing |
| `checkpoints_enforced` | Finalization boundary | Fork before finalized checkpoint | Reject | `chainstate.rs` | Partial | Pass (partial) |

## 18. Fuzz and Adversarial Testing

### Fuzzing Plan

| Target | Input Mutations | Must Never Happen | Recommended Harness |
|---|---|---|---|
| Transaction decoder (`Transaction::from_full_bytes`) | Truncation, count inflation, bad lengths, bad tx-PoW tail | Panic, OOM, accept malformed tx | `cargo-fuzz` target in `atho-core` |
| Witness decoder (`TxWitness::from_bytes`) | Signature length, pubkey length, ref counts, additional signer counts | Panic, parse malformed witness as valid | `cargo-fuzz` |
| Block decoder (`Block::from_canonical_bytes`) | Huge tx counts, truncated tx bytes, nested malformed txs | Panic, OOM, accept malformed block | `cargo-fuzz` |
| Compact-size encoder/decoder surfaces | Boundary ints, overflow-style values | Divergent parse between nodes | Property tests |
| Address parser | Wrong prefix, wrong checksum, huge strings | Panic or cross-network accept | `cargo-fuzz` + proptest |
| Tx-PoW preimage / parser | Random witness/txpow combinations | Accept wrong nonce/bits | Property tests |
| UTXO key/value parser | Corrupted LMDB bytes | Panic or silent false-validity | Storage-level fuzz harness |
| P2P block payload decode | Malformed bincode payloads, huge vectors | Panic, memory blowup, wrong network accept | `atho-p2p` fuzz harness |
| Compact block reconstruction | Same txid / different witness or tx-PoW variants | Reconstruct wrong block variant silently | Adversarial unit tests |
| Snapshot bundle deserialize | Corrupt bundles, wrong network, wrong tip | Crash, load false-valid chainstate | `atho-node` fuzz + adversarial tests |

### Adversarial Cases To Add Immediately

- Same header, same txids, different `tx_pow_nonce` / `tx_pow_bits`
- Same header, same txids, different `fees_total_atoms` / `fees_miner_atoms`
- Non-empty malformed coinbase witness bytes
- Truncated full tx bytes that omit tx-PoW fields
- Very large `input_count`, `output_count`, `tx_count`, `additional_group_count`
- Corrupted persisted LMDB UTXO entry surviving restart

## 19. Grade Every Subsystem

| Subsystem | Grade | Mainnet Ready? | Biggest Risk | Required Fix |
|---|---:|---|---|---|
| Block validation | 5/10 | No | Validity depends on uncommitted fields | Commit tx-PoW / stop trusting fee metadata |
| Transaction validation | 7/10 | Not alone | Decoder strictness and tx-PoW under-commitment | Strict full decode + commitment fix |
| UTXO accounting | 5/10 | No | Dirty restart trusts persisted UTXOs too much | Persist UTXO root or rebuild/verify on startup |
| Monetary policy | 8/10 | Mostly | No cap by design; fee metadata coupling | Keep issuance math, decouple fees from block metadata |
| Coinbase validation | 4/10 | No | Missing coinbase witness/tx-PoW strictness | Enforce empty witness and zero tx-PoW |
| Falcon signature validation | 8/10 | Mostly | Depends on surrounding serialization correctness | Keep, add more malformed-block tests |
| Serialization/hashing | 4/10 | No | Block header under-commits consensus data | Redesign witness commitment / strict decode |
| Mempool alignment | 7/10 | Almost | Comments and miner self-validation gap | Self-validate block templates |
| Miner consensus logic | 3/10 | No | Deterministic public reward keys | Use configured private reward destination |
| PoW/difficulty | 6/10 | Not yet | No future timestamp ceiling | Add deterministic future cap |
| Chain selection/reorgs | 7/10 | Close | Restart depends on persisted UTXO trust | Verify/rebuild UTXOs on startup |
| Network constants | 8/10 | Yes | Few issues here | Keep |
| Storage/LMDB safety | 4/10 | No | UTXO integrity not re-proven on dirty restart | UTXO root + fail-closed recovery |
| Legacy bypass risk | 7/10 | Mostly | Permissive full-tx decode / malformed witness commitment collapse | Remove fallback paths |
| API/RPC consensus safety | 7/10 | Mostly | External callers can feed consensus objects; no admin bypass found | Keep canonical checks, add template self-validation |
| Test coverage | 7/10 | Not yet | Missing tests for the actual blockers above | Add tests from Section 17 |

## 20. Mainnet Launch Decision

### Decision: **MAINNET BLOCKED**

### What must be fixed before mainnet

1. Commit tx-PoW bytes into the block commitment path.
2. Remove deterministic/public mining reward keys.
3. Verify or rebuild persisted UTXO state on dirty startup.
4. Add strict coinbase witness / coinbase tx-PoW rules.
5. Make full transaction decoding strict.
6. Bound manual decode allocations.
7. Add future timestamp ceiling.
8. Add the missing tests in Section 17.

### What should be fixed immediately after mainnet if launch were ever forced

- Compact-block ambiguity hardening
- Stronger fuzzing and malformed-input CI
- Better differentiation between relay policy and consensus comments
- Full storage integrity metrics / offline verifier

### What tests must pass before mainnet

- All missing blocker tests listed in Section 17
- Adversarial variants proving there is no alternate-valid raw payload for one header hash
- Dirty restart / corrupted UTXO recovery tests
- Candidate miner template self-validation tests

### Areas that need a second audit

- P2P compact block reconstruction after tx-PoW commitment changes
- Snapshot bootstrap trust model
- Release engineering / reproducible builds
- GPU miner compatibility after tx-PoW commitment changes

### Is an external audit recommended?

**Yes.** The under-commitment issue around tx-PoW and block metadata is exactly the kind of thing that deserves an external second set of eyes before any irreversible launch.

## 21. Required Fix Patches

### Fix 1: Commit all consensus-relevant transaction bytes into the block commitment path

- File: `crates/atho-core/src/transaction.rs`
- Function: `Transaction::witness_commitment_hash`
- Bug:
  - The function commits base bytes and parsed witness commitment bytes only.
  - It omits `tx_pow_nonce` and `tx_pow_bits`.
  - Malformed witness bytes can collapse to the same commitment as “no parsed witness”.
- Why consensus-critical:
  - Block validity depends on tx-PoW.
  - Two payloads with the same header can differ in tx-PoW validity.
- Safe patch:

```rust
pub fn witness_commitment_hash(&self) -> [u8; 48] {
    let mut hasher = Sha3_384::new();
    self.update_base_hasher(&mut hasher);
    hasher.update((self.witness.len() as u32).to_le_bytes());
    hasher.update(&self.witness);
    hasher.update(self.tx_pow_nonce.to_le_bytes());
    hasher.update([self.tx_pow_bits]);
    hasher.finalize().into()
}
```

- Why low-regression:
  - This preserves deterministic hashing while finally binding the same bytes validation depends on.
  - Pre-mainnet, changing block commitments is the right time to do it.
- Test to add:
  - `block_header_commitment_changes_when_txpow_changes`
  - `malformed_nonempty_witness_changes_witness_root`

### Fix 2: Remove fee metadata from consensus validity

- Files:
  - `crates/atho-storage/src/validation.rs`
  - `crates/atho-node/src/mining.rs`
  - `crates/atho-p2p/src/protocol.rs`
- Functions:
  - `validate_block_with_context_and_schedule`
  - `collect_prepared_block_transactions_with_schedule`
- Bug:
  - `fees_total_atoms` / `fees_miner_atoms` affect validity but are not committed by the header and are not in canonical block bytes.
- Why consensus-critical:
  - Same header + same transactions can produce different validity outcomes depending on out-of-band fee metadata.
- Safe patch:
  - Compute fees from transactions inside block validation.
  - Compare coinbase output amount directly to `subsidy + computed_sum_fees`.
  - Stop using `block.fees_*` as validity inputs; keep them as post-validation cached metadata only.

Pseudo-change:

```rust
let computed_fees = sum_fees;
let expected_reward = subsidy::block_subsidy_atoms_for_network(network, height)
    .checked_add(computed_fees)
    .ok_or(ValidationError::CoinbaseRewardMismatch)?;
validate_coinbase_transaction_strict(&block.transactions[0], expected_reward, height, schedule)?;
```

- Why low-regression:
  - It removes ambiguous out-of-band consensus input rather than adding new consensus state.
- Test to add:
  - `block_fee_metadata_cannot_change_validity`

### Fix 3: Replace deterministic public reward keys

- File: `crates/atho-node/src/mining.rs`
- Function: `reward_target_for_height`
- Bug:
  - Reward keypair is derived from public `(network, height)` data.
- Why consensus-critical / launch-critical:
  - Mined rewards are economically stealable by anyone.
- Safe patch:
  - Remove `reward_target_for_height`.
  - Require a configured reward address / payment digest from node config or wallet.
  - On mainnet, refuse template construction without an explicit reward destination.

Pseudo-change:

```rust
let reward_script = node
    .configured_mining_reward_script()
    .ok_or(NodeError::Configuration("missing mining reward address"))?;
```

- Why low-regression:
  - Changes miner output destination only; does not weaken validation.
- Test to add:
  - `mainnet_candidate_block_requires_configured_reward_destination`

### Fix 4: Verify or rebuild UTXO state on dirty startup

- Files:
  - `crates/atho-storage/src/db.rs`
  - `crates/atho-storage/src/chainstate.rs`
- Functions:
  - `run_startup_consistency_checks`
  - `verify_persisted_chainstate_consistency`
  - `load_persisted_chainstate`
- Bug:
  - Startup check proves tip/header consistency, not UTXO consistency.
- Why consensus-critical:
  - Corrupted UTXOs can make a node accept or mine invalid spends after restart.
- Safe patch:
  - Persist a deterministic UTXO root/hash in `ChainstateSnapshot` or metadata.
  - On dirty startup, either:
    1. recompute and compare the UTXO root, or
    2. rebuild the UTXO set from canonical blocks and atomically replace chainstate.
  - Treat missing spent-key deletes as corruption outside explicit recovery/dev mode.

- Why low-regression:
  - Uses canonical blocks as source of truth.
  - Affects startup/recovery, not steady-state consensus math.
- Test to add:
  - `dirty_restart_detects_corrupted_utxo_and_repairs_or_fails_closed`

### Fix 5: Make coinbase witness/tx-PoW strict

- File: `crates/atho-storage/src/validation.rs`
- Function: `validate_coinbase_transaction_with_schedule`
- Bug:
  - Coinbase does not require empty witness or zero tx-PoW fields.
- Why consensus-critical:
  - Coinbase serialization and witness commitment semantics are under-specified.
- Safe patch:

```rust
if !tx.witness.is_empty() || tx.tx_pow_nonce != 0 || tx.tx_pow_bits != 0 {
    return Err(ValidationError::InvalidCoinbase);
}
```

- Why low-regression:
  - Coinbase transactions do not need witness or tx-PoW under current design.
- Test to add:
  - `coinbase_with_nonempty_witness_rejected`
  - `coinbase_with_nonzero_txpow_rejected`

### Fix 6: Make canonical full transaction decoding strict

- File: `crates/atho-core/src/transaction.rs`
- Function: `Transaction::from_full_bytes`
- Bug:
  - Missing tx-PoW fields default to `(0, 0)`.
- Why consensus-critical:
  - Non-canonical payloads can be parsed as canonical objects.
- Safe patch:

```rust
let lock_time = read_u32(bytes, &mut offset)?;
let tx_pow_nonce = read_u64(bytes, &mut offset)?;
let tx_pow_bits = *bytes.get(offset)?;
offset += 1;
if offset != bytes.len() {
    return None;
}
```

- Why low-regression:
  - Aligns decoder with encoder exactly.
- Test to add:
  - `full_tx_missing_txpow_tail_is_rejected`

### Fix 7: Bound manual decoder allocations before allocation

- Files:
  - `crates/atho-core/src/transaction.rs`
  - `crates/atho-core/src/block.rs`
- Functions:
  - `TxWitness::from_bytes`
  - `Transaction::from_full_bytes`
  - `Block::from_canonical_bytes`
- Bug:
  - Counts are trusted before allocation.
- Why consensus-critical:
  - Malformed inputs should never be able to exhaust memory or crash the node.
- Safe patch:
  - Before any `Vec::with_capacity(count)`, derive a maximum feasible count from remaining bytes and fixed minimum element sizes.
  - Reject payloads whose counts exceed what the payload length could possibly encode.

- Why low-regression:
  - Tightens parser rejection only for impossible or abusive inputs.
- Test to add:
  - `block_decoder_rejects_impossible_tx_count_before_allocation`
  - `tx_decoder_rejects_impossible_input_count_before_allocation`

### Fix 8: Add a future timestamp ceiling

- Files:
  - `crates/atho-core/src/constants.rs`
  - `crates/atho-storage/src/validation.rs`
- Functions:
  - `validate_contextual_header_precheck`
- Bug:
  - No upper future timestamp bound exists.
- Why consensus-critical:
  - Time manipulation can destabilize chain progression and difficulty.
- Safe patch:
  - Add a deterministic ceiling such as:

```rust
const MAX_FUTURE_BLOCK_DRIFT_SECONDS: u64 = 7_200;
if let Some(mtp) = pow::median_time_past_from_blocks(previous_blocks) {
    if block.header.timestamp > mtp.saturating_add(MAX_FUTURE_BLOCK_DRIFT_SECONDS) {
        return Err(ValidationError::InvalidBlockTimestamp);
    }
}
```

- Why low-regression:
  - Keeps the rule deterministic across nodes.
- Test to add:
  - `future_dated_block_above_drift_limit_rejected`

## 22. Final Deliverables

### Critical Fixes

1. Commit tx-PoW bytes in the witness commitment path.
2. Remove fee metadata from consensus validity or commit it canonically.
3. Replace deterministic public mining reward keys.
4. Verify or rebuild UTXO state on dirty startup.
5. Enforce empty coinbase witness and zero coinbase tx-PoW.

### High-Priority Fixes

1. Make `Transaction::from_full_bytes()` strict.
2. Bound decoder allocations before allocation.
3. Add future timestamp ceiling.
4. Self-validate miner block templates before exposing them.
5. Harden compact-block reconstruction against txid-only ambiguity.

### Missing Tests

- All tests marked `Missing` in Section 17, especially:
  - tx-PoW commitment / block identity tests
  - fee metadata validity independence
  - coinbase strictness tests
  - dirty-restart corrupted UTXO detection
  - strict decoder tests
  - future timestamp ceiling tests

### Production-Readiness Grade

- **4/10**

### Mainnet Launch Recommendation

- **MAINNET BLOCKED**

### Subsystem Grade Table

- See Section 19.

### Regression Test Plan

- See Section 17.

### Fuzz Test Plan

- See Section 18.

### Summary of All Consensus Risks

The dominant pattern in the current Atho codebase is not “missing basic validation.” It is **under-committed consensus data** and **restart-state trust**:

1. Block validity depends on bytes the header does not commit.
2. Miner rewards are not actually private to the miner.
3. Restarted nodes do not fully re-prove UTXO correctness before trusting persisted state.

Until those are fixed, Atho should not launch a value-bearing mainnet.
