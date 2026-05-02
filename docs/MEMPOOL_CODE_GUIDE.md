# Atho Mempool Code Guide

## Scope

Mempool logic mainly lives in:

- `crates/atho-node/src/mempool.rs`
- `crates/atho-storage/src/validation.rs`

## Role

The mempool stores unconfirmed transactions that:

- passed standard relay policy
- passed chainstate validation
- do not conflict with already-admitted mempool inputs

## Policy Rules

Current documented relay defaults include:

- minimum fee rate: `1 atom / vbyte`
- dust threshold: `50 atoms`

Outputs below the dust threshold should not be created by wallet code and should be rejected for relay policy.

## Important Distinction

Mempool policy is not automatically the same thing as historical consensus.

Comments and docs should clearly say when a rule is:

- local relay policy
- wallet construction policy
- consensus block validity

## Revalidation Rule

After the tip changes, mempool entries must be revalidated against the new chainstate before they continue to be mined or relayed.
