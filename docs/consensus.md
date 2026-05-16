# Consensus

This document summarizes the current consensus and policy rules visible in the repo. It is not a replacement for the Rust validation code.

## Network Modes

Atho defines `mainnet`, `testnet`, `regnet`, and `prunetest` network identities with separate consensus IDs, P2P magic values, address prefixes, and ports.

This repo disables mainnet operator launch paths. Testnet, regnet, and prunetest can be run from this checkout.

## Proof Of Work

- Block hash profile: SHA3-384
- Target block time: 75 seconds
- Retarget interval: every block
- Averaging window: 17 blocks
- Median-time-past window: 11 blocks
- Retarget damping factor: 4
- Max upward adjustment per retarget: 16 percent
- Max downward adjustment per retarget: 32 percent

## Blocks

- Block version: 1
- Max block virtual size: 3,000,000 vbytes
- Max raw serialized block size: 12,000,000 bytes
- Witness scale factor: 4
- Block files rotate around 128 MiB

Blocks must match the selected network, have valid proof of work, connect to known history, and pass contextual validation before becoming canonical.

## Transactions

Atho uses a UTXO transaction model.

Policy limits:

- Max transaction raw size: 250,000 bytes
- Max transaction virtual size: 250,000 vbytes
- Max standard inputs: 1,024
- Max standard outputs: 64
- Minimum relay fee rate: 1 atom per vbyte
- Minimum transaction fee: 500 atoms
- Minimum output amount: 1,000 atoms
- Transaction anti-spam Proof-of-Work range: 16 to 28 bits

## Signatures

Transaction witnesses use Falcon-512.

Key and signature sizes:

- Public key: 897 bytes
- Secret key: 1,281 bytes
- Signature: 666 bytes

Signature domain separation is part of the consensus and wallet signing path.

## Coinbase And Supply

- Display precision: 12 decimals
- 1 ATHO: 1,000,000,000,000 atoms
- Initial block reward: 6.25 ATHO
- Halving interval: 1,680,000 blocks
- Tail reward: 0.78125 ATHO
- Tail reward starts after the third halving
- Coinbase maturity: 150 blocks
- Fixed max supply: none in the current code

## Reorgs And Forks

Chainstate supports fork handling and canonical-chain updates. Prunetest keeps a pruning profile for storage and reorg boundary testing.

Current constants:

- Prune depth: 100,000 blocks
- Max reorg depth: 100,000 blocks
- Finalization depth: 100,000 blocks

## Validation Source

The authoritative implementation is in:

- `crates/atho-core/src/consensus/`
- `crates/atho-storage/src/validation.rs`
- `crates/atho-storage/src/chainstate.rs`
- `crates/atho-node/src/validation.rs`
