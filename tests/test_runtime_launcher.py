from __future__ import annotations

import os
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import runtime_launcher


class RuntimeLauncherTests(unittest.TestCase):
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
            config = runtime_launcher.LauncherConfig(
                network="mainnet",
                repo_root=root,
                release_dir=root / "target" / "release",
                runtime_root=root / "runtime",
                cargo_bin="cargo",
                rebuild=False,
                no_build=False,
                dry_run=True,
                forwarded_args=(),
            )
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
            qt_binary.write_text("bin", encoding="utf-8")
            node_binary.write_text("bin", encoding="utf-8")
            if os.name != "nt":
                qt_binary.chmod(0o755)
                node_binary.chmod(0o755)
            old_time = time.time() - 10
            os.utime(qt_binary, (old_time, old_time))
            os.utime(node_binary, (old_time, old_time))
            source_file = root / "crates" / "fresh.rs"
            source_file.write_text("fn main() {}\n", encoding="utf-8")
            config = runtime_launcher.LauncherConfig(
                network="mainnet",
                repo_root=root,
                release_dir=release_dir,
                runtime_root=root / "runtime",
                cargo_bin="cargo",
                rebuild=False,
                no_build=False,
                dry_run=True,
                forwarded_args=(),
            )
            self.assertEqual(
                runtime_launcher.build_reason(config),
                "source tree is newer than release binaries",
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


if __name__ == "__main__":
    unittest.main()
