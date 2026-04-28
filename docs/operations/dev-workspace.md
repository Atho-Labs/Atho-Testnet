# Dev Workspace

The `dev/` tree is Atho’s sandbox state area. It is not part of the production data model and should not be treated as release content.

## Root Selection

By default, Atho uses:

- `./dev`

If `ATHO_DATA_DIR` is set, Atho uses that directory as the sandbox root instead.

Implemented in:

- `crates/atho-storage/src/path.rs`
- `crates/atho-node/src/dev.rs`

## Layout

Current subdirectories:

- `dev/logs/` for node, miner, P2P, and Qt logs
- `dev/db/<network>/` for the per-network LMDB environment
- `dev/chain/` for chain and transaction TSV exports
- `dev/wallet/` for disposable wallet files
- `dev/audit/` for audit exports
- `dev/quarantine/` for recovered or rejected local state

## Wipe And Reset

Wipe disposable state:

```bash
cargo run -p atho-node --bin athod -- dev wipe
```

Reset and restart from genesis:

```bash
cargo run -p atho-node --bin athod -- dev reset regnet
```

Why:

- fast sandbox resets are essential for lifecycle and regression testing

## Logs

Watch the unified activity stream:

```bash
cargo run -p atho-node --bin athod -- dev watch
```

Typical log domains include:

- `athod`
- `chain`
- `miner`
- `mempool`
- `p2p`
- `atho-qt`

## Chain Exports

Export human-readable chain and transaction views:

```bash
cargo run -p atho-node --bin athod -- dev export chain
cargo run -p atho-node --bin athod -- dev export tx
```

Why:

- TSV exports are useful for audits and debugging, even though they are not consensus-authoritative storage

## Quarantine

If persisted local state is detected as incomplete or corrupt, Atho may move files into:

- `dev/quarantine/`

and emit a recovery note.

Why:

- local failures should preserve evidence without trusting damaged state

## Shipping Rule

Do not treat `dev/` as release content. It is for local sandbox execution only.

## Related Documentation

- [Commands](commands.md)
- [Troubleshooting](troubleshooting.md)
- [Chainstate and Persistence](../storage/chainstate-and-persistence.md)
