# Atho Commands

## Tooling check
This file is the short operator guide for building and running Atho.
Use the node first when you want a Bitcoin-style setup.
Use the Qt client when you want the desktop wallet to manage startup for you.

## 1. Install and check tooling

```bash
cargo --version
rustc --version
```

## Build and test the whole workspace
## 2. Build everything

Build the full workspace:

```bash
cargo check
```

Compile the full workspace without running tests:

```bash
cargo test --workspace --no-run
```

## 3. Test everything

Run all workspace tests:

```bash
cargo test
```

## Build and test the main layers
Run only the main Atho crates:

```bash
cargo check -p atho-core -p atho-crypto -p atho-storage -p atho-wallet -p atho-p2p -p atho-rpc -p atho-node -p atho-qt
cargo test -p atho-core -p atho-crypto -p atho-storage -p atho-wallet -p atho-p2p -p atho-rpc -p atho-node -p atho-qt
```

## Run the node
## 4. Run the full node

Start the node on the default network:

```bash
cargo run -p atho-node --bin athod
```

Start the node on a specific network:

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin athod -- verify mainnet
cargo run -p atho-node --bin athod -- run regnet
```

## Wipe dev state
Verify the hardcoded genesis and bootstrap state:

```bash
cargo run -p atho-node --bin athod -- dev wipe
cargo run -p atho-node --bin athod -- verify mainnet
cargo run -p atho-node --bin athod -- verify testnet
```

## Watch live dev logs
## 5. Run the desktop client

Start the Qt client:

```bash
cargo run -p atho-node --bin athod -- dev watch
cargo run -p atho-qt --bin atho-qt
```

## Export audit files
Start the Qt client on a specific network and RPC address:

```bash
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:18443
cargo run -p atho-qt --bin atho-qt -- --network testnet --rpc-addr 127.0.0.1:18444
```

## Mine once in dev
The Qt client will try to start the node automatically if the RPC endpoint is not already reachable.

## 6. Run the miner

Run the standalone miner against a live node:

```bash
cargo run -p atho-node --bin athod -- dev mine mainnet
cargo run -p atho-node --bin athod -- dev mine testnet
cargo run -p atho-node --bin athod -- dev mine regnet
cargo run -p atho-node --bin atho-mine -- --network mainnet --rpc-addr 127.0.0.1:18443
cargo run -p atho-node --bin atho-mine -- --network testnet --cores 8 --rpc-addr 127.0.0.1:18444
```

## Mine with the standalone miner
Bitcoin-style flow:

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet --rpc-addr 127.0.0.1:18443
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin atho-mine -- --network testnet --cores 8 --rpc-addr 127.0.0.1:18444
```

## Generate and inspect addresses
## 7. Wallet and address tools

Generate addresses:

```bash
cargo run -p atho-wallet --bin atho-address -- generate mainnet --seed-hex 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
cargo run -p atho-wallet --bin atho-address -- generate testnet --phrase "..." --count 2
cargo run -p atho-wallet --bin atho-address -- inspect A...
```

## Quick local loop
Inspect an address:

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet
cargo run -p atho-node --bin athod -- dev mine mainnet
cargo run -p atho-node --bin athod -- dev export tx
cargo run -p atho-wallet --bin atho-address -- inspect A...
```

Run the same `mine` command with `testnet` to check the other hardcoded seed path.
Use `regnet` when you want the fastest local loop.
## 8. Dev commands

## Mainnet run bundle
Wipe local dev state:

```bash
cargo check
cargo test
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet
cargo run -p atho-qt --bin atho-qt
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
cargo run -p atho-node --bin athod -- dev wipe
```

## Network quick starts
Watch dev logs:

### Mainnet
```bash
cargo run -p atho-node --bin athod -- dev watch
```

Export audit data:

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

### Testnet
Mine once in dev mode:

```bash
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin atho-mine -- --network testnet
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
cargo run -p atho-node --bin athod -- dev mine mainnet
cargo run -p atho-node --bin athod -- dev mine testnet
cargo run -p atho-node --bin athod -- dev mine regnet
```

### Regnet
## 9. Recommended quick flows

Local development loop:

```bash
cargo test
cargo run -p atho-node --bin athod -- run regnet
cargo run -p atho-node --bin atho-mine -- --network regnet
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
cargo run -p atho-node --bin atho-mine -- --network regnet --rpc-addr 127.0.0.1:18445
```

Desktop wallet flow:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:18443
```

## Difficulty values
## 10. Consensus notes

- SHA3-384 hash size: `96` hex characters
- Target size: `384` bits
- Standard transaction allocation: `9500 bps`
- Difficulty bounds are logged automatically during `dev mine`
- Mainnet/testnet/regnet initial targets are hardcoded in `consensus::pow`
- Mainnet, testnet, and regnet initial targets are hardcoded in `consensus::pow`

## Run the desktop client
## 11. Package a release

```bash
cargo run -p atho-qt --bin atho-qt
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:18443
bash scripts/package.sh
```

## Run the hot-path benchmarks
## 12. Short version

```bash
cargo bench -p atho-core -p atho-wallet
```

## Package a release
If you only remember three commands, use these:

```bash
bash scripts/package.sh
cargo check
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:18443
```

## Notes

- The desktop client is intentionally thin.
- The node owns the heavy work.
- Keep the lowest unresolved layer moving first.