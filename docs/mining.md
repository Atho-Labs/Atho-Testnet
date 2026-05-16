# Mining

Atho includes a standalone miner binary named `atho-mine`.

For the simplest path, start the desktop client with `runmainnet.py`, `runtestnet.py`, or `runregnet.py` and mine from the client after the managed local node finishes syncing. This page covers the standalone miner and GPU-related operator flow.

The miner connects to a running `athod` node over local RPC, requests a block template, solves Proof-of-Work, and submits the solved block back to the node.

## Start A Node

```bash
cargo run -p atho-node --bin athod -- --network testnet
```

Wait until the node reports that it is safe to mine.

```bash
cargo run -p atho-node --bin athod -- status --network testnet
```

## CPU Mining

```bash
cargo run -p atho-node --bin atho-mine -- --network testnet --backend cpu --loop
```

## Auto Backend

```bash
cargo run -p atho-node --bin atho-mine -- --network testnet --backend auto --loop
```

`auto` is the default backend. It can use GPU support when the binary was built with `gpu-native` and the host supports it.

## GPU Probe

```bash
cargo run -p atho-node --bin atho-mine -- --network testnet --probe-gpu
```

Expected result: the miner prints whether a GPU backend is usable and why if it is not.

## Useful Options

- `--cores N`: number of CPU worker cores
- `--rpc-addr HOST:PORT`: connect to a non-default node RPC address
- `--loop`: keep mining continuously
- `--retry-delay SECS`: wait before retrying failed continuous rounds

## Rewards

Mined blocks include a coinbase reward plus block fees. Coinbase outputs mature after 100 blocks.

The current emission schedule starts at 5 ATHO per block, halves every 1,260,000 blocks, and uses a 0.625 ATHO tail reward after the third halving.

## Stop Mining

Press `Ctrl-C` in the miner terminal.

## Common Errors

- RPC connection refused: start `athod` first or pass `--rpc-addr`.
- Node not synced: wait for sync to finish before mining.
- GPU unavailable: run `--probe-gpu`, install OpenCL prerequisites, or use `--backend cpu`.
- Block rejected: check node sync state and peer health with `athod status`.
