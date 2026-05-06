# Network and Identity

## Purpose

This document defines how Atho identifies networks, distinguishes peers, and prevents cross-network confusion.

## Supported Networks

Atho currently defines three networks:

| Network | Internal ID | CLI tag | P2P port | RPC port | Visible address prefix |
| --- | --- | --- | ---: | ---: | --- |
| Mainnet | `atho-mainnet` | `mainnet` | `56000` | `9010` | `A` |
| Testnet | `atho-testnet` | `testnet` | `9100` | `9110` | `T` |
| Regnet | `atho-regnet` | `regnet` | `9200` | `9210` | `R` |

Implemented in:

- `crates/atho-core/src/network.rs`

Why:

- a node must be able to reject the wrong chain, wrong peer, and wrong address family deterministically

## Consensus Identity

Each network also has a one-byte consensus identifier:

- mainnet: `1`
- testnet: `2`
- regnet: `3`

That identifier is carried inside block headers through `network_id`.

Why:

- it keeps chain identity part of canonical block data instead of a runtime-only side choice

## P2P Message Magic

The current wire magic values are:

| Network | Magic |
| --- | --- |
| Mainnet | `a7 54 48 01` |
| Testnet | `a7 54 48 02` |
| Regnet | `a7 54 48 03` |

Implemented in:

- `crates/atho-p2p/src/config.rs`

Why:

- a wire frame must be rejected before payload decoding if it belongs to the wrong network

## Genesis Identity

Each network has a hardcoded genesis block, genesis txid, genesis reward address, and genesis block hash.

Implemented in:

- `crates/atho-core/src/genesis.rs`

The current hardcoded block hashes are:

| Network | Genesis block hash |
| --- | --- |
| Mainnet | `00004eab876c38017f5f3f512e38ff9192106253912e0eefbe2eee4af732f7798e951f8ee2a3e2afe876927c2f21688f` |
| Testnet | `000083b1a17dc251043f4a7dd9d5981c35382e6d17bb6fb05eab2bb83dde5fe8a08dc766c9fb3ce9e1342f6f2238ac8a` |
| Regnet | `0000747cfb613e8e66e9cf9af1c6eb1c666f4879aa3a99fb90b5dc948c129587ed20112e5fd43d131c8eaedeca7d465a` |

Why:

- genesis is the root of chain identity
- every peer, wallet, and store needs an immutable origin point

Mainnet genesis must not change. Testnet may receive new genesis parameters during development testing, but those changes are testnet-only and do not apply to mainnet.

## Replay And Cross-Network Protection

Transactions, signatures, transaction PoW preimages, addresses, peers, storage, UTXOs, mempool entries, and blocks are network-scoped.

Consequences:

- a testnet transaction must not be valid on mainnet
- a mainnet transaction must not be valid on testnet
- testnet coins cannot be spent on mainnet
- mainnet coins cannot be spent on testnet
- wrong-network peers are rejected during handshake
- wrong-network addresses are rejected by wallet and send flows

Mainnet has strict production behavior:

- no faucet
- no automatic storage self-healing
- no testnet difficulty stall reset
- no genesis changes
- strict replay protection

Testnet is for development and testing:

- no faucet in the software
- testnet ATHO is distributed manually by the Atho founders or development team
- testnet may reset during development
- testnet may self-heal local testnet storage after configured network or storage changes
- testnet difficulty may reset to minimum after more than 10 minutes without a block
- testnet coins have no mainnet value

## Protocol Versioning

Current explicit version constants:

- protocol version: `1`
- ruleset version: `1`
- block version: `1`
- transaction version: `1`
- storage schema version: `3`

There is also an inactive placeholder for version `2` rules.

Implemented in:

- `crates/atho-core/src/consensus/rules.rs`

Why:

- future upgrades need a stable routing point from the first release, even if no upgrade is active yet

## DNS Seeds

Current state:

- mainnet DNS seeds: blank
- testnet DNS seeds: blank
- regnet DNS seeds: blank

Implemented in:

- `crates/atho-p2p/src/config.rs`

Why:

- seed infrastructure has intentionally been left empty until the live peer runtime is hardened enough to justify public bootstrap metadata

## Wire Frame Format

Current frame layout:

1. 4-byte network magic
2. 12-byte padded command
3. 4-byte payload length, little-endian
4. 4-byte checksum (`SHA3-256(payload)[0..4]`)
5. payload bytes

Implemented in:

- `crates/atho-p2p/src/codec.rs`

Why:

- it mirrors the operational simplicity of Bitcoin-style framing while staying consistent with Atho’s hash choices

## Current Limitations

- no DNS seed population yet
- live TCP peer runtime exists, but peer bootstrap is still manual
- no peer discovery persistence beyond local storage plumbing

## Related Documentation

- [Blocks and Consensus](blocks-and-consensus.md)
- [Versioning and Activations](../consensus/versioning-and-activations.md)
- [Node Runtime and P2P](../node-runtime/node-runtime-and-p2p.md)
