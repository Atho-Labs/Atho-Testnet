# Atho Commands

## Tooling check

```bash
cargo --version
rustc --version
```

## Build and test the whole workspace

```bash
cargo check
cargo test
```

## Build and test the main layers

```bash
cargo check -p atho-core -p atho-crypto -p atho-storage -p atho-wallet -p atho-p2p -p atho-rpc -p atho-node -p atho-qt
cargo test -p atho-core -p atho-crypto -p atho-storage -p atho-wallet -p atho-p2p -p atho-rpc -p atho-node -p atho-qt
```

## Run the node

```bash
cargo run -p atho-node --bin athod
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin athod -- verify mainnet
```

## Wipe dev state

```bash
cargo run -p atho-node --bin athod -- dev wipe
```

## Watch live dev logs

```bash
cargo run -p atho-node --bin athod -- dev watch
```

## Export audit files

```bash
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

## Mine once in dev

```bash
cargo run -p atho-node --bin athod -- dev mine mainnet
cargo run -p atho-node --bin athod -- dev mine testnet
cargo run -p atho-node --bin athod -- dev mine regnet
```

## Mine with the standalone miner

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet --rpc-addr 127.0.0.1:18443
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin atho-mine -- --network testnet --cores 8 --rpc-addr 127.0.0.1:18444
```

## Generate and inspect addresses

```bash
cargo run -p atho-wallet --bin atho-address -- generate mainnet --seed-hex 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
cargo run -p atho-wallet --bin atho-address -- generate testnet --phrase "..." --count 2
cargo run -p atho-wallet --bin atho-address -- inspect A...
```

## Quick local loop

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet
cargo run -p atho-node --bin athod -- dev mine mainnet
cargo run -p atho-node --bin athod -- dev export tx
```

Run the same `mine` command with `testnet` to check the other hardcoded seed path.
Use `regnet` when you want the fastest local loop.

## Mainnet run bundle

```bash
cargo check
cargo test
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet
cargo run -p atho-qt --bin atho-qt
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

## Network quick starts

### Mainnet

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

### Testnet

```bash
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin atho-mine -- --network testnet
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

### Regnet

```bash
cargo run -p atho-node --bin athod -- run regnet
cargo run -p atho-node --bin atho-mine -- --network regnet
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

## Difficulty values

- SHA3-384 hash size: `96` hex characters
- Target size: `384` bits
- Standard transaction allocation: `9500 bps`
- Difficulty bounds are logged automatically during `dev mine`
- Mainnet/testnet/regnet initial targets are hardcoded in `consensus::pow`

## Run the desktop client

```bash
cargo run -p atho-qt --bin atho-qt
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:18443
```

## Run the hot-path benchmarks

```bash
cargo bench -p atho-core -p atho-wallet
```

## Package a release

```bash
bash scripts/package.sh
```

## Notes

- The desktop client is intentionally thin.
- The node owns the heavy work.
- Keep the lowest unresolved layer moving first.
