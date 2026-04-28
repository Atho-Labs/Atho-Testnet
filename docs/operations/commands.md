# Commands

This is the canonical operator command guide for Atho.

## Stable Entry Points

There are three primary binaries:

1. `athod`
2. `atho-mine`
3. `atho-qt`

Recommended roles:

- `athod`: full node / daemon / VPS node
- `atho-mine`: standalone miner process
- `atho-qt`: desktop wallet and client

## Quick Start

Full node:

```bash
cargo run -p atho-node --bin athod -- --network mainnet
```

Desktop client with a managed local node:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

Standalone miner:

```bash
cargo run -p atho-node --bin atho-mine -- --network regnet --rpc-addr 127.0.0.1:9210
```

## Data Root

Default Atho runtime root:

- macOS: `~/Library/Application Support/Atho`
- Linux: `${XDG_DATA_HOME:-~/.local/share}/Atho`
- Windows: `%APPDATA%\\Atho`

Override it explicitly:

```bash
--data-dir /absolute/path
```

or:

```bash
export ATHO_DATA_DIR=/absolute/path
```

The runtime root contains:

- `db/`
- `logs/`
- `wallet/`
- `chain/`
- `audit/`
- `quarantine/`

## Build

Check the workspace:

```bash
cargo check
```

Build release binaries:

```bash
cargo build --release -p atho-node -p atho-qt
```

## Test

Run the full workspace:

```bash
cargo test
```

Run the adversarial campaign:

```bash
cargo run --release -p atho-node --bin atho-adversarial -- --cases 52000 --seed 12345
```

Run the targeted attack sweep:

```bash
cargo run -p atho-node --bin atho-attack -- --network regnet
```

## Full Node

Preferred command:

```bash
cargo run -p atho-node --bin athod -- --network mainnet
```

Useful flags:

- `--network <mainnet|testnet|regnet>`
- `--data-dir PATH`
- `--rpc-addr HOST:PORT`
- `--p2p-addr HOST:PORT`
- `--peer HOST:PORT` (repeatable)
- `--public-rpc`

Examples:

```bash
cargo run -p atho-node --bin athod -- --network regnet --data-dir /tmp/atho-regnet
cargo run -p atho-node --bin athod -- --network mainnet --peer 198.51.100.10:56000 --peer 198.51.100.11:56000
cargo run -p atho-node --bin athod -- --network mainnet --rpc-addr 127.0.0.1:9010
```

Status:

```bash
cargo run -p atho-node --bin athod -- status --network mainnet
cargo run -p atho-node --bin athod -- status --rpc-addr 127.0.0.1:9010
```

Verification:

```bash
cargo run -p atho-node --bin athod -- verify --network mainnet
```

Important:

- RPC is local-only by default
- P2P listens publicly by default
- DNS seeds are still blank, so use `--peer` for live bootstrap

## Desktop Client

Attach to an existing node:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:9010
```

Start a managed local node:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

Managed local node with explicit bootstrap peers:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node --peer 198.51.100.10:56000
```

Useful flags:

- `--network <mainnet|testnet|regnet>`
- `--rpc-addr HOST:PORT`
- `--local-node`
- `--peer HOST:PORT` (repeatable)
- `--p2p-addr HOST:PORT`
- `--data-dir PATH`

## Miner

Run the node first, then the miner:

```bash
cargo run -p atho-node --bin athod -- --network regnet
cargo run -p atho-node --bin atho-mine -- --network regnet --rpc-addr 127.0.0.1:9210
```

Useful flags:

- `--network <mainnet|testnet|regnet>`
- `--rpc-addr HOST:PORT`
- `--cores N`
- `--data-dir PATH`

## Wallet Tools

Generate or inspect addresses:

```bash
cargo run -p atho-wallet --bin atho-address -- generate mainnet --seed-hex 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
cargo run -p atho-wallet --bin atho-address -- generate testnet --phrase "..." --count 2
cargo run -p atho-wallet --bin atho-address -- inspect A...
```

## Dev Workspace

Wipe disposable state:

```bash
cargo run -p atho-node --bin athod -- dev wipe --data-dir /tmp/atho-dev
```

Reset from genesis:

```bash
cargo run -p atho-node --bin athod -- dev reset --network regnet --data-dir /tmp/atho-dev
```

Watch logs:

```bash
cargo run -p atho-node --bin athod -- dev watch --data-dir /tmp/atho-dev
```

Export TSV audit views:

```bash
cargo run -p atho-node --bin athod -- dev export chain --data-dir /tmp/atho-dev
cargo run -p atho-node --bin athod -- dev export tx --data-dir /tmp/atho-dev
```

## Packaging

Stage local release artifacts:

```bash
./scripts/package.sh
```

Current staged files:

- `athod`
- `atho-mine`
- `atho-qt`
- `README.md`
- `COMMANDS.md`
- `RELEASE_NOTES.md`
- `PACKAGING.md`

## Deprecated Launch Forms

Still supported for compatibility, but no longer recommended:

- `athod run mainnet`
- `athod run testnet`
- `athod run regnet`

Preferred form:

```bash
athod --network mainnet
```

## Related Documentation

- [Runtime Model](runtime-model.md)
- [Windows Quick Start](windows-quick-start.md)
- [VPS Full Node](vps-full-node.md)
- [Dev Workspace](dev-workspace.md)
- [Build and Packaging](../build-deployment/packaging.md)
- [Troubleshooting](troubleshooting.md)
