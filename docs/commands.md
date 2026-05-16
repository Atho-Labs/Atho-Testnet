# Commands

This page is for power users. The quick launcher flow stays in the main [README](../README.md).

## Install Dependencies

Checks that Rust, Cargo, and Python are available.

```bash
rustc --version
cargo --version
python3 --version
```

Expected result: each command prints a version.

Common error: `command not found` means the tool is not installed or is not on `PATH`.

## Build

Builds the main workspace binaries.

```bash
cargo build
```

Expected result: Cargo finishes successfully and creates binaries under `target/debug/`.

## Launch Mainnet

Starts the desktop client with a managed local mainnet node.

```bash
python3 runmainnet.py
```

Expected result: the launcher builds missing binaries, prepares runtime directories, and opens `atho-qt`.

## Launch Testnet

Starts the desktop client with a managed local testnet node.

```bash
python3 runtestnet.py
```

Expected result: the launcher builds missing binaries, prepares runtime directories, and opens `atho-qt`.

## Launch Regnet

Atho uses `regnet` as the local deterministic test network.

```bash
python3 runregnet.py
```

Expected result: a local node starts without public mainnet or public testnet bootstrap peers.

Run a direct regnet node:

```bash
cargo run -p atho-node --bin athod -- --network regnet --data-dir /tmp/atho-regnet
```

## Run Node

Starts a mainnet node with local RPC and public P2P.

```bash
cargo run -p atho-node --bin athod -- --network mainnet
```

Expected result: the node prints its network, RPC address, height, sync state, and peer status.

Common error: if the RPC port is already in use, stop the old node or set `ATHO_RPC_ADDR`.

## Run Miner

Runs the standalone miner against the selected node RPC endpoint.

```bash
cargo run -p atho-node --bin atho-mine -- --network mainnet --loop
```

Expected result: the miner requests block templates, solves blocks, and submits them over RPC.

Common error: `node_not_synced` or unsafe mining status means the node needs to finish sync first.

## Run Wallet

Starts the desktop wallet directly.

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

Expected result: the wallet opens and starts or connects to a managed local node.

## Run CLI

Runs a local RPC command.

```bash
cargo run -p atho-node --bin atho-cli -- --network mainnet getstatus
```

Look up error codes:

```bash
cargo run -p atho-node --bin atho-cli -- --network mainnet geterrorcodes ATHO-DB-009
```

Expected result: JSON or pretty output describing the command result.

## Run Tests

Runs the launcher tests and main Rust package tests.

```bash
python3 -m unittest tests.test_runtime_launcher
cargo test -p atho-errors -p atho-core -p atho-crypto -p atho-storage -p atho-p2p -p atho-rpc -p atho-wallet -p atho-node
```

Expected result: all tests pass.

## Clean Local Data

Wipes chain/runtime data for a selected data directory while preserving wallets.

```bash
ATHO_DATA_DIR="${ATHO_DATA_DIR:-$HOME/.local/share/Atho}"
cargo run -p atho-node --bin athod -- wipe --network mainnet --data-dir "$ATHO_DATA_DIR" --all
```

Expected result: `wiped <path>`.

Common error: the command refuses to wipe while a live local node for the same network is running. Stop the node first.

Do not add `--include-wallets` unless you intentionally want to delete wallet files.
