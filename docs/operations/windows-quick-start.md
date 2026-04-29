# Windows Quick Start

## Goal

This is the shortest path from source checkout to a working Atho node, miner, or desktop client on Windows.

## Prerequisites

Install:

1. Rust with the MSVC toolchain
2. Visual Studio Build Tools with C++ support
3. Git

Use a normal PowerShell window for the commands below.

## Build

Clone the repo and build the release binaries:

```powershell
git clone https://github.com/Atho-Labs/Atho-Alpha.git
cd Atho-Alpha
cargo build --release -p atho-node -p atho-qt
```

Built binaries:

- `target\release\athod.exe`
- `target\release\atho-mine.exe`
- `target\release\atho-qt.exe`

## Runtime Root

Default runtime root:

```text
%APPDATA%\Atho
```

Override it explicitly if needed:

```powershell
$env:ATHO_DATA_DIR = "D:\Atho"
```

or per command:

```powershell
.\target\release\athod.exe --network regnet --data-dir D:\Atho-Regnet
```

## Run The Full Node

Start a node:

```powershell
.\target\release\athod.exe --network regnet
```

Because DNS seeds are still blank, add peers explicitly for live network sync:

```powershell
.\target\release\athod.exe --network mainnet --peer 74.208.219.116:56000
```

Check status:

```powershell
.\target\release\athod.exe status --network regnet
```

## Run The Desktop Client

Use the simplest desktop path first:

```powershell
.\target\release\atho-qt.exe --network regnet --local-node
```

If you installed the packaged Windows release, launch `Atho.exe` from the Start Menu or Desktop shortcut instead of the raw `atho-qt.exe` binary.

If you want the client to connect to an already-running node instead:

```powershell
.\target\release\atho-qt.exe --network regnet --rpc-addr 127.0.0.1:9210
```

If the managed local node must bootstrap manually:

```powershell
.\target\release\atho-qt.exe --network mainnet --local-node --peer 74.208.219.116:56000
```

## Run The Miner

Start the node first, then the miner:

```powershell
.\target\release\athod.exe --network regnet
.\target\release\atho-mine.exe --network regnet
```

## Logs And Recovery

Common paths under the runtime root:

- `logs\athod.log`
- `logs\atho-qt.log`
- `logs\activity.log`
- `quarantine\`

## First-Run Troubleshooting

If the desktop client says `Connection refused`:

1. verify the node is running
2. verify the RPC port matches the selected network
3. check `logs\athod.log` and `logs\atho-qt.log`

If you want a disposable local sandbox:

```powershell
$env:ATHO_DATA_DIR = "$PWD\dev"
```

## Related Documentation

- [Commands](commands.md)
- [VPS Full Node](vps-full-node.md)
- [Troubleshooting](troubleshooting.md)
