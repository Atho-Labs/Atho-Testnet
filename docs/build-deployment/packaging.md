# Atho Rust Packaging and Installer Guide

## Purpose

This repo uses a Rust build plus a small Python packager that stages a clean, downloadable bundle for the current host OS. The bundle includes a native installer front-end called `Atho Setup` so the user flow is closer to Bitcoin Core: download, run installer, then open Atho.

The packager also stages a self-contained installer asset for the current host OS so the GitHub release page can expose a real launcher file instead of only a wrapper archive.

The goal is simple:

download -> install -> open Atho -> sync -> use

## What The Packager Builds

The current release bundle packages the binaries that exist in this repo today:

- `athod`
- `atho-mine`
- `atho-qt`
- `atho-address`
- `atho-setup` installer front-end, staged as:
  - Windows: `Atho Setup.exe`
  - macOS: `Atho Setup.app`
  - Linux: `Atho Setup`

The GitHub release asset set also includes the direct installer download for the current host:

- Windows: `Atho Setup.exe`
- macOS: `Atho Setup.dmg`

Those direct installers are self-contained and validate their embedded payload checksum before extracting or launching anything.
On Windows, the installer asks for an install directory and creates a Start Menu shortcut that points directly to the GUI client executable.

It also stages:

- `atho` launcher for the desktop client
- OS-specific install and uninstall helpers
- quickstart and operator docs
- the desktop icon asset
- checksums and a manifest

Optional developer tools can be added with `--include-dev-tools`.

## One-Command Rebuild

Use the Python packager from the repo root:

```bash
python3 scripts/release.py
```

Windows:

```powershell
py -3 scripts\release.py
```

Compatibility wrappers also exist:

- `./scripts/package.sh`
- `./scripts/package.ps1`

The packager always rebuilds cleanly for the current host platform and then stages the release bundle.

## Output Layout

The packager writes three paths:

- versioned release output: `dist/releases/<version>/<platform>-<arch>/`
- current compatibility mirror: `dist/release/`
- shareable desktop tree: `desktop/releases/<version>/<platform>-<arch>/`
- active desktop mirror: `desktop/latest/<platform>-<arch>/`
- download-first installers: `dist/releases/<version>/<platform>-<arch>/installers/`
- shareable desktop root installers: `desktop/install.sh`, `desktop/install.ps1`, `desktop/uninstall.sh`, `desktop/uninstall.ps1`
- each active bundle also includes the native installer front-end (`Atho Setup` for the current platform)

Inside the release bundle you will find:

- top-level binaries and launchers
- the native installer front-end for the host platform
- `INSTALL.md`
- `README.md`
- `QUICKSTART.md`
- `COMMANDS.md`
- `RELEASE_NOTES.md`
- `PACKAGING.md`
- `TROUBLESHOOTING.md`
- `VPS_FULL_NODE.md`
- `LINUX_QUICK_START.md`
- `MACOS_QUICK_START.md`
- `WINDOWS_QUICK_START.md`
- `athod.service.example`
- `Atho.png`
- `manifest.json`
- `checksums.sha256`
- `archives/`

The archive inside `archives/` is the downloadable release artifact for that host.

## Installer Behavior By OS

Linux:

- default app location: `~/.local/share/Atho`
- default command location: `~/.local/bin`
- installer front-end: `Atho Setup`
- installer creates shell launchers and a desktop entry

macOS:

- default app location: `~/Applications/Atho`
- default command location: `~/bin`
- installer front-end: `Atho Setup.app`
- installer creates a `Atho.command` launcher for double-click launch
- the app bundle is self-contained, but unsigned releases can still trigger Gatekeeper until signing/notarization is added

Windows:

- default app location: `%LOCALAPPDATA%\Programs\Atho`
- installer front-end: `Atho Setup.exe`
- installer adds the bundle directory to the user PATH
- installer asks for the install directory and creates a Start Menu shortcut to the GUI client
- installer verifies the embedded payload checksum before install
- installer launches the installed client after the install step succeeds

## Launcher Model

The release bundle exposes a simple operator model:

- `atho` launches the desktop client
- `athod` runs the full node
- `atho-mine` runs the miner
- `atho-address` handles address helper workflows

Network selection is still explicit:

- `ATHO_NETWORK=testnet` or `ATHO_NETWORK=regnet` switches networks
- `ATHO_MAINNET_PEER=74.208.219.116:56000` overrides the default mainnet bootstrap peer

## Update Flow

To cut an update, bump the version and rerun the packager:

```bash
python3 scripts/release.py --version <new-version>
```

The script rewrites the versioned release directory, refreshes `dist/release/` as the current mirror, and mirrors the bundle into `desktop/releases/<version>/<platform>-<arch>/` for sharing.

The `desktop/` folder also contains root installer dispatchers so the shared tree can be opened once and run on the local operating system. They always target `desktop/latest/<platform>-<arch>/`.

The active bundle itself contains the native installer front-end, so testers can also open `Atho Setup` directly from the release tree.

The packaged installer itself is self-contained:

- the Windows `Atho Setup.exe` asset carries the payload inside the executable
- the macOS `Atho Setup.app` bundle carries its payload inside `Contents/Resources/payload.zip`
- the release tree still keeps the extracted bundle for operator workflows and debugging

## GitHub Release Publishing

This repository includes a GitHub Actions workflow at [`.github/workflows/publish-packages.yml`](../../.github/workflows/publish-packages.yml) that builds the per-OS packages and publishes them as GitHub Release assets.

It produces one package set per platform:

- `linux-x86_64`
- `macos-arm64`
- `windows-x86_64`

It also publishes one combined cross-platform download:

- `Atho-<version>-desktop.zip`

That combined package contains the full `desktop/` tree with all platform bundles, so a user can download one file and then run the installer for their OS from inside the extracted folder.

Each release asset contains:

- the platform archive from `dist/releases/<version>/<platform>-<arch>/archives/`
- the direct installer asset from `dist/releases/<version>/<platform>-<arch>/installers/`
- a platform-specific checksum file
- a platform-specific manifest file

Download the asset that matches your OS, extract it if needed, and run the native setup front-end inside the bundle:

- Windows: `Atho Setup.exe`
- macOS: `Atho Setup.dmg`
- Linux: `Atho Setup`

If you want one download that carries every platform package, pick `Atho-<version>-desktop.zip`.

## What To Edit Later

If Atho gains new release binaries or a native installer pipeline, the first places to update are:

- `scripts/release.py`
- `quickstart.md`
- `docs/operations/commands.md`
- `docs/operations/runtime-model.md`

That keeps release behavior in one place instead of scattering it across ad hoc scripts.
