# Atho Quick Start

Atho has four primary binaries:

- `athod` for a full node
- `atho-mine` for the miner
- `atho-qt` for the desktop client
- `atho-cli` for Bitcoin Core-style local RPC commands

If you downloaded a packaged release instead of building from source, start with the native installer:

- Windows: `Atho Setup.exe`
- macOS: `Atho Setup.app`
- Linux: `Atho Setup`

GitHub Releases will host the per-OS packages produced by the release workflow in [`.github/workflows/publish-packages.yml`](.github/workflows/publish-packages.yml).

If you want one download that contains all OS bundles, use `Atho-<version>-desktop.zip` from GitHub Releases. Extract it, open the extracted `desktop/` folder, and run the installer for your OS.

## 1. Install

Install these basics first:

- Git
- Rust with `rustup`
- a native C/C++ build toolchain

Platform notes:

- Linux: `build-essential`, `pkg-config`, `curl`
- macOS: Xcode Command Line Tools
- Windows: Visual Studio Build Tools with C++ support and the MSVC Rust toolchain

## 2. Clone

```bash
git clone https://github.com/Atho-Labs/Atho-Alpha.git
cd Atho-Alpha
```

## 3. Main Entry Commands

The intended top-level user commands are:

```bash
python mainnet.py
python testnet.py
python regnet.py
```

The launcher:

- tries a GPU-enabled build of `athod`, `atho-mine`, and `atho-qt` first if they are missing
- rebuild them if the source tree is newer than the release binaries
- prepares the runtime root
- launches the desktop client in managed-local-node mode

If the GPU-native build fails because the host is missing the native prerequisites, the launcher prints a clear OS-specific warning and falls back to a CPU-only release build.
GPU-native builds typically need:

- macOS: Xcode Command Line Tools
- Linux: a C/C++ compiler plus OpenCL headers/runtime
- Windows: Visual Studio Build Tools with C++ support plus the vendor OpenCL runtime

Extra flags can still be passed through when needed:

```bash
python testnet.py --peer HOST:PORT
python testnet.py --data-dir /absolute/path
```

If your shell exposes Python as `python3` instead of `python`, use `python3 testnet.py`.

Built binaries still live in `target/release/`, and the launcher reuses them when they are already current.

The launcher already builds with the native GPU feature enabled. If you want to build the binaries manually instead:

```bash
cargo build --release -p atho-node --bin atho-mine --features gpu-native
cargo build --release -p atho-qt --features gpu-native
```

That feature enables the native FFI wrapper and keeps the node as the final authority.
`--backend gpu` requires a real OpenCL GPU.
`--backend auto` prefers GPU and falls back to CPU, and Atho surfaces the fallback reason in the miner status.

## 4. Run The Client

Linux or macOS:

```bash
python mainnet.py
python testnet.py
python regnet.py
```

Windows PowerShell:

```powershell
py -3 .\mainnet.py
py -3 .\testnet.py
py -3 .\regnet.py
```

For testnet:

```bash
python testnet.py
```

or on Windows:

```powershell
py -3 .\testnet.py
```

The launcher ends by executing `atho-qt --local-node`, so the client still uses the real Rust node path.

If you want a disposable pruning and recovery sandbox, use `--network prunetest` instead of `regnet`.

## 5. Raw Binary Commands

Linux or macOS:

```bash
./target/release/athod --network regnet
```

Windows PowerShell:

```powershell
.\target\release\athod.exe --network regnet
```

Mainnet now resolves the configured DNS seed first and still keeps the static fallback peer as a last resort:

```bash
./target/release/athod --network mainnet
```

Use `--peer HOST:PORT` only when you want to override or add peers manually.

## 6. Run The Miner

Linux or macOS:

```bash
./target/release/atho-mine --network regnet
```

Windows PowerShell:

```powershell
.\target\release\atho-mine.exe --network regnet
```

The miner uses the network default RPC port unless you override `--rpc-addr`.

For GPU mining, use `--backend gpu` or `--backend auto` and build with `--features gpu-native`.
If you omit `--backend`, the miner defaults to auto-select.
Use `./target/release/atho-mine --probe-gpu` to print the detected device name, vendor, driver, and OpenCL capability before starting a long mining session.
If probe fails, the output now includes a stable code such as `ATHO-MINE-102` or `ATHO-MINE-103`.

In Qt, open `Settings > Mining` to pick `Auto`, `GPU only`, or `CPU only` and inspect the detected device details directly in the app.

## 7. Testnet And Mainnet

Mainnet:

```bash
python mainnet.py
./target/release/athod --network mainnet
./target/release/atho-mine --network mainnet
```

Testnet:

```bash
python testnet.py
./target/release/athod --network testnet
./target/release/atho-qt --network testnet --local-node
./target/release/atho-mine --network testnet
```

Testnet ATHO is distributed manually by the Atho founders or development team. Contact the Atho team to request testnet funds.

Mainnet and testnet are strictly isolated. Mainnet has no faucet and no testnet difficulty stall reset. Storage recovery is shared across networks but remains network-scoped: recoverable local chainstate/index issues are quarantined under that network's data directory and rebuilt without mixing data. Testnet may reset during development and may reset difficulty to minimum after more than 10 minutes without a block.

## 8. Know Sync Is Working

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

## 9. What To Expect On First Run

- the runtime root is created automatically under the OS-native Atho data directory
- the top-level launcher builds release binaries automatically if they are missing or stale
- the first wallet open or import may take a little longer while it scans
- `--local-node` may take a moment while the managed node starts
- mainnet and testnet resolve their configured DNS seed first and keep a static fallback peer as a last resort

## 10. Override The Data Root

Use a custom root if you want all state in one place:

```bash
export ATHO_DATA_DIR=/absolute/path
```

or:

```powershell
$env:ATHO_DATA_DIR = "D:\Atho"
```

The launcher honors the same environment variable.

## 11. Key Commands

- `python mainnet.py`
- `python testnet.py`
- `python regnet.py`
- `athod --network <mainnet|testnet|regnet|prunetest>`
- `atho-qt --network <mainnet|testnet|regnet|prunetest> --local-node`
- `atho-mine --network <mainnet|testnet|regnet|prunetest>`
- `atho-cli --network <mainnet|testnet|regnet|prunetest> getblockchaininfo`
- `atho-cli help getblocktemplate`
- `athod status --network <mainnet|testnet|regnet|prunetest>`
- `athod verify --network <mainnet|testnet|regnet|prunetest>`

The Qt client also includes a built-in `Debug Console` entry under `Help`, backed by the same command registry as `atho-cli`.
