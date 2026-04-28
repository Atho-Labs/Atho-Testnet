# Commands

This is the operator guide for building, testing, running, resetting, mining, and packaging Atho.

## Build

Check the full workspace:

```bash
cargo check
```

Compile tests without executing them:

```bash
cargo test --workspace --no-run
```

## Test

Run the full workspace:

```bash
cargo test
```

Run the main crates only:

```bash
cargo test -p atho-core -p atho-crypto -p atho-storage -p atho-wallet -p atho-p2p -p atho-rpc -p atho-node -p atho-qt
```

Run the adversarial campaign:

```bash
cargo run --release -p atho-node --bin atho-adversarial -- --cases 52000 --seed 12345
```

Run the targeted attack sweep:

```bash
cargo run -p atho-node --bin atho-attack -- --network regnet
```

## Node

Run the node on a specific network:

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin athod -- run regnet
```

Inspect node status:

```bash
cargo run -p atho-node --bin athod -- status mainnet
```

Verify hardcoded genesis/bootstrap state:

```bash
cargo run -p atho-node --bin athod -- verify mainnet
cargo run -p atho-node --bin athod -- verify testnet
cargo run -p atho-node --bin athod -- verify regnet
```

## Qt Client

Run against an explicit RPC address:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:9010
```

Run with a managed local node:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

Use `testnet` or `regnet` for faster sandbox work.

## Miner

Run the daemon and standalone miner:

```bash
cargo run -p atho-node --bin athod -- run regnet
cargo run -p atho-node --bin atho-mine -- --network regnet --rpc-addr 127.0.0.1:9210
```

## Wallet Tools

Generate or inspect addresses:

```bash
cargo run -p atho-wallet --bin atho-address -- generate mainnet --seed-hex 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
cargo run -p atho-wallet --bin atho-address -- generate testnet --phrase "..." --count 2
cargo run -p atho-wallet --bin atho-address -- inspect A...
```

## Dev Workspace

Wipe disposable local state:

```bash
cargo run -p atho-node --bin athod -- dev wipe
```

Reset a network from genesis:

```bash
cargo run -p atho-node --bin athod -- dev reset mainnet
cargo run -p atho-node --bin athod -- dev reset testnet
cargo run -p atho-node --bin athod -- dev reset regnet
```

Watch the shared activity log:

```bash
cargo run -p atho-node --bin athod -- dev watch
```

Export chain and transaction TSVs:

```bash
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

## Packaging

Stage local release artifacts:

```bash
./scripts/package.sh
```

## Environment

Override the sandbox root:

```bash
export ATHO_DATA_DIR=/absolute/path/to/sandbox
```

This controls where Atho writes local databases, logs, chain exports, wallet files, and quarantine output.

## Related Documentation

- [Dev Workspace](dev-workspace.md)
- [Build and Packaging](../build-deployment/packaging.md)
- [Troubleshooting](troubleshooting.md)

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet --rpc-addr 127.0.0.1:18443
```

## Consensus Notes

- All amounts are integer atoms.
- Minimum transaction fee policy is `1 atom/vbyte`.
- Witness input references are fixed-size and collision-resistant.
- The block pruning retention target is `70,000` blocks.
- Mainnet, testnet, and regnet initial targets are hardcoded in `consensus::pow`.
