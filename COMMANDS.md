# Atho Commands

This is the short operator guide for building, running, resetting, and testing Atho.

## Build

Check the full workspace:

```bash
cargo check
```

Run compile checks without executing tests:

```bash
cargo test --workspace --no-run
```

## Test

Run all workspace tests:

```bash
cargo test
```

Run only the main Atho crates:

```bash
cargo test -p atho-core -p atho-crypto -p atho-storage -p atho-wallet -p atho-p2p -p atho-rpc -p atho-node -p atho-qt
```

## Reset Dev State

Wipe the local blockchain database, wallet files, audit exports, and logs:

```bash
cargo run -p atho-node --bin athod -- dev wipe
```

Wipe everything and immediately restart the node from genesis:

```bash
cargo run -p atho-node --bin athod -- dev reset mainnet
```

You can also use `testnet` or `regnet`:

```bash
cargo run -p atho-node --bin athod -- dev reset testnet
cargo run -p atho-node --bin athod -- dev reset regnet
```

## Run The Node

Start the node with defaults:

```bash
cargo run -p atho-node --bin athod
```

Inspect the live node state from the terminal:

```bash
cargo run -p atho-node --bin athod -- status mainnet
```

Start the node on a specific network:

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin athod -- run regnet
```

Verify the hardcoded genesis and bootstrap state:

```bash
cargo run -p atho-node --bin athod -- verify mainnet
cargo run -p atho-node --bin athod -- verify testnet
```

## Run The Desktop Client

Start the Qt client on mainnet:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --rpc-addr 127.0.0.1:18443
```

Start the Qt client on testnet:

```bash
cargo run -p atho-qt --bin atho-qt -- --network testnet --rpc-addr 127.0.0.1:18444
```

The Qt client can start a local node automatically if the RPC port is not already reachable.

Run the client with an embedded node in one command:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

This is the one-command “run everything” path for local development. It starts the desktop client and embedded node together, and the client can still fall back to separate node or miner commands when you need them.

## Run The Miner

Bitcoin-style flow:

```bash
cargo run -p atho-node --bin athod -- run mainnet
cargo run -p atho-node --bin atho-mine -- --network mainnet --rpc-addr 127.0.0.1:18443
```

Use `testnet` for a faster development loop:

```bash
cargo run -p atho-node --bin athod -- run testnet
cargo run -p atho-node --bin atho-mine -- --network testnet --cores 8 --rpc-addr 127.0.0.1:18444
```

For the fastest local loop, use `regnet`:

```bash
cargo run -p atho-node --bin athod -- run regnet
cargo run -p atho-node --bin atho-mine -- --network regnet --rpc-addr 127.0.0.1:18445
```

## Wallet And Address Tools

Generate and inspect addresses:

```bash
cargo run -p atho-wallet --bin atho-address -- generate mainnet --seed-hex 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
cargo run -p atho-wallet --bin atho-address -- generate testnet --phrase "..." --count 2
cargo run -p atho-wallet --bin atho-address -- inspect A...
```

## Dev Exports

Export chain and transaction audit files:

```bash
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

Watch live dev logs:

```bash
cargo run -p atho-node --bin athod -- dev watch
```

The watch feed now tails `dev/logs/activity.log`, which combines node, miner, p2p, and Qt activity in one stream.

Durable chainstate lives in per-network LMDB environments under `dev/db/<network>/<dataset>/`. Each dataset has its own LMDB environment for `meta`, `blocks`, `transactions`, `utxos`, `peers`, and `addresses`. The mempool stays in RAM and is rebuilt on restart.

## Quick Flows

Build and test:

```bash
cargo check
cargo test
```

Reset and restart from genesis:

```bash
cargo run -p atho-node --bin athod -- dev reset mainnet
```

Run the full desktop setup:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

Run the wallet miner path:

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
