# Atho Release Notes

## Current build

- Rust workspace rebuild in progress
- Core protocol constants and consensus scaffolding are in place
- Wallet mnemonic, datafile persistence, and password encryption are in place
- Thin node runtime, RPC, and Qt client are in place
- Qt shell is placeholder-level UI with Atho branding and Core-inspired layout

## Shipping notes

- `athod` is the node daemon
- `atho-qt` is the desktop client
- Wallet datafiles use `.datafile`
- Password encryption is the default datafile path

## Known limitations

- Qt data is currently stale/mock-backed in the UI shell
- Packaging is local build and staging only
- Release signing and distribution metadata are not yet wired

