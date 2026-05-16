# Troubleshooting

## Dependency Missing

Check the tools:

```bash
rustc --version
cargo --version
python3 --version
```

Install Rust, Cargo, or Python if any command is missing.

## Build Failure

Run the command again and read the first compiler error:

```bash
cargo build -p atho-node -p atho-qt
```

For GPU build failures, install OpenCL prerequisites or build without `--features gpu-native`.

## Port Already In Use

Default testnet ports are P2P `9100` and RPC `9110`.

Stop the old node, or override ports:

```bash
ATHO_RPC_ADDR=127.0.0.1:19110 ATHO_P2P_ADDR=0.0.0.0:19100 cargo run -p atho-node --bin athod -- --network testnet
```

## Database Locked

Only one node should use a data directory at a time. Stop the existing node or choose a different `--data-dir`.

## Node Will Not Start

Verify genesis/runtime wiring:

```bash
cargo run -p atho-node --bin athod -- verify --network testnet
```

If local data is stale after a consensus or network update, resync while preserving wallets:

```bash
cargo run -p atho-node --bin athod -- --network testnet --network-overrides-local
```

## Peer Connection Issues

Check status:

```bash
cargo run -p atho-node --bin athod -- status --network testnet
```

If needed, add an explicit peer:

```bash
cargo run -p atho-node --bin athod -- --network testnet --peer 162.222.206.163:9100
```

## Testnet Or Regnet Data Mismatch

Do not reuse one data directory across different networks. Use separate directories:

```bash
cargo run -p atho-node --bin athod -- --network regnet --data-dir /tmp/atho-regnet
```

## Wallet File Missing

Wallet files live under the wallet directory, not inside the chain database. If `ATHO_WALLET_DIR` was set previously, start with the same value.

Never delete wallet files unless you have a backup or seed phrase.

## API Not Responding

Make sure the node is running and the API is enabled:

```bash
curl http://127.0.0.1:8080/api/v1/health
```

If `ATHO_API_PORT` or `ATHO_API_BIND` is set, use that address instead.

## Mining Not Starting

Check node sync and mining safety:

```bash
cargo run -p atho-node --bin athod -- status --network testnet
```

Then run:

```bash
cargo run -p atho-node --bin atho-mine -- --network testnet --backend cpu
```

## Exit Code 101

Rust programs usually exit with `101` after a panic. Check `logs/activity.log` and the component log under your data directory. Then run:

```bash
cargo run -p atho-node --bin atho-cli -- --network testnet geterrorcodes
```

For one error code:

```bash
cargo run -p atho-node --bin atho-cli -- --network testnet geterrorcodes ATHO-DB-009
```

## Permission Errors

Use a data directory owned by your user:

```bash
python3 runmainnet.py --data-dir ~/.atho-mainnet
```
