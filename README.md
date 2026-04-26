# Atho

Atho is a from-scratch Rust rebuild of the Atho core stack. The workspace is split into small crates so the trusted core stays small, auditable, and fast.

## Workspace

- `crates/atho-core` - protocol constants, consensus, transaction, and block primitives
- `crates/atho-crypto` - thin Falcon and Kyber boundary layer
- `crates/atho-storage` - chainstate and UTXO storage
- `crates/atho-wallet` - HD wallet, mnemonic, keypool, and wallet datafile handling
- `crates/atho-p2p` - wire codec, peer protocol, and sync state
- `crates/atho-rpc` - small RPC surface for the thin client
- `crates/atho-node` - node runtime, validation, mempool, and orchestration
- `crates/atho-qt` - thin desktop client

## Status

The rebuild is being done brick by brick. The core protocol, wallet, storage, node, and thin client crates already build, test, and package cleanly.

## Start here

- Read `TODO.md` for the current build order
- Read `COMMANDS.md` for the exact commands to check, test, run, and package
- Read `PACKAGING.md` for the release layout
- Read `dev/README.md` for the local wipe workflow and log locations
