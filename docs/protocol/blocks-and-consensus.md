# Blocks and Consensus

## Purpose

Blocks bundle transactions, commit to their ordering, prove work, and drive deterministic chainstate transitions.

## Block Structure

Implemented in:

- `crates/atho-core/src/block.rs`

### Header fields

- `version: u16`
- `network_id: Network`
- `height: u64`
- `previous_block_hash: [u8; 48]`
- `merkle_root: [u8; 48]`
- `witness_root: [u8; 48]`
- `timestamp: u64`
- `difficulty_target_or_bits: [u8; 48]`
- `nonce: u64`

### Body fields

- `transactions: Vec<Transaction>`
- fee accounting totals
- in-memory witness map keyed by txid

Important design point:

- the canonical witness-root owner is the header
- the block object no longer keeps a duplicated second witness-root field

Why:

- one commitment must have one owner in consensus-critical code

## Merkle And Witness Commitments

The block computes:

- transaction merkle root from transaction ids
- witness root from transaction witness data

Implemented in:

- `crates/atho-core/src/block.rs`

Why:

- separating transaction identity and witness commitment keeps pruning and signature reference logic explicit

## Proof of Work

The current proof-of-work hash is:

- `SHA3-384`

A valid block hash must be less than or equal to the stated target.

Implemented in:

- `crates/atho-core/src/consensus/pow.rs`

Why:

- Atho intentionally uses SHA3-384 as part of its protocol identity while retaining a Bitcoin-style target comparison model

## Candidate Block Construction

The miner builds candidate blocks by:

1. reading current height and tip hash
2. selecting mempool transactions
3. computing fees
4. building coinbase outputs
5. computing merkle and witness roots
6. assembling a header with the next target
7. searching a valid nonce

Implemented in:

- `crates/atho-node/src/miner.rs`

Why:

- construction and acceptance are separate stages so miners never bypass full validation

## Validation Path

Canonical block validation lives in:

- `crates/atho-storage/src/validation.rs`

Current checks include:

- block version
- network match
- height continuity
- parent hash match
- timestamp bounds
- target bounds
- proof of work
- block size and weight
- non-empty transaction list
- exactly one coinbase at position zero
- merkle root match
- witness root match
- transaction validity
- fee totals
- coinbase reward correctness

Why:

- chainstate mutation must only happen after one canonical full-block validation path succeeds

## Chain Acceptance

The node connects a valid block by:

1. validating with context
2. applying UTXO changes in memory
3. atomically committing block archive, tx archive, snapshot, and UTXO snapshot
4. updating tip state
5. clearing or revalidating mempool entries

Implemented in:

- `crates/atho-storage/src/chainstate.rs`
- `crates/atho-storage/src/db.rs`
- `crates/atho-node/src/node.rs`

Why:

- acceptance should be one irreversible transition, not a sequence of loosely related writes

## Reward And Fees

Current emission model:

- initial subsidy: `50 ATHO`
- halving interval: `1,680,000 blocks`
- total cap: `168,000,000 ATHO`

Fees are tracked in atoms and split into explicit accounting fields on the block structure.

Why:

- monetary rules should be visible and testable, not hidden inside runtime accounting

## Current Limitations

- no optional UTXO/state commitment in headers yet
- no compact-block relay path yet
- no active post-V1 ruleset to exercise mixed-version histories

## Related Documentation

- [Proof of Work and Emission](../consensus/proof-of-work-and-emission.md)
- [Consensus Rules](../consensus/consensus-rules.md)
- [Chainstate and Persistence](../storage/chainstate-and-persistence.md)
