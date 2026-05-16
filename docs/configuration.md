# Configuration

Atho currently uses CLI flags and environment variables. No static config file is required for normal operation.

## Networks

| Network | CLI value | P2P port | RPC port | Notes |
| --- | --- | ---: | ---: | --- |
| Mainnet | `mainnet` | `56000` | `9010` | Desktop launcher available through `runmainnet.py` |
| Testnet | `testnet` | `9100` | `9110` | Public testnet |
| Regnet | `regnet` or `regtest` | `9200` | `9210` | Local deterministic network |
| Prunetest | `prunetest` | `9300` | `9310` | Low-difficulty pruning/storage test network |

The simplest desktop launch flow is `runmainnet.py`. Use `runtestnet.py` for public testnet and `runregnet.py` for the local deterministic network.

## Data Paths

`ATHO_DATA_DIR` sets the runtime root for node databases, logs, chain exports, audit files, and quarantine data.

If `ATHO_DATA_DIR` is not set, Atho uses the platform data directory:

- Linux: `$XDG_DATA_HOME/Atho` or `$HOME/.local/share/Atho`
- macOS: `$HOME/Library/Application Support/Atho`
- Windows: `%APPDATA%\\Atho`

`ATHO_WALLET_DIR` can override the wallet directory. Wallet data is intentionally separate from ordinary chain wipes.

## Node Flags

```bash
cargo run -p atho-node --bin athod -- --network mainnet --data-dir ~/.atho-mainnet
```

Useful flags:

- `--network <mainnet|testnet|regnet|prunetest>`
- `--data-dir PATH`
- `--rpc-addr HOST:PORT`
- `--p2p-addr HOST:PORT`
- `--peer HOST:PORT`
- `--public-rpc`
- `--network-overrides-local`

RPC binds to loopback by default. Public RPC requires `--public-rpc` or `ATHO_RPC_ALLOW_PUBLIC=1`.

## P2P

`ATHO_P2P_ADDR` overrides the local P2P bind address.

`ATHO_P2P_PEERS` sets explicit outbound peers as a comma-separated list:

```bash
ATHO_P2P_PEERS="127.0.0.1:9200,127.0.0.1:9201"
```

If no explicit peers are set, testnet uses DNS seeds and built-in bootstrap peers.

## API

The HTTP API is enabled by default on `127.0.0.1:8080`.

Environment variables:

- `ATHO_API_ENABLED`
- `ATHO_API_BIND`
- `ATHO_API_PORT`
- `ATHO_API_PUBLIC_READ_ONLY`
- `ATHO_API_ADMIN_ENABLED`
- `ATHO_API_WALLET_ENABLED`
- `ATHO_API_MINING_ENABLED`
- `ATHO_API_MAX_RESPONSE_BYTES`
- `ATHO_API_ALLOWED_ORIGINS`
- `ATHO_API_RATE_LIMIT_ENABLED`
- `ATHO_API_RATE_LIMIT_RPM`
- `ATHO_API_HEAVY_RATE_LIMIT_RPM`
- `ATHO_EXPLORER_INDEX_ENABLED`
- `ATHO_EXPLORER_SNAPSHOT_ENABLED`

Defaults are local, read-only, rate-limited, and CORS-restricted to `https://atho.io` and `https://www.atho.io`.

## Mining

`atho-mine` accepts:

- `--network <mainnet|testnet|regnet|prunetest>`
- `--rpc-addr HOST:PORT`
- `--cores N`
- `--backend <cpu|gpu|auto>`
- `--probe-gpu`
- `--loop`
- `--retry-delay SECS`

`ATHO_NETWORK`, `ATHO_DATA_DIR`, and `ATHO_RPC_ADDR` are also honored by the miner.
