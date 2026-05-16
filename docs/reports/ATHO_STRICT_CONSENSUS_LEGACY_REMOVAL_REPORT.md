# Atho Strict Consensus and Legacy Acceptance Removal Report

## Summary

This pass removed the unsafe ownership fallback that let non-32-byte locking scripts behave like anyone-can-spend outputs, then tightened the surrounding consensus path so Atho now accepts only the current canonical 32-byte payment-digest lock format for new spends and new outputs.

The central fix is in [crates/atho-storage/src/validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs): spend validation now rejects noncanonical UTXO locks, noncanonical unlocking scripts, and any witness public key that does not hash to the exact UTXO lock under the current network.

## The Fixed `validation.rs` Ownership Issue

Before this pass, non-32-byte locks were dangerous because ownership validation only enforced public-key binding for 32-byte locks. Any shorter or alternate-length lock could slip through as long as the unlocking script bytes matched the stored UTXO bytes.

That meant a legacy/test-form output could become effectively anyone-can-spend once created.

### New rule

All spendable UTXO locks must now be:

- exactly 32 bytes
- interpreted as the canonical Atho payment digest
- matched exactly by the input unlocking script
- matched exactly by `public_key_digest(network, witness_pubkey)`

Anything else fails with `LegacyLockFormatRejected` or `InputOwnershipMismatch`.

## Files Changed

- [crates/atho-core/src/address.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-core/src/address.rs)
- [crates/atho-errors/src/lib.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-errors/src/lib.rs)
- [crates/atho-errors/src/registry.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-errors/src/registry.rs)
- [crates/atho-storage/src/validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs)
- [crates/atho-storage/src/chainstate.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/chainstate.rs)
- [crates/atho-wallet/src/wallet.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-wallet/src/wallet.rs)
- [crates/atho-node/src/dev.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-node/src/dev.rs)
- [crates/atho-node/src/service.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-node/src/service.rs)
- [crates/atho-node/src/bin/atho-attack.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-node/src/bin/atho-attack.rs)
- [crates/atho-node/src/bin/atho-adversarial.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-node/src/bin/atho-adversarial.rs)

## Legacy Acceptance Paths Removed

### Removed or hardened

- permissive non-32-byte lock acceptance in spend validation
- acceptance of noncanonical output locks during normal transaction validation
- acceptance of noncanonical coinbase output locks
- wallet signer grouping on noncanonical selected UTXOs
- chainstate bootstrap fallback that reconstructed genesis UTXO locks from legacy/internal-HPK address text instead of using the actual frozen genesis block bytes
- stale adversarial/raw-tx fixtures that still generated legacy short-form lock scripts

### Reviewed and left in place intentionally

- storage quarantine/recovery logic for legacy TSV layouts:
  this is fail-closed migration handling, not consensus compatibility
- Qt/local-node legacy RPC detection:
  this is operator compatibility logic outside consensus validation

## Monetary Policy Review

I re-audited the current runtime paths for:

- block subsidy
- halving interval
- tail emission
- coinbase maturity
- fee floors
- dust rules
- network-specific consensus params

Result: I did not find an active runtime fallback that accepts old subsidy, halving, tail, maturity, or fee-policy rules in current validation/mining/mempool code. The enforcement remains centralized in:

- `atho_core::consensus::subsidy`
- `atho_core::consensus::params`
- `atho_core::consensus::tx_policy`
- `validate_coinbase_transaction_with_schedule(...)`

No monetary-policy code path was loosened in this pass. The one meaningful monetary/consensus tightening added here is that coinbase outputs now must also use canonical 32-byte locks.

## Network-Specific Legacy Behavior Review

Reviewed current network-bound consensus paths for:

- transaction signing digests
- public key digest binding
- block/network id checks
- target checks
- genesis anchoring
- wrong-network rejection

Result:

- wrong-network witness/signing replay tests still pass
- wrong-network block and transaction cases still fail
- no runtime compatibility mode was added

## Regression Tests Added

### Validation

- `noncanonical_output_lock_is_rejected_before_witness_verification`
- `empty_output_lock_is_rejected_before_witness_verification`
- `spending_legacy_locking_script_utxo_is_rejected`
- `coinbase_with_legacy_lock_is_rejected`
- `block_spending_legacy_locking_script_utxo_is_rejected`

### Wallet

- `signer_input_groups_reject_noncanonical_selected_locking_script`

### Service / API surface

- `sendrawtransaction_rejects_legacy_lock_format`

### Existing strictness checks now confirmed green again

- trailing-byte raw-tx rejection
- wrong-parent contextual validation ordering
- wrong public key / wrong network replay rejection
- oversized block fail-fast
- full storage reorg/reload suites

## Fuzz / Adversarial Coverage

I updated the `atho-attack` adversarial harness so its “valid” fixture now uses canonical 32-byte payment locks rather than the old 4-byte short-form script.

That keeps the harness aligned with the stricter consensus rules instead of accidentally testing a now-invalid fixture format.

Attack harness results:

- `cargo run -p atho-node --bin atho-attack -- --network regnet` -> `19/19`
- `cargo run -p atho-node --bin atho-attack -- --network testnet` -> `19/19`

## Commands Run

Passed:

- `cargo fmt --all`
- `cargo fmt --check`
- `cargo check --workspace`
- `cargo test -p atho-storage --no-run`
- `cargo test -p atho-wallet --no-run`
- `cargo test -p atho-node --no-run`
- `cargo test -p atho-wallet signer_input_groups_reject_noncanonical_selected_locking_script -- --nocapture`
- `cargo test -p atho-node sendrawtransaction_rejects_legacy_lock_format -- --nocapture`
- `cargo test -p atho-storage select_branch_rejects_reorg_deeper_than_max_depth -- --nocapture`
- `cargo test -p atho-storage -- --nocapture`
- `cargo run -p atho-node --bin atho-attack -- --network regnet`
- `cargo run -p atho-node --bin atho-attack -- --network testnet`

Failed:

- `cargo clippy --workspace --all-targets`

Clippy still fails in the bundled Falcon upstream crates under `Falcon 512 rs/`, specifically `fn-dsa-kgen`, on an existing `clippy::never_loop` deny plus many upstream style warnings. That failure is outside the Atho consensus/runtime changes made in this pass.

## Benchmark / Performance Notes

This pass was consensus-hardening first, not a hot-path optimization pass.

The only performance-adjacent runtime change was beneficial:

- contextual block validation now performs parent/target contextual prechecks before merkle/body commitment checks

That slightly reduces wasted work on obviously wrong-parent candidate blocks and matches the expected rejection order more cleanly.

I did not record new benchmark numbers in this pass.

## Remaining Risks

### 1. Frozen genesis reward scripts are still legacy-format

The hard-coded genesis reward scripts in [crates/atho-core/src/genesis.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-core/src/genesis.rs) are still 48-byte legacy/internal-HPK-style values.

This pass deliberately did not rewrite genesis constants, because doing so is consensus-breaking and creates a new network.

Impact:

- runtime consensus is now strict for all new outputs and new spends
- old persisted legacy outputs become intentionally unspendable
- genesis remains a frozen historical exception until the network is reset

### 2. Old testnet databases/chains with legacy outputs should be wiped

Recommendation: yes, wipe old testnet/regnet databases if they contain legacy short-form outputs you still expect to spend.

Reason:

- there is no compatibility mode now
- there should not be one
- old unsafe outputs are correctly rejected under the current rules

### 3. Non-consensus legacy/operator compatibility paths still exist

Examples:

- legacy storage quarantine handling
- Qt legacy RPC detection

These are not consensus-acceptance paths, but they should still be reviewed before a mainnet-grade release.

## Mainnet Readiness Score After These Fixes

**7/10 for consensus hardening**, but **not mainnet-ready overall**.

Why it improved:

- the anyone-can-spend legacy lock bug is fixed
- canonical lock enforcement now spans transaction validation, contextual spend validation, coinbase validation, wallet selection, and raw-tx submission
- storage and adversarial regressions are green

Why it is still not mainnet-ready:

- frozen genesis reward scripts still use legacy 48-byte lock bytes
- old chain/db state with legacy outputs now needs a deliberate reset/wipe strategy
- broader non-consensus production issues from earlier audits still exist outside this pass

## Wipe Recommendation

**Recommended:** wipe old testnet/regnet DBs and chains before continuing the next serious launch-prep stage.

Reason:

- the network now correctly rejects old unsafe output forms
- keeping old chainstate around would preserve unusable historical outputs and create confusion
- a clean canonical-only testnet state is the simplest path forward

## Final Verdict

This pass did what it needed to do:

- non-32-byte spend locks can no longer be spent
- witness public keys must bind to the exact current canonical lock digest
- wallet construction no longer tolerates noncanonical selected UTXOs
- raw transaction submission no longer admits legacy lock payloads
- no compatibility mode was added

The remaining gap is no longer “runtime consensus accepts legacy ownership.” It is now “the frozen network history still contains legacy genesis-era bytes, so decide whether to reset that history before mainnet-track work.”
