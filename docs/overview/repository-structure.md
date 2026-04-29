# Repository Structure

## Why The Repo Is Structured This Way

The repository is intentionally organized around subsystem ownership, not around delivery artifacts or UI concerns.

That means:

- the protocol core lives in dedicated crates
- storage, wallet, networking, RPC, node runtime, and GUI are separated
- operational docs are centralized under `docs/`
- repo-local sandbox artifacts can stay under `dev/` when explicitly requested
- staged release artifacts stay under `dist/releases/<version>/<platform>-<arch>/` with `dist/release/` as the current compatibility mirror, `dist/releases/<version>/<platform>-<arch>/installers/` as the direct-download installer staging area, `desktop/releases/<version>/<platform>-<arch>/` as the shareable release tree, and `desktop/latest/<platform>-<arch>/` as the active mirror
- direct installers are self-contained and validate their embedded payload checksums before install
- macOS bundles carry both the native `Atho Setup.app` installer and the `Atho.app` client bundle

This keeps the trusted core auditable and avoids mixing design notes, runtime state, and build outputs at repo root.

## Before Cleanup

The repo root had a strong crate layout already, but it was cluttered by:

- markdown files at root
- PDFs at root and under `crates/`
- a Qt reference map inside the GUI crate
- a vendor README inside the vendored Falcon tree
- release docs are staged into the release mirror under `dist/release/` and mirrored into `desktop/releases/...` and `desktop/latest/...`
- direct installer downloads are staged under `dist/releases/.../installers/` before GitHub publishing
- a dev workspace README inside `dev/`

That made documentation discovery harder and weakened the repo’s front-door quality.

## After Cleanup

The project now keeps documentation under one canonical top-level tree:

```text
docs/
  overview/
  architecture/
  protocol/
  consensus/
  storage/
  node-runtime/
  wallet/
  gui-client/
  crypto/
  operations/
  build-deployment/
  testing-audits/
  production-readiness/
  project/
  reference/
  whitepaper/
```

The root now keeps only:

- `README.md`
- workspace and build files
- code and asset directories
- local runtime/output directories such as `dev/`, `dist/`, and `target/`

## Final Top-Level Layout

```text
Cargo.toml
Cargo.lock
README.md
crates/
docs/
scripts/
dev/
dist/
target/
rust-toolchain.toml
```

## Workspace Ownership Model

### `crates/`

Owns the software implementation.

- `atho-core`: protocol constants, consensus rules, serialization, hashes, addresses, blocks, transactions
- `atho-crypto`: Falcon wrapper and secret-handling boundary
- `atho-storage`: chainstate, LMDB persistence, validation, UTXO management, recovery
- `atho-wallet`: HD wallet, mnemonic, keypool, encrypted datafile support
- `atho-p2p`: protocol messages, framing, peer/session logic, sync scaffolding
- `atho-rpc`: local RPC request/response and transport
- `atho-node`: node runtime, miner, mempool, service surface, orchestration
- `atho-qt`: desktop client and view orchestration
- `atho-installer`: native installer front-end and release setup wizard

### `docs/`

Owns all project documentation, reference materials, operational guides, readiness reports, and whitepaper content.

### `scripts/`

Owns project automation such as release staging.

### `dev/`

Owns optional repo-local sandbox state only:

- logs
- local databases
- exported chain TSVs
- wallet files
- quarantine output

It is not the default operator root and it is not a documentation source.

### `dist/`

Owns release staging output only.

It is not the canonical source of package documentation.

The canonical release builder writes versioned bundles into `dist/releases/<version>/<platform>-<arch>/` and refreshes `dist/release/` for local compatibility.

### `desktop/`

Owns the shareable desktop release tree.

It mirrors the built release bundle under `desktop/releases/<version>/<platform>-<arch>/`, maintains an active bundle mirror under `desktop/latest/<platform>-<arch>/`, carries a top-level `README.md` for the release share folder, and ships the platform-native `Atho Setup` installer front-end inside each active bundle.

## Moved Documentation Sources

Moved into `docs/`:

- root operator guide
- crypto migration report
- packaging notes
- release notes
- rebuild roadmap
- Qt reference map
- dev workspace guide
- planning/reference PDFs
- vendored FN-DSA README

Removed as duplicates:

- `dist/release/PACKAGING.md`
- `dist/release/RELEASE_NOTES.md`

Those files are now sourced from `docs/` during packaging instead of living as separate staging copies.

## Related Documentation

- [Documentation Index](../index.md)
- [Reference Materials](../reference/reference-materials.md)
- [Build and Packaging](../build-deployment/packaging.md)
