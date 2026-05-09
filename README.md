# Atho Testnet

This repository is the public Atho testnet client and node source. It is intentionally testnet-only and kept focused on the pieces needed to run, test, and inspect the Atho public test network.

Mainnet launch paths, mainnet DNS seeds, mainnet bootstrap peers, and mainnet operator flows are disabled here. Use the Atho-Alpha repository for mainnet work.

- Website: <https://atho.io>
- Testnet explorer: <https://atho.io/explore/>
- Current testnet release: `v0.1.3`
- Public testnet seed/API node: `testnet-node1.atho.io`
- Public testnet peer: `162.222.206.163:9100`

## Requirements

- Rust and Cargo
- Python 3
- A C/C++ toolchain and OpenCL headers are optional. If GPU build prerequisites are missing, the launcher falls back to a CPU-only build.

## Run The Testnet Client

```bash
python3 runtestnet.py
```

Useful launcher flags:

```bash
python3 runtestnet.py --dry-run
python3 runtestnet.py --rebuild
python3 runtestnet.py --data-dir ~/.atho-testnet
python3 runtestnet.py --network-overrides-local
```

The launcher builds the release binaries when needed, streams the real Cargo/Rust compiler output, prepares the local data directory, and starts `atho-qt` in managed local-node mode on testnet.

## Run The Testnet Node

```bash
cargo run -p atho-node --bin athod -- --network testnet
```

Check status:

```bash
cargo run -p atho-node --bin athod -- status --network testnet
```

Verify local genesis/runtime wiring:

```bash
cargo run -p atho-node --bin athod -- verify --network testnet
```

Force a local chain resync while preserving wallet files:

```bash
cargo run -p atho-node --bin athod -- --network testnet --network-overrides-local
```

## Build

```bash
cargo build --release -p atho-node -p atho-qt
```

GPU-enabled builds can be requested with:

```bash
cargo build --release -p atho-node -p atho-qt --features gpu-native
```

## Validation

```bash
python3 -m unittest tests.test_runtime_launcher
cargo check --workspace
cargo test -p atho-errors -p atho-core -p atho-crypto -p atho-storage -p atho-p2p -p atho-rpc -p atho-wallet -p atho-node
cargo check --manifest-path fuzz/Cargo.toml --all-targets
```

This repo intentionally keeps the public surface small: `runtestnet.py`, `runtime_launcher.py`, this README, and the Rust source needed to run the public testnet software.
