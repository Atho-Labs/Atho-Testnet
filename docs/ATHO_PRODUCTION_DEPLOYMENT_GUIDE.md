# Atho Production Deployment Guide

## Scope

This guide covers the current Atho node/operator deployment posture for controlled environments. It is written for extended testnet and pre-mainnet preparation, not for a claim of finished mainnet operations.

## Build

Debug build:

```bash
cargo build
```

Release build:

```bash
cargo build --release
```

If you need standalone binaries for operations, prefer release builds.

## Network Separation

Always set network intentionally:

- `mainnet`
- `testnet`
- `regnet`
- `prunetest`

Recommended rule: **one data directory per network**.

Example:

```bash
ATHO_DATA_DIR=/var/lib/atho/mainnet \
cargo run -p atho-node --bin athod --release -- --network mainnet
```

## Recommended Data Layout

Example filesystem layout:

```text
/var/lib/atho/
  mainnet/
    db/
    logs/
    wallet/
    audit/
    quarantine/
```

Wallet data should remain separate from ordinary chain wipes whenever possible.

## Safe Bind Defaults

### RPC

Keep RPC on loopback unless you intentionally opt in:

```bash
ATHO_RPC_ADDR=127.0.0.1:9010
```

### HTTP API

Keep the API on loopback unless it is fronted by a reverse proxy and external access controls:

```bash
ATHO_API_BIND=127.0.0.1
ATHO_API_PORT=8080
```

### P2P

Bind P2P explicitly:

```bash
ATHO_P2P_ADDR=0.0.0.0:56000
```

## Example Node Launch

```bash
ATHO_DATA_DIR=/var/lib/atho/mainnet \
ATHO_RPC_ADDR=127.0.0.1:9010 \
ATHO_P2P_ADDR=0.0.0.0:56000 \
ATHO_API_BIND=127.0.0.1 \
ATHO_API_PORT=8080 \
./target/release/athod --network mainnet
```

## Reverse Proxy / TLS Recommendation

If you expose the read-only HTTP API publicly:

1. keep Atho bound to loopback,
2. terminate TLS in a reverse proxy,
3. apply request limits,
4. cache stable block data where appropriate,
5. do **not** expose wallet-enabled broadcast routes publicly unless you have a deliberate gateway design.

## Firewall Recommendations

Allow only the ports you need:

- P2P port for the selected network
- reverse-proxy HTTPS port if exposing read-only API

Keep RPC and local HTTP API off the public interface unless there is an explicit reason.

## systemd Example

```ini
[Unit]
Description=Atho Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=atho
WorkingDirectory=/opt/atho
Environment=ATHO_DATA_DIR=/var/lib/atho/mainnet
Environment=ATHO_RPC_ADDR=127.0.0.1:9010
Environment=ATHO_P2P_ADDR=0.0.0.0:56000
Environment=ATHO_API_BIND=127.0.0.1
Environment=ATHO_API_PORT=8080
ExecStart=/opt/atho/target/release/athod --network mainnet
Restart=on-failure
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

## Health and Readiness

Check health:

```bash
curl http://127.0.0.1:8080/api/v1/health
```

Check status:

```bash
curl http://127.0.0.1:8080/api/v1/status
```

Useful things to watch:

- local height vs target height
- peer count
- topology health score
- mempool size
- API responsiveness

## Logging

Use structured service logs where possible and keep log retention bounded.

Minimum operator expectations:

- retain startup/shutdown logs
- retain sync warnings
- retain validation rejection summaries
- never log seeds or private keys

## Backups

Recommended backup priorities:

1. wallet data
2. operator config/service files
3. explorer/index snapshots if you rely on them operationally

Do **not** assume chain data backups are a substitute for wallet backups.

## Wipe / Recovery Guidance

When chain rules or historical validation policy change, prefer a clean wipe and resync over in-place compatibility logic.

Examples:

- legacy lock policy changes
- schema changes without a trusted migration path
- corrupted chainstate detection
- pre-mainnet genesis/history resets

## Upgrade Process

1. build new release
2. stop node cleanly
3. backup wallet and operator configs
4. deploy binaries
5. start node
6. verify health/status endpoints
7. verify network, peer count, and sync progress

## Rollback Process

1. stop node
2. restore previous binaries
3. restore only known-good configs
4. if schema or consensus changes are involved, assess whether data wipe/resync is required before restart

Do not blindly roll back across schema or consensus transitions without checking storage compatibility.

## Current Operational Blockers

- mainnet seed/bootstrap infrastructure is not provisioned in-repo yet
- wallet plaintext persistence is still possible when password is empty
- no built-in HTTP auth layer exists
- end-to-end performance benchmarking is not yet stable enough for strong release signoff

## Deployment Verdict

For local development, regnet, and extended testnet operations, Atho is deployable with the current launcher and node stack.

For mainnet-style production operations, treat this guide as a preparation document, not a final launch runbook.
