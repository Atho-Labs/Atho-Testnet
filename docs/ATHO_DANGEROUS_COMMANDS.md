# Atho Dangerous Commands

This document tracks command classes that require explicit confirmation or mainnet blocking.

## Current State

The initial Atho command registry and Qt debug console ship with a read-focused surface.

That means:

- the confirmation model exists
- the command metadata supports dangerous/test-only/mainnet-blocked flags
- the currently exposed command set does not yet include the larger destructive admin surface

## Why This Matters

The order is intentional:

1. build the shared registry
2. build the CLI and Qt console on top of it
3. enforce permissions and confirmations
4. expand into destructive/admin commands only after the control plane is stable

## Future Dangerous/Admin Classes

These are the command categories that should stay behind confirmation and stronger gating as the surface expands:

- pruning
- repair
- reindex
- migration
- chain invalidation/reconsideration
- rollback/reorg simulation
- wallet-secret export/import
- test-only fault injection

## Operator Guidance

For now, treat the current command system as:

- production-safe for local read/debug usage
- not yet a full public/admin RPC surface
