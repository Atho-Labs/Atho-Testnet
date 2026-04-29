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

The shortest useful commands are:

```bash
cargo build --release -p atho-node -p atho-qt
./target/release/atho-qt --network regnet --local-node
./target/release/athod --network regnet
./target/release/atho-mine --network regnet
```

`--local-node` starts a managed `athod` child process over RPC so the desktop client uses the real node path.

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

Run the end-to-end benchmark harness:

```bash
cargo run --release -p atho-node --bin atho-benchmark -- --network regnet --tx-count 256 --inputs-per-tx 1 --samples 3 --output benchmark.md
```

Heavier block and relay coverage:

```bash
cargo run --release -p atho-node --bin atho-benchmark -- --network regnet --tx-count 6000 --inputs-per-tx 1 --samples 3 --output benchmark.md
cargo run --release -p atho-node --bin atho-benchmark -- --network regnet --tx-count 5084 --inputs-per-tx 2 --samples 3 --output benchmark.md
```

## Full Node

Preferred command:

```bash
./target/release/athod --network mainnet
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
./target/release/athod --network regnet --data-dir /tmp/atho-regnet
./target/release/athod --network mainnet --peer 74.208.219.116:56000
./target/release/athod --network mainnet --rpc-addr 127.0.0.1:9010
```

Status:

```bash
./target/release/athod status --network mainnet
./target/release/athod status --rpc-addr 127.0.0.1:9010
```

The status command reports:

- connected peer count
- inbound/outbound split
- total bytes sent and received
- peer diagnostics with endpoint, direction, height, protocol version, and traffic totals

Verification:

```bash
./target/release/athod verify --network mainnet
```

Important:

- RPC is local-only by default
- P2P listens publicly by default
- DNS seeds are still blank, so use `--peer 74.208.219.116:56000` for live mainnet bootstrap

## Desktop Client

Attach to an existing node:

```bash
./target/release/atho-qt --network mainnet --rpc-addr 127.0.0.1:9010
```

Start a managed local node:

```bash
./target/release/atho-qt --network mainnet --local-node
```

Managed local node with explicit bootstrap peers:

```bash
./target/release/atho-qt --network mainnet --local-node --peer 74.208.219.116:56000
```

Useful flags:

- `--network <mainnet|testnet|regnet>`
- `--rpc-addr HOST:PORT`
- `--local-node`
- `--peer HOST:PORT` (repeatable)
- `--p2p-addr HOST:PORT`
- `--data-dir PATH`

The settings page includes a controlled network diagnostics view with:

- chain height and sync state
- connected peer counts
- sent/received byte counters
- per-peer endpoint, direction, protocol, and traffic details

## Miner

Run the node first, then the miner:

```bash
./target/release/athod --network regnet
./target/release/atho-mine --network regnet
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
cargo run -p atho-node --bin athod -- wipe --network regnet --data-dir /tmp/atho-dev --all
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

Notes:

- `wipe` refuses mainnet unless `--dangerously-allow-mainnet` is passed explicitly
- keep benchmark and disposable node state under a sandbox directory such as `/tmp/atho-dev`

## Packaging

Stage a local release bundle with the native installer front-end:

```bash
python3 scripts/release.py
```

Windows:

```powershell
py -3 scripts\release.py
```

The release script stages:

- `Atho Setup.exe` on Windows
- `Atho Setup.app` on macOS
- `Atho Setup` on Linux
- the `desktop/` share tree with matching root dispatchers

It also stages the direct installer download under `dist/releases/<version>/<platform>-<arch>/installers/`:

- Windows: `Atho Setup.exe`
- macOS: `Atho Setup.dmg`

Before opening a direct installer, verify the matching `checksums.sha256` file from the same GitHub release. The Windows and macOS installers also validate their embedded payload checksums before they install or launch anything.
On Windows, the installer prompts for an install directory and creates a Start Menu shortcut to the GUI client executable.

For GitHub publishing, use [`.github/workflows/publish-packages.yml`](../../.github/workflows/publish-packages.yml). It builds the same per-OS packages and uploads the release assets to GitHub Releases.

The workflow also publishes one combined `Atho-<version>-desktop.zip` package that contains the full `desktop/` tree for all supported OS bundles.

The legacy wrappers still work:

```bash
./scripts/package.sh
```
