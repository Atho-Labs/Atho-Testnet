# Runtime Model

## Stable Operator Entry Points

Atho now has three primary operator-facing binaries:

1. `athod`
2. `atho-mine`
3. `atho-qt`

That is the intended steady-state launch model.

## `athod`

Purpose:

- full node
- validation daemon
- chainstate owner
- P2P participant
- RPC provider for the miner and desktop client

Recommended use:

- VPS deployment
- headless always-on node
- local backend for the desktop client

Default behavior:

- RPC binds to loopback only
- P2P binds publicly on the network port
- data, logs, wallet files, and quarantine output live under the Atho runtime root

Why:

- RPC should be private by default
- P2P should be reachable by default
- operators should not need to discover hidden environment variables to get a real node online

## `atho-mine`

Purpose:

- dedicated mining client
- requests a block template from a node
- solves it
- submits the solved block back to the node

Recommended use:

- separate process from the node
- same host as the node or a trusted LAN/VPN peer

Why:

- mining is operationally cleaner when it does not depend on the desktop client

## `atho-qt`

Purpose:

- desktop wallet and status client
- user-facing send/receive/history UI
- operator view of chain height, sync state, and activity

Supported modes:

1. attach to an existing RPC node
2. manage a local node with `--local-node`

Why:

- normal users should have a one-command desktop path
- operators should still be able to point the client at a separately managed node
- the settings page should expose local diagnostics similar in spirit to Bitcoin Core without turning public RPC into a topology leak

## Runtime Root

By default, Atho stores operator state under an OS-native application directory:

- macOS: `~/Library/Application Support/Atho`
- Linux: `${XDG_DATA_HOME:-~/.local/share}/Atho`
- Windows: `%APPDATA%\\Atho`

Override it explicitly with:

```bash
--data-dir /absolute/path
```

or:

```bash
ATHO_DATA_DIR=/absolute/path
```

That root contains:

- `db/`
- `logs/`
- `wallet/`
- `chain/`
- `audit/`
- `quarantine/`

## Network Bootstrap

DNS seeds are intentionally blank right now.

That means manual peers are still required for live sync:

```bash
athod --network mainnet --peer host1:56000 --peer host2:56000
```

The same applies to the desktop client when it manages a local node:

```bash
atho-qt --network mainnet --local-node --peer host1:56000
```

Why:

- the operator surface should already support the right manual bootstrap path before DNS seeds exist

## Related Documentation

- [Commands](commands.md)
- [Linux Quick Start](linux-quick-start.md)
- [macOS Quick Start](macos-quick-start.md)
- [Windows Quick Start](windows-quick-start.md)
- [VPS Full Node](vps-full-node.md)
- [RPC and Client Backend](../node-runtime/rpc-and-client.md)
