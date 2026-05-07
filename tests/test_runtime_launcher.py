from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import runtime_launcher


class RuntimeLauncherTests(unittest.TestCase):
    def make_config(self, root: Path, network: str = "mainnet") -> runtime_launcher.LauncherConfig:
        return runtime_launcher.LauncherConfig(
            network=network,
            repo_root=root,
            release_dir=root / "target" / "release",
            runtime_root=root / "runtime",
            cargo_bin="cargo",
            rebuild=False,
            no_build=False,
            dry_run=True,
            forwarded_args=(),
        )

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
                dry_run=True,
                forwarded_args=("--peer", "127.0.0.1:9100"),
            )
            command = runtime_launcher.build_client_command(config)
            self.assertEqual(command[1:4], ["--network", "testnet", "--local-node"])
            self.assertEqual(command[-2:], ["--peer", "127.0.0.1:9100"])

    def test_parse_launcher_args_keeps_unknown_qt_flags(self) -> None:
        config = runtime_launcher.parse_launcher_args(
            "mainnet",
            ["--data-dir", "/tmp/atho", "--peer", "1.2.3.4:56000"],
        )
        self.assertEqual(config.runtime_root, Path("/tmp/atho"))
        self.assertEqual(config.forwarded_args, ("--peer", "1.2.3.4:56000"))

    def test_parse_launcher_args_supports_regnet_wrapper_name(self) -> None:
        config = runtime_launcher.parse_launcher_args(
            "regnet",
            ["--data-dir", "/tmp/atho-regnet"],
        )
        self.assertEqual(config.network, "regnet")
        self.assertEqual(config.runtime_root, Path("/tmp/atho-regnet"))

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
                dry_run=False,
                forwarded_args=config.forwarded_args,
            )
            with mock.patch("runtime_launcher.verify_cargo_available"), mock.patch(
                "runtime_launcher.gpu_build_preflight_reason",
                return_value=None,
            ), mock.patch(
                "runtime_launcher.subprocess.run",
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
                dry_run=False,
                forwarded_args=config.forwarded_args,
            )
            with mock.patch("runtime_launcher.verify_cargo_available"), mock.patch(
                "runtime_launcher.gpu_build_preflight_reason",
                return_value=None,
            ), mock.patch(
                "runtime_launcher.subprocess.run",
                return_value=subprocess.CompletedProcess(
                    args=["cargo"], returncode=0, stdout="", stderr=""
                ),
            ) as run_mock:
                runtime_launcher.build_release_binaries(config)

            self.assertEqual(run_mock.call_count, 1)
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
                dry_run=False,
                forwarded_args=config.forwarded_args,
            )
            with mock.patch("runtime_launcher.verify_cargo_available"), mock.patch(
                "runtime_launcher.gpu_build_preflight_reason",
                return_value="OpenCL headers not found (missing CL/cl.h)",
            ), mock.patch(
                "runtime_launcher.subprocess.run",
                return_value=subprocess.CompletedProcess(
                    args=["cargo"], returncode=0, stdout="", stderr=""
                ),
            ) as run_mock:
                runtime_launcher.build_release_binaries(config)

            self.assertEqual(run_mock.call_count, 1)
            self.assertEqual(config.gpu_build_stamp.read_text(encoding="utf-8"), "cpu-only\n")


if __name__ == "__main__":
    unittest.main()
