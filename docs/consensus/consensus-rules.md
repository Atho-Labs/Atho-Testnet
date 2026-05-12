# Consensus Rules

## Canonical Validation Owner

The canonical consensus validation path lives in:

- `crates/atho-storage/src/validation.rs`

That module is the authority for:

- transaction validation
- contextual transaction validation
- block validation
- contextual block validation
- witness reference checks
- fee and reward checks

Why:

- Atho should not have multiple competing implementations of validity

## Transaction Rules

A non-coinbase transaction must satisfy:

- supported version at the target height
- at least one output
- maximum raw size limit
- maximum vsize limit
- non-zero outputs
- unique inputs
- sufficient fee
- valid witness payload
- valid Falcon signature
- matching UTXO ownership
- maturity requirements for spent outputs

## Block Rules

A block must satisfy:

- supported block version at the target height
- correct network
- correct next height
- correct parent hash
- bounded target
- proof of work
- valid timestamp
- raw, vbyte, and weight limits
- non-empty transaction list
- first transaction is coinbase
- no second coinbase
- merkle root match
- witness root match
- valid constituent transactions
- exact coinbase reward bound

## Canonical State Transition Owner

The canonical state transition path lives in:

- `crates/atho-storage/src/chainstate.rs`

This is where Atho:

- validates before connect
- applies UTXO changes
- persists chainstate atomically
- disconnects blocks during rollback
- evaluates branch preference

Why:

- consensus validity is incomplete without deterministic state mutation rules

## Error Model

Consensus failures are explicit through structured enums such as:

- `ValidationError`
- `StorageError`

Why:

- deterministic rejection reasons improve testing, auditing, and GUI/backend behavior

## Monetary Rules

Current constants:

- `1 ATHO = 1,000,000,000,000 atoms`
- fixed max supply: none; Atho uses permanent tail emission
- initial block reward: `5 ATHO` on mainnet/regnet; legacy testnet remains `6.25 ATHO`
- tail block reward: `0.625 ATHO forever` on mainnet/regnet; legacy testnet remains `0.78125 ATHO`
- halving interval: `1,260,000 blocks` on mainnet/regnet; legacy testnet remains `1,680,000 blocks`
- coinbase maturity: `150 blocks`
- standard transaction confirmations: `6` on mainnet/regnet

Implemented in:

- `crates/atho-core/src/constants.rs`
- `crates/atho-core/src/consensus/subsidy.rs`
- `crates/atho-storage/src/utxo.rs`

## Why This Design

Chosen approach:

- explicit constants
- explicit functions for height-based rules
- explicit storage-owned validation context

Rejected approach:

- silent rule changes
- UI-driven interpretation
- multiple alternative validation helpers spread across the stack

## Current Limitations

- no active ruleset beyond V1
- no deployed soft-fork style activation mechanism beyond scheduled height gating
- pruning-aware deep reorg recovery remains conservative

## Related Documentation

- [Proof of Work and Emission](proof-of-work-and-emission.md)
- [Versioning and Activations](versioning-and-activations.md)
- [Reorg, Fork, and Pruning Rules](reorg-fork-pruning.md)
