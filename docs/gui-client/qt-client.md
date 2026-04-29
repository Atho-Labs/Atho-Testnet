# Qt Client

## Purpose

The Qt client is Atho’s thin desktop wallet and operator UI.

It is designed to:

- talk to the backend over RPC
- remain responsive during wallet scans and mining actions
- reflect the real node tip, mempool, and wallet-visible state

It is not designed to own validation or chainstate logic.

## Major UI Areas

Implemented in:

- `crates/atho-qt/src/app.rs`
- `crates/atho-qt/src/app/pages/overview.rs`
- `crates/atho-qt/src/app/pages/send.rs`
- `crates/atho-qt/src/app/pages/receive.rs`
- `crates/atho-qt/src/app/pages/transactions.rs`
- `crates/atho-qt/src/app/pages/settings.rs`
- `crates/atho-qt/src/app/shell.rs`

Current major surfaces:

- welcome / startup dialogs
- overview dashboard
- send page
- receive page
- transaction history
- settings and mining controls
- settings/network diagnostics with peer and traffic visibility
- activity feed
- status bar with block height, best height, mempool count, and connectivity state

## Backend Connection

Implemented in:

- `crates/atho-qt/src/connection.rs`

Current behavior:

- runtime uses RPC for real node interaction
- `--local-node` manages a local node child process
- `--peer` and `--p2p-addr` can be passed through when DNS seeds are unavailable
- tests can still use an in-process backend hook for deterministic lifecycle coverage

Why:

- the same client should be able to run against a real backend and a deterministic sandbox without splitting into separate UI architectures

## Wallet UX Model

The client supports:

- create wallet
- import wallet
- open wallet
- receive-address generation
- send submission
- mining controls
- backup export
- passphrase change
- recovery phrase view

Why:

- a wallet client should manage wallet lifecycle end to end, but still delegate chain truth to the backend

## Sync-To-Tip Behavior

The client polls connection status and refreshes wallet-related views after backend changes.

The current hardening work specifically targeted:

- stale block height after accepted blocks
- stale managed local-node child startup behavior
- stale wallet scans when the RPC backend was not ready

Why:

- incorrect tip display is a product-level trust failure even if the backend is correct

## Network Diagnostics

The settings page now exposes a local operator diagnostics view with:

- connected peer count
- inbound/outbound split
- total bytes sent and received
- per-peer endpoint
- handshake-ready state
- peer-reported height
- protocol version and user agent
- recent receive time
- persisted peer-quality information when available

Why:

- Atho needs Bitcoin-Core-like operator visibility for a fast-block network
- the peer view belongs in an intentional diagnostics surface, not in public-facing RPC defaults

## Current Limitations

The biggest remaining GUI limitations are now:

- no full OS-level automation in CI
- the client module is still larger than ideal
- local-node bootstrap still depends on manual peers until DNS seeds exist

## Related Documentation

- [Qt Reference Map](qt-reference-map.md)
- [RPC and Client Backend](../node-runtime/rpc-and-client.md)
- [Wallet Model](../wallet/wallet-model.md)
