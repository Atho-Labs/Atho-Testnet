# Proof of Work and Emission

## Proof-of-Work Profile

Implemented in:

- `crates/atho-core/src/consensus/pow.rs`

Current constants:

| Parameter | Value |
| --- | ---: |
| Target block time | `75 seconds` |
| Retarget interval | `1 block` |
| Averaging window | `17 blocks` |
| Median window | `11 blocks` |
| Damping factor | `4` |
| Max upward adjustment | `16%` |
| Max downward adjustment | `32%` |

Current hash family:

- `SHA3-384`
- 48-byte block hashes
- 96-character hex representation

Why:

- Atho keeps the Bitcoin idea of target-based proof of work while selecting SHA3-384 as part of its protocol identity

## Difficulty Targets

The code defines:

- genesis target
- minimum difficulty target
- maximum difficulty target

The next target is derived from recent history and bounded by explicit adjustment limits.

Why:

- bounded retargeting reduces abrupt shifts and keeps target movement inspectable

## Bootstrap Behavior

The target logic uses a bootstrap path when insufficient history exists and transitions to the full median-time-past style path once the full window is available.

Why:

- early chain heights do not have enough history for full-window behavior to be representative

## Chainwork

Branch preference uses accumulated chainwork rather than height alone.

Implemented in:

- `accumulated_chain_work`
- `compare_branch_work`
- `branch_is_preferred`

Why:

- height alone is not a safe fork-choice metric

## Subsidy Schedule

Implemented in:

- `crates/atho-core/src/consensus/subsidy.rs`

Current rules:

| Parameter | Value |
| --- | ---: |
| Initial reward | `50 ATHO` |
| Halving interval | `1,680,000 blocks` |
| Maximum supply | `168,000,000 ATHO` |

Why:

- explicit fixed monetary rules are easier to audit and harder to drift accidentally

## Fee Accounting

Blocks track:

- total fees
- miner fees
- burned fees
- pooled fees
- cumulative burned amount

Why:

- making fee accounting explicit at the block level simplifies auditability and later policy evolution

## Current Limitations

- no alternate proof-of-work version active
- no live production tuning from observed public-network behavior

## Related Documentation

- [Blocks and Consensus](../protocol/blocks-and-consensus.md)
- [Reorg, Fork, and Pruning Rules](reorg-fork-pruning.md)
