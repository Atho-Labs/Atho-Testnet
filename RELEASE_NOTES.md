# Atho Release Notes

## Current build

- Rust workspace builds cleanly
- Core protocol constants and consensus scaffolding are in place
- Wallet mnemonic, datafile persistence, and password encryption are in place
- Node RPC now exposes live node status, block template, block submission, transaction submission, UTXO listing, and mempool info
- Qt status now reflects real node data and includes a live activity feed from the shared dev log stream
- Qt supports an embedded one-command local-node mode via `--local-node`
- Supply constants now include the derived atom total for the full cap

## Shipping notes

- `athod` is the node daemon
- `atho-qt` is the desktop client
- Wallet datafiles use `.datafile`
- Password encryption is the default datafile path
- `athod status` prints the live RPC snapshot and recent activity lines
- `athod dev watch` tails the unified `dev/logs/activity.log` feed

## Known limitations

- P2P networking is still scaffold-level rather than a full peer mesh
- Packaging is local build and staging only
- Release signing and distribution metadata are not yet wired
