# Setup

This guide starts from a clean machine and gets Atho running with the desktop client and a managed local node.

## Requirements

- Rust and Cargo from the stable toolchain
- Python 3 for `runmainnet.py`, `runtestnet.py`, and `runregnet.py`
- A C/C++ toolchain for native builds
- Optional OpenCL headers/runtime for GPU mining builds

Check the required tools:

```bash
rustc --version
cargo --version
python3 --version
```

If Rust is missing, install it with `rustup`:

```bash
curl https://sh.rustup.rs -sSf | sh
```

## Clone

```bash
git clone <repo-url>
cd Atho-Testnet-main
```

## Build

Quick build:

```bash
cargo build
```

Release build:

```bash
cargo build --release
```

GPU-enabled release build:

```bash
cargo build --release -p atho-node -p atho-qt --features gpu-native
```

## Launch A Network

Mainnet:

```bash
python3 runmainnet.py
```

Public testnet:

```bash
python3 runtestnet.py
```

Local regnet:

```bash
python3 runregnet.py
```

Useful first check:

```bash
python3 runmainnet.py --dry-run
```

The launcher prepares runtime directories named `db`, `logs`, `wallet`, `audit`, and `quarantine` under the selected data directory and then starts `atho-qt` in managed local-node mode.

You can mine from the client after sync. Standalone mining and GPU instructions live in [Mining](mining.md). Direct node, CLI, and advanced commands live in [Commands](commands.md).

## Run A Node Directly

```bash
cargo run -p atho-node --bin athod -- --network mainnet
```

Run a local regnet node:

```bash
cargo run -p atho-node --bin athod -- --network regnet --data-dir /tmp/atho-regnet
```

## Verify It Works

In another terminal, check status:

```bash
cargo run -p atho-node --bin athod -- status --network mainnet
```

Check the HTTP API:

```bash
curl http://127.0.0.1:8080/api/v1/health
```

Check the command console:

```bash
cargo run -p atho-node --bin atho-cli -- --network mainnet getstatus
```

## Common Problems

- `cargo` is missing: install Rust with `rustup`.
- Port already in use: stop the old node or set `ATHO_RPC_ADDR` / `ATHO_P2P_ADDR`.
- Old chain data after a network update: use `--network-overrides-local` or the wipe command in [Commands](commands.md).
- API not responding: make sure `athod` is running and `ATHO_API_ENABLED` is not set to `false`.
