# Atho Command Permissions

The Atho command registry assigns each command a permission level.

This metadata is shared by:

- `atho-cli`
- the Qt debug console
- the node service router

## Permission Levels

- `PUBLIC_READ`
  - safe read-only commands
- `LOCAL_READ`
  - read-only but node-sensitive commands
- `LOCAL_WRITE`
  - changes local node behavior
- `WALLET_READ`
  - wallet reads
- `WALLET_WRITE`
  - wallet mutations
- `WALLET_SECRET`
  - wallet-secret or secret-using commands
- `NODE_ADMIN`
  - maintenance/admin actions
- `TEST_ONLY`
  - only valid on test networks
- `DANGEROUS_MAINNET_BLOCKED`
  - dangerous commands that should not run on mainnet by default

## Current State

The initial registry-backed command surface is intentionally conservative and read-focused.

Most currently implemented commands are:

- `PUBLIC_READ`
- `LOCAL_READ`

That is deliberate. The registry and confirmation system are in place before the broader destructive/admin command set is exposed.
