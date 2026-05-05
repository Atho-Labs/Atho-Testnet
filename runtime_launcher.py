#!/usr/bin/env python3
"""Top-level Atho runtime launcher orchestration.

This module is startup-only glue. It never proxies consensus, validation,
networking, or mining hot paths. After it verifies the environment and builds
the required release binaries when needed, it replaces itself with `atho-qt`
running in managed-local-node mode.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence


SOURCE_PATHS = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "crates",
    "Falcon 512 rs",
)

RUNTIME_DIRS = ("db", "logs", "wallet", "audit", "quarantine")


class LauncherError(RuntimeError):
    """Raised when the launcher cannot safely build or start Atho."""


@dataclass(frozen=True)
class LauncherConfig:
    """Resolved launcher settings for one network."""

    network: str
    repo_root: Path
    release_dir: Path
    runtime_root: Path
    cargo_bin: str
    rebuild: bool
    no_build: bool
    dry_run: bool
    forwarded_args: tuple[str, ...]

    @property
    def qt_binary(self) -> Path:
        return self.release_dir / binary_name("atho-qt")

    @property
    def node_binary(self) -> Path:
        return self.release_dir / binary_name("athod")


def binary_name(base: str) -> str:
    return f"{base}.exe" if os.name == "nt" else base


def default_runtime_root() -> Path:
    override = os.environ.get("ATHO_DATA_DIR")
    if override:
        return Path(override).expanduser()
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "Atho"
    if os.name == "nt":
        appdata = os.environ.get("APPDATA")
        if appdata:
            return Path(appdata) / "Atho"
        return Path.home() / "AppData" / "Roaming" / "Atho"
    xdg_data_home = os.environ.get("XDG_DATA_HOME")
    if xdg_data_home:
        return Path(xdg_data_home) / "Atho"
    return Path.home() / ".local" / "share" / "Atho"


def parse_launcher_args(network: str, argv: Sequence[str] | None = None) -> LauncherConfig:
    repo_root = Path(__file__).resolve().parent
    wrapper_name = {
        "mainnet": "runmainnet.py",
        "testnet": "runtestnet.py",
        "regnet": "runregnet.py",
    }.get(network, f"run{network}.py")
    parser = argparse.ArgumentParser(
        prog=wrapper_name,
        description=f"Build if needed and launch Atho {network} with the desktop client and managed local node.",
    )
    parser.add_argument(
        "--data-dir",
        help="Override ATHO_DATA_DIR for the launched client and managed node.",
    )
    parser.add_argument(
        "--release-dir",
        default=str(repo_root / "target" / "release"),
        help="Directory containing atho-qt and athod release binaries.",
    )
    parser.add_argument(
        "--cargo",
        default=shutil.which("cargo") or "cargo",
        help="Cargo executable to use when a rebuild is required.",
    )
    parser.add_argument(
        "--rebuild",
        action="store_true",
        help="Force a release rebuild before launch.",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Refuse to build missing or stale binaries.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the resolved build and launch commands without executing them.",
    )
    args, forwarded = parser.parse_known_args(argv)
    runtime_root = Path(args.data_dir).expanduser() if args.data_dir else default_runtime_root()
    return LauncherConfig(
        network=network,
        repo_root=repo_root,
        release_dir=Path(args.release_dir).expanduser(),
        runtime_root=runtime_root,
        cargo_bin=args.cargo,
        rebuild=args.rebuild,
        no_build=args.no_build,
        dry_run=args.dry_run,
        forwarded_args=tuple(forwarded),
    )


def iter_source_files(repo_root: Path) -> Iterable[Path]:
    for relative in SOURCE_PATHS:
        path = repo_root / relative
        if not path.exists():
            continue
        if path.is_file():
            yield path
            continue
        for child in path.rglob("*"):
            if child.is_file():
                yield child


def latest_source_mtime(repo_root: Path) -> float:
    latest = 0.0
    for path in iter_source_files(repo_root):
        latest = max(latest, path.stat().st_mtime)
    return latest


def binary_is_usable(path: Path) -> bool:
    if not path.is_file():
        return False
    if os.name == "nt":
        return True
    return os.access(path, os.X_OK)


def build_reason(config: LauncherConfig) -> str | None:
    if config.rebuild:
        return "forced rebuild requested"
    if not binary_is_usable(config.qt_binary):
        return f"missing {config.qt_binary.name}"
    if not binary_is_usable(config.node_binary):
        return f"missing {config.node_binary.name}"
    newest_source = latest_source_mtime(config.repo_root)
    oldest_binary = min(
        config.qt_binary.stat().st_mtime,
        config.node_binary.stat().st_mtime,
    )
    if newest_source > oldest_binary:
        return "source tree is newer than release binaries"
    return None


def verify_cargo_available(cargo_bin: str) -> None:
    if shutil.which(cargo_bin) is None:
        raise LauncherError(
            f"cargo executable not found: {cargo_bin}. Install Rust with rustup or pass --cargo /path/to/cargo."
        )


def build_release_binaries(config: LauncherConfig) -> None:
    verify_cargo_available(config.cargo_bin)
    command = [
        config.cargo_bin,
        "build",
        "--release",
        "-p",
        "atho-node",
        "-p",
        "atho-qt",
    ]
    print(f"[atho-launch] building release binaries for {config.network}: {' '.join(command)}")
    if config.dry_run:
        return
    subprocess.run(command, cwd=config.repo_root, check=True)


def prepare_runtime_root(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    for directory in RUNTIME_DIRS:
        (path / directory).mkdir(exist_ok=True)


def build_client_command(config: LauncherConfig) -> list[str]:
    command = [
        str(config.qt_binary),
        "--network",
        config.network,
        "--local-node",
    ]
    command.extend(config.forwarded_args)
    return command


def build_launch_env(config: LauncherConfig) -> dict[str, str]:
    env = os.environ.copy()
    env["ATHO_DATA_DIR"] = str(config.runtime_root)
    env["ATHO_NETWORK"] = config.network
    return env


def ensure_binaries(config: LauncherConfig) -> None:
    reason = build_reason(config)
    if reason is None:
        return
    if config.no_build:
        raise LauncherError(
            f"release binaries are not ready: {reason}. Re-run without --no-build or build with `cargo build --release -p atho-node -p atho-qt`."
        )
    build_release_binaries(config)
    if not binary_is_usable(config.qt_binary) or not binary_is_usable(config.node_binary):
        raise LauncherError(
            f"release build finished but required binaries are still missing: {config.qt_binary} {config.node_binary}"
        )


def run_launcher(network: str, argv: Sequence[str] | None = None) -> int:
    config = parse_launcher_args(network, argv)
    prepare_runtime_root(config.runtime_root)
    ensure_binaries(config)
    command = build_client_command(config)
    env = build_launch_env(config)
    print(f"[atho-launch] network={config.network}")
    print(f"[atho-launch] runtime_root={config.runtime_root}")
    print(f"[atho-launch] qt_binary={config.qt_binary}")
    print(f"[atho-launch] node_binary={config.node_binary}")
    print(f"[atho-launch] launching: {' '.join(command)}")
    if config.dry_run:
        return 0
    # Replace the Python wrapper with the Rust desktop client so Python is not
    # in the steady-state runtime path.
    os.execvpe(command[0], command, env)
    return 0


def main(network: str, argv: Sequence[str] | None = None) -> int:
    try:
        return run_launcher(network, argv)
    except subprocess.CalledProcessError as exc:
        print(
            f"[atho-launch] build failed with exit code {exc.returncode}. "
            "Fix the Rust build errors above, then re-run the launcher.",
            file=sys.stderr,
        )
        return exc.returncode
    except LauncherError as exc:
        print(f"[atho-launch] {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main("mainnet"))
