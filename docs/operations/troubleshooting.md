# Troubleshooting

## Local Node Startup Fails

Common causes:

- stale or corrupted local chainstate
- incomplete persisted block history
- wrong sandbox root
- RPC port conflict

What to check:

1. `dev/logs/athod.log`
2. `dev/logs/activity.log`
3. `dev/quarantine/RECOVERY.txt` if recovery was triggered

Useful reset:

```bash
cargo run -p atho-node --bin athod -- dev wipe
```

## Qt Shows `Connection refused`

Typical causes:

- the node is not running yet
- the managed child node failed to start
- the wrong RPC port was supplied

Use the managed local-node path first:

```bash
cargo run -p atho-qt --bin atho-qt -- --network mainnet --local-node
```

Then inspect:

- `dev/logs/atho-qt.log`
- `dev/logs/athod.log`
- `dev/logs/athod-mainnet-stdio.log`

## Local State Rebuild Happened Unexpectedly

If Atho quarantines local state on startup, that means it detected a recoverable local storage problem such as:

- schema mismatch
- incomplete block history
- corrupt snapshot data

Why this happens:

- Atho prefers fail-closed plus rebuild over silently trusting broken local state

## Wrong Data Root

If state is appearing in the wrong location, set:

```bash
export ATHO_DATA_DIR=/absolute/path/to/sandbox
```

Current limitation:

- without `ATHO_DATA_DIR`, the default root still depends on the working directory

## Release Staging Looks Stale

Regenerate staged artifacts with:

```bash
./scripts/package.sh
```

The package script now sources release notes and packaging docs from `docs/`, not from duplicate files under `dist/release/`.

## Related Documentation

- [Dev Workspace](dev-workspace.md)
- [Build and Packaging](../build-deployment/packaging.md)
- [Current Production Status](../production-readiness/current-status.md)
