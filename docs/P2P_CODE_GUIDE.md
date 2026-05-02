# Atho P2P Code Guide

## Scope

Networking code mainly lives in:

- `crates/atho-p2p/src/protocol.rs`
- `crates/atho-p2p/src/codec.rs`
- `crates/atho-p2p/src/connection.rs`
- `crates/atho-p2p/src/handshake.rs`
- `crates/atho-p2p/src/relay.rs`
- `crates/atho-p2p/src/address_manager.rs`

## Trust Boundary

Peers are untrusted by default.

Before a peer reaches relay or sync logic, the node must validate:

- network identity
- protocol compatibility
- genesis identity
- message framing
- checksum
- handshake order

## Handshake Rule

Only `version` and `verack` are legal before the handshake is ready.

Pending or half-open sessions should not be treated as stable connected peers in diagnostics or operator UI.

## Relay Rule

Relay code may forward inventory and requests, but it never bypasses block or transaction validation performed by the node.

## Diversity Rule

Address management and peer scoring should prevent a small number of operators or one subnet from dominating all outbound choices.
