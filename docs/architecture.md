# Architecture

This is a high-level map of the Atho repo.

## Top-Level Layout

- `crates/`: Rust workspace crates
- `tests/`: Python launcher tests
- `scripts/`: launcher helpers plus smoke and regression scripts
- `fuzz/`: cargo-fuzz targets
- `docs/`: current documentation and reports
- `runmainnet.py`, `runtestnet.py`, and `runregnet.py`: simple desktop launch entrypoints

## Core Crates

- `atho-core`: blocks, transactions, network identity, consensus constants, hashing, signatures, and policy types
- `atho-crypto`: Falcon and secret-handling primitives
- `atho-storage`: chainstate, block storage, UTXO state, pruning, and storage recovery
- `atho-p2p`: P2P messages, handshakes, peer management, sync, address sharing, and relay
- `atho-rpc`: local RPC request/response types and command registry
- `atho-wallet`: wallet model, mnemonic handling, address derivation, and wallet data files
- `atho-node`: node service, runtime, mining, mempool, HTTP API, CLI, and daemon binaries
- `atho-qt`: desktop wallet UI
- `atho-gpu-native`: optional native GPU mining backend
- `atho-installer`: installer UI and packaging support

## Runtime Flow

`runmainnet.py`, `runtestnet.py`, and `runregnet.py` all call `scripts/runtime_launcher.py`, which builds if needed and then replaces itself with `atho-qt --local-node`.

`athod` loads node configuration, opens storage, starts the node service, starts P2P, starts local RPC, and starts the HTTP API when enabled.

`atho-qt --local-node` starts or connects to a managed local `athod` process over RPC.

`atho-mine` connects to `athod`, asks for a block template, solves Proof-of-Work, and submits the solved block.

`atho-cli` sends command invocations to the local RPC endpoint.

## Data Flow

P2P messages enter through `atho-p2p`, are handled by the node service, and become validated chainstate through `atho-storage`.

Wallet and CLI actions go through local RPC. HTTP API reads go through the node service and explorer index views instead of reading database files directly.

## Runtime Data

Runtime data is local and should not be committed:

- `db/`
- `chain/`
- `wallet/`
- `logs/`
- `audit/`
- `quarantine/`

Use separate data directories for mainnet, public testnet, and local regnet work.
