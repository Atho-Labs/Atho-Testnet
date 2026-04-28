# Qt Reference Map

This document records how Atho’s desktop client borrows the discipline of Bitcoin Qt without copying its codebase or carrying its architecture wholesale.

## Purpose

Bitcoin Qt is used as a behavioral and layout reference, not as a code source.

Atho keeps:

- shell-oriented navigation
- a thin wallet/operator client model
- explicit overview/send/receive/history/settings surfaces

Atho does not keep:

- Bitcoin branding
- Qt/C++ implementation structure
- RPC console and PSBT-specific UI
- coin-control complexity at the current stage

## Current Atho GUI Structure

Core modules:

- `crates/atho-qt/src/app.rs`
- `crates/atho-qt/src/app/models.rs`
- `crates/atho-qt/src/app/shell.rs`
- `crates/atho-qt/src/app/startup.rs`
- `crates/atho-qt/src/app/theme.rs`
- `crates/atho-qt/src/app/pages/*.rs`
- `crates/atho-qt/src/app/dialogs/*.rs`
- `crates/atho-qt/src/connection.rs`
- `crates/atho-qt/src/state.rs`
- `crates/atho-qt/src/view.rs`
- `crates/atho-qt/src/resources.rs`

## Reference Mapping

| Bitcoin-style responsibility | Atho owner |
| --- | --- |
| main shell | `app.rs`, `app/shell.rs` |
| overview page | `app/pages/overview.rs` |
| send flow | `app/pages/send.rs` |
| receive flow | `app/pages/receive.rs` |
| transaction history | `app/pages/transactions.rs` |
| settings/options | `app/pages/settings.rs` |
| wallet create/open/import | `app/dialogs/wallet.rs` |
| startup shell | `app/startup.rs`, `app/dialogs/welcome.rs` |
| client/backend bridge | `connection.rs`, `view.rs`, `state.rs` |

## What Atho Chose Differently

### Native Rust UI

Chosen because:

- it keeps the desktop client inside the Rust workspace
- it avoids a separate C++ GUI stack
- backend types and tests integrate more directly

### White Atho-branded visual language

Chosen because:

- Atho needs its own UI identity
- a direct Bitcoin visual clone would be the wrong product signal

### Backend-owned truth

Chosen because:

- the UI should display chainstate, not own it
- validation and mining must stay outside the GUI process model

## Current Limitation

The client is structurally closer to the intended architecture than the wallet-history model is. The main remaining gap is the lack of a canonical backend history API.

## Related Documentation

- [Qt Client](qt-client.md)
- [RPC and Client Backend](../node-runtime/rpc-and-client.md)
