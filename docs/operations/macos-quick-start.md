# macOS Quick Start

## Goal

This is the shortest path from a GitHub checkout to a working Atho node, desktop client, or miner on macOS.

## Prerequisites

Install:

1. Xcode Command Line Tools
2. Rust with `rustup`
3. Git

Example setup:

```bash
xcode-select --install
curl https://sh.rustup.rs -sSf | sh
. "$HOME/.cargo/env"
```

## Build

Clone the repo:

```bash
git clone https://github.com/Atho-Labs/Atho-Alpha.git
cd Atho-Alpha
```

Main entry commands:

```bash
python runmainnet.py
python runtest.py
```

The launchers build the release binaries automatically when needed and then exec into `atho-qt --local-node`.

Built binaries still land in:

- `target/release/athod`
- `target/release/atho-mine`
- `target/release/atho-qt`

## Runtime Root

Default runtime root:

```text
~/Library/Application Support/Atho
```

Override it explicitly if needed:

```bash
export ATHO_DATA_DIR="$HOME/Atho"
```

## Run The Full Node

Start a node:

```bash
./target/release/athod --network regnet
```

Mainnet now uses the configured DNS seed first and keeps the static fallback peer as a last resort:

```bash
./target/release/athod --network mainnet
```

Check status:

```bash
./target/release/athod status --network mainnet
```

What to look for:

- `peer_count`
- `bytes_sent`
- `bytes_received`
- `headers_synced`
- a populated `peers:` section once the node is connected

## Run The Desktop Client

Use the simplest desktop path first:

```bash
python runmainnet.py
```

If you want the client to connect to an already-running node instead:

```bash
./target/release/atho-qt --network regnet --rpc-addr 127.0.0.1:9210
```

The managed local-node path uses the same DNS-seed-first bootstrap flow on mainnet:

```bash
python runmainnet.py
```

The settings page includes a local-only diagnostics section with peer counts, byte counters, and per-peer transport details.

## Run The Miner

Start the node first, then the miner:

```bash
./target/release/athod --network regnet
./target/release/atho-mine --network regnet
```

## Related Documentation

- [Commands](commands.md)
- [Runtime Model](runtime-model.md)
- [Troubleshooting](troubleshooting.md)
