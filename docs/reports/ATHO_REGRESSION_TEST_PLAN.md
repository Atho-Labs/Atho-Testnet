# Atho Regression Test Plan

## Purpose

This plan defines the regression categories that must stay green while Atho is optimized. The goal is to keep validation strict while reducing wasted work.

## Principles

- Every optimization must preserve the same accept/reject outcomes.
- Cheap-check-before-expensive-check changes must be covered by negative tests.
- Prepared-state reuse must be covered by valid and invalid-path regressions.
- Mempool, miner, block acceptance, and API transaction handling must remain consistent.

## Required Regression Categories

### 1. Consensus block validity

- valid block accepted
- inflated coinbase rejected
- wrong parent rejected
- wrong network rejected
- invalid height rejected
- oversized block rejected
- duplicate-input block rejected
- timestamp warp rejected

### 2. Transaction validity

- valid signed transaction accepted
- invalid signature rejected
- missing-input spend rejected
- duplicate input rejected
- insufficient confirmations rejected
- dust output rejected
- zero-value output rejected
- oversized transaction rejected
- output overflow rejected

### 3. Network isolation

- cross-network signature replay rejected
- persisted cross-network UTXOs fail closed
- wrong-network block rejected

### 4. Serialization and canonical encoding

- canonical raw transaction accepted
- malformed raw transaction rejected
- trailing-byte raw transaction rejected

### 5. Falcon regressions

- wrong public key rejected
- wrong message rejected
- wrong domain rejected
- truncated signature rejected safely
- oversized signature rejected safely
- malformed public key rejected safely

### 6. Wallet safety

- wallet datafile permissions remain owner-only on Unix
- wallet errors remain sanitized

### 7. Storage / rollback / reorg

- commit fault injection rolls back cleanly
- reloaded chainstate reorgs without wiping database
- restart/reload preserves chain consistency

## Current High-Signal Commands

### Fast adversarial / safety set

```bash
cargo test -p atho-storage cross_network_signature_replay_is_rejected_even_with_valid_local_pow -- --nocapture
cargo test -p atho-storage output_total_overflow_is_rejected_during_context_validation -- --nocapture
cargo test -p atho-storage wrong_public_key_for_standard_output_is_rejected -- --nocapture
cargo test -p atho-storage oversized_block_is_rejected_before_contextual_pow_checks -- --nocapture
cargo test -p atho-storage reloaded_chainstate_reorgs_without_wiping_database -- --nocapture
cargo test -p atho-node transaction_broadcast_route_accepts_signed_raw_transaction_when_enabled -- --nocapture
cargo test -p atho-crypto falcon_verify_rejects_malformed_inputs_without_panicking -- --nocapture
cargo test -p atho-wallet wallet_datafile_permissions_are_owner_only -- --nocapture
```

### Attack-harness runs

```bash
cargo run -p atho-node --bin atho-attack -- --network regnet
cargo run -p atho-node --bin atho-attack -- --network testnet
```

### Hygiene / integration checks

```bash
cargo fmt --check
cargo check --workspace
```

## Suggested Expansion

### Mempool consistency

- mempool accepts valid tx that block acceptance also accepts
- mempool rejects invalid tx that block acceptance would reject
- mined txs removed from mempool
- conflicting txs removed from mempool after block acceptance

### Sync

- sync from zero with one honest peer
- sync with stale peer replacement
- sync resume after restart
- sync rejects invalid downloaded block

### P2P

- malformed message rejected
- oversized message rejected
- duplicate announcement suppressed
- bad peer penalized

### API

- malformed JSON rejected
- oversized body rejected
- public read route remains read-only
- write route remains disabled when wallet API is disabled

## Pass / Fail Criteria

An optimization pass is not complete unless:

- all currently relevant targeted regressions pass
- the attack harness passes on the exercised networks
- formatting and workspace checks pass
- any new helper or cache path has at least one negative regression and one good-path regression

## Notes

This plan is intentionally smaller than a full mainnet-readiness matrix. It is meant to be run frequently during optimization work so regressions are caught early instead of after larger refactors land.
