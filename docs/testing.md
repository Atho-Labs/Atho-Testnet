# Testing

Run tests from the repository root.

## Launcher Tests

```bash
python3 -m unittest tests.test_runtime_launcher
```

Expected result: Python reports all launcher tests passed.

## Rust Package Tests

```bash
cargo test -p atho-errors -p atho-core -p atho-crypto -p atho-storage -p atho-p2p -p atho-rpc -p atho-wallet -p atho-node
```

Expected result: Cargo finishes with passing tests for the core packages.

## Workspace Check

```bash
cargo check --workspace
```

Expected result: the workspace type-checks.

## Formatting

```bash
cargo fmt --check
```

Expected result: no formatting diff.

## Fuzz Target Build Check

```bash
cargo check --manifest-path fuzz/Cargo.toml --all-targets
```

Expected result: all fuzz targets compile.

## Regression Scripts

Network sync regression:

```bash
scripts/sync_regression_same_box.sh
```

macOS UI smoke test:

```bash
scripts/run_qt_ui_smoke.sh
```

The UI smoke test exits with `2` when the host is not macOS or Accessibility automation is unavailable.

## Benchmarks

Some crates include benchmark targets. Run them only when you need performance data:

```bash
cargo bench -p atho-core
cargo bench -p atho-crypto
cargo bench -p atho-wallet
```

## If Tests Fail

Read the first failing test and its error message. If the failure mentions old storage or data mismatch, wipe local chain data while preserving wallets using the command in [Commands](commands.md).
