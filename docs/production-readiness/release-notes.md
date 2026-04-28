# Release Notes

## Current Build Snapshot

- the Rust workspace builds and tests cleanly
- core protocol constants, genesis data, and consensus scaffolding are explicit
- Falcon-512 is the only active signature path
- chainstate persistence uses a single atomic LMDB environment per network
- local recovery quarantines incomplete or corrupt state instead of crashing through it
- node RPC exposes live status, block template, block submission, transaction submission, UTXO listing, and mempool information
- the Qt client follows the real backend tip and supports managed local-node startup
- the repository documentation is centralized under `docs/`

## Current Network State

- message framing, handshake, peer/session logic, and headers-first sync scaffolding exist
- DNS seeds are intentionally blank
- the live TCP runtime is not complete yet

## Shipping Caveats

- this is not a public production release
- packaging is local staging only
- release signing and distribution metadata are not complete
- public peer mesh deployment would be premature

## Related Documentation

- [Current Production Status](current-status.md)
- [Build and Packaging](../build-deployment/packaging.md)
