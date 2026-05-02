# Atho Developer Onboarding

## Start Here

Primary crates:

- `atho-core`: canonical protocol types and consensus primitives
- `atho-storage`: LMDB-backed persistence and UTXO state
- `atho-node`: full-node runtime, mempool, mining, RPC service
- `atho-p2p`: wire protocol, relay, handshake, sync helpers
- `atho-wallet`: deterministic wallet model and encrypted persistence
- `atho-rpc`: command registry and request/response types
- `atho-qt`: desktop operator and wallet client

## Trust Boundaries

When reading the code, pay attention to these boundaries:

- peer traffic entering the node
- RPC commands entering the service layer
- wallet secrets entering UI or persistence flows
- blocks and transactions entering validation
- storage commits updating the chain tip and UTXO set

## Where To Read First

1. `crates/atho-core/src/block.rs`
2. `crates/atho-core/src/transaction.rs`
3. `crates/atho-storage/src/validation.rs`
4. `crates/atho-node/src/node.rs`
5. `crates/atho-node/src/service.rs`
6. `crates/atho-p2p/src/handshake.rs`
7. `crates/atho-wallet/src/wallet.rs`

## Reading Labels

Comments use a few strong labels:

- `CONSENSUS`
- `SECURITY`
- `STORAGE`
- `POLICY`
- `PERFORMANCE`
- `INVARIANT`

Treat those as review signposts.

## Safe Contribution Rule

If you are touching consensus, storage atomicity, wallet-secret handling, or P2P identity checks:

- document the assumption you are changing
- add or update tests
- keep public error messages safe
- do not change behavior casually
