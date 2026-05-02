# Atho Consensus Code Guide

## Scope

Consensus-critical code mainly lives in:

- `crates/atho-core/src/block.rs`
- `crates/atho-core/src/transaction.rs`
- `crates/atho-core/src/consensus/`
- `crates/atho-storage/src/validation.rs`

## How To Recognize Consensus Code

Consensus code typically:

- hashes canonical bytes
- checks transaction or block validity
- computes subsidy or difficulty
- compares competing branches by chainwork
- rejects malformed or wrong-network blocks

If a change can make one honest node accept a block or transaction that another honest node rejects, it is consensus-sensitive.

## Key Rules

- Header hashing must use canonical header bytes only.
- Txids must use canonical base transaction bytes only.
- Witness roots and Merkle roots must be deterministic.
- Proof-of-work target comparisons must remain deterministic.
- Subsidy calculations must use integer arithmetic only.
- Block and transaction version checks must follow the active schedule.

## Policy vs Consensus

Atho intentionally separates relay policy from consensus where possible.

Examples:

- dust rejection is policy in the mempool/wallet path
- exact canonical block/transaction structure is consensus
- fee floor checks may be policy when relaying, but fee/value underflow checks are consensus

When documenting a rule, say explicitly whether it is:

- `CONSENSUS`
- `POLICY`

## Safe Editing Rule

If you are changing:

- serialization
- hashing
- difficulty logic
- subsidy logic
- transaction or block validity

you should assume the change can fork the network unless proven otherwise.
