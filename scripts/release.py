#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform as platform_mod
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import struct
import zipfile
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DIST_ROOT = ROOT / "dist"
CURRENT_RELEASE_ROOT = DIST_ROOT / "release"
VERSIONED_RELEASES_ROOT = DIST_ROOT / "releases"
DESKTOP_ROOT = ROOT / "desktop"
DESKTOP_VERSIONED_RELEASES_ROOT = DESKTOP_ROOT / "releases"
DESKTOP_LATEST_ROOT = DESKTOP_ROOT / "latest"
BOOTSTRAP_PEER = os.environ.get("ATHO_MAINNET_PEER", "74.208.219.116:56000")
INSTALLERS_DIR_NAME = "installers"
PAYLOAD_FOOTER_MAGIC = b"ATHOPLD1"
PAYLOAD_DIGEST_BYTES = 32

DOC_FILES = [
    ("README.md", ROOT / "README.md"),
    ("QUICKSTART.md", ROOT / "quickstart.md"),
    ("COMMANDS.md", ROOT / "docs" / "operations" / "commands.md"),
    ("LAUNCH_CHECKLIST.md", ROOT / "docs" / "operations" / "launch-checklist.md"),
    ("RUNTIME_MODEL.md", ROOT / "docs" / "operations" / "runtime-model.md"),
    ("TROUBLESHOOTING.md", ROOT / "docs" / "operations" / "troubleshooting.md"),
    ("VPS_FULL_NODE.md", ROOT / "docs" / "operations" / "vps-full-node.md"),
    ("LINUX_QUICK_START.md", ROOT / "docs" / "operations" / "linux-quick-start.md"),
    ("MACOS_QUICK_START.md", ROOT / "docs" / "operations" / "macos-quick-start.md"),
    ("WINDOWS_QUICK_START.md", ROOT / "docs" / "operations" / "windows-quick-start.md"),
    ("RELEASE_NOTES.md", ROOT / "docs" / "production-readiness" / "release-notes.md"),
    ("PACKAGING.md", ROOT / "docs" / "build-deployment" / "packaging.md"),
    ("athod.service.example", ROOT / "docs" / "build-deployment" / "athod.service.example"),
    ("Atho.png", ROOT / "crates" / "atho-qt" / "assets" / "branding" / "atho-icon.png"),
]


def main() -> int:
    args = parse_args()
    version = args.version or detect_version()
    host_platform = normalize_platform(platform_mod.system())
    host_arch = normalize_arch(platform_mod.machine())
    release_tag = f"{host_platform}-{host_arch}"
    release_root = VERSIONED_RELEASES_ROOT / version / release_tag
    mirror_root = CURRENT_RELEASE_ROOT
    desktop_release_root = DESKTOP_VERSIONED_RELEASES_ROOT / version / release_tag
    desktop_latest_root = DESKTOP_LATEST_ROOT / release_tag
    archive_name = f"Atho-{version}-{release_tag}.{archive_suffix(host_platform)}"
    archive_path = release_root / "archives" / archive_name

    remove_path(release_root)
    remove_path(mirror_root)
    remove_path(desktop_release_root)
    remove_path(desktop_latest_root)

    release_root.mkdir(parents=True, exist_ok=True)

    if not args.skip_build:
        build_binaries(args.include_dev_tools)

    stage_release_root(
        release_root,
        version,
        host_platform,
        host_arch,
        args.include_dev_tools,
    )
    stage_download_artifacts(release_root, host_platform)
    remove_path(release_root / "archives")
    archive_path.parent.mkdir(parents=True, exist_ok=True)
    build_archive(release_root, archive_path, archive_prefix(version, release_tag))

    manifest = build_manifest(
        release_root=release_root,
        version=version,
        platform=host_platform,
        arch=host_arch,
        archive_path=archive_path,
    )
    write_text_file(release_root / "manifest.json", json.dumps(manifest, indent=2) + "\n")
    write_checksums(release_root)

    remove_path(mirror_root)
    shutil.copytree(release_root, mirror_root)
    write_desktop_readme(version, host_platform, host_arch)
    desktop_release_root.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(release_root, desktop_release_root)
    desktop_latest_root.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(release_root, desktop_latest_root)

    print(f"release_root={release_root}")
    print(f"mirror_root={mirror_root}")
    print(f"desktop_root={desktop_release_root}")
    print(f"desktop_latest={desktop_latest_root}")
    print(f"archive={archive_path}")
    print(f"install_script={release_root / install_script_name(host_platform)}")
    print(f"launchers={', '.join(launcher_names(host_platform))}")
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build and stage Atho release bundles for the current host platform."
    )
    parser.add_argument("--version", help="override the release version")
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="stage files from target/release without rebuilding the Rust binaries",
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="remove the current host release staging directories before packaging",
    )
    parser.add_argument(
        "--include-dev-tools",
        action="store_true",
        help="also build the internal attack/adversarial binaries",
    )
    return parser.parse_args()


def detect_version() -> str:
    cargo_toml = ROOT / "crates" / "atho-node" / "Cargo.toml"
    text = cargo_toml.read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        raise RuntimeError(f"could not determine version from {cargo_toml}")
    return match.group(1)


def normalize_platform(system_name: str) -> str:
    value = system_name.lower()
    if value.startswith("darwin"):
        return "macos"
    if value.startswith("windows"):
        return "windows"
    return "linux"


def normalize_arch(machine: str) -> str:
    value = machine.strip().lower()
    if value in {"x86_64", "amd64"}:
        return "x86_64"
    if value in {"aarch64", "arm64"}:
        return "arm64"
    return value.replace(" ", "_")


def exe_suffix() -> str:
    return ".exe" if normalize_platform(platform_mod.system()) == "windows" else ""


def archive_suffix(platform_name: str) -> str:
    return "zip" if platform_name == "windows" else "tar.gz"


def archive_prefix(version: str, release_tag: str) -> str:
    return f"Atho-{version}-{release_tag}"


def install_script_name(platform_name: str) -> str:
    return "install.ps1" if platform_name == "windows" else "install.sh"


def uninstall_script_name(platform_name: str) -> str:
    return "uninstall.ps1" if platform_name == "windows" else "uninstall.sh"


def launcher_names(platform_name: str) -> list[str]:
    if platform_name == "windows":
        return [
            "atho.cmd",
            "atho-mainnet.cmd",
            "atho-testnet.cmd",
            "atho-regnet.cmd",
        ]
    if platform_name == "macos":
        return ["atho", "atho-mainnet", "atho-testnet", "atho-regnet", "Atho.command"]
    return ["atho", "atho-mainnet", "atho-testnet", "atho-regnet"]


def installer_source_name() -> str:
    return "atho-setup"


def installer_artifact_name(platform_name: str) -> str:
    if platform_name == "windows":
        return "Atho Setup.exe"
    if platform_name == "macos":
        return "Atho Setup.app"
    return "Atho Setup"


def installer_desktop_entry_name() -> str:
    return "Atho Setup.desktop"


def build_binaries(include_dev_tools: bool) -> None:
    commands = [
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "atho-node",
            "--bin",
            "athod",
            "--bin",
            "atho-mine",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
        ],
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "atho-qt",
            "--bin",
            "atho-qt",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
        ],
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "atho-wallet",
            "--bin",
            "atho-address",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
        ],
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "atho-installer",
            "--bin",
            "atho-setup",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
        ],
    ]
    if include_dev_tools:
        commands.append(
            [
                "cargo",
                "build",
                "--release",
                "--locked",
                "-p",
                "atho-node",
                "--bin",
                "atho-attack",
                "--bin",
                "atho-adversarial",
                "--manifest-path",
                str(ROOT / "Cargo.toml"),
            ]
        )

    for command in commands:
        run(command)


def stage_release_root(
    release_root: Path,
    version: str,
    platform_name: str,
    arch: str,
    include_dev_tools: bool,
) -> None:
    binary_suffix = exe_suffix()
    for target_name in ("athod", "atho-mine", "atho-qt", "atho-address"):
        copy_binary(target_name, release_root / f"{target_name}{binary_suffix}")

    stage_installer_frontend(release_root, version, platform_name, arch)

    if platform_name == "windows":
        write_text_file(release_root / "atho.cmd", windows_launcher_cmd("mainnet"))
        write_text_file(release_root / "atho-mainnet.cmd", windows_launcher_cmd("mainnet"))
        write_text_file(release_root / "atho-testnet.cmd", windows_launcher_cmd("testnet"))
        write_text_file(release_root / "atho-regnet.cmd", windows_launcher_cmd("regnet"))
    else:
        write_text_file(release_root / "atho", unix_launcher_script("mainnet"))
        write_text_file(release_root / "atho-mainnet", unix_launcher_script("mainnet"))
        write_text_file(release_root / "atho-testnet", unix_launcher_script("testnet"))
        write_text_file(release_root / "atho-regnet", unix_launcher_script("regnet"))
        if platform_name == "macos":
            write_text_file(release_root / "Atho.command", macos_command_launcher())

    if platform_name == "linux":
        write_text_file(release_root / "Atho.desktop", linux_desktop_entry())
        if installer_artifact_name(platform_name) == "Atho Setup":
            write_text_file(release_root / installer_desktop_entry_name(), installer_desktop_entry())

    for name, src in DOC_FILES:
        if src is None:
            continue
        copy_file(src, release_root / name)

    if include_dev_tools:
        dev_tools_dir = release_root / "dev-tools"
        dev_tools_dir.mkdir(parents=True, exist_ok=True)
        for target_name in ("atho-attack", "atho-adversarial"):
            copy_binary(target_name, dev_tools_dir / f"{target_name}{binary_suffix}")

    copy_text_file(release_root / "INSTALL.md", install_md(version, platform_name, arch))
    copy_text_file(release_root / install_script_name(platform_name), install_script(platform_name))
    copy_text_file(release_root / uninstall_script_name(platform_name), uninstall_script(platform_name))

    if platform_name != "windows":
        make_executable(release_root / "atho")
        make_executable(release_root / "atho-mainnet")
        make_executable(release_root / "atho-testnet")
        make_executable(release_root / "atho-regnet")
        make_executable(release_root / install_script_name(platform_name))
        make_executable(release_root / uninstall_script_name(platform_name))
        if platform_name == "macos":
            make_executable(release_root / "Atho.command")
    else:
        make_executable(release_root / install_script_name(platform_name))
        make_executable(release_root / uninstall_script_name(platform_name))


def stage_download_artifacts(release_root: Path, platform_name: str) -> None:
    payload_path = create_installer_payload(release_root, platform_name)
    try:
        if platform_name == "windows":
            append_payload_to_windows_installer(release_root / "Atho Setup.exe", payload_path)
            installers_dir = release_root / INSTALLERS_DIR_NAME
            installers_dir.mkdir(parents=True, exist_ok=True)
            copy_file(release_root / "Atho Setup.exe", installers_dir / "Atho Setup.exe")
        elif platform_name == "macos":
            app_root = release_root / "Atho Setup.app"
            installers_dir = release_root / INSTALLERS_DIR_NAME
            installers_dir.mkdir(parents=True, exist_ok=True)
            copy_file(payload_path, app_root / "Contents" / "Resources" / "payload.zip")
            (app_root / "Contents" / "Resources" / "payload.sha256").write_bytes(
                hashlib.sha256(payload_path.read_bytes()).digest()
            )
            build_macos_dmg(app_root, installers_dir / "Atho Setup.dmg")
    finally:
        remove_path(payload_path)


def create_installer_payload(release_root: Path, platform_name: str) -> Path:
    with tempfile.NamedTemporaryFile(delete=False, suffix=".zip", prefix="atho-payload-") as tmp:
        payload_path = Path(tmp.name)
    with zipfile.ZipFile(payload_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in iter_payload_files(release_root, platform_name):
            archive.write(path, arcname=path.relative_to(release_root).as_posix())
    return payload_path


def iter_payload_files(root: Path, platform_name: str):
    installer_name = installer_artifact_name(platform_name)
    installer_root = Path(installer_name)
    for path in sorted(root.rglob("*")):
        if path.is_dir():
            continue
        relative = path.relative_to(root)
        if relative.parts and relative.parts[0] == INSTALLERS_DIR_NAME:
            continue
        if relative.parts and relative.parts[0] == "archives":
            continue
        if relative == installer_root or relative.is_relative_to(installer_root):
            continue
        yield path


def append_payload_to_windows_installer(installer_path: Path, payload_path: Path) -> None:
    payload = payload_path.read_bytes()
    footer = struct.pack(
        f"<8sQ{PAYLOAD_DIGEST_BYTES}s",
        PAYLOAD_FOOTER_MAGIC,
        len(payload),
        hashlib.sha256(payload).digest(),
    )
    with installer_path.open("ab") as handle:
        handle.write(payload)
        handle.write(footer)


def build_macos_dmg(app_root: Path, dmg_path: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="atho-dmg-") as temp_dir:
        staging_dir = Path(temp_dir) / "staging"
        staging_dir.mkdir(parents=True, exist_ok=True)
        shutil.copytree(app_root, staging_dir / app_root.name)
        applications_link = staging_dir / "Applications"
        if not applications_link.exists():
            os.symlink("/Applications", applications_link)
        run([
            "hdiutil",
            "create",
            "-volname",
            "Atho Setup",
            "-srcfolder",
            str(staging_dir),
            "-ov",
            "-format",
            "UDZO",
            str(dmg_path),
        ])


def stage_installer_frontend(release_root: Path, version: str, platform_name: str, arch: str) -> None:
    installer_binary = ROOT / "target" / "release" / f"{installer_source_name()}{exe_suffix()}"
    if not installer_binary.exists():
        raise FileNotFoundError(f"missing installer binary: {installer_binary}")

    artifact_name = installer_artifact_name(platform_name)
    if platform_name == "macos":
        app_root = release_root / artifact_name
        contents_root = app_root / "Contents"
        macos_root = contents_root / "MacOS"
        macos_root.mkdir(parents=True, exist_ok=True)
        copy_binary(installer_source_name(), macos_root / "Atho Setup")
        write_text_file(contents_root / "Info.plist", macos_info_plist(version, arch))
        make_executable(macos_root / "Atho Setup")
        return

    if platform_name == "linux":
        copy_binary(installer_source_name(), release_root / artifact_name)
        make_executable(release_root / artifact_name)
        write_text_file(release_root / installer_desktop_entry_name(), installer_desktop_entry())
        return

    copy_binary(installer_source_name(), release_root / artifact_name)
    make_executable(release_root / artifact_name)


def copy_binary(source_name: str, destination: Path) -> None:
    source = ROOT / "target" / "release" / f"{source_name}{exe_suffix()}"
    if not source.exists():
        raise FileNotFoundError(f"missing release binary: {source}")
    copy_file(source, destination)
    if destination.suffix != ".exe":
        make_executable(destination)


def copy_file(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def copy_text_file(destination: Path, text: str) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    write_text_file(destination, text)


def write_text_file(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    newline = "\r\n" if path.suffix in {".cmd", ".ps1", ".bat"} else "\n"
    with path.open("w", encoding="utf-8", newline=newline) as handle:
        handle.write(text)


def make_executable(path: Path) -> None:
    mode = path.stat().st_mode
    path.chmod(mode | 0o755)


def run(command: list[str]) -> None:
    print("+ " + " ".join(command))
    subprocess.run(command, cwd=ROOT, check=True)


def unix_launcher_script(network: str) -> str:
    peer_line = ""
    if network == "mainnet":
        peer_line = f'  args+=(--peer "${{ATHO_MAINNET_PEER:-{BOOTSTRAP_PEER}}}")\n'
    return f"""#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${{BASH_SOURCE[0]}}")" && pwd)"
args=(--network {network} --local-node)
{peer_line}exec "$script_dir/atho-qt" "${{args[@]}}" "$@"
"""


def windows_launcher_cmd(network: str) -> str:
    peer_block = ""
    if network == "mainnet":
        peer_block = (
            'if "%PEER_ARG%"=="" set "PEER_ARG=--peer %ATHO_MAINNET_PEER%"\n'
            f'if "%ATHO_MAINNET_PEER%"=="" set "PEER_ARG=--peer {BOOTSTRAP_PEER}"\n'
        )
    return f"""@echo off
setlocal EnableDelayedExpansion
set "NETWORK={network}"
if not "%ATHO_NETWORK%"=="" set "NETWORK=%ATHO_NETWORK%"
set "PEER_ARG="
{peer_block}set "SCRIPT_DIR=%~dp0"
"%SCRIPT_DIR%atho-qt.exe" --network %NETWORK% --local-node %PEER_ARG% %*
endlocal
"""


def macos_command_launcher() -> str:
    return """#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec "$script_dir/atho" "$@"
"""


def linux_desktop_entry() -> str:
    return """[Desktop Entry]
Type=Application
Name=Atho
Comment=Atho desktop wallet and client
Exec=atho
Icon=Atho.png
Terminal=false
Categories=Finance;Network;
StartupNotify=true
"""


def installer_desktop_entry() -> str:
    return """[Desktop Entry]
Type=Application
Name=Atho Setup
Comment=Atho release installer
Exec=./Atho Setup
Terminal=false
Categories=Finance;Utility;
StartupNotify=true
"""


def macos_info_plist(version: str, arch: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>Atho Setup</string>
  <key>CFBundleExecutable</key>
  <string>Atho Setup</string>
  <key>CFBundleIdentifier</key>
  <string>io.atho.setup</string>
  <key>CFBundleName</key>
  <string>Atho Setup</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundleVersion</key>
  <string>{version}</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
</dict>
</plist>
"""


def install_md(version: str, platform_name: str, arch: str) -> str:
    release_tag = f"{platform_name}-{arch}"
    install_name = install_script_name(platform_name)
    uninstall_name = uninstall_script_name(platform_name)
    launcher_list = ", ".join(launcher_names(platform_name))
    installer_name = installer_artifact_name(platform_name)
    if platform_name == "windows":
        install_command = ".\\Atho Setup.exe"
        install_prefix = r"%LOCALAPPDATA%\Programs\Atho"
        bin_dir = r"%LOCALAPPDATA%\Programs\Atho"
        launcher_hint = "double-click `Atho` from the Start Menu shortcut or run `atho.cmd`"
        installer_hint = "double-click `Atho Setup.exe`"
    elif platform_name == "macos":
        install_command = 'open "Atho Setup.app"'
        install_prefix = "~/Applications/Atho"
        bin_dir = "~/bin"
        launcher_hint = "double-click `Atho.command` or run `atho`"
        installer_hint = "double-click `Atho Setup.app`"
    else:
        install_command = "./Atho Setup"
        install_prefix = "~/.local/share/Atho"
        bin_dir = "~/.local/bin"
        launcher_hint = "run `atho` for the GUI, `athod` for the node, or `atho-mine` for mining"
        installer_hint = "run `./Atho Setup` or open `Atho Setup.desktop`"

    return f"""Atho Installer Notes
======================

Release version: {version}
Platform: {platform_name}
Architecture: {arch}
Release directory: dist/releases/{version}/{release_tag}
Compatibility mirror: dist/release

What is packaged
-----------------
- `athod`
- `atho-mine`
- `atho-qt`
- `atho-address`
- `atho` launcher
- installer front-end: `{installer_name}`
- quickstart and operator docs
- install and uninstall helpers
- checksums and manifest

Primary installer
-----------------
- {installer_hint}
- verify the matching `checksums.sha256` file from the release before running the installer
- the Windows and macOS installers verify their embedded payload checksums before install
- on Windows, the installer asks where to install and creates a Start Menu shortcut directly to `atho-qt.exe`

Default install locations
-------------------------
- application files: {install_prefix}
- command symlinks or PATH entries: {bin_dir}

Launcher commands
-----------------
- client: `atho`
- full node: `athod`
- miner: `atho-mine`
- address helper: `atho-address`
- network-specific client launchers: {launcher_list}

Network overrides
-----------------
- set `ATHO_NETWORK=testnet` or `ATHO_NETWORK=regnet` to switch networks
- set `ATHO_MAINNET_PEER={BOOTSTRAP_PEER}` to override the default mainnet bootstrap peer

Install
-------
Use:

```bash
{install_command}
```

or run the platform install helper directly from this folder:

- install: `{install_name}`
- uninstall: `{uninstall_name}`

Update flow
-----------
Rerun the same release command with a new version number:

```bash
python3 scripts/release.py --version {version} --clean
```

The release script writes a versioned bundle under `dist/releases/{version}/{release_tag}` and refreshes `dist/release` as the current mirror.

Expected first run
------------------
- the GUI opens with the `atho` launcher
- the client starts a managed local node on the selected network
- mainnet uses the bootstrap peer until DNS seeds are added
- `athod` and `atho-mine` remain available as direct commands

Launch hint
-----------
{launcher_hint}
"""


def install_script(platform_name: str) -> str:
    if platform_name == "windows":
        return windows_install_script()
    return unix_install_script(platform_name)


def uninstall_script(platform_name: str) -> str:
    if platform_name == "windows":
        return windows_uninstall_script()
    return unix_uninstall_script(platform_name)


def unix_install_script(platform_name: str) -> str:
    if platform_name == "macos":
        default_app_dir = "${HOME}/Applications/Atho"
        default_bin_dir = "${HOME}/bin"
        desktop_file = ""
    else:
        default_app_dir = "${HOME}/.local/share/Atho"
        default_bin_dir = "${HOME}/.local/bin"
        desktop_file = """cat > "${desktop_dir}/Atho.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Atho
Comment=Atho desktop wallet and client
Exec=${bin_dir}/atho
Icon=${app_dir}/Atho.png
Terminal=false
Categories=Finance;Network;
StartupNotify=true
EOF
"""

    macos_command_chmod = 'chmod +x "$app_dir"/Atho.command\n' if platform_name == "macos" else ""

    return f"""#!/usr/bin/env bash
set -euo pipefail

source_dir="$(cd -- "$(dirname -- "${{BASH_SOURCE[0]}}")" && pwd)"
app_dir="${{ATHO_INSTALL_DIR:-{default_app_dir}}}"
bin_dir="${{ATHO_BIN_DIR:-{default_bin_dir}}}"
desktop_dir="${{XDG_DATA_HOME:-$HOME/.local/share}}/applications"

mkdir -p "$app_dir" "$bin_dir"
cp -R "$source_dir"/. "$app_dir"/
rm -rf "$app_dir/archives"
rm -rf "$app_dir/Atho Setup.app" "$app_dir/Atho Setup"
rm -f "$app_dir/Atho Setup.command" "$app_dir/Atho Setup.desktop" "$app_dir/Atho Setup.exe"
mkdir -p "$desktop_dir"

chmod +x "$app_dir"/atho "$app_dir"/atho-mainnet "$app_dir"/atho-testnet "$app_dir"/atho-regnet
chmod +x "$app_dir"/install.sh "$app_dir"/uninstall.sh
{macos_command_chmod}

ln -sf "$app_dir/atho" "$bin_dir/atho"
ln -sf "$app_dir/atho-mainnet" "$bin_dir/atho-mainnet"
ln -sf "$app_dir/atho-testnet" "$bin_dir/atho-testnet"
ln -sf "$app_dir/atho-regnet" "$bin_dir/atho-regnet"
ln -sf "$app_dir/athod" "$bin_dir/athod"
ln -sf "$app_dir/atho-mine" "$bin_dir/atho-mine"
ln -sf "$app_dir/atho-address" "$bin_dir/atho-address"

{desktop_file}

echo "Atho installed to: $app_dir"
echo "Commands linked in: $bin_dir"
"""


def unix_uninstall_script(platform_name: str) -> str:
    if platform_name == "macos":
        default_app_dir = "${HOME}/Applications/Atho"
        default_bin_dir = "${HOME}/bin"
    else:
        default_app_dir = "${HOME}/.local/share/Atho"
        default_bin_dir = "${HOME}/.local/bin"
    return f"""#!/usr/bin/env bash
set -euo pipefail

app_dir="${{ATHO_INSTALL_DIR:-{default_app_dir}}}"
bin_dir="${{ATHO_BIN_DIR:-{default_bin_dir}}}"
desktop_dir="${{XDG_DATA_HOME:-$HOME/.local/share}}/applications"

rm -f "$bin_dir/atho" "$bin_dir/atho-mainnet" "$bin_dir/atho-testnet" "$bin_dir/atho-regnet"
rm -f "$bin_dir/athod" "$bin_dir/atho-mine" "$bin_dir/atho-address"
rm -f "$desktop_dir/Atho.desktop"
rm -rf "$app_dir"

echo "Atho removed from: $app_dir"
"""


def windows_install_script() -> str:
    return r"""param(
  [string]$Destination = "$env:LOCALAPPDATA\Programs\Atho",
  [switch]$NoShortcut
)

$ErrorActionPreference = 'Stop'
$sourceDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$destination = [Environment]::ExpandEnvironmentVariables($Destination)

New-Item -ItemType Directory -Force -Path $destination | Out-Null
Copy-Item -Path (Join-Path $sourceDir '*') -Destination $destination -Recurse -Force
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $destination 'archives')
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $destination 'Atho Setup.app')
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $destination 'Atho Setup')
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $destination 'Atho Setup.exe')
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $destination 'Atho Setup.command')

$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Atho'
New-Item -ItemType Directory -Force -Path $startMenu | Out-Null

if (-not $NoShortcut) {
  $wsh = New-Object -ComObject WScript.Shell
  $shortcut = $wsh.CreateShortcut((Join-Path $startMenu 'Atho.lnk'))
  $shortcut.TargetPath = (Join-Path $destination 'atho-qt.exe')
  $shortcut.Arguments = '--local-node'
  $shortcut.WorkingDirectory = $destination
  $shortcut.Save()
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ([string]::IsNullOrWhiteSpace($userPath)) {
  [Environment]::SetEnvironmentVariable('Path', $destination, 'User')
} elseif ($userPath -notlike "*$destination*") {
  [Environment]::SetEnvironmentVariable('Path', "$userPath;$destination", 'User')
}

Write-Host "Atho installed to: $destination"
Write-Host "Start Menu shortcut: $startMenu\Atho.lnk"
Write-Host "Launcher target: atho-qt.exe --local-node"
"""


def windows_uninstall_script() -> str:
    return r"""param(
  [string]$Destination = "$env:LOCALAPPDATA\Programs\Atho"
)

$ErrorActionPreference = 'Stop'
$destination = [Environment]::ExpandEnvironmentVariables($Destination)
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Atho'

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $destination
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $startMenu

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -and $userPath -like "*$destination*") {
  $segments = $userPath.Split(';') | Where-Object { $_ -and $_ -ne $destination }
  [Environment]::SetEnvironmentVariable('Path', ($segments -join ';'), 'User')
}

Write-Host "Atho removed from: $destination"
"""


def build_archive(source_dir: Path, archive_path: Path, arc_prefix: str) -> None:
    suffixes = archive_path.suffixes
    if suffixes[-2:] == [".tar", ".gz"]:
        with tarfile.open(archive_path, "w:gz") as archive:
            for path in iter_release_files(source_dir):
                archive.add(path, arcname=f"{arc_prefix}/{path.relative_to(source_dir).as_posix()}")
    elif archive_path.suffix == ".zip":
        with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for path in iter_release_files(source_dir):
                archive.write(path, arcname=f"{arc_prefix}/{path.relative_to(source_dir).as_posix()}")
    else:
        raise RuntimeError(f"unsupported archive type: {archive_path}")


def iter_release_files(root: Path):
    for path in sorted(root.rglob("*")):
        if path.is_dir():
            continue
        if path.relative_to(root).parts and path.relative_to(root).parts[0] == INSTALLERS_DIR_NAME:
            continue
        if path.parent.name == "archives" and path.name.endswith((".tar.gz", ".zip")):
            if path.name.startswith("Atho-"):
                yield path
            continue
        if path.name == "checksums.sha256":
            continue
        yield path


def build_manifest(
    *,
    release_root: Path,
    version: str,
    platform: str,
    arch: str,
    archive_path: Path,
) -> dict[str, object]:
    files = [
        path.relative_to(release_root).as_posix()
        for path in sorted(release_root.rglob("*"))
        if path.is_file() and path.name != "checksums.sha256"
    ]
    return {
        "project": "Atho",
        "version": version,
        "platform": platform,
        "architecture": arch,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "release_root": str(release_root.relative_to(ROOT).as_posix()),
        "archive": {
            "path": str(archive_path.relative_to(ROOT).as_posix()),
            "sha256": sha256_file(archive_path),
        },
        "artifacts": files,
        "launcher_commands": launcher_names(platform),
        "bootstrap_peer": BOOTSTRAP_PEER,
    }


def write_checksums(root: Path) -> None:
    lines: list[str] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        if path.name == "checksums.sha256":
            continue
        lines.append(f"{sha256_file(path)}  {path.relative_to(root).as_posix()}")
    write_text_file(root / "checksums.sha256", "\n".join(lines) + "\n")


def write_desktop_readme(version: str, platform_name: str, arch: str) -> None:
    DESKTOP_ROOT.mkdir(parents=True, exist_ok=True)
    readme = f"""# Atho Desktop Releases

This folder is the shareable desktop release tree.

Current release version:
- {version}

Layout:
- `desktop/releases/<version>/<platform>-<arch>/`
- `desktop/latest/<platform>-<arch>/` as the active bundle mirror
- `dist/releases/<version>/<platform>-<arch>/installers/` for direct installer downloads
- direct installer downloads:
  - Windows: `Atho Setup.exe`
  - macOS: `Atho Setup.dmg`
- verify the matching `checksums.sha256` file from the same release before running the installer
- each direct installer validates its embedded payload checksum before install
- on Windows, the installer asks where to install and creates a Start Menu shortcut directly to `atho-qt.exe`
- `desktop/install.sh` and `desktop/install.ps1` dispatch to the active bundle
- `desktop/uninstall.sh` and `desktop/uninstall.ps1` remove the active install
- each active bundle includes the native installer front-end:
  - Windows: `Atho Setup.exe`
  - macOS: `Atho Setup.app`
  - Linux: `Atho Setup`

How to use:
- open the root `desktop/` folder
- download the combined `Atho-<version>-desktop.zip` release if you want every platform bundle in one file
- on GitHub Releases, prefer the direct installer asset from `dist/releases/<version>/<platform>-<arch>/installers/` when it is available
- run `install.sh` on Linux or macOS, or launch the native installer directly from the bundle
- run `install.ps1` on Windows, or launch `Atho Setup.exe` directly from the bundle
- keep all platform bundles in this folder if you are assembling a multi-OS release set

The packager writes the current host bundle into `dist/releases/...`, `desktop/releases/...`, and `desktop/latest/...`.
"""
    write_text_file(DESKTOP_ROOT / "README.md", readme)
    write_text_file(DESKTOP_ROOT / "install.sh", desktop_install_sh())
    write_text_file(DESKTOP_ROOT / "uninstall.sh", desktop_uninstall_sh())
    write_text_file(DESKTOP_ROOT / "install.ps1", desktop_install_ps1())
    write_text_file(DESKTOP_ROOT / "uninstall.ps1", desktop_uninstall_ps1())
    make_executable(DESKTOP_ROOT / "install.sh")
    make_executable(DESKTOP_ROOT / "uninstall.sh")


def desktop_install_sh() -> str:
    return """#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
platform="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m | tr '[:upper:]' '[:lower:]')"

case "$platform" in
  darwin*) os="macos" ;;
  linux*) os="linux" ;;
  *) echo "Unsupported platform: $platform" >&2; exit 1 ;;
esac

case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="arm64" ;;
esac

bundle_dir="$desktop_dir/latest/${os}-${arch}"
if [ "$os" = "macos" ] && [ -d "$bundle_dir/Atho Setup.app" ]; then
  open "$bundle_dir/Atho Setup.app"
  exit $?
fi

if [ -x "$bundle_dir/Atho Setup" ]; then
  exec "$bundle_dir/Atho Setup" "$@"
fi

if [ ! -x "$bundle_dir/install.sh" ]; then
  echo "No active Atho bundle found for ${os}-${arch} under $desktop_dir/latest" >&2
  exit 1
fi

exec "$bundle_dir/install.sh" "$@"
"""


def desktop_uninstall_sh() -> str:
    return """#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
platform="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m | tr '[:upper:]' '[:lower:]')"

case "$platform" in
  darwin*) os="macos" ;;
  linux*) os="linux" ;;
  *) echo "Unsupported platform: $platform" >&2; exit 1 ;;
esac

case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="arm64" ;;
esac

bundle_dir="$desktop_dir/latest/${os}-${arch}"
if [ ! -x "$bundle_dir/uninstall.sh" ]; then
  echo "No active Atho bundle found for ${os}-${arch} under $desktop_dir/latest" >&2
  exit 1
fi

exec "$bundle_dir/uninstall.sh" "$@"
"""


def desktop_install_ps1() -> str:
    return r"""param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Arguments
)

$ErrorActionPreference = 'Stop'
$desktopDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$platform = 'windows'

$archName = $env:PROCESSOR_ARCHITECTURE
if ([string]::IsNullOrWhiteSpace($archName) -and $env:PROCESSOR_ARCHITEW6432) {
  $archName = $env:PROCESSOR_ARCHITEW6432
}
if ([string]::IsNullOrWhiteSpace($archName)) {
  throw 'Unable to determine Windows architecture'
}
$archName = $archName.ToLowerInvariant()
switch ($archName) {
  'amd64' { $arch = 'x86_64' }
  'x86' { throw '32-bit Windows is not supported' }
  'arm64' { $arch = 'arm64' }
  default { throw "Unsupported architecture: $archName" }
}

$bundleDir = Join-Path (Join-Path $desktopDir 'latest') "$platform-$arch"
if (Test-Path (Join-Path $bundleDir 'Atho Setup.exe')) {
  Start-Process -FilePath (Join-Path $bundleDir 'Atho Setup.exe') -WorkingDirectory $bundleDir
  exit 0
}

if (-not (Test-Path (Join-Path $bundleDir 'install.ps1'))) {
  throw "No active Atho bundle found for $platform-$arch under $(Join-Path $desktopDir 'latest')"
}

& (Join-Path $bundleDir 'install.ps1') @Arguments
"""


def desktop_uninstall_ps1() -> str:
    return r"""param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Arguments
)

$ErrorActionPreference = 'Stop'
$desktopDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$platform = 'windows'

$archName = $env:PROCESSOR_ARCHITECTURE
if ([string]::IsNullOrWhiteSpace($archName) -and $env:PROCESSOR_ARCHITEW6432) {
  $archName = $env:PROCESSOR_ARCHITEW6432
}
if ([string]::IsNullOrWhiteSpace($archName)) {
  throw 'Unable to determine Windows architecture'
}
$archName = $archName.ToLowerInvariant()
switch ($archName) {
  'amd64' { $arch = 'x86_64' }
  'x86' { throw '32-bit Windows is not supported' }
  'arm64' { $arch = 'arm64' }
  default { throw "Unsupported architecture: $archName" }
}

$bundleDir = Join-Path (Join-Path $desktopDir 'latest') "$platform-$arch"
if (-not (Test-Path (Join-Path $bundleDir 'uninstall.ps1'))) {
  throw "No active Atho bundle found for $platform-$arch under $(Join-Path $desktopDir 'latest')"
}

& (Join-Path $bundleDir 'uninstall.ps1') @Arguments
"""


def sha256_file(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def remove_path(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
    elif path.is_dir():
        shutil.rmtree(path)


if __name__ == "__main__":
    raise SystemExit(main())
