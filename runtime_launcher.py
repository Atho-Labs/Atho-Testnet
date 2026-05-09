#!/usr/bin/env python3
"""Atho-Testnet startup launcher.

This module is startup-only glue. It never proxies consensus, validation,
networking, wallet, API, explorer, or mining hot paths. The public testnet
entry script verifies/builds the required binaries when needed and replaces
themselves with `atho-qt` in managed-local-node mode.
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
BUILD_STAMP = ".atho-launch-build.stamp"
ENTRY_SCRIPT_NAMES = {
    "testnet": "runtestnet.py",
}
SUPPORTED_ENTRY_NETWORKS = frozenset(ENTRY_SCRIPT_NAMES)
RESERVED_FORWARDED_FLAGS = {
    "--network",
    "-n",
    "--local-node",
    "--embedded-node",
    "--data-dir",
}
NETWORK_TOKENS = {
    "testnet",
    "atho-testnet",
}


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
    network_overrides_local: bool
    dry_run: bool
    forwarded_args: tuple[str, ...]

    @property
    def qt_binary(self) -> Path:
        return self.release_dir / binary_name("atho-qt")

    @property
    def node_binary(self) -> Path:
        return self.release_dir / binary_name("athod")

    @property
    def miner_binary(self) -> Path:
        return self.release_dir / binary_name("atho-mine")

    @property
    def gpu_build_stamp(self) -> Path:
        return self.release_dir / BUILD_STAMP


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


def binary_dir_is_usable(path: Path) -> bool:
    return all(
        binary_is_usable(path / binary_name(name))
        for name in ("atho-qt", "athod", "atho-mine")
    )


def release_binary_dir_is_ready(repo_root: Path, release_dir: Path) -> bool:
    if not binary_dir_is_usable(release_dir):
        return False
    stamp = release_dir / BUILD_STAMP
    if not stamp.is_file():
        return False
    newest_source = latest_source_mtime(repo_root)
    oldest_release_file = min(
        (release_dir / binary_name("atho-qt")).stat().st_mtime,
        (release_dir / binary_name("athod")).stat().st_mtime,
        (release_dir / binary_name("atho-mine")).stat().st_mtime,
        stamp.stat().st_mtime,
    )
    return newest_source <= oldest_release_file


def default_binary_dir(repo_root: Path) -> Path:
    release_dir = repo_root / "target" / "release"
    debug_dir = repo_root / "target" / "debug"
    if release_binary_dir_is_ready(repo_root, release_dir):
        return release_dir
    if binary_dir_is_usable(debug_dir):
        return debug_dir
    return release_dir


def normalize_entry_network(network: str) -> str:
    network = network.strip().lower()
    if network not in SUPPORTED_ENTRY_NETWORKS:
        raise LauncherError(
            f"unsupported launcher network {network!r}; Atho-Testnet only launches testnet"
        )
    return network


def parse_launcher_args(
    network: str,
    argv: Sequence[str] | None = None,
    *,
    prog: str | None = None,
) -> LauncherConfig:
    network = normalize_entry_network(network)
    repo_root = Path(__file__).resolve().parent
    wrapper_name = prog or ENTRY_SCRIPT_NAMES[network]
    parser = argparse.ArgumentParser(
        prog=wrapper_name,
        allow_abbrev=False,
        description=(
            f"Build if needed and launch Atho {network} with the desktop client "
            "and managed local node."
        ),
    )
    parser.add_argument(
        "--data-dir",
        help="Override ATHO_DATA_DIR for the launched client and managed node.",
    )
    parser.add_argument(
        "--release-dir",
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
        "--network-overrides-local",
        action="store_true",
        help=(
            "Force the next launch to discard local chain databases before syncing, "
            "so network state wins over local state. Wallet files are preserved."
        ),
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the resolved build and launch commands without executing them.",
    )
    args, forwarded = parser.parse_known_args(argv)
    validate_forwarded_args(network, wrapper_name, forwarded)
    runtime_root = Path(args.data_dir).expanduser() if args.data_dir else default_runtime_root()
    binary_dir = (
        Path(args.release_dir).expanduser()
        if args.release_dir
        else default_binary_dir(repo_root)
    )
    return LauncherConfig(
        network=network,
        repo_root=repo_root,
        release_dir=binary_dir,
        runtime_root=runtime_root,
        cargo_bin=args.cargo,
        rebuild=args.rebuild,
        no_build=args.no_build,
        network_overrides_local=args.network_overrides_local,
        dry_run=args.dry_run,
        forwarded_args=tuple(forwarded),
    )


def validate_forwarded_args(network: str, prog: str, forwarded: Sequence[str]) -> None:
    for value in forwarded:
        flag = value.split("=", 1)[0]
        if flag in RESERVED_FORWARDED_FLAGS:
            raise LauncherError(
                f"{prog} owns the {network} network, data directory, and managed local-node "
                f"mode; remove forwarded argument {value!r} or use the launcher's --data-dir option."
            )
        if value.lower() in NETWORK_TOKENS:
            raise LauncherError(
                f"{prog} always launches {network}; remove forwarded network argument {value!r}."
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
    if not binary_is_usable(config.miner_binary):
        return f"missing {config.miner_binary.name}"
    if config.release_dir.name == "debug":
        return None
    if not config.gpu_build_stamp.is_file():
        return "missing launcher build stamp"
    newest_source = latest_source_mtime(config.repo_root)
    oldest_binary = min(
        config.qt_binary.stat().st_mtime,
        config.node_binary.stat().st_mtime,
        config.miner_binary.stat().st_mtime,
        config.gpu_build_stamp.stat().st_mtime,
    )
    if newest_source > oldest_binary:
        return "source tree is newer than release binaries"
    return None


def verify_cargo_available(cargo_bin: str) -> None:
    if shutil.which(cargo_bin) is None:
        raise LauncherError(
            f"cargo executable not found: {cargo_bin}. Install Rust with rustup or pass --cargo /path/to/cargo."
        )


def gpu_build_help() -> str:
    if sys.platform == "darwin":
        return (
            "GPU-enabled Atho builds on macOS require Xcode Command Line Tools and the system "
            "OpenCL framework. Run `xcode-select --install`, reopen the terminal, and rebuild."
        )
    if os.name == "nt":
        return (
            "GPU-enabled Atho builds on Windows require Visual Studio Build Tools with C++ "
            "support and a working vendor OpenCL SDK/runtime. Install the MSVC C++ build "
            "tools, reopen an x64 developer shell or terminal, and rebuild."
        )
    return (
        "GPU-enabled Atho builds on Linux require a C/C++ toolchain plus OpenCL headers and "
        "runtime libraries. Install a compiler such as `g++` or `clang++`, install "
        "`opencl-headers` and `ocl-icd-opencl-dev` (package names vary by distro), install "
        "your vendor OpenCL ICD/runtime, then rebuild."
    )


def gpu_build_preflight_reason() -> str | None:
    if sys.platform == "darwin" or os.name == "nt":
        return None

    compiler = os.environ.get("CXX")
    if compiler:
        compiler_available = shutil.which(compiler) is not None
        compiler_label = compiler
    else:
        compiler_label = next(
            (candidate for candidate in ("c++", "g++", "clang++") if shutil.which(candidate)),
            "",
        )
        compiler_available = bool(compiler_label)
    if not compiler_available:
        return "no C++ compiler found for gpu-native build"

    include_candidates = [
        Path("/usr/include/CL/cl.h"),
        Path("/usr/local/include/CL/cl.h"),
        Path("/opt/include/CL/cl.h"),
    ]
    if not any(path.is_file() for path in include_candidates):
        return "OpenCL headers not found (missing CL/cl.h)"

    return None


def run_build_command(
    config: LauncherConfig,
    command: list[str],
    description: str,
) -> subprocess.CompletedProcess[str] | None:
    print(f"[atho-launch] {description}: {' '.join(command)}")
    if config.dry_run:
        return None
    return subprocess.run(command, cwd=config.repo_root, text=True)


def write_build_stamp(config: LauncherConfig, mode: str) -> None:
    config.gpu_build_stamp.parent.mkdir(parents=True, exist_ok=True)
    config.gpu_build_stamp.write_text(f"{mode}\n", encoding="utf-8")


def build_release_binaries(config: LauncherConfig) -> None:
    verify_cargo_available(config.cargo_bin)
    if config.release_dir.name == "debug":
        command = [
            config.cargo_bin,
            "build",
            "-p",
            "atho-node",
            "-p",
            "atho-qt",
        ]
        result = run_build_command(
            config,
            command,
            f"building debug binaries for {config.network}",
        )
        if config.dry_run:
            return
        assert result is not None
        if result.returncode != 0:
            raise LauncherError(
                "Atho debug build failed. Fix the Rust/toolchain errors above and rerun the launcher."
            )
        return

    gpu_skip_reason = gpu_build_preflight_reason()
    if gpu_skip_reason is not None:
        print(
            "[atho-launch] GPU-native build skipped. "
            f"{gpu_skip_reason}. Building CPU-only release binaries instead."
        )
        cpu_command = [
            config.cargo_bin,
            "build",
            "--release",
            "-p",
            "atho-node",
            "-p",
            "atho-qt",
        ]
        fallback = run_build_command(
            config,
            cpu_command,
            f"building CPU-only release binaries for {config.network}",
        )
        if config.dry_run:
            return
        assert fallback is not None
        if fallback.returncode != 0:
            raise LauncherError(
                "Atho build failed.\n"
                "The launcher skipped the GPU-native build because the required native "
                "prerequisites were not found, and the CPU-only build also failed. "
                "Fix the Rust/toolchain errors above and rerun the launcher."
            )
        write_build_stamp(config, "cpu-only")
        return

    gpu_command = [
        config.cargo_bin,
        "build",
        "--release",
        "-p",
        "atho-node",
        "-p",
        "atho-qt",
        "--features",
        "gpu-native",
    ]
    result = run_build_command(
        config,
        gpu_command,
        f"building GPU-enabled release binaries for {config.network}",
    )
    if config.dry_run:
        print(
            "[atho-launch] note: if the GPU-native build fails, the launcher will warn and fall back to a CPU-only release build."
        )
        return
    assert result is not None
    if result.returncode == 0:
        write_build_stamp(config, "gpu-native")
        return

    print(
        "[atho-launch] warning: GPU-enabled build failed. "
        f"{gpu_build_help()} Falling back to a CPU-only release build.",
        file=sys.stderr,
    )
    cpu_command = [
        config.cargo_bin,
        "build",
        "--release",
        "-p",
        "atho-node",
        "-p",
        "atho-qt",
    ]
    fallback = run_build_command(
        config,
        cpu_command,
        f"building CPU-only fallback release binaries for {config.network}",
    )
    assert fallback is not None
    if fallback.returncode != 0:
        raise LauncherError(
            "Atho build failed.\n"
            "The GPU-enabled build failed, and the CPU-only fallback build also failed. "
            "Fix the Rust/native toolchain errors above and rerun the launcher."
        )
    write_build_stamp(config, "cpu-only")


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
    if config.network_overrides_local:
        env["ATHO_NETWORK_OVERRIDES_LOCAL"] = "1"
    return env


def ensure_binaries(config: LauncherConfig) -> None:
    reason = build_reason(config)
    if reason is None:
        return
    if config.no_build:
        raise LauncherError(
            "binaries are not ready: "
            f"{reason}. Re-run without --no-build or build with "
            "`cargo build --release -p atho-node -p atho-qt --features gpu-native` "
            "or let the launcher rebuild automatically."
        )
    build_release_binaries(config)
    if config.dry_run:
        return
    if (
        not binary_is_usable(config.qt_binary)
        or not binary_is_usable(config.node_binary)
        or not binary_is_usable(config.miner_binary)
        or (config.release_dir.name != "debug" and not config.gpu_build_stamp.is_file())
    ):
        raise LauncherError(
            "release build finished but required binaries are still missing: "
            f"{config.qt_binary} {config.node_binary} {config.miner_binary}"
        )


def run_launcher(
    network: str,
    argv: Sequence[str] | None = None,
    *,
    prog: str | None = None,
    compatibility_note: str | None = None,
) -> int:
    config = parse_launcher_args(network, argv, prog=prog)
    prepare_runtime_root(config.runtime_root)
    ensure_binaries(config)
    command = build_client_command(config)
    env = build_launch_env(config)
    if compatibility_note:
        print(f"[atho-launch] {compatibility_note}")
    print(f"[atho-launch] network={config.network}")
    print(f"[atho-launch] runtime_root={config.runtime_root}")
    print(f"[atho-launch] qt_binary={config.qt_binary}")
    print(f"[atho-launch] node_binary={config.node_binary}")
    print(f"[atho-launch] env ATHO_NETWORK={env['ATHO_NETWORK']}")
    if config.network_overrides_local:
        print("[atho-launch] network_overrides_local=true")
    print(f"[atho-launch] launching: {' '.join(command)}")
    if config.dry_run:
        return 0
    # Replace the Python wrapper with the Rust desktop client so Python is not
    # in the steady-state runtime path.
    os.execvpe(command[0], command, env)
    return 0


def main(
    network: str,
    argv: Sequence[str] | None = None,
    *,
    prog: str | None = None,
    compatibility_note: str | None = None,
) -> int:
    try:
        return run_launcher(
            network,
            argv,
            prog=prog,
            compatibility_note=compatibility_note,
        )
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
    raise SystemExit(main("testnet", sys.argv[1:], prog="runtime_launcher.py"))
