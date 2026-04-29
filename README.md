# Atho

Atho is a from-scratch Rust blockchain payment stack built around a small trusted core, explicit consensus rules, durable chainstate, a thin desktop client, and a Bitcoin-style operational model adapted to Atho’s own hashing, signature, and address choices.

## Status

Atho is beyond prototype stage, but it is not a finished production network.

Current posture:

- local consensus, storage, wallet, miner, RPC, and Qt lifecycle paths have strong sandbox coverage
- the node, miner, and desktop client now have a cleaner operator command model
- `athod status` now reports peer counts, per-peer direction, and byte counters
- the Qt settings page now includes Bitcoin-Core-style peer and traffic diagnostics
- the default runtime root is OS-native instead of working-directory driven
- the live TCP peer runtime exists and is sandbox-tested over real sockets
- the public VPS node path now has live restart/recovery and one-block propagation coverage
- DNS seeds are still intentionally blank, so live bootstrap still needs manual peers

Production-readiness summary:

- overall readiness: `8/10`
- local core lifecycle: `strong`
- public-network hardening: `materially improved, but still needs longer multi-peer soak coverage`

Detailed status lives in:

- [`docs/production-readiness/current-status.md`](docs/production-readiness/current-status.md)
- [`docs/production-readiness/roadmap.md`](docs/production-readiness/roadmap.md)

## Quick Start

Use the short operator guide first:

- [`quickstart.md`](quickstart.md)

If you are using a packaged release, run the native installer first:

- Windows: `Atho Setup.exe`
- macOS: `Atho Setup.app`
- Linux: `Atho Setup`

## Design Principles

- keep the trusted core small
- keep consensus deterministic and explicit
- keep validation on one canonical path
- keep the GUI thin and backend-owned
- keep storage durable and fail-closed
- keep runtime commands boring and predictable
- keep protocol evolution explicit through versioning and activation heights

## How To Run Atho

There are three primary binaries:

1. `athod`
2. `atho-mine`
3. `atho-qt`

Recommended roles:

- `athod`: full node / daemon / VPS node
- `atho-mine`: standalone miner process
- `atho-qt`: desktop wallet and client

### Full Node

```bash
cargo run -p atho-node --bin athod -- --network mainnet
```

### Desktop Client

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

For repeated local launches after the first build, run the binary directly:

```bash
cargo build -p atho-qt
./target/debug/atho-qt --network mainnet --local-node
```

### Miner

```bash
cargo run -p atho-node --bin atho-mine -- --network regnet --rpc-addr 127.0.0.1:9210
```

More commands and operator notes live in [`docs/operations/commands.md`](docs/operations/commands.md).

## Release Packaging

Build and stage a local release bundle for the current host with:

```bash
python3 scripts/release.py
```

Windows:

```powershell
py -3 scripts\release.py
```

The packaging guide is in [`docs/build-deployment/packaging.md`](docs/build-deployment/packaging.md).

The shareable release tree is `desktop/`, which mirrors each built bundle under `desktop/releases/<version>/<platform>-<arch>/`.
The root `desktop/install.sh` and `desktop/install.ps1` scripts dispatch to `desktop/latest/<platform>-<arch>/` for the current OS, and the active bundle contains the native `Atho Setup` installer front-end plus the platform client launcher (`Atho.exe` on Windows or `Atho.app` on macOS).

GitHub release publishing is automated by [`.github/workflows/publish-packages.yml`](.github/workflows/publish-packages.yml). It builds Windows, macOS, and Linux packages and publishes the matching archive for each OS to GitHub Releases.

GitHub Releases also gets one combined download named `Atho-<version>-desktop.zip`. That archive contains the full `desktop/` tree with every platform bundle, so you can download one file, extract it, and run the installer for your OS from inside the folder.

The release assets also include a direct installer download for the current host platform:

- Windows: `Atho Setup.exe`
- macOS: `Atho Setup-arm64.dmg` or `Atho Setup-x86_64.dmg`
- Linux: use the platform archive and run `Atho Setup` from inside the extracted folder
On Windows, the installed client launcher is `Atho.exe`.

Before running a direct installer, verify the matching `checksums.sha256` file from the same release. The Windows `.exe` and macOS `.dmg` also verify their embedded payload checksums before installing.
On Windows, the installer also asks for an install location, creates Start Menu and Desktop shortcuts to `Atho.exe`, and launches the client after install.
On macOS, the installer installs the bundled `Atho.app` client and opens it after install when requested.

## Runtime Roots

Default runtime root:

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

## Windows Quick Start

Shortest path:

1. install Rust with the MSVC toolchain
2. install Visual Studio Build Tools with C++ support
3. run:

```powershell
git clone https://github.com/Atho-Labs/Atho-Alpha.git
cd Atho-Alpha
cargo build --release -p atho-node -p atho-qt
.\target\release\atho-qt.exe --network regnet --local-node
```

Full Windows instructions:

- [`docs/operations/linux-quick-start.md`](docs/operations/linux-quick-start.md)
- [`docs/operations/macos-quick-start.md`](docs/operations/macos-quick-start.md)
- [`docs/operations/windows-quick-start.md`](docs/operations/windows-quick-start.md)
- [`docs/operations/launch-checklist.md`](docs/operations/launch-checklist.md)

## VPS Full Node

Recommended command shape:

```bash
./athod --network mainnet --data-dir /var/lib/atho --peer 198.51.100.10:56000
```

Important defaults:

- RPC stays on loopback
- P2P listens publicly
- because DNS seeds are still blank, manual `--peer` values are still required for live bootstrap

Full VPS guidance:

- [`docs/operations/vps-full-node.md`](docs/operations/vps-full-node.md)

## Repository Layout

```text
crates/
  atho-core/      protocol types, consensus rules, genesis, blocks, txs, addresses
  atho-crypto/    Falcon boundary and secret handling
  atho-storage/   LMDB storage, chainstate, validation, UTXO state
  atho-wallet/    HD wallet, mnemonic, keypool, encrypted wallet datafiles
  atho-p2p/       wire protocol, handshake, peer/session logic, sync scaffolding
  atho-rpc/       local RPC request/response and transport
  atho-node/      runtime, service, miner, mempool, orchestration
  atho-qt/        thin desktop client
  atho-installer/ native installer front-end
docs/             documentation, operator guides, readiness notes, whitepaper
scripts/          build and packaging helpers
dev/              optional repo-local sandbox workspace when explicitly selected
dist/             staged release artifacts
desktop/          shareable desktop release tree
```

## Documentation Map

Start here:

- [`docs/index.md`](docs/index.md)

Key sections:

- [`docs/operations/runtime-model.md`](docs/operations/runtime-model.md)
- [`docs/operations/commands.md`](docs/operations/commands.md)
- [`docs/operations/linux-quick-start.md`](docs/operations/linux-quick-start.md)
- [`docs/operations/macos-quick-start.md`](docs/operations/macos-quick-start.md)
- [`docs/operations/windows-quick-start.md`](docs/operations/windows-quick-start.md)
- [`docs/operations/vps-full-node.md`](docs/operations/vps-full-node.md)
- [`docs/operations/launch-checklist.md`](docs/operations/launch-checklist.md)
- [`docs/node-runtime/node-runtime-and-p2p.md`](docs/node-runtime/node-runtime-and-p2p.md)
- [`docs/node-runtime/rpc-and-client.md`](docs/node-runtime/rpc-and-client.md)
- [`docs/gui-client/qt-client.md`](docs/gui-client/qt-client.md)
- [`docs/testing-audits/testing-and-hardening.md`](docs/testing-audits/testing-and-hardening.md)
- [`docs/whitepaper/atho-whitepaper-apa.md`](docs/whitepaper/atho-whitepaper-apa.md)

## Production Notes

The strongest parts of the stack today are the canonical validation core, storage integrity, replay/restart handling, miner flow, RPC-driven Qt behavior, and the cleaner operator launch model.

The weakest parts are still:

- peer-served snapshot sync
- deeper pruning coverage
- broader migration/upgrade coverage
- long-run public-network soak coverage
- OS-level Qt automation in CI
- broader release/distribution hardening

Those gaps are documented explicitly instead of being hidden.
