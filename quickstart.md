# Atho Quick Start

Atho has three primary commands:

- `athod` for a full node
- `atho-mine` for the miner
- `atho-qt` for the desktop client

If you downloaded a packaged release instead of building from source, start with the native installer:

- Windows: `Atho Setup.exe`
- macOS: `Atho Setup.app`
- Linux: `Atho Setup`

GitHub Releases will host the per-OS packages produced by the release workflow in [`.github/workflows/publish-packages.yml`](.github/workflows/publish-packages.yml).

## 1. Install

Install these basics first:

- Git
- Rust with `rustup`
- a native C/C++ build toolchain

Platform notes:

- Linux: `build-essential`, `pkg-config`, `curl`
- macOS: Xcode Command Line Tools
- Windows: Visual Studio Build Tools with C++ support and the MSVC Rust toolchain

## 2. Clone And Build

```bash
git clone https://github.com/Atho-Labs/Atho-Alpha.git
cd Atho-Alpha
cargo build --release -p atho-node -p atho-qt
```

Built binaries live in `target/release/`.

## 3. Run The Client

Linux or macOS:

```bash
./target/release/atho-qt --network regnet --local-node
```

Windows PowerShell:

```powershell
.\target\release\atho-qt.exe --network regnet --local-node
```

`--local-node` starts a managed `athod` child process over RPC, so the client uses the real node path.

## 4. Run The Full Node

Linux or macOS:

```bash
./target/release/athod --network regnet
```

Windows PowerShell:

```powershell
.\target\release\athod.exe --network regnet
```

Mainnet uses the canonical bootstrap peer until DNS seeds are added:

```bash
./target/release/athod --network mainnet --peer 74.208.219.116:56000
```

## 5. Run The Miner

Linux or macOS:

```bash
./target/release/atho-mine --network regnet
```

Windows PowerShell:

```powershell
.\target\release\atho-mine.exe --network regnet
```

The miner uses the network default RPC port unless you override `--rpc-addr`.

## 6. Testnet And Mainnet

Testnet:

```bash
./target/release/athod --network testnet
./target/release/atho-qt --network testnet --local-node
./target/release/atho-mine --network testnet
```

Mainnet:

```bash
./target/release/athod --network mainnet --peer 74.208.219.116:56000
./target/release/atho-qt --network mainnet --local-node --peer 74.208.219.116:56000
./target/release/atho-mine --network mainnet
```

## 7. Know Sync Is Working

Run the status command against the node:

```bash
./target/release/athod status --network regnet
```

What healthy looks like:

- `running=true`
- `headers_synced=true`
- `peer_count` is non-zero on a live network
- `bytes_received` increases while the node is syncing
- `sync_best_height` rises toward the current tip

In the Qt client, the Settings page should show:

- connected peers
- inbound and outbound counts
- sent and received bytes
- per-peer height, protocol, and traffic details

## 8. What To Expect On First Run

- the runtime root is created automatically under the OS-native Atho data directory
- the first wallet open or import may take a little longer while it scans
- `--local-node` may take a moment while the managed node starts
- mainnet sync will not move until you point at the bootstrap peer or another live peer

## 9. Override The Data Root

Use a custom root if you want all state in one place:

```bash
export ATHO_DATA_DIR=/absolute/path
```

or:

```powershell
$env:ATHO_DATA_DIR = "D:\Atho"
```

## 10. Key Commands

- `athod --network <mainnet|testnet|regnet>`
- `atho-qt --network <mainnet|testnet|regnet> --local-node`
- `atho-mine --network <mainnet|testnet|regnet>`
- `athod status --network <mainnet|testnet|regnet>`
- `athod verify --network <mainnet|testnet|regnet>`
