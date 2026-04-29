# Dev Workspace

The repo-local `dev/` tree is Atho’s disposable sandbox workspace.

It is not the default production/operator root anymore.

## How To Use It

Point Atho at it explicitly:

```bash
export ATHO_DATA_DIR="$PWD/dev"
```

or per command:

```bash
cargo run -p atho-node --bin athod -- --network regnet --data-dir "$PWD/dev"
```

Why:

- developers often want all node, wallet, audit, and log artifacts to stay inside the checkout
- operators usually want OS-native paths or an explicit service-owned directory such as `/var/lib/atho`

## Layout

When `ATHO_DATA_DIR` points at `./dev`, Atho uses:

- `dev/db/`
- `dev/logs/`
- `dev/wallet/`
- `dev/chain/`
- `dev/audit/`
- `dev/quarantine/`

## Wipe And Reset

Wipe disposable state:

```bash
cargo run -p atho-node --bin athod -- wipe --network regnet --data-dir "$PWD/dev" --all
```

Reset and restart from genesis:

```bash
cargo run -p atho-node --bin athod -- dev reset --network regnet --data-dir "$PWD/dev"
```

## Logs

Watch the unified activity stream:

```bash
cargo run -p atho-node --bin athod -- dev watch --data-dir "$PWD/dev"
```

Typical log domains:

- `athod`
- `chain`
- `miner`
- `mempool`
- `p2p`
- `atho-qt`

## Chain Exports

Export human-readable audit views:

```bash
cargo run -p atho-node --bin athod -- dev export chain --data-dir "$PWD/dev"
cargo run -p atho-node --bin athod -- dev export tx --data-dir "$PWD/dev"
```

Important:

- these TSVs are audit/debug artifacts
- they are not the canonical persisted source of chain truth

## Shipping Rule

Do not treat `dev/` as release content. It exists only for disposable local execution.

If you ever need to wipe mainnet data, the CLI now requires an explicit `--dangerously-allow-mainnet` flag. The normal sandbox workflow should stay on `regnet` or another disposable root.

## Related Documentation

- [Commands](commands.md)
- [Troubleshooting](troubleshooting.md)
- [Chainstate and Persistence](../storage/chainstate-and-persistence.md)
