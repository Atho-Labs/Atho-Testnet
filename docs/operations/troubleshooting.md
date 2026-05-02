# Troubleshooting

## Find The Runtime Root First

If you did not pass `--data-dir` and did not set `ATHO_DATA_DIR`, Atho uses the OS-native default root:

- macOS: `~/Library/Application Support/Atho`
- Linux: `${XDG_DATA_HOME:-~/.local/share}/Atho`
- Windows: `%APPDATA%\\Atho`

If you did override it, use that path instead.

## Full Node Startup Fails

Common causes:

- stale or corrupted local chainstate
- incomplete persisted block history
- wrong data root
- RPC or P2P port conflict

What to inspect:

1. `logs/athod.log`
2. `logs/activity.log`
3. `quarantine/RECOVERY.txt` if recovery was triggered

Useful reset:

```bash
cargo run -p atho-node --bin athod -- wipe --network regnet --data-dir /tmp/atho-dev --all
```

## Qt Shows `Connection refused`

Typical causes:

- the node is not running yet
- the managed child node failed to start
- the wrong RPC port was supplied
- the client is pointing at the wrong data root

Try the managed local-node path first:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

Then inspect:

- `logs/atho-qt.log`
- `logs/athod.log`
- `logs/athod-mainnet-stdio.log`

## Node Does Not Sync

Current likely cause:

- DNS seeds are still blank

Fix:

```bash
cargo run -p atho-node --bin athod -- --network mainnet
```

For the desktop client:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

What to check after that:

1. `athod status` shows `peer_count > 0`
2. `bytes_received` is increasing
3. the Qt settings page shows at least one ready peer

## Local State Rebuild Happened Unexpectedly

If Atho quarantines local state on startup, it found a recoverable local storage problem such as:

- schema mismatch
- incomplete block history
- corrupt snapshot data

Why:

- Atho prefers fail-closed rebuild behavior over trusting damaged local state

## Wrong Data Root

Use an explicit root:

```bash
cargo run -p atho-node --bin athod -- --network regnet --data-dir /absolute/path
```

or:

```bash
export ATHO_DATA_DIR=/absolute/path
```

## Public RPC Bind Is Rejected

This is expected by default.

RPC is intentionally local-only unless you opt in:

```bash
cargo run -p atho-node --bin athod -- --network mainnet --rpc-addr 0.0.0.0:9010 --public-rpc
```

Only do that if you have a real access-control plan.

## Release Staging Looks Stale

Rebuild the staged artifacts:

```bash
python3 scripts/release.py
```

Or on Windows:

```powershell
py -3 scripts\release.py
```

## Related Documentation

- [Commands](commands.md)
- [Runtime Model](runtime-model.md)
- [VPS Full Node](vps-full-node.md)
- [Build and Packaging](../build-deployment/packaging.md)
- [Current Production Status](../production-readiness/current-status.md)
