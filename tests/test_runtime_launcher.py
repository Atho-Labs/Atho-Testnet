# SPDX-License-Identifier: Apache-2.0
# Copyright (c) Atho contributors

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

from scripts import runtime_launcher


REPO_ROOT = Path(__file__).resolve().parents[1]


class RuntimeLauncherTests(unittest.TestCase):
    def make_config(
        self, root: Path, network: str = "testnet"
    ) -> runtime_launcher.LauncherConfig:
        return runtime_launcher.LauncherConfig(
            network=network,
            repo_root=root,
            release_dir=root / "target" / "release",
            runtime_root=root / "runtime",
            cargo_bin="cargo",
            rebuild=False,
            no_build=False,
            network_overrides_local=False,
            dry_run=True,
            forwarded_args=(),
        )

    def make_release_dir(self, root: Path) -> Path:
        release_dir = root / "release"
        release_dir.mkdir(parents=True)
        for name in ("atho-qt", "athod", "atho-mine"):
            binary = release_dir / runtime_launcher.binary_name(name)
            binary.write_text("bin", encoding="utf-8")
            if os.name != "nt":
                binary.chmod(0o755)
        stamp = release_dir / runtime_launcher.BUILD_STAMP
        stamp.write_text("gpu-native\n", encoding="utf-8")
        return release_dir

    def test_default_runtime_root_honors_environment_override(self) -> None:
        with mock.patch.dict(os.environ, {"ATHO_DATA_DIR": "/tmp/atho-launch-root"}, clear=False):
            self.assertEqual(
                runtime_launcher.default_runtime_root(),
                Path("/tmp/atho-launch-root"),
            )

    def test_build_reason_reports_missing_binaries(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "crates").mkdir()
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            config = self.make_config(root)
            self.assertIn("missing", runtime_launcher.build_reason(config) or "")

    def test_build_reason_detects_stale_binaries(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            release_dir = root / "target" / "release"
            release_dir.mkdir(parents=True)
            (root / "crates").mkdir()
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            qt_binary = release_dir / runtime_launcher.binary_name("atho-qt")
            node_binary = release_dir / runtime_launcher.binary_name("athod")
            miner_binary = release_dir / runtime_launcher.binary_name("atho-mine")
            qt_binary.write_text("bin", encoding="utf-8")
            node_binary.write_text("bin", encoding="utf-8")
            miner_binary.write_text("bin", encoding="utf-8")
            if os.name != "nt":
                qt_binary.chmod(0o755)
                node_binary.chmod(0o755)
                miner_binary.chmod(0o755)
            old_time = time.time() - 10
            os.utime(qt_binary, (old_time, old_time))
            os.utime(node_binary, (old_time, old_time))
            os.utime(miner_binary, (old_time, old_time))
            stamp = release_dir / runtime_launcher.BUILD_STAMP
            stamp.write_text("gpu-native\n", encoding="utf-8")
            os.utime(stamp, (old_time, old_time))
            source_file = root / "crates" / "fresh.rs"
            source_file.write_text("fn main() {}\n", encoding="utf-8")
            config = self.make_config(root)
            self.assertEqual(
                runtime_launcher.build_reason(config),
                "source tree is newer than release binaries",
            )

    def test_build_reason_requires_gpu_build_stamp(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            release_dir = root / "target" / "release"
            release_dir.mkdir(parents=True)
            (root / "crates").mkdir()
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            for name in ("atho-qt", "athod", "atho-mine"):
                binary = release_dir / runtime_launcher.binary_name(name)
                binary.write_text("bin", encoding="utf-8")
                if os.name != "nt":
                    binary.chmod(0o755)
            config = self.make_config(root)
            self.assertEqual(
                runtime_launcher.build_reason(config),
                "missing launcher build stamp",
            )

    def test_build_client_command_uses_managed_local_node(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            config = runtime_launcher.LauncherConfig(
                network="testnet",
                repo_root=root,
                release_dir=root / "target" / "release",
                runtime_root=root / "runtime",
                cargo_bin="cargo",
                rebuild=False,
                no_build=False,
                network_overrides_local=False,
                dry_run=True,
                forwarded_args=("--peer", "127.0.0.1:9100"),
            )
            command = runtime_launcher.build_client_command(config)
            self.assertEqual(command[1:4], ["--network", "testnet", "--local-node"])
            self.assertEqual(command[-2:], ["--peer", "127.0.0.1:9100"])

    def test_build_launch_env_sets_active_network_and_runtime_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            config = self.make_config(root)
            env = runtime_launcher.build_launch_env(config)
            self.assertEqual(env["ATHO_NETWORK"], "testnet")
            self.assertEqual(env["ATHO_DATA_DIR"], str(root / "runtime"))

    def test_parse_launcher_args_keeps_unknown_qt_flags(self) -> None:
        config = runtime_launcher.parse_launcher_args(
            "testnet",
            ["--data-dir", "/tmp/atho", "--peer", "1.2.3.4:9100"],
        )
        self.assertEqual(config.runtime_root, Path("/tmp/atho"))
        self.assertEqual(config.network, "testnet")
        self.assertEqual(config.forwarded_args, ("--peer", "1.2.3.4:9100"))

    def test_network_override_flag_sets_launch_env(self) -> None:
        config = runtime_launcher.parse_launcher_args(
            "testnet",
            ["--data-dir", "/tmp/atho", "--network-overrides-local"],
        )
        env = runtime_launcher.build_launch_env(config)
        self.assertTrue(config.network_overrides_local)
        self.assertEqual(env["ATHO_NETWORK_OVERRIDES_LOCAL"], "1")

    def test_parse_launcher_args_rejects_network_override(self) -> None:
        with self.assertRaisesRegex(runtime_launcher.LauncherError, "always launches testnet"):
            runtime_launcher.parse_launcher_args("testnet", ["atho-testnet"])

        with self.assertRaisesRegex(runtime_launcher.LauncherError, "owns the testnet network"):
            runtime_launcher.parse_launcher_args("testnet", ["--network", "testnet"])

        with self.assertRaisesRegex(runtime_launcher.LauncherError, "owns the testnet network"):
            runtime_launcher.parse_launcher_args("testnet", ["--network=testnet"])

    def test_startup_scripts_smoke(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            release_dir = self.make_release_dir(root)
            data_dir = root / "testnet-data"
            result = subprocess.run(
                [
                    sys.executable,
                    "-B",
                    str(REPO_ROOT / "runtestnet.py"),
                    "--dry-run",
                    "--release-dir",
                    str(release_dir),
                    "--data-dir",
                    str(data_dir),
                ],
                cwd=REPO_ROOT,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=(
                    f"runtestnet.py failed\nstdout:\n{result.stdout}\n"
                    f"stderr:\n{result.stderr}"
                ),
            )
            self.assertIn("[atho-launch] network=testnet", result.stdout)
            self.assertIn("[atho-launch] env ATHO_NETWORK=testnet", result.stdout)
            self.assertIn("--network testnet --local-node", result.stdout)
            self.assertTrue((data_dir / "db").is_dir())
            self.assertTrue((data_dir / "logs").is_dir())

    def test_public_branch_contains_only_testnet_launcher(self) -> None:
        self.assertTrue((REPO_ROOT / "runtestnet.py").is_file())
        self.assertFalse((REPO_ROOT / "runmainnet.py").exists())
        self.assertFalse((REPO_ROOT / "runregnet.py").exists())

        with self.assertRaisesRegex(
            runtime_launcher.LauncherError,
            "supported launchers are testnet",
        ):
            runtime_launcher.parse_launcher_args("mainnet", ["--dry-run"])

        with self.assertRaisesRegex(
            runtime_launcher.LauncherError,
            "supported launchers are testnet",
        ):
            runtime_launcher.parse_launcher_args("regnet", ["--dry-run"])

    def test_startup_scripts_work_outside_repo_cwd(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            release_dir = self.make_release_dir(root)
            outside_cwd = root / "outside"
            outside_cwd.mkdir()
            data_dir = root / "external-testnet-data"
            result = subprocess.run(
                [
                    sys.executable,
                    "-B",
                    str(REPO_ROOT / "runtestnet.py"),
                    "--dry-run",
                    "--release-dir",
                    str(release_dir),
                    "--data-dir",
                    str(data_dir),
                ],
                cwd=outside_cwd,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, msg=result.stderr)
            self.assertIn("[atho-launch] network=testnet", result.stdout)

    def test_gpu_build_help_mentions_host_requirements(self) -> None:
        message = runtime_launcher.gpu_build_help()
        self.assertTrue(message)
        if sys.platform == "darwin":
            self.assertIn("Xcode Command Line Tools", message)
        elif os.name == "nt":
            self.assertIn("Visual Studio Build Tools", message)
        else:
            self.assertIn("OpenCL", message)

    def test_build_release_binaries_falls_back_to_cpu_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            release_dir = root / "target" / "release"
            release_dir.mkdir(parents=True)
            config = self.make_config(root)
            config = runtime_launcher.LauncherConfig(
                network=config.network,
                repo_root=config.repo_root,
                release_dir=release_dir,
                runtime_root=config.runtime_root,
                cargo_bin=config.cargo_bin,
                rebuild=config.rebuild,
                no_build=config.no_build,
                network_overrides_local=config.network_overrides_local,
                dry_run=False,
                forwarded_args=config.forwarded_args,
            )
            with mock.patch("scripts.runtime_launcher.verify_cargo_available"), mock.patch(
                "scripts.runtime_launcher.gpu_build_preflight_reason",
                return_value=None,
            ), mock.patch(
                "scripts.runtime_launcher.subprocess.run",
                side_effect=[
                    subprocess.CompletedProcess(
                        args=["cargo"], returncode=1, stdout="", stderr="gpu failed"
                    ),
                    subprocess.CompletedProcess(
                        args=["cargo"], returncode=0, stdout="", stderr=""
                    ),
                ],
            ) as run_mock:
                runtime_launcher.build_release_binaries(config)

            self.assertEqual(run_mock.call_count, 2)
            self.assertFalse(
                any("capture_output" in call.kwargs for call in run_mock.call_args_list)
            )
            self.assertEqual(config.gpu_build_stamp.read_text(encoding="utf-8"), "cpu-only\n")

    def test_build_release_binaries_marks_gpu_native_success(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            release_dir = root / "target" / "release"
            release_dir.mkdir(parents=True)
            config = self.make_config(root)
            config = runtime_launcher.LauncherConfig(
                network=config.network,
                repo_root=config.repo_root,
                release_dir=release_dir,
                runtime_root=config.runtime_root,
                cargo_bin=config.cargo_bin,
                rebuild=config.rebuild,
                no_build=config.no_build,
                network_overrides_local=config.network_overrides_local,
                dry_run=False,
                forwarded_args=config.forwarded_args,
            )
            with mock.patch("scripts.runtime_launcher.verify_cargo_available"), mock.patch(
                "scripts.runtime_launcher.gpu_build_preflight_reason",
                return_value=None,
            ), mock.patch(
                "scripts.runtime_launcher.subprocess.run",
                return_value=subprocess.CompletedProcess(
                    args=["cargo"], returncode=0, stdout="", stderr=""
                ),
            ) as run_mock:
                runtime_launcher.build_release_binaries(config)

            self.assertEqual(run_mock.call_count, 1)
            self.assertNotIn("capture_output", run_mock.call_args.kwargs)
            self.assertEqual(
                config.gpu_build_stamp.read_text(encoding="utf-8"), "gpu-native\n"
            )

    def test_build_release_binaries_skips_gpu_when_prereqs_are_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            release_dir = root / "target" / "release"
            release_dir.mkdir(parents=True)
            config = self.make_config(root)
            config = runtime_launcher.LauncherConfig(
                network=config.network,
                repo_root=config.repo_root,
                release_dir=release_dir,
                runtime_root=config.runtime_root,
                cargo_bin=config.cargo_bin,
                rebuild=config.rebuild,
                no_build=config.no_build,
                network_overrides_local=config.network_overrides_local,
                dry_run=False,
                forwarded_args=config.forwarded_args,
            )
            with mock.patch("scripts.runtime_launcher.verify_cargo_available"), mock.patch(
                "scripts.runtime_launcher.gpu_build_preflight_reason",
                return_value="OpenCL headers not found (missing CL/cl.h)",
            ), mock.patch(
                "scripts.runtime_launcher.subprocess.run",
                return_value=subprocess.CompletedProcess(
                    args=["cargo"], returncode=0, stdout="", stderr=""
                ),
            ) as run_mock:
                runtime_launcher.build_release_binaries(config)

            self.assertEqual(run_mock.call_count, 1)
            self.assertNotIn("capture_output", run_mock.call_args.kwargs)
            self.assertEqual(config.gpu_build_stamp.read_text(encoding="utf-8"), "cpu-only\n")


if __name__ == "__main__":
    unittest.main()
