# Atho Testnet

This repository is the public Atho testnet client and node source. It is intentionally testnet-only and kept focused on the pieces needed to run, test, and inspect the Atho public test network.

Mainnet launch paths, mainnet DNS seeds, mainnet bootstrap peers, and mainnet operator flows are disabled here. Use the Atho-Alpha repository for mainnet work.

- Website: <https://atho.io>
- Testnet explorer: <https://atho.io/explore/>
- Current testnet release: `v0.1.8`
- Public testnet seed/API nodes: `testnet-node1.atho.io`, `testnet-node2.atho.io`
- Public testnet peers: `162.222.206.163:9100`, `74.208.219.116:9100`

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

## v0.1.8 Patch Notes

- Added header-first block scheduling improvements from Alpha so peers can keep pipelined block request windows full without one-block-at-a-time stalls.
- Hardened header acceptance by rejecting wrong-network headers, out-of-bounds targets, and headers that fail their committed proof of work before full block download.
- Improved low-peer sync behavior so the only useful peer is retried for stale block responses instead of being disconnected for ordinary slowness.
- Added per-peer stale request requeue handling so timed-out blocks can be retried cleanly without stranding pending downloads.
- Added same-box sync regression coverage for Alpha, Testnet release, and the local Testnet checkout.

## v0.1.7 Patch Notes

- Replaced peer-local fork buffering with a global side-branch pool indexed by block hash and parent hash, so fork chains can be reconstructed even when blocks arrive out of order from different peers.
- Hardened reorg recovery after bootstrap outages where local miners continued on an isolated fork and later need to switch back to a higher-work network branch.
- Preserved bounded side-branch storage while keeping low-height bridge blocks needed to reconnect a branch back to the canonical fork point.
- Dropped invalid assembled side branches after failed contextual validation so one bad fork candidate cannot poison later sync attempts.
- Added cross-peer side-branch regression coverage for rebuilding a higher-work fork over a local competing fork.

## v0.1.6 Patch Notes

- Fixed fork recovery when a node already has winning-chain blocks archived locally but they are no longer canonical after mining on an isolated fork.
- Header serving now ignores archived side-branch locator hashes and only anchors responses to the node's canonical chain, preventing invalid header sequences after reorgs.
- Sync now replays known non-canonical blocks from local storage during header catch-up instead of skipping them as already downloaded.
- Branch buffering now preserves the low-height bridge back to the fork point and backfills known archived ancestors, preventing deep fork recovery from dropping the blocks needed to reconnect.

## v0.1.5 Patch Notes

- Hardened fork recovery after bootstrap outages by building header sync locators from persisted chain history instead of only the recent in-memory reload window.
- Added periodic, relay-safe peer address sharing so connected nodes can organically learn `testnet-node2` and other healthy peers from the network.
- Seeded configured testnet bootstrap peers into the live discovery graph so bootstrap nodes can relay both public testnet peers to older connected clients.
- Tightened TCP sync regression tests around chain-sync readiness, real-socket reorg recovery, transaction relay, and peer address gossip.
