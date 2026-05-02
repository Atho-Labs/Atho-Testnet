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

Clone the repo:

```powershell
git clone https://github.com/Atho-Labs/Atho-Alpha.git
cd Atho-Alpha
```

Main entry commands:

```powershell
py -3 .\runmainnet.py
py -3 .\runtest.py
```

The launchers build the release binaries automatically when needed and then exec into `atho-qt --local-node`.

Built binaries still land in:

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

Mainnet now uses the configured DNS seed first and keeps the static fallback peer as a last resort:

```powershell
.\target\release\athod.exe --network mainnet
```

Check status:

```powershell
.\target\release\athod.exe status --network regnet
```

## Run The Desktop Client

Use the simplest desktop path first:

```powershell
py -3 .\runmainnet.py
```

If you installed the packaged Windows release, launch `Atho.exe` from the Start Menu or Desktop shortcut instead of the raw `atho-qt.exe` binary.

If you want the client to connect to an already-running node instead:

```powershell
.\target\release\atho-qt.exe --network regnet --rpc-addr 127.0.0.1:9210
```

The managed local-node path uses the same DNS-seed-first bootstrap flow on mainnet:

```powershell
py -3 .\runmainnet.py
```

Renderer note:

- Windows now defaults the desktop client to `wgpu`
- that avoids the `glutin` WGL ES-context startup failure seen on some machines
- if you need to override it manually, pass `--renderer glow` or `--renderer wgpu`

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
