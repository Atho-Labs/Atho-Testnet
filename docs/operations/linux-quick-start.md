# Linux Quick Start

## Goal

This is the shortest path from a GitHub checkout to a working Atho node, desktop client, or miner on Linux.

## Prerequisites

Install:

1. Rust with `rustup`
2. a C/C++ build toolchain
3. Git

On Ubuntu or Debian:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config git curl
curl https://sh.rustup.rs -sSf | sh
. "$HOME/.cargo/env"
```

## Build

Clone the repo and build the release binaries:

```bash
git clone https://github.com/Atho-Labs/Atho-Alpha.git
cd Atho-Alpha
cargo build --release -p atho-node -p atho-qt
```

Built binaries:

- `target/release/athod`
- `target/release/atho-mine`
- `target/release/atho-qt`

## Runtime Root

Default runtime root:

```text
${XDG_DATA_HOME:-~/.local/share}/Atho
```

Override it explicitly if needed:

```bash
export ATHO_DATA_DIR=/srv/atho
```

or per command:

```bash
./target/release/athod --network regnet --data-dir /tmp/atho-regnet
```

## Run The Full Node

Start a node:

```bash
./target/release/athod --network regnet
```

Because DNS seeds are still blank, add peers explicitly for live network sync:

```bash
./target/release/athod --network mainnet --peer 203.0.113.10:56000 --peer 203.0.113.11:56000
```

Check status:

```bash
./target/release/athod status --network mainnet
```

What to look for:

- `peer_count`
- `peer_count_inbound`
- `peer_count_outbound`
- `bytes_sent`
- `bytes_received`
- `headers_synced`

## Run The Desktop Client

Use the simplest desktop path first:

```bash
./target/release/atho-qt --network regnet --local-node
```

If you want the client to connect to an already-running node instead:

```bash
./target/release/atho-qt --network regnet --rpc-addr 127.0.0.1:18445
```

If the managed local node must bootstrap manually:

```bash
./target/release/atho-qt --network mainnet --local-node --peer 203.0.113.10:56000
```

The settings page includes a local-only network diagnostics view with:

- connected peer counts
- inbound/outbound split
- sent/received byte counters
- per-peer endpoint, protocol, and traffic information

## Run The Miner

Start the node first, then the miner:

```bash
./target/release/athod --network regnet
./target/release/atho-mine --network regnet --rpc-addr 127.0.0.1:18445
```

## Related Documentation

- [Commands](commands.md)
- [Runtime Model](runtime-model.md)
- [VPS Full Node](vps-full-node.md)
- [Troubleshooting](troubleshooting.md)
