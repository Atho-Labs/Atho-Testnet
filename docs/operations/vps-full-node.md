# VPS Full Node

## Goal

Run `athod` as an always-on headless full node on a VPS with a private RPC surface and a public P2P surface.

## Recommended Model

- `athod` runs under a service manager
- RPC stays on loopback
- P2P listens on the public node port
- the data directory is explicit
- upgrades are performed with clean stop / replace / start cycles
- SSH host identity is verified before first deployment

## Build

Build the release binaries:

```bash
cargo build --release -p atho-node
```

The node binary is:

```bash
target/release/athod
```

## Recommended Runtime Root

Use an explicit path instead of relying on the default:

```bash
/var/lib/atho
```

## Start Command

Example:

```bash
./athod --network mainnet --data-dir /var/lib/atho
```

Because DNS seeds are still blank, add at least one manual peer for live bootstrap:

```bash
./athod --network mainnet --data-dir /var/lib/atho --peer 198.51.100.10:56000
```

## Network Exposure

Recommended defaults:

- expose the P2P port to the internet
- keep RPC on `127.0.0.1`

Do not expose RPC publicly unless you have an intentional reverse-proxy or access-control design. Atho now refuses public RPC binds unless the operator explicitly opts in.

## Example `systemd` Unit

```ini
[Unit]
Description=Atho full node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=atho
WorkingDirectory=/opt/atho
ExecStart=/opt/atho/athod --network mainnet --data-dir /var/lib/atho --peer 198.51.100.10:56000
Restart=always
RestartSec=5
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

Packaged example:

- `dist/release/athod.service.example`

## Status Checks

Use:

```bash
./athod status --network mainnet
```

or, with an explicit RPC port:

```bash
./athod status --rpc-addr 127.0.0.1:9010
```

## Logs And State

Under the runtime root:

- `logs/athod.log`
- `logs/activity.log`
- `db/`
- `quarantine/`

## Upgrade Workflow

1. stop the service
2. replace the binary
3. start the service
4. run `athod status --network mainnet`
5. inspect logs if the service fails

## Current Caveats

- DNS seeds are still blank, so peer bootstrap is manual
- public-node hardening is improved, but the network layer still needs more long-run soak coverage
- snapshot sync is not yet a peer-served protocol
- deployment to `74.208.219.116` is blocked from this shell until the changed SSH host key is verified out of band

## Related Documentation

- [Runtime Model](runtime-model.md)
- [Commands](commands.md)
- [Troubleshooting](troubleshooting.md)
- [Launch Checklist](launch-checklist.md)
