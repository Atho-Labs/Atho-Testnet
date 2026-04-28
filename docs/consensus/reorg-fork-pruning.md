# Reorg, Fork, and Pruning Rules

## Reorg Model

Implemented in:

- `crates/atho-storage/src/chainstate.rs`
- `crates/atho-node/src/node.rs`

Current branch selection flow:

1. validate the candidate branch sequence
2. locate the fork point in retained history
3. compare accumulated chainwork
4. disconnect the current suffix if the new branch is preferred
5. connect the candidate suffix in order
6. restore the previous suffix if any candidate block fails
7. rebuild mempool validity against the new tip

Why:

- reorg safety is about rollback correctness, not just fork choice

## Fork Choice

Preferred branch metric:

- cumulative chainwork
- then deterministic tie-breaking behavior

Why:

- height-only selection can choose the wrong history

## Invalid Branch Handling

If a branch is malformed or invalid:

- it is rejected
- final state is not mutated
- the previous best chain remains canonical

Why:

- failed reorg attempts must not partially contaminate local state

## Mempool Behavior Across Reorgs

During a successful reorg:

- transactions mined in the adopted branch are removed from the mempool
- transactions from disconnected non-coinbase blocks may be reconsidered for re-entry if still valid

Implemented in:

- `crates/atho-node/src/node.rs`

Why:

- the mempool should track the new chainstate, not the old tip’s spend graph

## Pruning

Current core constant:

- `PRUNE_DEPTH_BLOCKS = 70,000`

History is pruned conservatively, and branch selection rejects forks whose fork point is unavailable in retained history.

Why:

- rejecting an unsupported deep reorg is safer than guessing through incomplete history

## Current Limitations

- deep pruned-history reorg recovery is not fully featured
- pruning lifecycle coverage is still weaker than the main unpruned path
- snapshot sync is not deployed yet

## Related Documentation

- [Chainstate and Persistence](../storage/chainstate-and-persistence.md)
- [Testing and Hardening](../testing-audits/testing-and-hardening.md)
