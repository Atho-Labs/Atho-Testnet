# Node Runtime and P2P

## Purpose

The node runtime composes validation, storage, mempool, mining, sync state, and RPC-facing service behavior into one operational backend.

Implemented in:

- `crates/atho-node/src/`
- `crates/atho-p2p/src/`

## Runtime Components

Primary node-side owners:

- `Node`: chainstate + mempool owner
- `NodeRuntime`: running/stopped lifecycle owner
- `NodeOrchestrator`: runtime + sync + rpc-server composition
- `NodeService` / `AthoSystem`: frontend-facing control surface
- `NodeSync`: network-message integration owner

Why:

- one composed runtime is easier to test than scattered free functions and global state

## P2P Foundation

The network layer currently includes:

- network parameters and limits
- message commands and typed payloads
- Bitcoin-style frame codec
- handshake state machine
- address manager
- ban score tracking
- connection/session manager
- headers-first sync state
- block/tx inventory relay logic

Implemented in:

- `crates/atho-p2p/src/config.rs`
- `crates/atho-p2p/src/protocol.rs`
- `crates/atho-p2p/src/codec.rs`
- `crates/atho-p2p/src/handshake.rs`
- `crates/atho-p2p/src/connection.rs`
- `crates/atho-p2p/src/sync.rs`
- `crates/atho-p2p/src/relay.rs`

## Message Surface

Current message set:

- `version`
- `verack`
- `ping`
- `pong`
- `getaddr`
- `addr`
- `inv`
- `getdata`
- `notfound`
- `getheaders`
- `headers`
- `block`
- `tx`
- `mempool`

Why:

- the message set is intentionally small and Bitcoin-shaped while the live network runtime is still being hardened

## Limits

Current P2P defaults include:

- max message size: `8 MiB`
- max addresses per message: `1,000`
- max inventory entries: `50,000`
- max headers per message: `2,000`
- max blocks in flight: `128`
- max requests per peer: `256`
- inbound peers: `32`
- outbound peers: `8`
- ban threshold: `100`

Implemented in:

- `crates/atho-p2p/src/config.rs`

Why:

- hard bounds need to exist before the public peer runtime is considered trustworthy

## Handshake Model

Handshake requires:

- matching network
- supported protocol version
- matching genesis hash
- matching active ruleset
- `version` then `verack`

Why:

- a peer should be rejected before sync work begins if it cannot validate the same chain

## Readiness and Buffer Semantics

These lifecycle states are intentionally separate:

- `handshake_ready` means the transport session is ready for normal peer messages.
- `headers_synced` means the peer header view has been integrated enough for header-first sync progress.
- neither of those means wallet spendability or mining readiness
- wallet spendability is owned by the local canonical chain height and wallet scan snapshot
- orphan/branch buffers are peer-local, transient, and non-persistent
- buffered branches are rechecked globally after each accepted block so a parent arriving on one peer can still unlock a child chain buffered from another peer
- pending compact-block reconstruction state is cleared on disconnect and on final accept/reject

Why:

- readiness bugs become operator bugs when a client or wallet assumes a peer is “fully ready” too early
- orphan buffers become correctness bugs if they outlive the chain context that made them meaningful

## Headers-First Sync

The current sync layer:

- primes a block locator from local history
- sends `getheaders`
- validates returned header linkage
- requests missing blocks by inventory

Why:

- header-first sync is the safest minimal foundation for future downloader work

## Current Network Limitations

The current implementation has a real live runtime, but it is not yet a finished production peer mesh.

Already exercised live:

- public P2P bind on a real VPS node
- wrong-network rejection on unsolicited internet traffic
- remote full-node reconnect after VPS restart
- one-block propagation from a remote peer into the VPS node
- operator-local peer and traffic diagnostics over a real WAN peer

Still incomplete:

- DNS seed bootstrap
- broader parallel downloader stress coverage
- compact block burst hardening
- peer-served snapshot sync
- long-run public peer mesh soak coverage

## Operator Diagnostics

Operator-local diagnostics currently surface:

- total connected peers
- inbound/outbound split
- per-peer endpoint
- per-peer handshake-ready state
- per-peer best height
- per-peer protocol metadata
- cumulative sent and received bytes

Implemented in:

- `crates/atho-node/src/service.rs`
- `crates/atho-node/src/tcp_p2p.rs`
- `crates/atho-node/src/bin/athod.rs`
- `crates/atho-qt/src/connection.rs`
- `crates/atho-qt/src/app/pages/settings.rs`

Why:

- operators need enough visibility to diagnose sync and propagation problems
- public RPC still stays loopback-only by default so these details are not exposed to the network by accident

That is an intentional documentation point, not a hidden omission.

## Related Documentation

- [RPC and Client Backend](rpc-and-client.md)
- [Mining and Mempool](mining-and-mempool.md)
- [Current Production Status](../production-readiness/current-status.md)
