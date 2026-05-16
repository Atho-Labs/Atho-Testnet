# Atho Full Adversarial Security Audit

## Executive Summary

I treated this pass as a hostile review of the current `Atho-Testnet-main` checkout, with the strongest emphasis on consensus validation, block/transaction rejection paths, Falcon signature handling, chainstate safety, API transaction submission, and attack-harness execution.

The short version:

- The core validation surface is materially stronger than an early prototype. Targeted inflation, double-spend, cross-network replay, malformed signature, fee underflow, output overflow, commit-rollback, and persisted cross-network state tests all passed.
- I confirmed and fixed one real hardening gap in the contextual block-validation path: oversized blocks were reaching PoW rejection before hitting the cheaper block-size gate. That did not allow invalid blocks in, but it was the wrong rejection order for abuse resistance.
- I also hardened wallet datafile permissions on Unix, added malformed-Falcon regression coverage, and added canonical raw-transaction parsing coverage for trailing-byte junk.
- A high-priority production blocker remains: non-32-byte locking scripts are not bound to the witness public key, which means those legacy/test-form outputs are effectively anyone-can-spend once created.

## Final Verdict

**Safe for extended testnet only.**

I would not call this checkout production-ready or mainnet-ready yet. The standard digest-bound spend path looks much healthier than the average alpha chain, but the legacy locking-script ownership model and incomplete adversarial coverage on P2P/API deployment scenarios still leave too much room for production misuse.

## Overall Security Score

**7/10**

Why not higher:

- one confirmed high-severity ownership-model weakness remains in consensus-adjacent spend semantics for legacy/nonstandard outputs
- no full hostile P2P flood harness was executed in this pass
- no full fuzz run or long soak test was executed in this pass
- no production-grade auth story exists for any future non-local privileged API surfaces

Why not lower:

- targeted bad blocks and bad transactions were rejected consistently
- cross-network replay resistance is present and tested
- overflow/underflow accounting checks are present and tested
- chainstate rollback and reload/reorg safety have meaningful coverage
- the standalone adversarial harness passed after the validation-order hardening

## Layer-by-Layer Scores

- Consensus security: **7/10**
- Monetary policy security: **8/10**
- Transaction validation security: **7/10**
- Block validation security: **7/10**
- UTXO accounting security: **7/10**
- Falcon implementation security: **7/10**
- Hashing/domain separation security: **8/10**
- Serialization security: **7/10**
- Mempool security: **7/10**
- Mining security: **7/10**
- Database/storage security: **8/10**
- P2P/network security: **6/10**
- Sync security: **6/10**
- API/RPC security: **6/10**
- Wallet/key management security: **6/10**
- Config/network isolation security: **7/10**
- Logging/secrets security: **7/10**
- Test coverage: **7/10**
- Production readiness: **6/10**

## Critical Findings

No confirmed critical inflation, chain-split, or invalid-block-acceptance bug was reproduced in the executed test set.

## High Findings

### 1. Legacy/nonstandard locking scripts are not pubkey-bound

- **What was found:** `crates/atho-storage/src/validation.rs` only binds ownership to the witness public key when `locking_script.len() == ADDRESS_DIGEST_BYTES`. For every other locking-script length, `locking_script_matches_public_key(...)` returns `true`.
- **Why it matters:** any non-32-byte output is effectively protected only by script-byte equality, not by address/pubkey ownership. If such an output exists, anyone who knows the script bytes can sign with an unrelated Falcon key and spend it.
- **How it can be attacked:** create or receive a nonstandard output (including legacy/test-form outputs), then spend it with any Falcon keypair while reusing the same short script bytes.
- **Where it appears:** `crates/atho-storage/src/validation.rs`, function `locking_script_matches_public_key(...)`. The dev/test paths in `crates/atho-node/src/dev.rs` and `crates/atho-node/src/bin/atho-attack.rs` still use 1-byte/4-byte script forms, which is strong evidence that the insecure path remains live.
- **How to fix it:** before mainnet, either:
  1. enforce 32-byte digest locking scripts for all standard and mined outputs, or
  2. introduce an explicitly versioned script system with real ownership semantics and activation rules.
- **How to test the fix:** add a negative regression proving that an unrelated Falcon key cannot spend any accepted output type.
- **Consensus impact:** **yes**, this is consensus-affecting if changed for existing accepted transaction forms.
- **Severity:** **High**

## Medium Findings

### 1. Contextual block validation was checking PoW before cheap size rejection

- **What was found:** the contextual path performed `ProofOfWorkInvalid` rejection before block-size rejection for oversized blocks.
- **Why it matters:** it increases useless work on obviously invalid data and weakens the rejection order against oversized-block abuse.
- **Fix applied:** added an early shared size check in `crates/atho-storage/src/validation.rs`, plus a regression test for the contextual path.
- **How to test it:** `cargo test -p atho-storage oversized_block_is_rejected_before_contextual_pow_checks -- --nocapture`
- **Consensus impact:** no accept/reject set change intended; this is rejection-order hardening only.
- **Severity:** **Medium**

### 2. No production-grade auth model for future privileged RPC/API surfaces

- **What was found:** `crates/atho-rpc/src/command.rs` exposes an `auth_required` flag in command metadata, but command definitions are still marked `auth_required: false`. Current HTTP defaults are safer because bind is local and wallet/admin/mining flags default off, but this is still operator-discipline security.
- **Why it matters:** if a future deployment enables non-local write/admin/mining surfaces without an auth layer, the risk becomes operationally severe.
- **How to fix it:** add explicit auth for privileged commands and enforce it at the transport boundary, not just in metadata.
- **How to test it:** remote unauthorized requests must fail for admin/mining/wallet-signing commands; public read-only endpoints must remain accessible where intended.
- **Consensus impact:** no.
- **Severity:** **Medium**

### 3. P2P hostile-wire and flood coverage is still incomplete in this pass

- **What was found:** this pass did not execute a dedicated malformed-wire-message flood harness across live peer sockets.
- **Why it matters:** DoS and state-machine bugs often hide in framing, backpressure, reconnect, and partial-message logic rather than pure consensus code.
- **How to fix it:** build a repeatable P2P adversarial harness around malformed frames, oversized frames, duplicate inventories, stale headers, and slowloris behavior.
- **Consensus impact:** no.
- **Severity:** **Medium**

### 4. Full fuzz execution was not completed in this pass

- **What was found:** the repo already contains useful fuzz targets, but I did not run a full fuzz corpus or extended fuzz campaign in this turn.
- **Why it matters:** parser and serialization issues often require volume and mutation depth to surface.
- **How to fix it:** wire the existing fuzz targets into CI/nightly and keep crashing inputs as regression fixtures.
- **Consensus impact:** potentially yes if a parser divergence is found.
- **Severity:** **Medium**

## Low Findings

### 1. Attack harness used the wrong block-size constant

- **What was found:** `crates/atho-node/src/bin/atho-attack.rs` used `MAX_BLOCK_SIZE_BYTES` for the oversized-block case, which is the vsize alias, not the raw-byte threshold the validator uses.
- **Fix applied:** switched that adversarial case to `MAX_BLOCK_RAW_BYTES + 1`.
- **Consensus impact:** no.
- **Severity:** **Low**

### 2. API module documentation had drifted from the implementation

- **What was found:** the module comment still described the API as read-only even though optional transaction broadcast exists when wallet support is enabled.
- **Fix applied:** refreshed the module comment.
- **Consensus impact:** no.
- **Severity:** **Low**

## Informational Findings

- Wallet datafiles now explicitly tighten Unix permissions to `0600` during atomic save and after rename.
- Falcon malformed-input regressions now explicitly cover truncated/oversized signatures and malformed/truncated public keys.
- Raw-transaction parsing now has a dedicated regression for noncanonical trailing bytes.
- Existing chainstate tests already do useful work around rollback, cross-network fail-closed behavior, and reload/reorg persistence.

## Consensus Security Review

### Block validation

Targeted block attacks rejected correctly after the contextual size-order hardening:

- inflation block
- wrong parent
- wrong network
- invalid height
- oversized block
- duplicate-coinbase style failure
- timestamp warp
- duplicate-input/double-spend block

### Transaction validation

Targeted transaction attacks rejected correctly in the executed suite:

- bad Falcon signature
- fee below policy minimum
- witness reference mismatch
- immature coinbase spend
- dust-like output
- duplicate input
- zero-value output
- oversized transaction
- cross-network replay
- wrong public key for standard outputs
- output overflow

### UTXO rules

The standard digest-bound spend path looks correct in the tested cases, but ownership semantics for non-digest outputs remain a high-priority gap.

### Monetary policy

The executed tests and attack harness did not find a supply-inflation path. Inflationary coinbase variants were rejected.

### Coinbase rules

Inflated reward cases were rejected; coinbase-only block paths and maturity checks behaved as expected in the executed set.

### Difficulty / Proof-of-Work

PoW rejection worked for invalid contextual submissions. One validation-order issue was fixed so oversized blocks now fail the cheap size gate before contextual PoW checking.

### Reorgs

Persistence/reorg tests passed, including reloaded reorg without database wipe.

### Network separation

Cross-network signature replay and persisted cross-network UTXO contamination both failed closed in the executed suite.

## Monetary Policy and Accounting Review

### What was tested

- inflated coinbase reward rejection
- fee floor rejection
- output total overflow rejection
- higher-fee block acceptance
- commit rollback safety

### Result

I did **not** reproduce a money-creation bug in the targeted set. The accounting path appears materially safer than average for an alpha/testnet chain, especially because overflow and fee mismatch cases already have direct tests.

## Transaction Attack Results

Executed results:

- `valid_tx_accepts`: pass
- `bad_signature_rejects`: pass
- `bad_fee_rejects`: pass
- `witness_ref_rejects`: pass
- `immature_coinbase_spend_rejects`: pass
- `dust_like_spend_rejects`: pass
- `duplicate_input_rejects`: pass
- `zero_output_rejects`: pass
- `oversized_tx_rejects`: pass
- `cross_network_signature_replay_is_rejected_even_with_valid_local_pow`: pass
- `output_total_overflow_is_rejected_during_context_validation`: pass
- `wrong_public_key_for_standard_output_is_rejected`: pass

## Block Attack Results

Executed results:

- `valid_block_accepts`: pass
- `inflation_block_rejects`: pass
- `second_coinbase_rejects`: pass
- `wrong_parent_rejects`: pass
- `wrong_network_rejects`: pass
- `invalid_height_rejects`: pass
- `oversized_block_rejects`: pass after contextual size-order hardening
- `double_spend_block_rejects`: pass
- `timestamp_warp_rejects`: pass

## Falcon Signature Audit

### Findings

- Domain separation is present and already tested across signature domains.
- Malformed length checks are present.
- Verification returns `Ok(false)` on malformed key/signature inputs rather than panicking.
- Existing tests already cover wrong public key, wrong message, concurrency, and fixed signature size.

### Fixes / added coverage

- Added `falcon_verify_rejects_malformed_inputs_without_panicking` in `crates/atho-crypto/src/falcon.rs`.

### Remaining concerns

- I did not perform side-channel measurement or constant-time microanalysis in this pass.
- Wallet signing should remain isolated from any public remote request surface.

## Hashing and Serialization Audit

### What looks good

- network-aware transaction signing digests are already tested across mainnet/testnet
- canonical raw-transaction parsing rejects malformed full-byte layouts
- cross-network signature replay rejection is present and exercised

### Added regression

- `parse_raw_transaction_hex_rejects_trailing_bytes_noncanonical_encoding` in `crates/atho-node/src/service.rs`

### Remaining work

- keep fuzzing transaction/block/message decoders in CI
- extend canonical/noncanonical mutation tests beyond the currently targeted raw-transaction path

## UTXO and Database Audit

### What passed

- commit fault injection rolls back chainstate mutation
- persisted cross-network UTXOs fail closed
- reloaded chainstate can reorg without wiping the database

### Result

The storage layer looks meaningfully more robust than the average hobby-chain alpha. I did not reproduce partial-commit state corruption in the targeted suite.

## Mempool Attack Review

The executed set hit duplicate inputs, fee floor, dust-like outputs, invalid signatures, witness mismatches, and oversize rejection through mempool admission and transaction submission. I did **not** run a sustained spam/flood benchmark in this pass, so memory-growth and eviction behavior under attack still need dedicated load testing.

## Mining Attack Review

The adversarial harness verified:

- valid block acceptance
- inflation block rejection
- wrong-parent rejection
- wrong-height rejection
- oversized block rejection
- timestamp warp rejection

I did not run a long-lived stale-template/stale-tip miner soak in this pass.

## Difficulty / PoW Attack Review

Target and PoW rejection worked in the executed cases. The only meaningful issue I fixed here was rejection order: oversize should fail cheaply before contextual PoW checks.

## Reorg / Fork Attack Review

Executed results:

- rollback on commit failure: pass
- persisted reorg without wipe: pass

I did not run a dedicated multi-peer competing-branch live network harness in this pass.

## P2P Network Attack Review

### What I can say with confidence

- the repo has peer-health, retry, and sync machinery that is more mature than a toy implementation
- network separation errors are already covered in several places

### What I did not fully prove in this pass

- malformed live wire-message flood resistance
- full eclipse-address-poisoning resistance
- slowloris/backpressure behavior under socket abuse

This layer is one of the biggest remaining reasons I do not recommend production yet.

## Sync Attack Review

Sync-adjacent storage and reorg safety looked good in the executed chainstate tests, but I did not execute a full hostile multi-peer sync harness from zero in this turn.

## API/RPC Penetration Test

### Good findings

- local bind by default
- wallet/admin/mining write surfaces disabled by default
- CORS allowlist and rate limiting exist
- malformed/noncanonical raw transaction submission is rejected
- broadcast route accepted a valid signed raw transaction in the enabled regnet test path

### Concerns

- command metadata still does not constitute a real auth model for any future exposed privileged surface
- if operators expose write/admin/mining features beyond loopback without another protection layer, risk increases sharply

## Wallet and Key Management Audit

### Good findings

- wallet datafile encryption exists
- wallet error surfaces are sanitized
- wallet datafile permissions are now explicitly owner-only on Unix

### Fix applied

- owner-only permission hardening in `crates/atho-wallet/src/wallet/datafile.rs`

### Remaining concerns

- I did not execute mnemonic corruption/recovery adversarial cases in this turn
- the broader legacy locking-script issue is consensus-side, but any wallet path that ever emits nonstandard short locking scripts would inherit that risk

## Config and Network Isolation Audit

Defaults are conservative enough for testnet work:

- API bind: `127.0.0.1`
- wallet/admin/mining API flags: off by default

Cross-network persisted UTXO contamination was explicitly tested and failed closed.

## Logging and Secret Exposure Review

I did not find a confirmed private-key or mnemonic leak in the executed paths. Wallet-datafile errors remain sanitized. I did not do a full log-grep over long-running production sessions in this turn.

## Performance DoS Review

### Confirmed improvement

- oversized contextual blocks now fail the cheap size gate before contextual PoW checks

### Remaining gaps

- no sustained invalid-signature flood measurement
- no sustained malformed-P2P flood measurement
- no API parallel saturation benchmark

## Fuzzing Results

No long fuzz run was executed in this pass.

Existing fuzz targets already present in the repo:

- `address_decode.rs`
- `block_decode.rs`
- `block_template_decode.rs`
- `block_validate.rs`
- `compact_block_reconstruct.rs`
- `mempool_admission.rs`
- `network_message_decode.rs`
- `p2p_frame_decode.rs`
- `p2p_message_roundtrip.rs`
- `rpc_request_decode.rs`
- `sighash.rs`
- `tx_decode.rs`
- `tx_roundtrip.rs`
- `tx_witness_parse.rs`

## Test Suite Created

I did **not** build the full `tests/adversarial/` tree requested in the ideal target layout during this pass.

I did add targeted adversarial regressions inline:

- `crates/atho-crypto/src/falcon.rs`
  - `falcon_verify_rejects_malformed_inputs_without_panicking`
- `crates/atho-node/src/service.rs`
  - `parse_raw_transaction_hex_rejects_trailing_bytes_noncanonical_encoding`
- `crates/atho-storage/src/validation.rs`
  - `oversized_block_is_rejected_before_contextual_pow_checks`
- `crates/atho-wallet/src/wallet/datafile.rs`
  - `wallet_datafile_permissions_are_owner_only`

## Test Results

Executed and passed:

- `cargo test -p atho-wallet wallet_datafile_permissions_are_owner_only -- --nocapture`
- `cargo test -p atho-crypto falcon_verify_rejects_malformed_inputs_without_panicking -- --nocapture`
- `cargo test -p atho-node parse_raw_transaction_hex_rejects_trailing_bytes_noncanonical_encoding -- --nocapture`
- `cargo test -p atho-node sendrawtransaction_rejects_noncanonical_raw_transaction_bytes -- --nocapture`
- `cargo test -p atho-node transaction_broadcast_route_accepts_signed_raw_transaction_when_enabled -- --nocapture`
- `cargo test -p atho-storage cross_network_signature_replay_is_rejected_even_with_valid_local_pow -- --nocapture`
- `cargo test -p atho-storage output_total_overflow_is_rejected_during_context_validation -- --nocapture`
- `cargo test -p atho-storage wrong_public_key_for_standard_output_is_rejected -- --nocapture`
- `cargo test -p atho-storage commit_fault_injection_rolls_back_chainstate_mutation -- --nocapture`
- `cargo test -p atho-storage persisted_cross_network_utxos_fail_closed -- --nocapture`
- `cargo test -p atho-storage reloaded_chainstate_reorgs_without_wiping_database -- --nocapture`
- `cargo test -p atho-storage oversized_block_is_rejected_before_contextual_pow_checks -- --nocapture`
- `cargo run -p atho-node --bin atho-attack -- --network regnet`
- `cargo run -p atho-node --bin atho-attack -- --network testnet`
- `cargo fmt --check`
- `cargo check --workspace`

## 100% Pass Rate Status

**No for the full requested adversarial matrix.**

I did **not** execute every category in the ideal full-system negative matrix from the prompt.

**Yes for the targeted executed suite in this pass.**

- the standalone `atho-attack` harness finished **19/19** on both `regnet` and `testnet`
- all added targeted regressions passed after fixes

## Fixes Applied

### 1. Wallet datafile permission hardening

- **Issue:** wallet files relied too much on ambient filesystem defaults
- **File/function:** `crates/atho-wallet/src/wallet/datafile.rs`, `atomic_write(...)`
- **Fix:** force owner-only Unix permissions during temp-file write and after rename
- **Test added:** `wallet_datafile_permissions_are_owner_only`
- **Consensus affected:** no

### 2. Contextual block validation now rejects oversize before PoW

- **Issue:** contextual path performed PoW rejection before cheap block-size rejection
- **File/function:** `crates/atho-storage/src/validation.rs`
- **Fix:** added shared `validate_block_size_metrics(...)` and invoked it before contextual PoW checks
- **Test added:** `oversized_block_is_rejected_before_contextual_pow_checks`
- **Consensus affected:** no intended accept/reject set change; rejection ordering only

### 3. Attack harness oversized-block boundary corrected

- **Issue:** adversarial runner used the wrong constant for raw-size overflow
- **File/function:** `crates/atho-node/src/bin/atho-attack.rs`
- **Fix:** switched oversize case from `MAX_BLOCK_SIZE_BYTES` to `MAX_BLOCK_RAW_BYTES`
- **Test added:** existing harness path now passes
- **Consensus affected:** no

### 4. Falcon malformed-input regression coverage

- **Issue:** malformed-input handling was good by inspection but under-specified in tests
- **File/function:** `crates/atho-crypto/src/falcon.rs`
- **Fix:** added malformed signature/public-key verification coverage
- **Test added:** `falcon_verify_rejects_malformed_inputs_without_panicking`
- **Consensus affected:** no

### 5. Canonical raw-transaction trailing-byte regression

- **Issue:** noncanonical raw transaction padding needed explicit regression coverage
- **File/function:** `crates/atho-node/src/service.rs`
- **Fix:** added trailing-byte rejection coverage
- **Test added:** `parse_raw_transaction_hex_rejects_trailing_bytes_noncanonical_encoding`
- **Consensus affected:** no

## Fixes Still Needed

- remove or formally version the legacy/nonstandard locking-script ownership bypass
- build and run a dedicated hostile P2P/message flood harness
- run/automate the existing fuzz targets
- design and enforce a real auth model before exposing any privileged RPC/API surface off-loopback
- execute longer soak tests for sync, reorg, miner stale-template handling, and API under sustained load

## Production Blockers

- legacy/nonstandard locking-script ownership bypass
- incomplete hostile P2P/API/fuzz coverage
- no demonstrated production-grade authentication boundary for future privileged RPC/API use
- no completed long-duration soak evidence in this pass

## Recommended Improvements

- **Consensus:** require digest-bound outputs or activate a real script-versioning model before mainnet
- **Falcon:** keep adding malformed-input regressions; run dedicated fuzz on key/signature decoding
- **Networking:** build a malformed-frame and message-flood harness
- **Database:** keep expanding restart/reorg/partial-commit regression coverage
- **Testing:** promote attack-harness scenarios into a first-class CI job
- **Performance:** benchmark invalid-signature flood, malformed-message flood, and API saturation

## Mainnet Readiness Checklist

- [ ] All consensus tests pass
- [x] All targeted adversarial tx tests executed in this pass pass
- [x] All targeted adversarial block tests executed in this pass pass
- [x] All targeted monetary policy tests executed in this pass pass
- [x] All targeted Falcon signature tests executed in this pass pass
- [x] All targeted serialization tests executed in this pass pass
- [x] All targeted UTXO accounting tests executed in this pass pass
- [ ] All mempool attack tests from the full requested matrix pass
- [ ] All P2P attack tests from the full requested matrix pass
- [ ] All API penetration tests from the full requested matrix pass
- [ ] All wallet security tests from the full requested matrix pass
- [x] Targeted reorg tests executed in this pass pass
- [x] Targeted database crash-safety tests executed in this pass pass
- [ ] All fuzz targets pass baseline runs
- [ ] No known critical issues
- [ ] No known high production blockers
- [x] Mainnet/testnet/regtest isolation has meaningful targeted coverage
- [x] No private key or seed leakage was confirmed in the executed paths
- [ ] Sync from zero tested under hostile peers
- [ ] Node restart soak-tested
- [x] Miner submit-block path exercised through the attack harness
- [ ] Production config fully reviewed for public deployment
- [ ] Logs reviewed under attack load
- [ ] Performance DoS fully reviewed

## Final Recommendation

**Not safe for production yet.**

**Safe for extended testnet only** is the honest recommendation.

The executed adversarial set is encouraging and the core path rejected the major bad-transaction and bad-block cases I targeted. But the remaining legacy locking-script ownership weakness and the still-incomplete hostile-network/deployment coverage are enough that I would not sign off on a mainnet launch from this checkout.
