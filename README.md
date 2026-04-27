# Atho

Atho is a from-scratch Rust payment system built around a full node, HD wallet, miner, RPC layer, storage engine, and thin desktop client.

It is designed to behave like a real blockchain payment stack:
- transactions are signed, validated, and stored in atoms
- blocks are mined against difficulty and only accepted after validation
- chainstate, UTXO, block archive, transaction archive, peer metadata, and address metadata are stored durably in per-network LMDB environments, one dataset per environment
- the mempool stays in RAM and is rebuilt after restart
- the desktop client stays thin and talks to the node over RPC

The codebase is intentionally split into small crates so the trusted core stays simple, auditable, and fast.

## Core Specs

- Currency unit: `atoms`
- `1 ATHO = 100,000,000 atoms`
- Max supply: `168,000,000 ATHO`
- Initial block reward: `50 ATHO`
- Halving interval: `1,680,000` blocks
- Target block time: `75` seconds
- Minimum transaction fee: `1 atom/vbyte`
- Proof of work hash: `SHA3-384`
- Hash size: `384` bits / `96` hex characters
- Consensus math uses integers only
- Mainnet, testnet, and regnet have separate network identities, genesis data, and RPC defaults

## What Atho Includes

- `atho-core` - protocol constants, consensus, transaction, block, address, and genesis logic
- `atho-crypto` - thin Falcon and Kyber boundary layer
- `atho-storage` - LMDB-backed chainstate, UTXO, block archive, and peer/address storage
- `atho-wallet` - HD wallet, mnemonic, keypool, wallet datafile handling, and address generation CLI
- `atho-p2p` - wire codec, peer protocol, and sync state
- `atho-rpc` - small RPC surface for the client and miner
- `atho-node` - node runtime, validation, mempool, mining, and orchestration
- `atho-qt` - thin desktop wallet/client

## Architecture

Atho follows a Bitcoin-style split:

- `athod` is the always-on node daemon
- `atho-mine` is the standalone miner client
- `atho-qt` is the desktop wallet client
- `atho-address` is the address inspection and generation tool

The node owns validation, consensus, mempool admission, block acceptance, and chainstate updates.
The desktop client stays light and uses RPC instead of embedding the whole blockchain stack in the UI process.

## Consensus Summary

- Transactions use canonical serialization and txid generation
- Witness data is committed separately for pruning-safe verification
- Blocks are validated before chain acceptance
- Genesis blocks are hardcoded per network
- PoW is checked against the network target
- Invalid blocks and transactions are rejected before they can mutate final state

## Transaction Lifecycle

The transaction path in Atho is:

1. The wallet derives an address from the HD seed and keeps the wallet state in the local datafile.
2. A spend transaction is assembled with explicit inputs, outputs, automatic 1 atom/vbyte fee policy, and witness data.
3. The transaction digest is signed with Falcon.
4. The node checks canonical serialization, duplicate inputs, fee policy, input ownership, and signature validity.
5. Valid transactions enter the mempool.
6. The miner pulls only valid mempool transactions into a block template.
7. The block is mined against the network target.
8. The node validates the full block again before accepting it.
9. Accepted blocks update chainstate and remove confirmed transactions from the mempool.

Current note:
- The backend transaction path is implemented.
- The Qt send screen now builds and submits a wallet spend using wallet-owned UTXOs, HD change, and RPC broadcast.
- The remaining improvements are mostly UX polish and richer wallet history/indexing.

## Running Atho

Use [COMMANDS.md](/Users/eyeanonymous/Desktop/Atho-Alpha /COMMANDS.md) for the exact commands.

Quick start:

```bash
cargo check
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:18443
```

## Development Docs

- [COMMANDS.md](/Users/eyeanonymous/Desktop/Atho-Alpha /COMMANDS.md) - build, run, mine, and wallet commands
- [PACKAGING.md](/Users/eyeanonymous/Desktop/Atho-Alpha /PACKAGING.md) - release artifacts and package layout
- [dev/README.md](/Users/eyeanonymous/Desktop/Atho-Alpha /dev/README.md) - local wipe workflow and log locations
- [TODO.md](/Users/eyeanonymous/Desktop/Atho-Alpha /TODO.md) - current build order and remaining work

## Status

The core protocol, wallet, storage, node, RPC, and Qt client crates build and test cleanly.
The desktop client is intentionally thin and still depends on the node for heavy work.

## Repository Layout

```text
crates/
  atho-core/      consensus, tx, block, address, genesis
  atho-crypto/    Falcon and Kyber wrappers
  atho-storage/   LMDB-backed chainstate, UTXO, block archive, peer/address storage
  atho-wallet/    HD wallet and address generation
  atho-p2p/       peer protocol and sync
  atho-rpc/       node/client RPC surface
  atho-node/     daemon, validation, mempool, mining
  atho-qt/       desktop client
```

## Notes

- Keep consensus math integer-only.
- Keep the node authoritative.
- Keep the client thin.
- Keep the trusted core small.
