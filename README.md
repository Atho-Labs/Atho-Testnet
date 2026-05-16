# Atho

Atho is a post-quantum payment-focused blockchain built around Proof-of-Work, UTXO accounting, Falcon-512 signatures, and SHA3 hashing.

This repository contains the desktop client, node, wallet, miner, API, and validation code for Atho.

## What Is Atho?

Atho is a Rust blockchain implementation for payment infrastructure. It uses a UTXO transaction model, Proof-of-Work block production, Falcon transaction signatures, a desktop wallet, local RPC, a read-only HTTP API, and P2P networking.

## Quick Build

Install Rust first if `cargo` is missing:

```bash
curl https://sh.rustup.rs -sSf | sh
```

Build the project:

```bash
cargo build
```

Launch mainnet:

```bash
python3 runmainnet.py
```

That launcher starts the desktop client and a managed local node. If binaries are missing or stale, it rebuilds them before handing off to `atho-qt`.

You can mine from the client after sync. Standalone miner, GPU build, and operator commands live in the docs instead of the main README.

## Other Networks

Public testnet:

```bash
python3 runtestnet.py
```

Local regnet:

```bash
python3 runregnet.py
```

## Power Users

For direct `cargo run` commands, node-only operation, standalone mining, GPU builds, CLI usage, and testing commands, use:

- [Setup Guide](docs/setup.md)
- [Commands](docs/commands.md)
- [Mining](docs/mining.md)
- [Testing](docs/testing.md)

## Documentation

- [White Paper (PDF)](ATHO_WHITE_PAPER.pdf)
- [Monetary Policy Attachment (PDF)](ATHO_MONETARY_POLICY_AND_150_YEAR_SUPPLY_SCHEDULE.pdf)
- [Setup Guide](docs/setup.md)
- [Commands](docs/commands.md)
- [Configuration](docs/configuration.md)
- [Mining](docs/mining.md)
- [Testing](docs/testing.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Architecture](docs/architecture.md)
- [Consensus](docs/consensus.md)
- [API](docs/api.md)
- [Production API Guide](docs/ATHO_PRODUCTION_API_GUIDE.md)
- [Production Deployment Guide](docs/ATHO_PRODUCTION_DEPLOYMENT_GUIDE.md)
- [Reports](docs/reports/README.md)

Historical release notes live in [docs/archive/release-notes.md](docs/archive/release-notes.md). Audit and engineering reports live in [docs/reports/README.md](docs/reports/README.md).

## Security Notice

Never commit private keys, wallet files, seed phrases, local databases, node identity files, API tokens, RPC cookies, `.env` files, production secrets, or generated chain data. The repo `.gitignore` excludes these local artifacts by default.

## License

Atho is licensed under the Apache License 2.0. See [LICENSE](LICENSE).
