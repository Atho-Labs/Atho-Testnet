# Atho Falcon-512 Security Audit

## Executive Summary

This pass audited Atho's Falcon-512 implementation across key generation, deterministic wallet derivation, signing, verification, witness serialization, address binding, wallet persistence, and regression coverage.

The Falcon core is in decent shape for alpha/testnet use:

- signing and verification are domain-separated
- transaction signing digests are network-scoped
- malformed public keys and malformed signatures fail closed
- wallet-side secret wrappers zeroize on drop
- no public node API endpoint was found that exposes arbitrary Falcon signing

This pass also applied concrete hardening:

- secret-bearing `Debug` output is now redacted in Falcon and wallet seed/mnemonic wrappers
- Falcon byte constructors now perform strict length and decode validation for public and secret keys
- wallet mnemonic-root seed and path-derivation buffers now use `Zeroizing`
- regression coverage was expanded for malformed input handling, deterministic keygen, replay boundaries, and secret redaction

The main blocker is not raw Falcon verification itself. It is the broader address-binding rule in [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:447): only 32-byte standard locking scripts are bound to the witness public key. Nonstandard or legacy locking scripts remain effectively anyone-can-spend once they exist on chain.

## Final Verdict

**Safe for testnet only.**

The Falcon implementation is strong enough for continued alpha/testnet operation after this hardening pass, but it is **not production-ready** while the legacy/nonstandard locking-script ownership rule remains and while plaintext wallet persistence is still allowed.

## Falcon Security Score

**7/10**

Why:

- strong domain separation and network-scoped signing digest
- strict malformed public-key and malformed signature rejection
- no public API signing oracle found
- better secret redaction and shorter secret lifetime after this pass
- still blocked by a consensus-adjacent ownership gap for nonstandard outputs
- still missing dedicated fuzz targets and a stronger wallet-at-rest policy

## Critical Findings

### 1. Nonstandard locking scripts are not bound to the Falcon public key

- **What was found:** [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:447) only compares the witness public key to the output locking script when the locking script is exactly `ADDRESS_DIGEST_BYTES` long.
- **Why it matters:** a standard 32-byte payment-digest output is correctly bound to its Falcon key, but any legacy or nonstandard output form is not.
- **How it can be attacked:** if a nonstandard locking script exists on chain, an attacker can provide their own Falcon key and valid signature for the transaction and still pass ownership checks.
- **Impact:** potential unauthorized spend of nonstandard outputs.
- **How to fix it:** replace the permissive `else { true }` path with an explicit supported-script policy or a canonical legacy-script verifier.
- **How to test it:** add negative tests for nonstandard script outputs spent by the wrong key and positive tests only for explicitly supported script types.
- **Severity:** Critical
- **Consensus impact:** **Consensus-breaking** if changed after network deployment. Likely requires a testnet reset or migration plan if nonstandard outputs already exist.

## High Findings

### 1. Secret-bearing Falcon and wallet types exposed raw `Debug` output before this pass

- **What was found:** `SecretBytes`, `FalconSecretKey`, `FalconKeypair`, `WalletSeed`, and `MnemonicPhrase` previously derived ordinary `Debug`.
- **Why it matters:** accidental debug logging, panic context, or test failure output could expose raw secret material.
- **How it can be attacked:** operational or support logs could leak signing secrets or recovery material.
- **Fix applied:** redacted `Debug` implementations in:
  - [secret.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/secret.rs:13)
  - [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs:42)
  - [hd.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/hd.rs:16)
  - [mnemonic/mod.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/mnemonic/mod.rs:73)
- **How to test it:** format the affected types with `{:?}` and assert the rendered string contains `<redacted>` and not raw material.
- **Severity:** High
- **Consensus impact:** No consensus impact

### 2. Wallet datafiles still allow plaintext persistence when password is empty

- **What was found:** [datafile.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet/datafile.rs:222) intentionally stores wallet state in plaintext when the password is empty.
- **Why it matters:** wallet state includes deterministic Falcon seed material and optional mnemonic phrase.
- **How it can be attacked:** disk compromise, backup leakage, or local malware can recover keys without brute force.
- **How to fix it:** require explicit opt-in for plaintext mode or reject empty-password saves in production builds.
- **How to test it:** assert default save paths reject empty passwords or require a dedicated flag.
- **Severity:** High
- **Consensus impact:** No consensus impact

## Medium Findings

### 1. Falcon fuzz targets are still missing

- **What was found:** no dedicated `cargo-fuzz` or equivalent fuzz harnesses were present for Falcon key parsing, signature handling, or transaction witness parsing.
- **Why it matters:** malformed-input bugs are more likely to survive unit-test-only coverage.
- **How to fix it:** add fuzz targets for `FalconPublicKey::from_bytes`, `FalconSecretKey::from_bytes`, `TxWitness::from_bytes`, and transaction verification entry points.
- **How to test it:** run baseline fuzz campaigns and ensure no panics or unexpected accepts.
- **Severity:** Medium
- **Consensus impact:** No consensus impact

### 2. Intermediate mnemonic and derivation buffers needed shorter secret lifetimes

- **What was found:** wallet mnemonic root-seed material and path-derivation seed buffers were previously ordinary stack/heap values.
- **Fix applied:** [wallet.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet.rs:157) and [wallet.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet.rs:310) now use `Zeroizing`.
- **Why it matters:** these buffers contain Falcon seed derivation material.
- **How to test it:** existing deterministic wallet-address and passphrase-differentiation tests must still pass.
- **Severity:** Medium
- **Consensus impact:** No consensus impact

### 3. External Falcon backend constant-time behavior was not independently re-audited here

- **What was found:** Atho relies on the vendored `fn-dsa` Falcon implementation for signing and verification behavior.
- **Why it matters:** side-channel guarantees depend partly on upstream implementation quality.
- **How to fix it:** review upstream side-channel documentation and optionally isolate signing into a narrower local wallet process.
- **How to test it:** code review, upstream audit review, and performance/timing variance measurements.
- **Severity:** Medium
- **Consensus impact:** No consensus impact

## Low Findings

- Falcon signature parsing is length-strict but still relies on full verification for semantic acceptance. That is acceptable, but it should stay documented.
- Benchmark coverage exists for keygen/sign/verify but not yet for malformed-input throughput, full block signature verification, or parallel scaling under transaction bundles.

## Informational Findings

- No Falcon verification cache is present today. That means no current cache-poisoning exposure, but also no cache-based speedup.
- No public node API route was found that calls `falcon::sign` or `Wallet::build_signed_payment_transaction`; current signing remains wallet-local.
- Atho already includes good domain separation labels in [signatures.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-core/src/consensus/signatures.rs:7).

## Falcon Usage Map

### Core Falcon implementation

- [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs)
  - `generate_from_seed`, `generate`, `sign`, `verify`
  - Consensus-critical for transaction verification
  - Handles private material: yes
  - Handles external input: yes

- [secret.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/secret.rs)
  - `SecretBytes`
  - Private material container

### Signing-message construction

- [signatures.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-core/src/consensus/signatures.rs)
  - `transaction_signing_digest`
  - `transaction_signing_digest_for_input_indexes`
  - Consensus-critical: yes

- [transaction.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-core/src/transaction.rs)
  - `TxWitness::from_bytes`
  - `Transaction::signing_digest`
  - `Transaction::signing_digest_for_input_indexes`
  - Consensus-critical: yes

### Validation and address binding

- [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs)
  - `verify_transaction_signature`
  - `verify_transaction_signature_prepared`
  - `prepare_transaction_validation`
  - `locking_script_matches_public_key`
  - Consensus-critical: yes

- [address.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-core/src/address.rs)
  - `public_key_digest`
  - `address_parts_from_public_key`
  - Address/public-key binding logic

### Wallet and persistence

- [wallet.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet.rs)
  - `keypair_for_path`
  - `build_signed_payment_transaction_with_progress`
  - Private material: yes

- [datafile.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet/datafile.rs)
  - encrypted wallet save/load
  - Private material: yes

- [hd.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/hd.rs)
  - `WalletSeed`
  - Private material: yes

- [mnemonic/mod.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/mnemonic/mod.rs)
  - `MnemonicPhrase`
  - `root_seed`
  - Private material: yes

### Tests and benchmarks

- [falcon_hot_paths.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/benches/falcon_hot_paths.rs)
- storage validation tests in [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs)
- wallet tests in [wallet.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet.rs)

## Key Generation Review

- Falcon parameters are frozen to Falcon-512 constants in [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs:22).
- `generate()` uses `getrandom` for a 48-byte seed.
- `generate_from_seed()` rejects empty seeds and deterministically derives a 48-byte internal seed using `sha3_384` when needed.
- Wallet path-derived Falcon keys include:
  - wallet seed
  - network domain tag
  - account
  - receive/change role
  - path index
- Deterministic wallet derivation is therefore separated across networks and address roles.
- No weak fixed seed path was found in production wallet code.

## Private Key Handling Review

- Secret keys live in `SecretBytes`, which zeroizes on drop.
- This pass redacted debug rendering for secret-bearing Falcon and wallet seed types.
- Wallet mnemonic-root seed and key-derivation buffers now use `Zeroizing`.
- No public node API route was found that exposes arbitrary Falcon signing.
- Wallet datafiles use AES-256-GCM plus PBKDF2-HMAC-SHA256 when a password is provided.
- Remaining issue: empty-password datafiles still persist Falcon root material in plaintext.

## Public Key Handling Review

- Witness parsing requires exact Falcon public-key length in [transaction.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-core/src/transaction.rs:197).
- This pass added `FalconPublicKey::from_bytes()` in [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs:36), which performs:
  - strict length validation
  - decode validation via `VerifyingKey512::decode`
- Empty, truncated, oversized, and malformed public keys are rejected safely.

## Signature Format Review

- Atho currently treats Falcon signatures as fixed-size `666`-byte values.
- Witness parsing rejects any signature whose byte length is not exact.
- `verify()` returns `Ok(false)` for malformed or wrong-length signatures and never accepts on parse failure.
- There is no standalone semantic signature parser beyond size validation. Final semantic acceptance still depends on full verification against message, public key, and domain.

## Signing Message Review

- Transaction signatures are bound to:
  - `ATHO_TX_SIGN_V1`
  - `network.consensus_id()`
  - `genesis_hash(network)`
  - canonical transaction signing digest
- Grouped signer flows additionally bind the covered input index set through `transaction_signing_digest_for_input_indexes`.
- This prevents replay across:
  - networks
  - whole-transaction vs grouped-input contexts
  - different covered input sets

## Verification Review

- Verification uses the same domain-separated digest family as signing.
- Structural and witness-shape checks occur before Falcon verification in transaction validation.
- Verification fails closed on:
  - wrong public key
  - wrong message
  - wrong network digest
  - malformed public key
  - malformed signature
  - parser failure
- Final block acceptance still performs full Falcon verification.

## Address/Public Key Binding Review

- Standard 32-byte payment-digest outputs are correctly bound through `public_key_digest(network, public_key)`.
- Nonstandard outputs are not.
- Result: standard wallet outputs are protected correctly; legacy or arbitrary locking scripts are not.

## Signature Cache Review

- No Falcon verification cache exists today.
- Security consequence: no current cache-poisoning surface.
- Performance consequence: no cache acceleration for repeated identical verifications.
- If a future cache is added, the cache key must include at least:
  - network context
  - public key bytes
  - signing message or signing-message hash
  - signature bytes

## Timing and Side-Channel Review

- Private signing stays wallet-local in the current codebase.
- This pass removed easy secret leakage through `Debug`.
- Secret wrappers and some temporary seed buffers now zeroize.
- Remaining side-channel assumption: the vendored `fn-dsa` backend is treated as the cryptographic constant-time authority and was not independently re-audited here.

## Fuzzing and Malformed Input Review

Added malformed-input regression coverage for:

- malformed Falcon public keys
- malformed Falcon secret keys
- empty Falcon signatures
- empty Falcon public keys
- truncated and oversized Falcon material
- wrong-network and wrong-input-set replay attempts

Still missing:

- dedicated fuzz targets for Falcon key parsing
- witness parser fuzzing
- transaction verification fuzzing around grouped signer sets

## Performance Review

Safe observations:

- Falcon verification is already relatively fast compared with keygen and signing.
- Validation stages reject malformed witness length before expensive crypto.
- Parallel verification already exists higher up in block validation.

Measured snapshot from this pass:

- keygen: `9.4490 ms .. 9.7379 ms`
- sign: `509.29 µs .. 550.63 µs`
- verify: `61.602 µs .. 64.348 µs`

## Wallet and Key Storage Review

- deterministic Falcon derivation is network-scoped
- encrypted wallet files use AES-256-GCM
- wallet files are restricted to owner-only permissions on Unix
- plaintext save mode remains a real at-rest risk
- no public API signing oracle was found

## Consensus Impact Review

### Changes applied in this pass

- Falcon parser helpers and secret redaction: **No consensus impact**
- wallet temporary-buffer zeroization: **No consensus impact**
- new tests and benchmark snapshots: **No consensus impact**

### Changes not applied, but required later

- fixing nonstandard locking-script ownership: **Consensus-breaking / policy-changing**
- changing signing message format, public-key format, or address binding format: **Consensus-breaking**

## Fixes Applied

### 1. Redacted secret-bearing debug output

- **Issue:** ordinary `Debug` could expose Falcon keys, wallet seeds, and mnemonics
- **File/function:** [secret.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/secret.rs:13), [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs:42), [hd.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/hd.rs:16), [mnemonic/mod.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/mnemonic/mod.rs:73)
- **Fix:** custom redacted `Debug` implementations
- **Test added:** debug-redaction unit tests for secret bytes, Falcon key material, wallet seed, mnemonic phrase, and wallet container
- **Severity:** High
- **Consensus impact:** No consensus impact

### 2. Added strict Falcon public/secret key constructors

- **Issue:** strict decode validation was not centralized for raw Falcon byte material
- **File/function:** [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs:36), [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs:51)
- **Fix:** `FalconPublicKey::from_bytes()` and `FalconSecretKey::from_bytes()` now enforce exact length plus decode success
- **Test added:** malformed constructor-validation regression
- **Severity:** Medium
- **Consensus impact:** No consensus impact

### 3. Tightened malformed Falcon verification coverage

- **Issue:** empty-value and replay-boundary cases were under-covered
- **File/function:** [falcon.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-crypto/src/falcon.rs)
- **Fix:** added empty key/signature rejection tests plus network/input-set replay tests
- **Test added:** `falcon_verify_rejects_malformed_inputs_without_panicking`, `falcon_signatures_are_bound_to_network_and_covered_input_set`
- **Severity:** Medium
- **Consensus impact:** No consensus impact

### 4. Reduced lifetime of wallet Falcon seed material

- **Issue:** mnemonic-root seed and derivation seed buffers were not explicitly zeroized
- **File/function:** [wallet.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet.rs:157), [wallet.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet.rs:310)
- **Fix:** wrapped temporary secret buffers with `Zeroizing`
- **Test added:** existing deterministic wallet restore and passphrase-differentiation tests rerun after the change
- **Severity:** Medium
- **Consensus impact:** No consensus impact

## Tests Added

- `secret::tests::secret_bytes_debug_is_redacted`
- `falcon::tests::falcon_deterministic_keygen_is_stable_and_empty_seed_is_rejected`
- `falcon::tests::falcon_constructor_validation_rejects_wrong_lengths_and_malformed_keys`
- `falcon::tests::falcon_debug_output_redacts_secret_material`
- extended `falcon_verify_rejects_malformed_inputs_without_panicking`
- `falcon::tests::falcon_signatures_are_bound_to_network_and_covered_input_set`
- `hd::tests::wallet_seed_debug_is_redacted`
- `mnemonic::tests::mnemonic_debug_is_redacted`
- `wallet::tests::wallet_debug_does_not_expose_mnemonic_or_seed_material`

## Test Results

See [ATHO_FALCON512_TEST_RESULTS.md](ATHO_FALCON512_TEST_RESULTS.md).

## Benchmark Results

See [ATHO_FALCON512_BENCHMARKS.md](ATHO_FALCON512_BENCHMARKS.md).

## Remaining Risks

- nonstandard locking scripts are still not Falcon-key-bound
- plaintext wallet persistence still exists when password is empty
- no dedicated Falcon fuzz harness yet
- upstream Falcon backend side-channel properties were not independently re-audited here

## Recommended Improvements

- enforce canonical supported locking-script types only
- remove or explicitly gate plaintext wallet save mode
- add `cargo-fuzz` targets for Falcon public key, secret key, witness parser, and tx verification
- document upstream `fn-dsa` side-channel assumptions
- consider public-key-only derivation helpers to avoid generating a full secret key when deriving wallet addresses
- if a verification cache is added later, use exact `(network, pubkey, message, signature)` keys and bound the cache

## Production Readiness Checklist

- [x] Valid Falcon signatures pass
- [x] Invalid Falcon signatures fail
- [x] Wrong-message signatures fail
- [x] Wrong-key signatures fail
- [x] Wrong-network signatures fail
- [x] Malformed public keys fail safely
- [x] Malformed signatures fail safely
- [x] Signature parser cannot panic
- [x] Public key parser cannot panic
- [x] Private key parser cannot panic
- [ ] Address/public key binding enforced for all accepted script forms
- [x] Transaction mutation invalidates signature
- [x] Replay across transactions rejected
- [x] Replay across inputs rejected
- [x] Replay across networks rejected
- [x] Signature cache cannot be poisoned today because no cache exists
- [x] Signing message is canonical
- [x] Domain separation reviewed
- [x] Private keys are not logged through `Debug`
- [x] Seed phrases are not logged through `Debug`
- [x] Wallet secrets are not exposed by node API
- [x] Private key storage reviewed
- [x] Timing/side-channel risks documented
- [x] Falcon tests pass
- [ ] Falcon fuzzing baseline passes
- [x] Falcon benchmarks recorded
- [ ] No critical Falcon issues remain
- [ ] No high Falcon production blockers remain

## Final Recommendation

**Safe for testnet only.**

Why:

- the Falcon signing and verification path is strict and materially better hardened after this pass
- malformed-input handling is stronger
- secret exposure through debug formatting was fixed
- wallet secret lifetimes are shorter in a few important places

But production should wait until:

- the nonstandard locking-script ownership rule is fixed
- plaintext wallet persistence is removed or explicitly fenced off
- Falcon fuzzing coverage is in place
